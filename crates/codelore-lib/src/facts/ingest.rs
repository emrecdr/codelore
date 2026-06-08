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
            let stats = ingest_loop(self, rx, &team_map)?;

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

        let path_rows = query_live_paths(self)?;

        // ── Parallel pass ────────────────────────────────────────────────────────
        // Each worker thread reads the file, dispatches the tree-sitter parser,
        // and de-duplicates entities.  `map_init(|| (), ...)` matches the plan's
        // design: no per-thread state is needed because `Parser::new()` is ~3 µs
        // and tree-sitter 0.25.x is both `Send + Sync`.
        // Per-file failures are logged via `tracing::warn!` but do NOT abort the
        // parallel scan; they surface as `None` entries that the serial drain skips.
        //
        // Return type: Vec<Option<(String, String, Vec<ComplexityEntity>)>>
        //   - None  → file skipped (no Tier-1 lang, unreadable, or parse error)
        //   - Some  → (path, head_rev, deduped_entities)
        let batches: Vec<Option<(String, String, Vec<crate::complexity::ComplexityEntity>)>> =
            path_rows
                .into_par_iter()
                .map_init(
                    || (),
                    |_state, (path, head_rev)| {
                        let lang = Tier1Language::from_path(&path)?;
                        let full_path = opts.repo_path.join(&path);
                        let source = match std::fs::read(&full_path) {
                            Ok(b) => b,
                            Err(e) => {
                                tracing::warn!("complexity: cannot read {path}: {e}");
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
                        Some((path, head_rev, deduped))
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
            let Some((path, head_rev, entities)) = batch else {
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
            // Normalize to forward-slash so the `clones.path` strings match
            // the `changes.path` strings (which git always emits with `/`).
            // Without this, on Windows the JOIN in `clone_coupling` and the
            // `same_parent_dir` filter both miss because `to_string_lossy`
            // returns native separators (`\` on Windows, `/` on Unix).
            // `std::path::MAIN_SEPARATOR` is `/` on Unix so the replace is
            // a no-op there.
            let rel = path.strip_prefix(&opts.repo_path).map_or_else(
                |_| {
                    path.to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/")
                },
                |p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"),
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

fn ingest_loop(
    db: &FactsDb,
    rx: crossbeam_channel::Receiver<CommitEvent>,
    team_map: &identity::TeamMap,
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
) -> Result<()> {
    use duckdb::params;
    let unit = bucket.as_sql_unit();
    // unit comes from a closed enum (Day/Week/Month) so the format!
    // interpolation is safe — no user-controlled input.
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
         FROM changes c \
         INNER JOIN commits m ON m.rev = c.rev \
         GROUP BY date_trunc('{unit}', m.date), c.path"
    );
    db.conn().execute(&sql, params![]).map_err(|e| {
        CodeLoreError::Analysis(format!("materialize changes_bucketed ({unit}): {e}"))
    })?;
    tracing::info!("materialized changes_bucketed at {unit} granularity");
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
