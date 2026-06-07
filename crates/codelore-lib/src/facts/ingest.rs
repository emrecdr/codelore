//! Stream commits from a Repo into `DuckDB` via the
//! N gix workers → bounded crossbeam channel → 1 Appender thread pattern.
//! See spec §3.2.2.
//!
//! Threading model: `duckdb::Connection` contains `RefCell`, which is `!Sync`.
//! Therefore `&FactsDb` is `!Send` and the Appender must run on the thread that
//! owns the `FactsDb`. We flip the design: the **producer** runs in a scoped
//! spawned thread (Repo is `Send + Sync`) and the **consumer** (Appender) runs
//! on the calling thread.

use crossbeam_channel::bounded;
use duckdb::Appender;

use super::FactsDb;
use crate::identity;
use crate::repo::Repo;
use crate::{ChangeType, CodeLoreError, CommitEvent, Options, Result};

const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Default)]
pub struct IngestStats {
    pub commits_ingested: usize,
    pub changes_ingested: usize,
    /// Plan 8 §4: number of clone-family member rows inserted into the
    /// `clones` table during HEAD-time extraction. `0` if no clones found
    /// or no Tier-1 source files exist.
    pub clones_ingested: usize,
}

impl FactsDb {
    /// # Panics
    ///
    /// Panics if the producer thread panics (internal logic error, not expected in normal use).
    pub fn ingest<R: Repo>(&self, repo: &R, opts: &Options) -> Result<IngestStats> {
        // Plan 1: single producer gix walker → bounded channel → Appender on calling thread.
        // Plan 4 will fan out N producers.
        let (tx, rx) = bounded::<CommitEvent>(CHANNEL_CAPACITY);

        let stats = std::thread::scope(|s| -> Result<IngestStats> {
            // Producer: runs in a scoped thread. Repo: Send + Sync, opts borrows fine.
            let producer = s.spawn(|| -> Result<()> {
                let walk = repo.walk_commits(opts)?;
                for event in walk {
                    let event = event?;
                    tx.send(event)
                        .map_err(|e| CodeLoreError::Analysis(format!("channel send: {e}")))?;
                }
                drop(tx); // Signals consumer to stop.
                Ok(())
            });

            // Consumer: runs on the calling thread — FactsDb / Connection stays single-threaded.
            let stats = ingest_loop(self, rx)?;

            producer.join().expect("producer panicked")?;
            Ok(stats)
        })?;

        // Plan 3: populate entities + complexity_metrics from the working tree at HEAD.
        // Plan 4 will replace this with proper gix blob reading.
        self.ingest_complexity_at_head(opts)?;

        // Plan 4: populate the Kamei 14-feature change vector via SQL UPDATE pass.
        crate::kamei::enrich(self)?;

        // Plan 8 §4: populate the `clones` table at HEAD so the
        // `clone-coupling` analysis (§6) can JOIN against it. Honors
        // `opts.min_clone_node_count` and `opts.exclude_patterns` (set via
        // `--exclude` + `.codeloreignore`).
        let clones_n = self.populate_clones_at_head(opts)?;

        let mut stats = stats;
        stats.clones_ingested = clones_n;
        Ok(stats)
    }

    fn ingest_complexity_at_head(&self, opts: &Options) -> Result<()> {
        use crate::complexity::{Tier1Language, compute_for_file};

        let path_rows = query_live_paths(self)?;

        let mut entities_app = self
            .conn()
            .appender("entities")
            .map_err(|e| CodeLoreError::Analysis(format!("appender entities: {e}")))?;
        let mut metrics_app = self
            .conn()
            .appender("complexity_metrics")
            .map_err(|e| CodeLoreError::Analysis(format!("appender complexity_metrics: {e}")))?;

        for (path, head_rev) in path_rows {
            let Some(lang) = Tier1Language::from_path(&path) else {
                continue;
            };
            let full_path = opts.repo_path.join(&path);
            let Ok(source) = std::fs::read(&full_path) else {
                continue;
            };
            let Ok(entities) = compute_for_file(&full_path, &source, lang) else {
                continue; // skip unparseable files; Plan 4 may log
            };

            // De-duplicate by name (codelore-rca may emit duplicate names such as
            // "<anonymous>"); keep first occurrence, preserve order.
            let deduped = dedup_entities(entities);
            for ent in deduped {
                append_entity_row(&mut entities_app, &path, &ent, &head_rev)?;
                append_metric_row(&mut metrics_app, &path, &ent, &head_rev)?;
            }
        }

        entities_app
            .flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush entities: {e}")))?;
        metrics_app
            .flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush metrics: {e}")))?;
        Ok(())
    }

