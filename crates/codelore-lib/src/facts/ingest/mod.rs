//! Stream commits from a Repo into `DuckDB` via the
//! N gix workers → bounded crossbeam channel → 1 Appender thread pattern.
//! See spec §3.2.2.
//!
//! Threading model: `duckdb::Connection` contains `RefCell`, which is `!Sync`.
//! Therefore `&FactsDb` is `!Send` and the Appender must run on the thread that
//! owns the `FactsDb`. We flip the design: the **producer** runs in a scoped
//! spawned thread (Repo is `Send + Sync`) and the **consumer** (Appender) runs
//! on the calling thread.

pub mod at_rev;
mod clones_head;
mod complexity_head;
pub(crate) mod consumer;
mod grouping;
mod imports_head;
mod lineage;

pub use at_rev::{ingest_complexity_at_rev, materialize_imports_at_rev};
pub use grouping::{apply_grouping, materialize_changes_bucketed};
pub use lineage::{materialize_changes_lineage, materialize_path_lineage};

use crossbeam_channel::bounded;

use super::FactsDb;
use crate::repo::Repo;
use crate::{CodeLoreError, CommitEvent, Options, Result};

use consumer::ingest_loop;

const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// Process-wide capacity override for the producer→consumer commit
/// channel. `0` means "use [`DEFAULT_CHANNEL_CAPACITY`]"; benches set
/// it via [`set_channel_capacity_override`] to sweep across e.g.
/// `[16, 64, 256, 1024]` in a single process without
/// `unsafe { env::set_var }` and without expanding the public CLI
/// surface. Production code never reads or writes this from CLI
/// dispatch — there is one reader (`channel_capacity`) and one writer
/// (`benches/end_to_end.rs::ingest_capacity_sweep`).
static CHANNEL_CAPACITY_OVERRIDE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Bench hook: set the channel-capacity override for subsequent
/// `ingest()` calls. Pass `0` to restore the default. Exists so V6's
/// `ingest_capacity_sweep` bench can vary the bound across
/// `[16, 64, 256, 1024]` in one `cargo bench` invocation —
/// `unsafe { env::set_var }` would violate the workspace
/// `unsafe_code = "forbid"` lint and a `cfg(test)` gate wouldn't fire
/// in `cargo bench`.
pub fn set_channel_capacity_override(n: usize) {
    CHANNEL_CAPACITY_OVERRIDE.store(n, std::sync::atomic::Ordering::SeqCst);
}

/// Read the effective channel capacity for THIS ingest call. Single
/// atomic load — negligible against the producer/consumer setup cost
/// — falls back to the const default when no override is set.
fn channel_capacity() -> usize {
    let n = CHANNEL_CAPACITY_OVERRIDE.load(std::sync::atomic::Ordering::SeqCst);
    if n == 0 { DEFAULT_CHANNEL_CAPACITY } else { n }
}

#[derive(Debug, Default)]
pub struct IngestStats {
    pub commits_ingested: usize,
    pub changes_ingested: usize,
    /// Number of clone-family member rows inserted into the
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

        // Single producer gix walker → bounded channel → Appender on the calling thread.
        let (tx, rx) = bounded::<CommitEvent>(channel_capacity());

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
            // Combined path filter — respects --exclude + .gitignore +
            // .codeloreignore for the commit-walk ingest path. Rel-path
            // matching is handled by PathsFilter internals.
            let paths_filter = crate::paths_filter::PathsFilter::from_opts(opts)?;
            let stats = ingest_loop(self, rx, &team_map, &bot_patterns, &paths_filter)?;

            // Map a panic in the commit-walker thread into a typed
            // `CodeLoreError::Repo` so `main()`'s chain-walker can
            // derive the correct spec §6.6 exit code (3 = repo).
            // Using `.expect()` here would surface the panic as
            // `thread 'main' panicked at 'producer panicked'` + exit
            // 101 — bypassing the typed-error chain and giving the
            // operator no breadcrumb back to the failing commit walk.
            let join_result = producer
                .join()
                .map_err(|payload| CodeLoreError::Repo(format_panic_payload(&payload)))?;
            join_result?;
            Ok(stats)
        })?;

        // Hoist HEAD-time context shared across all four HEAD-time passes.
        // `query_live_paths` runs a CTE + arg_max across `commits ⋈ changes`
        // and `current_head_rev` walks `commits` ordered by date — both are
        // pure functions of the just-ingested fact store, so computing them
        // once here avoids four redundant round-trips through the SQL planner.
        let head_rev = current_head_rev(self)?;
        let live_paths = query_live_paths(self)?;

        // Populate entities + complexity_metrics from blobs at HEAD via
        // `Repo::read_blob_at_head`. Avoids dirty-tree contamination and
        // works on bare repos (no working tree). Falls back to
        // `std::fs::read` for backends without a blob implementation
        // (default trait impl returns `Ok(None)`).
        self.ingest_complexity_at_head(repo, opts, &live_paths, &head_rev)?;

        // Populate the Kamei 14-feature change vector via SQL UPDATE pass.
        // Kamei history (ndev, nuc, age) joins changes-to-changes on path;
        // without lineage, renamed files lose all their pre-rename history.
        // Routing through `changes_lineage` merges histories under the canonical
        // post-rename name.
        crate::kamei::enrich(self, opts.use_canonical_lineage)?;

        // Populate the `clones` table at HEAD so the
        // `clone-coupling` analysis (§6) can JOIN against it. Honors
        // `opts.min_clone_node_count` and `opts.exclude_patterns` (set via
        // `--exclude` + `.codeloreignore`).
        let clones_n = self.populate_clones_at_head(repo, opts, &live_paths, &head_rev)?;

        // Populate the `imports` table at HEAD so the layered-
        // architecture rules, god-class detector, and architecture
        // force-graph analyses can JOIN against it. Uses the same
        // rayon-then-serial-drain shape as the complexity + clones
        // passes — no new threading primitives.
        let imports_n = self.populate_imports_at_head(repo, opts, &live_paths, &head_rev)?;
        tracing::info!("imports: {imports_n} edges ingested at HEAD across Tier-1 source files");

        // Per-language resolver UPDATE pass. Maps the raw `target`
        // strings to repo-relative tracked paths where resolution
        // succeeds. Covers Rust / Python / JS / TS today; Java FQN
        // → file mapping is project-layout-specific and skipped.
        let resolved_n = self.resolve_imports_at_head(&live_paths, &head_rev)?;
        tracing::info!(
            "imports: {resolved_n} of {imports_n} import edges resolved to tracked paths"
        );

        // Architectural grouping: after ingest, rewrite the
        // `changes.path` column to logical group names per --group-file.
        // Runs last so the rewrite sees all change rows from every commit.
        if let Some(group_file) = opts.group_file.as_ref() {
            let group_map = super::groups::GroupMap::from_file(group_file, opts.strict_grouping)
                .map_err(|e| match e {
                    // Read-side input failure (unreadable `--group-file`) →
                    // exit 3; parse failures stay analysis errors (exit 4).
                    super::groups::GroupParseError::Io(io) => {
                        CodeLoreError::RepoIo(std::io::Error::new(
                            io.kind(),
                            format!("read group-file {}: {io}", group_file.display()),
                        ))
                    }
                    other => CodeLoreError::Analysis(format!("--group-file: {other}")),
                })?;
            apply_grouping(self, &group_map)?;
        }

        let mut stats = stats;
        stats.clones_ingested = clones_n;
        Ok(stats)
    }
}

