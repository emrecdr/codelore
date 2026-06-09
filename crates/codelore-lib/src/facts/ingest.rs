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
        // Load the team-map ONCE before walk starts. Auto-discover
        // `.codelore-teams` in the repo root if `--team-map-file` wasn't
        // passed. Empty map means the projection is a no-op (the apply
        // helper passes through unmatched authors).
        let team_map_path = opts
            .team_map_file
            .clone()
            .or_else(|| crate::identity::discover_team_map(&opts.repo_path));
        let team_map = crate::identity::team_map::load(team_map_path.as_deref())?;
        // Same shape as team_map: load `.codelorebots` ONCE here so the
        // ingest_loop can route bot detection AND ai_attribution through
        // the user-extensible patterns. Before this, the production code
        // called the free `identity::is_bot` / `identity::ai_attribution`
        // functions which only see DEFAULT_BOT_PATTERNS — `.codelorebots`
        // was loaded into a struct that nothing in production ever
        // consulted (extension hook was dead code).
        let bot_patterns = crate::identity::BotPatterns::from_repo(&opts.repo_path);

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
            let stats = ingest_loop(self, rx, &team_map, &bot_patterns)?;

            producer.join().expect("producer panicked")?;
            Ok(stats)
        })?;

        // Plan 3: populate entities + complexity_metrics from the working tree at HEAD.
        // Plan 4 will replace this with proper gix blob reading.
        self.ingest_complexity_at_head(opts)?;

        // Plan 4: populate the Kamei 14-feature change vector via SQL UPDATE pass.
        // Kamei history (ndev, nuc, age) joins changes-to-changes on path;
        // without lineage, renamed files lose all their pre-rename history.
        // Routing through `changes_lineage` merges histories under the canonical
        // post-rename name.
        crate::kamei::enrich(self, opts.use_canonical_lineage)?;

        // Plan 8 §4: populate the `clones` table at HEAD so the
        // `clone-coupling` analysis (§6) can JOIN against it. Honors
        // `opts.min_clone_node_count` and `opts.exclude_patterns` (set via
        // `--exclude` + `.codeloreignore`).
        let clones_n = self.populate_clones_at_head(opts)?;

        // PAR-7: architectural grouping. After ingest, rewrite the
        // `changes.path` column to logical group names per --group-file.
        // Runs last so the rewrite sees all change rows from every commit.
        if let Some(group_file) = opts.group_file.as_ref() {
            let group_map = super::groups::GroupMap::from_file(group_file, opts.strict_grouping)
                .map_err(|e| CodeLoreError::Analysis(format!("--group-file: {e}")))?;
            apply_grouping(self, &group_map)?;
        }

        let mut stats = stats;
        stats.clones_ingested = clones_n;
        Ok(stats)
    }

    fn ingest_complexity_at_head(&self, opts: &Options) -> Result<()> {
        use crate::complexity::{Tier1Language, compute_for_file};
        use rayon::prelude::*;

        let live_paths = query_live_paths(self)?;
        // One HEAD rev shared by every row written in this pass — semantically
        // correct (the complexity was measured against THIS commit, not against
        // a per-path lex-max-of-SHA approximation).
        let head_rev = current_head_rev(self)?;

        // ── Parallel pass ────────────────────────────────────────────────────────
        // Each worker thread reads the file, dispatches the tree-sitter parser,
        // and de-duplicates entities.  `map_init(|| (), ...)` matches the plan's
        // design: no per-thread state is needed because `Parser::new()` is ~3 µs
        // and tree-sitter 0.25.x is both `Send + Sync`.
        // Per-file failures are logged via `tracing::warn!` but do NOT abort the
        // parallel scan; they surface as `None` entries that the serial drain skips.
        //
        // Return type: Vec<Option<(String, Vec<ComplexityEntity>)>>
        //   - None  → file skipped (no Tier-1 lang, unreadable, or parse error)
        //   - Some  → (path, deduped_entities)
        let batches: Vec<Option<(String, Vec<crate::complexity::ComplexityEntity>)>> = live_paths
            .into_par_iter()
            .map_init(
                || (),
                |_state, path| {
                    let lang = Tier1Language::from_path(&path)?;
                    let full_path = opts.repo_path.join(&path);
                    let source = match std::fs::read(&full_path) {
                        Ok(b) => b,
                        Err(e) => {
                            // ENOENT here means the path was tracked at HEAD per
                            // git history but the file is missing on disk — most
                            // commonly because the user has uncommitted deletions
                            // in the working tree. That's user-action territory,
                            // not a codelore bug, so log at debug not warn to
                            // avoid drowning the console on dirty trees.
                            if e.kind() == std::io::ErrorKind::NotFound {
                                tracing::debug!(
                                    "complexity: file missing from working tree {path} \
                                     (likely an uncommitted deletion; skipping)",
                                );
                            } else {
                                tracing::warn!("complexity: cannot read {path}: {e}");
                            }
                            return None;
                        }
                    };
                    let entities = match compute_for_file(&full_path, &source, lang) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("complexity: parse error {path}: {e}");
                            return None;
                        }
                    };
                    let deduped = dedup_entities(entities);
                    Some((path, deduped))
                },
            )
            .collect();

        // ── Serial drain ─────────────────────────────────────────────────────────
        // `duckdb::Appender<'conn>` is `!Send + !Sync`; it MUST live on the same
        // thread that owns the `Connection`.  We create the Appenders here (on the
        // calling/connection-owning thread) and feed them from the collected Vec.
        let mut entities_app = self
            .conn()
            .appender("entities")
            .map_err(|e| CodeLoreError::Analysis(format!("appender entities: {e}")))?;
        let mut metrics_app = self
            .conn()
            .appender("complexity_metrics")
            .map_err(|e| CodeLoreError::Analysis(format!("appender complexity_metrics: {e}")))?;

        for batch in batches {
            let Some((path, entities)) = batch else {
                continue;
            };
            for ent in &entities {
                append_entity_row(&mut entities_app, &path, ent, &head_rev)?;
                append_metric_row(&mut metrics_app, &path, ent, &head_rev)?;
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
        use rayon::prelude::*;
        use walkdir::WalkDir;

        // Compile the exclude globset once (.git / target / node_modules are
        // hard-skipped always; the user globs and .codeloreignore are added).
        let exclude_set = build_clones_exclude_set(opts)?;

        let head_rev = current_head_rev(self)?;

        // Phase 1 (serial, fast): walk the working tree, filter to Tier-1 files
        // that survive the exclude globset, capture (absolute path, POSIX rel, lang).
        let candidates: Vec<(std::path::PathBuf, String, CloneLanguage)> =
            WalkDir::new(&opts.repo_path)
                .into_iter()
                .filter_map(std::result::Result::ok)
                .filter_map(|entry| {
                    if !entry.file_type().is_file() {
                        return None;
                    }
                    let path = entry.path();
                    if path.components().any(|c| {
                        matches!(
                            c.as_os_str().to_str(),
                            Some(".git" | "target" | "node_modules")
                        )
                    }) {
                        return None;
                    }
                    let lang = CloneLanguage::from_path(path)?;
                    // Normalise to POSIX `/` so `clones.path` matches `changes.path`
                    // (git always emits `/`). See `crate::paths::to_posix`.
                    let rel = path
                        .strip_prefix(&opts.repo_path)
                        .map_or_else(|_| crate::paths::to_posix(path), crate::paths::to_posix);
                    if exclude_set.is_match(&rel) {
                        return None;
                    }
                    Some((path.to_path_buf(), rel, lang))
                })
                .collect();

        // Phase 2 (parallel): read each file + run tree-sitter fingerprinting on
        // the rayon pool. Mirrors the complexity pass above. Unreadable files
        // silently yield no fingerprints; extract errors short-circuit the pass
        // via `collect::<Result<_>>`.
        let per_file: Vec<Vec<_>> = candidates
            .into_par_iter()
            .map(|(full_path, rel, lang)| -> Result<Vec<_>> {
                let Ok(code) = std::fs::read(&full_path) else {
                    return Ok(Vec::new());
                };
                extract_functions(&rel, &code, lang)
                    .map_err(|e| CodeLoreError::Analysis(format!("clones: extract {rel}: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let all_fns: Vec<_> = per_file.into_iter().flatten().collect();

        let groups = group_clones(all_fns, opts.min_clone_node_count);
        if groups.is_empty() {
            return Ok(0);
        }

        // Second pass: INSERT one row per family member into `clones`.
        //
        // `clones` has PRIMARY KEY (clone_group_id, path, function, start_line).
        // In real source that's unique. In minified/bundled output (e.g. webpack
        // and Vite ship files like `dist/assets/index-<hash>.js`) many function
        // expressions are packed onto one line and tree-sitter walks them out
        // with the same `(function_name, start_line)`, so two members of the
        // same group collide on the PK and the appender flush fails — which
        // aborts the entire ingest, even when the user only asked for a non-
        // clones analysis. Dedup in-memory by the PK columns; log the count of
        // collapsed duplicates so the signal isn't silent. Users who want the
        // un-collapsed view should add minified bundles to `.codeloreignore`.
        let mut app = self
            .conn()
            .appender("clones")
            .map_err(|e| CodeLoreError::Analysis(format!("appender clones: {e}")))?;
        let mut n = 0usize;
        let mut collapsed = 0usize;
        let mut seen: std::collections::HashSet<(i64, String, String, u32)> =
            std::collections::HashSet::new();
        for group in groups {
            let clone_group_id = i64::from(group.clone_group_id);
            for member in &group.members {
                use duckdb::params;
                let key = (
                    clone_group_id,
                    member.path.clone(),
                    member.function_name.clone(),
                    member.start_line,
                );
                if !seen.insert(key) {
                    collapsed += 1;
                    continue;
                }
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
        if collapsed > 0 {
            tracing::info!(
                "clones: collapsed {collapsed} duplicate member(s) sharing \
                 (group, path, function, start_line) — typically minified/bundled \
                 output; add such files to .codeloreignore to skip them",
            );
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
/// Returns the set of paths that exist at HEAD per the recorded change history.
///
/// "Live at HEAD" = the path's MOST RECENT change event in the ingested history
/// is not a deletion. Earlier implementations used
/// `WHERE change_type != 'deleted' GROUP BY path` which is wrong: it includes
/// any path that has ever had a non-deleted change, even if a later deletion
/// removed it. That produced spurious "cannot read" warnings on every working
/// tree where files had been deleted in commits on the current branch.
///
/// Ordering uses `commits.date` (chronology) — NOT `changes.rev`, which is a
/// SHA-1 hex string whose lex order has no relationship to commit order.
/// `commits.date DESC` first, then `changes.rev DESC` as a deterministic
/// tiebreaker when two events share a timestamp.
fn query_live_paths(db: &FactsDb) -> Result<Vec<String>> {
    let sql = "
        WITH latest_per_path AS (
            SELECT
                c.path,
                c.change_type,
                ROW_NUMBER() OVER (
                    PARTITION BY c.path
                    ORDER BY commits.date DESC, c.rev DESC
                ) AS rn
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
        )
        SELECT path
        FROM latest_per_path
        WHERE rn = 1 AND change_type != 'deleted'
        ORDER BY path
    ";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare path query: {e}")))?;
    stmt.query_map([], |r| r.get::<_, String>(0))
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

fn ingest_loop(
    db: &FactsDb,
    rx: crossbeam_channel::Receiver<CommitEvent>,
    team_map: &identity::TeamMap,
    bot_patterns: &identity::BotPatterns,
) -> Result<IngestStats> {
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

    for mut event in rx {
        // Resolve canonical author, then apply the team-map projection.
        // Order matters: mailmap normalization happens at walk time (gix);
        // bot detection happens in parallel here; team-map is the LAST
        // projection so it takes the already-normalized identity. The
        // result lands on `event.canonical_author` so `append_commit`
        // (which reads that field) picks it up too.
        let pre_team_canonical = event
            .canonical_author
            .as_deref()
            .unwrap_or(&event.author_email);
        let canonical = identity::apply_team_map(team_map, pre_team_canonical).to_string();
        if !team_map.is_empty() {
            event.canonical_author = Some(canonical.clone());
        }
        let bot = bot_patterns.is_bot(&event.author_email, &event.author_name);
        // Re-classify ai_attribution through the user-extensible patterns
        // too. The walker computes it with the defaults-only free function
        // (no repo_path in scope there); we overwrite here so projects with
        // internal bot accounts get them tagged as `ai-authored` instead
        // of being miscounted as human contributors.
        event.ai_attribution = Some(
            identity::ai_attribution_with(
                bot_patterns,
                &event.author_email,
                &event.author_name,
                &event.message,
            )
            .to_string(),
        );
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

/// Materialize a session-temporary `changes_bucketed` table that collapses
/// commits within the same `date_trunc(<bucket>, commit.date)` window into
/// a single logical "commit" per (bucket, path). Coupling-family analyses
/// (`coupling`, `clone-coupling` indirectly, `soc`) query this table when
/// `opts.time_bucket.is_some()` so commits landed across the same
/// day/week/month count as one for pair-counting purposes.
///
/// The bucket key (a date string like `2024-01-15` for day-buckets) takes
/// the place of `rev`. Within a bucket, `loc_added` and `loc_deleted` are
/// summed; `change_type` collapses to MAX (string-alphabetical max — close
/// enough since the bucketed-table is consumed only by analyses that care
/// about pair counts, not type details).
///
/// Idempotent: `CREATE OR REPLACE TEMPORARY TABLE`. Call once per analysis
/// run after the main ingest finishes. Cheap — single SQL pass over
/// `changes` JOIN `commits`.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_changes_bucketed(
    db: &super::FactsDb,
    bucket: super::super::options::TimeBucket,
    use_lineage: bool,
) -> Result<()> {
    use duckdb::params;
    let unit = bucket.as_sql_unit();
    // When canonical lineage is on, bucket on top of the lineage-resolved
    // view so rename ancestry survives the temporal collapse. Without this,
    // a renamed file's pre- and post-rename commits aggregate under
    // separate paths inside the same bucket — the composition bug §2.1
    // flagged by the 2026-06-09 deep-analysis report.
    let src = if use_lineage {
        materialize_changes_lineage(db)?;
        "changes_lineage"
    } else {
        "changes"
    };
    // unit + src come from closed enums / a small set of literals so the
    // format! interpolation is safe (no user-controlled input).
    let sql = format!(
        "CREATE OR REPLACE TEMPORARY TABLE changes_bucketed AS \
         SELECT \
             CAST(date_trunc('{unit}', m.date) AS TEXT) AS rev, \
             c.path, \
             MAX(c.change_type) AS change_type, \
             ANY_VALUE(c.rename_from) AS rename_from, \
             ANY_VALUE(c.similarity) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM {src} c \
         INNER JOIN commits m ON m.rev = c.rev \
         GROUP BY date_trunc('{unit}', m.date), c.path"
    );
    db.conn().execute(&sql, params![]).map_err(|e| {
        CodeLoreError::Analysis(format!("materialize changes_bucketed ({unit}): {e}"))
    })?;
    tracing::info!("materialized changes_bucketed at {unit} granularity (lineage={use_lineage})");
    Ok(())
}

/// Materialize the rename-lineage map as a temporary table.
///
/// Walks `changes.rename_from` recursively to find the LATEST canonical path
/// for every path that has ever been renamed. `path_lineage` is a small
/// `(old_path, canonical_path)` lookup table — typically a handful of rows
/// even on large repos (renames are rare). Cycles are bounded by `depth < 50`,
/// far above any realistic rename chain; the `ROW_NUMBER() ... ORDER BY depth
/// DESC` deterministically picks the last name in the chain. Rows where
/// `old_path == canonical_path` are filtered out — the join then has nothing
/// to merge for files that have never been renamed (the common case).
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_path_lineage(db: &super::FactsDb) -> Result<()> {
    use duckdb::params;
    let sql = "CREATE OR REPLACE TEMPORARY TABLE path_lineage AS
        WITH RECURSIVE lineage(orig, current, depth) AS (
            SELECT DISTINCT rename_from, path, 1
            FROM changes
            WHERE rename_from IS NOT NULL
            UNION ALL
            SELECT l.orig, c.path, l.depth + 1
            FROM lineage l
            INNER JOIN changes c ON c.rename_from = l.current
            WHERE l.depth < 50
        )
        SELECT orig AS old_path, current AS canonical_path
        FROM (
            SELECT orig, current, depth,
                   ROW_NUMBER() OVER (
                       PARTITION BY orig
                       -- Secondary order on `current` so ties at the same
                       -- depth (possible when a non-linear rename graph
                       -- reaches the same intermediate via multiple paths)
                       -- break deterministically and run-to-run output stays
                       -- byte-equal.
                       ORDER BY depth DESC, current ASC
                   ) AS rn
            FROM lineage
        )
        WHERE rn = 1 AND orig != current";
    db.conn()
        .execute(sql, params![])
        .map_err(|e| CodeLoreError::Analysis(format!("materialize path_lineage: {e}")))?;
    Ok(())
}

/// Materialize `changes_lineage` — `changes` with `path` canonicalized via
/// the rename-lineage map. Idempotent (`CREATE OR REPLACE`). Calls
/// [`materialize_path_lineage`] first so the lookup table is in scope.
///
/// Analyses that opt into rename-aware aggregation should `FROM
/// changes_lineage` instead of `FROM changes`. The schema is identical
/// modulo `path` being the post-rename canonical name.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn materialize_changes_lineage(db: &super::FactsDb) -> Result<()> {
    use duckdb::params;
    materialize_path_lineage(db)?;
    let sql = "CREATE OR REPLACE TEMPORARY TABLE changes_lineage AS
        SELECT
            c.rev,
            COALESCE(pl.canonical_path, c.path) AS path,
            c.change_type,
            c.rename_from,
            c.similarity,
            c.loc_added,
            c.loc_deleted
        FROM changes c
        LEFT JOIN path_lineage pl ON pl.old_path = c.path";
    db.conn()
        .execute(sql, params![])
        .map_err(|e| CodeLoreError::Analysis(format!("materialize changes_lineage: {e}")))?;
    // Without explicit indexes the downstream GROUP BY path / GROUP BY rev
    // aggregations fall to full scans even when the base `changes` table
    // has covering indexes. Mirror them on the temp table.
    for stmt in [
        "CREATE INDEX IF NOT EXISTS idx_changes_lineage_path ON changes_lineage(path)",
        "CREATE INDEX IF NOT EXISTS idx_changes_lineage_rev ON changes_lineage(rev)",
    ] {
        db.conn()
            .execute(stmt, params![])
            .map_err(|e| CodeLoreError::Analysis(format!("index changes_lineage: {e}")))?;
    }
    tracing::info!("materialized changes_lineage with canonical rename paths");
    Ok(())
}

/// Apply architectural grouping in-place on the `changes` table. Called by
/// [`FactsDb::ingest`] after raw ingest if `opts.group_file.is_some()`.
///
/// Implementation:
///   1. Build a `(raw_path → group_name)` mapping in Rust from every distinct
///      path in `changes` against the [`GroupMap`].
///   2. Insert the mapping into a temporary table.
///   3. Build a `changes_grouped` temporary table that JOINs against the
///      mapping, replaces the path with the group name (or keeps raw under
///      non-strict mode for unmapped paths), and aggregates `loc_added` /
///      `loc_deleted` per `(rev, new_path)`.
///   4. Replace `changes` content with the aggregated rows.
///   5. Remove `hunks` rows whose `(rev, path)` no longer exists in
///      `changes` (strict mode + dropped paths produces orphans otherwise).
///
/// Strict vs non-strict:
/// - Strict (`opts.strict_grouping = true` / code-maat default): rows whose
///   path doesn't match any rule are DROPPED.
/// - Non-strict (`CodeLore` default): unmapped rows keep their raw path.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn apply_grouping(db: &super::FactsDb, group_map: &super::GroupMap) -> Result<()> {
    use duckdb::params;

    let conn = db.conn();

    // Step 1: enumerate distinct paths in `changes` and pre-compute the
    // mapping in Rust. Doing the regex matching here avoids embedding the
    // GroupMap rules into SQL (DuckDB has regexp_matches but doesn't
    // support fancy-regex's lookaround that some code-maat fixtures need).
    let distinct_paths: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT path FROM changes")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare distinct paths: {e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CodeLoreError::Analysis(format!("query distinct paths: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect distinct paths: {e}")))?
    };

    // Step 2: build the mapping table. Mapped paths get the group name; in
    // non-strict mode, unmapped paths get the raw path back; in strict mode,
    // unmapped paths get a sentinel NULL group_name that the WHERE filter
    // in step 3 uses to drop them.
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _grouping_v1 (\
             raw_path TEXT PRIMARY KEY, group_name TEXT\
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("create _grouping_v1: {e}")))?;

    {
        let mut stmt = conn
            .prepare("INSERT INTO _grouping_v1 (raw_path, group_name) VALUES (?, ?)")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare grouping insert: {e}")))?;
        for path in &distinct_paths {
            let mapped: Option<&str> = group_map.map_entity(path);
            // Strict: NULL → row gets dropped in step 3.
            // Non-strict: fall back to raw path → row keeps its original path.
            let effective: Option<&str> = if group_map.strict {
                mapped
            } else {
                Some(mapped.unwrap_or(path.as_str()))
            };
            stmt.execute(params![path, effective])
                .map_err(|e| CodeLoreError::Analysis(format!("grouping insert row: {e}")))?;
        }
    }

    // Step 3+4: rewrite `changes` in place. CREATE OR REPLACE TEMPORARY
    // TABLE _changes_grouped + DELETE+INSERT pattern keeps the FK from
    // hunks happy in step 5 (no period where changes is empty AND the
    // grouped data isn't yet ready to INSERT).
    conn.execute(
        "CREATE OR REPLACE TEMPORARY TABLE _changes_grouped AS \
         SELECT \
             c.rev, \
             g.group_name AS path, \
             MAX(c.change_type) AS change_type, \
             ANY_VALUE(c.rename_from) AS rename_from, \
             ANY_VALUE(c.similarity) AS similarity, \
             SUM(c.loc_added)::INTEGER AS loc_added, \
             SUM(c.loc_deleted)::INTEGER AS loc_deleted \
         FROM changes c \
         INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
         WHERE g.group_name IS NOT NULL \
         GROUP BY c.rev, g.group_name",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("build _changes_grouped: {e}")))?;

    // Step 5: clean hunks for paths that won't survive the swap. Do BEFORE
    // the changes-swap so the FK from hunks → changes never sees a missing
    // referent. Hunks aren't path-rewritten (line-range semantics don't
    // translate to group level), so they get dropped for any path that
    // collapsed or got removed.
    conn.execute(
        "DELETE FROM hunks WHERE (rev, path) NOT IN (\
             SELECT c.rev, g.group_name FROM changes c \
             INNER JOIN _grouping_v1 g ON g.raw_path = c.path \
             WHERE g.group_name = c.path\
         )",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("clean hunks: {e}")))?;

    // Swap the data in changes
    conn.execute("DELETE FROM changes", [])
        .map_err(|e| CodeLoreError::Analysis(format!("clear changes: {e}")))?;
    conn.execute(
        "INSERT INTO changes (rev, path, change_type, rename_from, similarity, loc_added, loc_deleted) \
         SELECT rev, path, change_type, rename_from, similarity, loc_added, loc_deleted \
         FROM _changes_grouped",
        [],
    )
    .map_err(|e| CodeLoreError::Analysis(format!("swap changes: {e}")))?;

    tracing::info!(
        "grouping applied: {} rules, {} distinct paths, strict={}",
        group_map.rules.len(),
        distinct_paths.len(),
        group_map.strict
    );

    Ok(())
}