    /// Plan 8 §4 Task 15: walk the working tree at HEAD, fingerprint every
    /// function in every Tier-1 file, group by structural digest, and INSERT
    /// one row per clone-family member into the `clones` table. Returns the
    /// number of rows inserted (0 if no clones found or no Tier-1 sources).
    ///
    /// Honors `opts.min_clone_node_count` (default 30) and `opts.exclude_patterns`
    /// (built from `--exclude` flags + `.codeloreignore`).
    fn populate_clones_at_head(&self, opts: &Options) -> Result<usize> {
        use crate::clones::{CloneLanguage, extract_functions, group_clones};
        use walkdir::WalkDir;

        // Compile the exclude globset once (.git / target / node_modules are
        // hard-skipped always; the user globs and .codeloreignore are added).
        let exclude_set = build_clones_exclude_set(opts)?;

        let head_rev = current_head_rev(self)?;

        // First pass: walk the working tree, collect FunctionFingerprints.
        let mut all_fns = Vec::new();
        for entry in WalkDir::new(&opts.repo_path)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some(".git" | "target" | "node_modules")
                )
            }) {
                continue;
            }
            let Some(lang) = CloneLanguage::from_path(path) else {
                continue;
            };
            let rel = path.strip_prefix(&opts.repo_path).map_or_else(
                |_| path.to_string_lossy().into_owned(),
                |p| p.to_string_lossy().into_owned(),
            );
            if exclude_set.is_match(&rel) {
                continue;
            }
            let Ok(code) = std::fs::read(path) else {
                continue;
            };
            let fns = extract_functions(&rel, &code, lang)
                .map_err(|e| CodeLoreError::Analysis(format!("clones: extract {rel}: {e}")))?;
            all_fns.extend(fns);
        }

        let groups = group_clones(all_fns, opts.min_clone_node_count);
        if groups.is_empty() {
            return Ok(0);
        }

        // Second pass: INSERT one row per family member into `clones`.
        let mut app = self
            .conn()
            .appender("clones")
            .map_err(|e| CodeLoreError::Analysis(format!("appender clones: {e}")))?;
        let mut n = 0usize;
        for group in groups {
            let clone_group_id = i64::from(group.clone_group_id);
            for member in &group.members {
                use duckdb::params;
                let fp_bytes: Vec<u8> = member.fingerprint.digest.to_vec();
                app.append_row(params![
                    clone_group_id,
                    fp_bytes,
                    head_rev,
                    member.path,
                    member.function_name,
                    i32::try_from(member.start_line).unwrap_or(i32::MAX),
                    i32::try_from(member.end_line).unwrap_or(i32::MAX),
                    i32::try_from(member.fingerprint.node_count).unwrap_or(i32::MAX),
                    1.0_f64, // Type 1 + Type 2 → exact match; T3 MinHash lands in v1.x
                ])
                .map_err(|e| CodeLoreError::Analysis(format!("append clone row: {e}")))?;
                n += 1;
            }
        }
        app.flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush clones appender: {e}")))?;
        Ok(n)
    }
}

/// Build the exclude `GlobSet` mirroring `analyses::clones::run_clones`'s
/// behavior so the two paths produce the same filter set.
fn build_clones_exclude_set(opts: &Options) -> Result<globset::GlobSet> {
    let mut b = globset::GlobSetBuilder::new();
    for pat in &opts.exclude_patterns {
        let g = globset::Glob::new(pat)
            .map_err(|e| CodeLoreError::Analysis(format!("clones: --exclude {pat:?}: {e}")))?;
        b.add(g);
    }
    let ignore_path = opts.repo_path.join(".codeloreignore");
    if let Ok(contents) = std::fs::read_to_string(&ignore_path) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let g = globset::Glob::new(line).map_err(|e| {
                CodeLoreError::Analysis(format!(".codeloreignore line {line:?}: {e}"))
            })?;
            b.add(g);
        }
    }
    b.build()
        .map_err(|e| CodeLoreError::Analysis(format!("clones: build globset: {e}")))
}