/// Resolve HEAD's rev from the commits table (most recent commit by date).
/// Used to stamp the `rev` column on inserted clone rows.
fn current_head_rev(db: &FactsDb) -> Result<String> {
    // SHA-1 lex order (`rev DESC`) is arbitrary and has no
    // relationship to git topology. When two commits share the exact
    // same second timestamp (common with bot commits, rebases,
    // scripted backfills), the lex tiebreak can pick the PARENT as
    // HEAD instead of the child. We use DuckDB's `rowid` pseudo-column
    // (insertion order) — gix walks reverse-chronologically (newest
    // first), so children arrive BEFORE their parents and get smaller
    // rowids. `rowid ASC` for same-second pairs correctly selects the
    // child as HEAD. Deterministic across runs of the same input.
    let sql = "SELECT rev FROM commits ORDER BY date DESC, rowid ASC LIMIT 1";
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
        Ok(row
            .get::<_, String>(0)
            .map_err(|e| CodeLoreError::Analysis(format!("head rev value: {e}")))?)
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
/// `commits.date DESC` first, then `commits.rowid ASC` (insertion order — gix
/// walks reverse-chronologically so children get smaller rowids than parents)
/// as a deterministic, topologically-correct tiebreak. Previously this
/// used `c.rev DESC` (SHA-1 lex) which is arbitrary and could pick the parent
/// commit as HEAD instead of the child on same-second pairs.
fn query_live_paths(db: &FactsDb) -> Result<Vec<String>> {
    // Hash-grouped `arg_max` picks the most-recent change_type per path in a
    // single streaming pass (O(K) memory, K = distinct paths). The ordering
    // key is `ROW(date, -rowid)` so that struct lex-compare reproduces
    // `ORDER BY date DESC, rowid ASC`: larger date wins, then smaller rowid.
    let sql = "
        WITH latest_per_path AS (
            SELECT
                c.path,
                arg_max(
                    c.change_type,
                    ROW(commits.date, -commits.rowid)
                ) AS change_type
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
            GROUP BY c.path
        )
        SELECT path
        FROM latest_per_path
        WHERE change_type != 'deleted'
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

/// Format a `Box<dyn Any + Send>` panic payload (the value
/// `std::thread::JoinHandle::join` returns on `Err`) into a stable
/// human-readable string. The two canonical concrete types `panic!`
/// produces are `&'static str` (from `panic!("literal")`) and `String`
/// (from `panic!("{}", val)`) — handle both. Anything else (custom
/// panic types, payload-less aborts) falls through to a generic
/// `<non-string panic payload>` so the typed-error path still
/// surfaces something operators can grep for.
pub(crate) fn format_panic_payload(payload: &Box<dyn std::any::Any + Send>) -> String {
    let detail = payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    format!("commit walker thread panicked: {detail}")
}

#[cfg(test)]
mod panic_payload_tests {
    use super::format_panic_payload;

    /// `panic!("literal")` → `Box<dyn Any>` containing `&'static str`.
    #[test]
    fn extracts_static_str_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("walker exploded");
        let msg = format_panic_payload(&payload);
        assert_eq!(msg, "commit walker thread panicked: walker exploded");
    }

    /// `panic!("{} {}", "walker", "exploded")` → `Box<dyn Any>` containing `String`.
    #[test]
    fn extracts_string_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("formatted reason"));
        let msg = format_panic_payload(&payload);
        assert_eq!(msg, "commit walker thread panicked: formatted reason");
    }

    /// Anything that isn't `&'static str` or `String` falls through to
    /// the placeholder. Exercises the `unwrap_or_else` branch so a
    /// future change to the helper can't silently break it.
    #[test]
    fn unknown_payload_falls_through_to_placeholder() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42_u32);
        let msg = format_panic_payload(&payload);
        assert_eq!(
            msg,
            "commit walker thread panicked: <non-string panic payload>"
        );
    }
}