/// Resolve HEAD's rev from the commits table (most recent commit by date).
/// Used to stamp the `rev` column on inserted clone rows.
fn current_head_rev(db: &FactsDb) -> Result<String> {
    let sql = "SELECT rev FROM commits ORDER BY date DESC, rev DESC LIMIT 1";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare head rev: {e}")))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| CodeLoreError::Analysis(format!("query head rev: {e}")))?;
    if let Some(row) = rows
        .next()
        .map_err(|e| CodeLoreError::Analysis(format!("head rev row: {e}")))?
    {
        Ok(row.get::<_, String>(0).unwrap_or_default())
    } else {
        Ok(String::new())
    }
}

/// Query all paths from `changes` that are not deleted, with their most recent rev.
fn query_live_paths(db: &FactsDb) -> Result<Vec<(String, String)>> {
    let sql = "
        SELECT changes.path, MAX(changes.rev) AS head_rev
        FROM changes
        WHERE changes.change_type != 'deleted'
        GROUP BY changes.path
    ";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare path query: {e}")))?;
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query paths: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect paths: {e}")))
}

/// De-duplicate a list of entities by name, preserving first-occurrence order.
fn dedup_entities(
    entities: Vec<crate::complexity::ComplexityEntity>,
) -> Vec<crate::complexity::ComplexityEntity> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(entities.len());
    for ent in entities {
        if seen.insert(ent.name.clone()) {
            out.push(ent);
        }
    }
    out
}

/// Safely clamp a finite non-negative f64 to i32 range.
fn f64_to_i32_clamped(v: f64) -> i32 {
    if v.is_finite() && v >= 0.0 {
        #[allow(clippy::cast_possible_truncation)]
        let clamped = v.round().min(f64::from(i32::MAX)) as i32;
        clamped
    } else {
        0
    }
}

fn append_entity_row(
    app: &mut duckdb::Appender<'_>,
    path: &str,
    ent: &crate::complexity::ComplexityEntity,
    rev: &str,
) -> Result<()> {
    use duckdb::params;
    app.append_row(params![
        path,
        ent.name,
        ent.kind,
        i32::try_from(ent.start_line).unwrap_or(i32::MAX),
        i32::try_from(ent.end_line).unwrap_or(i32::MAX),
        rev,
        rev,
    ])
    .map_err(|e| CodeLoreError::Analysis(format!("append entity: {e}")))
}

fn append_metric_row(
    app: &mut duckdb::Appender<'_>,
    path: &str,
    ent: &crate::complexity::ComplexityEntity,
    rev: &str,
) -> Result<()> {
    use duckdb::params;
    app.append_row(params![
        path,
        ent.name,
        rev,
        f64_to_i32_clamped(ent.cyclomatic),
        f64_to_i32_clamped(ent.cognitive),
        ent.halstead_volume,
        ent.halstead_difficulty,
        ent.halstead_effort,
        ent.mi,
        i32::try_from(ent.nom).unwrap_or(i32::MAX),
        i32::try_from(ent.nexits).unwrap_or(i32::MAX),
        i32::try_from(ent.loc).unwrap_or(i32::MAX),
        i32::try_from(ent.sloc).unwrap_or(i32::MAX),
        i32::try_from(ent.max_nesting).unwrap_or(i32::MAX),
        ent.mean_nesting,
        ent.sd_nesting,
        i32::try_from(ent.total_nesting).unwrap_or(i32::MAX),
    ])
    .map_err(|e| CodeLoreError::Analysis(format!("append metric: {e}")))
}

fn ingest_loop(db: &FactsDb, rx: crossbeam_channel::Receiver<CommitEvent>) -> Result<IngestStats> {
    use std::collections::HashMap;

    let mut stats = IngestStats::default();
    let mut commits_app = db
        .conn()
        .appender("commits")
        .map_err(|e| CodeLoreError::Analysis(format!("appender commits: {e}")))?;
    let mut changes_app = db
        .conn()
        .appender("changes")
        .map_err(|e| CodeLoreError::Analysis(format!("appender changes: {e}")))?;

    // Collect unique (raw_email, canonical, is_bot) for deferred author_aliases insert.
    let mut alias_map: HashMap<String, (String, bool)> = HashMap::new();

    for event in rx {
        // Track alias mapping for this author.
        let canonical = event
            .canonical_author
            .as_deref()
            .unwrap_or(&event.author_email)
            .to_string();
        let bot = identity::is_bot(&event.author_email, &event.author_name);
        alias_map
            .entry(event.author_email.clone())
            .or_insert((canonical, bot));

        append_commit(&mut commits_app, &event)?;
        for ch in &event.changes {
            append_change(&mut changes_app, &event.rev, ch)?;
            stats.changes_ingested += 1;
        }
        stats.commits_ingested += 1;
    }
    commits_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush commits: {e}")))?;
    changes_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush changes: {e}")))?;

    // Populate author_aliases table.
    let mut aliases_app = db
        .conn()
        .appender("author_aliases")
        .map_err(|e| CodeLoreError::Analysis(format!("appender author_aliases: {e}")))?;
    for (raw_email, (canonical, is_bot)) in &alias_map {
        use duckdb::params;
        aliases_app
            .append_row(params![raw_email, canonical, is_bot])
            .map_err(|e| CodeLoreError::Analysis(format!("append author_alias: {e}")))?;
    }
    aliases_app
        .flush()
        .map_err(|e| CodeLoreError::Analysis(format!("flush author_aliases: {e}")))?;

    Ok(stats)
}

/// Format a `time::Date` as `YYYY-MM-DD` without the `formatting` feature.
fn format_date(date: time::Date) -> String {
    let y = date.year();
    let m = date.month() as u8;
    let d = date.day();
    format!("{y:04}-{m:02}-{d:02}")
}

fn append_commit(app: &mut Appender<'_>, e: &CommitEvent) -> Result<()> {
    use duckdb::params;
    let date_str = format_date(e.date);
    let canonical = e
        .canonical_author
        .as_deref()
        .unwrap_or(&e.author_email)
        .to_string();
    let ai_attr = e.ai_attribution.as_deref().map(str::to_string);
    app.append_row(params![
        e.rev,
        e.author_email,
        e.author_name,
        e.committer_email,
        canonical,
        ai_attr,
        date_str,
        e.message,
        e.parents.len() > 1,
        i32::try_from(e.parents.len()).unwrap_or(i32::MAX),
        // Kamei nulls — Plan 4 fills
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<bool>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
        Option::<i32>::None,
        Option::<f64>::None,
        Option::<i32>::None,
    ])
    .map_err(|err| CodeLoreError::Analysis(format!("append commit: {err}")))?;
    Ok(())
}

fn append_change(app: &mut Appender<'_>, rev: &str, ch: &crate::FileChange) -> Result<()> {
    use duckdb::params;
    let (type_str, rename_from, similarity) = match &ch.change_type {
        ChangeType::Added => ("added", None, None),
        ChangeType::Modified => ("modified", None, None),
        ChangeType::Deleted => ("deleted", None, None),
        ChangeType::Renamed { from, similarity } => {
            ("renamed", Some(from.as_str()), Some(i32::from(*similarity)))
        }
        ChangeType::Copied { from, similarity } => {
            ("copied", Some(from.as_str()), Some(i32::from(*similarity)))
        }
        ChangeType::BinaryOrUnknown => ("binary", None, None),
    };
    app.append_row(params![
        rev,
        ch.path,
        type_str,
        rename_from,
        similarity,
        i32::try_from(ch.loc_added).unwrap_or(i32::MAX),
        i32::try_from(ch.loc_deleted).unwrap_or(i32::MAX),
    ])
    .map_err(|err| CodeLoreError::Analysis(format!("append change: {err}")))?;
    Ok(())
}
