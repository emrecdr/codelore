//! `DuckDB`-backed fact store. See spec §3.2 + §3.2.1 invariants.

pub mod groups;
pub mod ingest;
pub mod schema;

pub use groups::{GroupMap, GroupParseError, GroupRule};
pub use ingest::IngestStats;

use std::path::{Path, PathBuf};

use duckdb::{AccessMode, Config, Connection};

use crate::cache;
use crate::constants::DEFAULT_DUCKDB_MEMORY_LIMIT;
use crate::repo::Repo;
use crate::{CodeLoreError, Options, Result};

/// Resolve the default `DuckDB` spill directory when a caller supplies no
/// explicit override (`Options::temp_dir` / `--temp-dir`): a `spill/`
/// subdirectory of the cache root when one is in play, so spill files sit
/// alongside the persistent `.duckdb` cache under the same disk-space
/// expectations; the system temp directory otherwise (the plain `--no-cache`
/// in-memory path and `codelore calibrate-defects`'s mining store have no
/// cache root at all).
pub(crate) fn default_spill_dir(cache_root: Option<&Path>) -> PathBuf {
    match cache_root {
        Some(root) => root.join("codelore").join("spill"),
        None => std::env::temp_dir().join("codelore-spill"),
    }
}

/// Apply the `DuckDB` memory ceiling + spill-to-disk `PRAGMA`s that every
/// connection this binary opens must carry: `memory_limit` bounds resident
/// query state; `temp_directory` is where `DuckDB` spills once that ceiling
/// is hit, instead of growing unbounded and inviting the OS OOM killer on
/// very large repos. Creates `temp_dir` if it doesn't already exist. Safe to
/// call on a `ReadOnly`-mode connection — both PRAGMAs are session/engine
/// settings, not writes to the database file.
pub(crate) fn apply_memory_pragmas(conn: &Connection, temp_dir: &Path) -> Result<()> {
    apply_memory_pragmas_with_limit(conn, DEFAULT_DUCKDB_MEMORY_LIMIT, temp_dir)
}

/// Like [`apply_memory_pragmas`] but with an explicit `memory_limit` value.
/// Split out so tests can force a spill deterministically on a tiny fixture
/// (a real 4 GB ceiling never trips over test-sized data) without exposing a
/// `--memory-limit` CLI flag — the constant default covers real usage.
fn apply_memory_pragmas_with_limit(
    conn: &Connection,
    memory_limit: &str,
    temp_dir: &Path,
) -> Result<()> {
    std::fs::create_dir_all(temp_dir).map_err(|e| {
        CodeLoreError::Analysis(format!(
            "create duckdb temp_directory {}: {e}",
            temp_dir.display()
        ))
    })?;
    conn.pragma_update(None, "memory_limit", &memory_limit.to_string())
        .map_err(|e| CodeLoreError::Analysis(format!("set duckdb memory_limit: {e}")))?;
    conn.pragma_update(
        None,
        "temp_directory",
        &temp_dir.to_string_lossy().into_owned(),
    )
    .map_err(|e| CodeLoreError::Analysis(format!("set duckdb temp_directory: {e}")))?;
    Ok(())
}

pub struct FactsDb {
    conn: Connection,
    /// Process-local memo for [`crate::analyses::coupling::run_coupling`].
    /// That function is pure per `(db, coupling-affecting opts)` but is
    /// invoked 2-5× per CLI run on identical inputs (code-health,
    /// centrality, communities, clone-coupling, and the SPA dashboard all
    /// re-derive the same global coupling graph). The result is the full,
    /// Fisher-filtered, un-row-limited `Vec` — callers re-apply their own
    /// `rows_limit` after the lookup, so a `--rows N` choice never poisons
    /// the shared entry. `RefCell` + `Rc` (not a `Mutex`/`Arc`) because the
    /// `DuckDB` `Connection` is `!Send + !Sync` and every coupling call
    /// already runs on the single connection-owning thread.
    coupling_memo: std::cell::RefCell<
        std::collections::HashMap<
            crate::analyses::coupling::CouplingMemoKey,
            std::rc::Rc<Vec<crate::analyses::coupling::CouplingRow>>,
        >,
    >,
    /// Set once `changes_lineage` has been materialised for this fact
    /// store, so the recursive rename CTE + full table copy + index builds
    /// run ONCE per run instead of once per lineage-opt-in caller (12+
    /// analyses plus kamei under `--use-canonical-lineage`). The view's
    /// content is a pure function of the immutable `changes` / `commits`
    /// tables; the only post-build mutation is `apply_grouping`'s in-place
    /// `changes` swap, which calls `invalidate_changes_lineage` so the next
    /// materialise rebuilds against the grouped paths. `Cell` (not
    /// `RefCell`) — a plain `bool` on the single connection-owning thread.
    changes_lineage_built: std::cell::Cell<bool>,
    /// Idempotence guard for [`crate::analyses::knowledge::shares::materialize_knowledge_shares`].
    /// Mirrors `changes_lineage_built`: set on first materialise, checked at
    /// every entry to skip redundant temp-table rebuilds within a single run.
    knowledge_shares_built: std::cell::Cell<bool>,
    /// Process-local memo for the structural import graph
    /// ([`crate::analyses::import_graph::build_import_graph`]). The graph is
    /// a pure function of the immutable `imports` table, yet a `--format
    /// spa` render or a `codelore check` arch-suite rebuilds it (SQL scan +
    /// path interning + adjacency) once per arch analysis —
    /// `architecture-roles`, `modularity-violations`, `instability`,
    /// `architecture-metrics`, `dependency-cycles` — in a single process. A
    /// shared `Rc` handed to every caller collapses those into one build.
    /// Same single-thread `RefCell` + `Rc` rationale as `coupling_memo`.
    import_graph_memo:
        std::cell::RefCell<Option<std::rc::Rc<crate::analyses::import_graph::ImportGraph>>>,
    /// Process-local single-slot memo for
    /// [`crate::analyses::clones::run_clones_memoised`]. `run_clones` walks
    /// the working tree and tree-sitter-fingerprints every Tier-1 function —
    /// an O(files) filesystem + parse pass with no `changes` / `imports`
    /// dependency, so its result is fixed for a given (repo, clone-affecting
    /// opts) pair. The agent-loop gate's projected-health engine runs
    /// `run_code_health_scoped` twice on one `FactsDb` (HEAD baseline vs the
    /// substituted-complexity projection); both scoped runs re-walk clones for
    /// the DRY biomarker over the SAME working tree with the SAME
    /// `opts_scan`, so the second walk is pure waste. A single slot suffices
    /// because the only memoised caller is code-health, whose clone-affecting
    /// opts are fixed within a run. Same single-thread `RefCell` + `Rc`
    /// rationale as `import_graph_memo`.
    clones_memo: std::cell::RefCell<Option<std::rc::Rc<Vec<crate::analyses::clones::ClonesRow>>>>,
}

impl FactsDb {
    /// Wrap an open `DuckDB` connection with an empty coupling memo. The
    /// single point where the `coupling_memo` field is initialised so the
    /// four public constructors stay in lockstep.
    fn from_conn(conn: Connection) -> Self {
        Self {
            conn,
            coupling_memo: std::cell::RefCell::new(std::collections::HashMap::new()),
            changes_lineage_built: std::cell::Cell::new(false),
            knowledge_shares_built: std::cell::Cell::new(false),
            import_graph_memo: std::cell::RefCell::new(None),
            clones_memo: std::cell::RefCell::new(None),
        }
    }

    /// Look up a memoised coupling result for `key`. Returns a shared
    /// handle to the full, un-row-limited `Vec` so the caller re-applies
    /// its own `rows_limit` without recomputing the O(K²) self-join +
    /// Fisher pass. `None` on a miss; the caller then computes and stores.
    pub(crate) fn coupling_memo_get(
        &self,
        key: &crate::analyses::coupling::CouplingMemoKey,
    ) -> Option<std::rc::Rc<Vec<crate::analyses::coupling::CouplingRow>>> {
        self.coupling_memo.borrow().get(key).cloned()
    }

    /// Store the full, un-row-limited coupling result under `key`.
    pub(crate) fn coupling_memo_put(
        &self,
        key: crate::analyses::coupling::CouplingMemoKey,
        rows: std::rc::Rc<Vec<crate::analyses::coupling::CouplingRow>>,
    ) {
        self.coupling_memo.borrow_mut().insert(key, rows);
    }

    /// Shared handle to the memoised structural import graph, if built this
    /// run. `None` on a miss; the caller then builds and stores.
    pub(crate) fn import_graph_memo_get(
        &self,
    ) -> Option<std::rc::Rc<crate::analyses::import_graph::ImportGraph>> {
        self.import_graph_memo.borrow().clone()
    }

    /// Store the structural import graph for reuse across arch analyses.
    pub(crate) fn import_graph_memo_put(
        &self,
        graph: std::rc::Rc<crate::analyses::import_graph::ImportGraph>,
    ) {
        *self.import_graph_memo.borrow_mut() = Some(graph);
    }

    /// Shared handle to the memoised clones walk, if computed this run.
    /// `None` on a miss; the caller then walks and stores.
    pub(crate) fn clones_memo_get(
        &self,
    ) -> Option<std::rc::Rc<Vec<crate::analyses::clones::ClonesRow>>> {
        self.clones_memo.borrow().clone()
    }

    /// Store the clones walk for reuse across the two agent-loop scoped scans.
    pub(crate) fn clones_memo_put(
        &self,
        rows: std::rc::Rc<Vec<crate::analyses::clones::ClonesRow>>,
    ) {
        *self.clones_memo.borrow_mut() = Some(rows);
    }

    /// Whether `changes_lineage` is already materialised for this run.
    pub(crate) fn is_changes_lineage_built(&self) -> bool {
        self.changes_lineage_built.get()
    }

    /// Record that `changes_lineage` has been materialised.
    pub(crate) fn mark_changes_lineage_built(&self) {
        self.changes_lineage_built.set(true);
    }

    /// Invalidate the `changes_lineage` guard after a `changes` mutation
    /// (the `apply_grouping` swap) so the next materialise rebuilds the
    /// view against the new path set.
    pub(crate) fn invalidate_changes_lineage(&self) {
        self.changes_lineage_built.set(false);
    }

    /// Returns `true` if `knowledge_shares` and `doe_scores` temp tables
    /// have already been materialised in this run.
    pub(crate) fn is_knowledge_shares_built(&self) -> bool {
        self.knowledge_shares_built.get()
    }

    /// Record that the knowledge-share temp tables have been materialised.
    pub(crate) fn mark_knowledge_shares_built(&self) {
        self.knowledge_shares_built.set(true);
    }

    /// Open a fresh in-memory fact store, spilling to the default temp
    /// directory (see [`default_spill_dir`]) once `memory_limit` is
    /// exceeded. Equivalent to `new_in_memory_with_temp_dir(None)`.
    pub fn new_in_memory() -> Result<Self> {
        Self::new_in_memory_with_temp_dir(None)
    }

    /// Like [`new_in_memory`] but honors an explicit spill-directory
    /// override (falls back to [`default_spill_dir`] when `None`). Used by
    /// callers that resolved `Options::temp_dir` / `--temp-dir` — the plain
    /// `--no-cache` in-memory path bypasses the persistent cache entirely
    /// (so there is no cache root to derive a default from) but must still
    /// spill instead of OOM-ing on a very large repo.
    pub fn new_in_memory_with_temp_dir(temp_dir: Option<&Path>) -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| CodeLoreError::Analysis(format!("open in-memory duckdb: {e}")))?;
        let spill_dir = temp_dir.map_or_else(|| default_spill_dir(None), Path::to_path_buf);
        apply_memory_pragmas(&conn, &spill_dir)?;
        let db = Self::from_conn(conn);
        db.create_schema()?;
        Ok(db)
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| CodeLoreError::Analysis(format!("open duckdb: {e}")))?;
        apply_memory_pragmas(&conn, &default_spill_dir(None))?;
        let db = Self::from_conn(conn);
        db.create_schema()?;
        Ok(db)
    }

    /// Open (or create) a read-write `DuckDB` file at `path`, spilling to
    /// `temp_dir` once `memory_limit` is exceeded.
    /// Unlike `open()`, this does NOT call `create_schema` — the caller is
    /// responsible for schema initialisation (used internally by `open_or_ingest`).
    pub fn open_file(path: &Path, temp_dir: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| CodeLoreError::Analysis(format!("open_file duckdb: {e}")))?;
        apply_memory_pragmas(&conn, temp_dir)?;
        Ok(Self::from_conn(conn))
    }

    /// Open an existing `DuckDB` file in read-only mode.
    ///
    /// Validates the stored `schema_version` against the binary's expected
    /// version (`schema::CURRENT_SCHEMA_VERSION`) so an operator who hands
    /// a stale `.duckdb` to `--cache-dir` directly gets a typed parse-time
    /// error instead of cryptic `Catalog Error: Table … does not exist`
    /// at analysis time. The cache-hit path (`open_or_ingest`) is already
    /// guarded by the cache key — this check defends the direct-open path.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] if the file isn't a `DuckDB`
    /// fact store, lacks a `provenance` table, or has a different
    /// `schema_version` than this binary produces.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        Self::open_read_only_with_temp_dir(path, None)
    }

    /// Like [`open_read_only`] but honors an explicit spill-directory
    /// override (falls back to [`default_spill_dir`] when `None`). A
    /// read-only-mode connection can still build `TEMP` tables and
    /// materialize large intermediate query state (coupling, code-health,
    /// and friends all do), so it needs the same memory ceiling + spill
    /// target as the read-write constructors — `memory_limit` and
    /// `temp_directory` are session/engine settings, not writes to the
    /// (read-only) database file, so setting them here is safe.
    pub fn open_read_only_with_temp_dir(path: &Path, temp_dir: Option<&Path>) -> Result<Self> {
        let config = Config::default()
            .access_mode(AccessMode::ReadOnly)
            .map_err(|e| CodeLoreError::Analysis(format!("duckdb config read-only: {e}")))?;
        let conn = Connection::open_with_flags(path, config)
            .map_err(|e| CodeLoreError::Analysis(format!("open_read_only duckdb: {e}")))?;
        let spill_dir = temp_dir.map_or_else(|| default_spill_dir(None), Path::to_path_buf);
        apply_memory_pragmas(&conn, &spill_dir)?;
        let db = Self::from_conn(conn);
        db.validate_schema_version(path)?;
        Ok(db)
    }

    /// Read the `schema_version` row from the `provenance` table and bail
    /// if it differs from this binary's [`schema::CURRENT_SCHEMA_VERSION`].
    /// A missing `provenance` table or missing row is treated as schema
    /// mismatch — same operator-facing surface either way.
    fn validate_schema_version(&self, path: &Path) -> Result<()> {
        let stored: std::result::Result<String, duckdb::Error> = self.conn.query_row(
            "SELECT value FROM provenance WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        );
        match stored {
            Ok(v) if v == schema::CURRENT_SCHEMA_VERSION => Ok(()),
            Ok(v) => Err(CodeLoreError::Analysis(format!(
                "fact store {} has schema_version={v}, this binary expects {} — \
                 re-ingest with `--no-cache` or upgrade/downgrade codelore",
                path.display(),
                schema::CURRENT_SCHEMA_VERSION,
            ))),
            Err(e) => Err(CodeLoreError::Analysis(format!(
                "fact store {} is missing the provenance schema_version row \
                 (not a codelore fact store, or corrupted): {e}",
                path.display(),
            ))),
        }
    }

    /// Run `EXPLAIN <sql>` against the underlying `DuckDB` connection and
    /// return the optimizer plan as a single string (newline-separated
    /// rows). Used by `--explain` to emit per-analysis query plans
    /// without coupling the CLI to `duckdb::params!` macros.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] if the underlying `EXPLAIN`
    /// query fails to prepare or iterate.
    pub fn explain_sql<P: duckdb::Params>(&self, sql: &str, params: P) -> Result<String> {
        let explain_sql = format!("EXPLAIN {sql}");
        let mut stmt = self
            .conn
            .prepare(&explain_sql)
            .map_err(|e| CodeLoreError::Analysis(format!("explain prepare: {e}")))?;
        let mut rows = stmt
            .query(params)
            .map_err(|e| CodeLoreError::Analysis(format!("explain query: {e}")))?;
        let mut out = String::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| CodeLoreError::Analysis(format!("explain next: {e}")))?
        {
            // DuckDB's EXPLAIN returns 2 columns: (explain_key, explain_value).
            // The plan goes in column 1.
            let line: String = row
                .get(1)
                .map_err(|e| CodeLoreError::Analysis(format!("explain col 1: {e}")))?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Flush any pending writes to disk.
    /// Called before an atomic rename to ensure durability (APFS gotcha).
    pub fn flush(&self) -> Result<()> {
        self.conn
            .execute_batch("CHECKPOINT")
            .map_err(|e| CodeLoreError::Analysis(format!("duckdb checkpoint: {e}")))?;
        Ok(())
    }

    /// Content-addressed persistent cache constructor.
    ///
    /// Cache key: `(canonical_repo_path, head_sha, pkg_version, opts_thresholds, schema_v1)`.
    ///
    /// Hit path: open existing `.duckdb` file in read-only mode.
    /// Miss path: ingest to `.duckdb.tmp`, `CHECKPOINT`, `sync_all`, atomic rename,
    ///            prune stale entries, open result in read-only mode.
    ///
    /// Use `--no-cache` in the CLI to bypass this constructor.
    pub fn open_or_ingest<R: Repo>(opts: &Options, repo: &R) -> Result<Self> {
        Self::open_or_ingest_with_cache_root(opts, repo, &cache::default_cache_root())
    }

    /// Same as [`open_or_ingest`] but with an explicit cache root for testing
    /// and for the `--cache-dir` CLI flag.
    pub fn open_or_ingest_with_cache_root<R: Repo>(
        opts: &Options,
        repo: &R,
        cache_root: &Path,
    ) -> Result<Self> {
        let head_sha = repo.head_sha()?;
        let key = cache::cache_key(&opts.repo_path, &head_sha, opts);
        let cache_p = cache::cache_path_with_root(&key, &opts.repo_path, cache_root);
        // `--temp-dir` wins when set; otherwise default to a subdir of THIS
        // cache root (not the global default) so `--cache-dir` and
        // `--temp-dir` stay consistent with each other.
        let spill_dir = opts
            .temp_dir
            .clone()
            .unwrap_or_else(|| default_spill_dir(Some(cache_root)));

        if cache_p.exists() {
            tracing::info!("cache hit: {}", cache_p.display());
            // Warn when a cache hit happens on a dirty working tree.
            // The cache key is (canonical_repo_path, head_sha, opts, version,
            // schema) — it does NOT hash worktree state, so HEAD-time metrics
            // (complexity, clones) that were computed from disk at ingest can
            // be silently stale relative to what's on disk NOW. Most analyses
            // (revisions, churn, coupling, ownership, etc.) only read commit
            // history and are unaffected, but the user can't tell from
            // looking at the output which kind of analysis they're running.
            // Surface the situation so they know to pass `--no-cache` if it
            // matters.
            if repo.is_worktree_dirty() {
                tracing::warn!(
                    "cache hit on a working tree with uncommitted changes; \
                     HEAD-time metrics (hotspots' complexity, clones) may be \
                     stale relative to disk. Pass `--no-cache` to recompute \
                     against the current working tree."
                );
            }
            return Self::open_read_only_with_temp_dir(&cache_p, Some(&spill_dir));
        }

        tracing::info!("cache miss: ingesting to {}", cache_p.display());

        // Skip cache WRITE when the working tree is dirty.
        // HEAD-time metrics (complexity, clones) are computed from disk at
        // ingest time; persisting them under the clean head_sha cache key
        // would poison the cache — a later run on a CLEAN tree would
        // cache-hit and silently serve the dirty metrics with no warning
        // (the read-time warn fires only when the CURRENT tree is dirty).
        //
        // Fall back to an in-memory FactsDb so the analysis still runs;
        // the user just doesn't get the persistent-cache speedup until
        // they commit (or run with `--no-cache` and accept the slow path
        // explicitly).
        if repo.is_worktree_dirty() {
            tracing::warn!(
                "working tree has uncommitted changes; skipping persistent \
                 cache write to avoid caching dirty HEAD-time metrics \
                 (complexity, clones) under the clean head_sha key. \
                 Commit changes or pass `--no-cache` to suppress this notice."
            );
            let mem = Self::new_in_memory_with_temp_dir(Some(&spill_dir))?;
            mem.create_schema()?;
            mem.ingest(repo, opts)?;
            return Ok(mem);
        }

        // Create the parent directory if it doesn't exist yet.
        if let Some(parent) = cache_p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CodeLoreError::Analysis(format!("create cache dir: {e}")))?;
        }

        // Write to a process-unique .tmp file first; atomic-rename on
        // success. The PID suffix prevents two concurrent runs on the same
        // cache key (e.g. parallel CI jobs, multiple terminals) from
        // clobbering each other's in-flight writes — DuckDB would either
        // refuse the file lock or produce a partially-written cache file.
        // Stale `.tmp.<dead_pid>` artifacts from crashed runs are swept by
        // `cache::cleanup_stale_tmp_files` during prune.
        let tmp = cache_p.with_extension(format!("duckdb.tmp.{}", std::process::id()));
        // Remove any leftover .tmp from a prior aborted run by THIS PID.
        let _ = std::fs::remove_file(&tmp);

        let db = Self::open_file(&tmp, &spill_dir)?;
        db.create_schema()?;
        db.ingest(repo, opts)?;
        // CHECKPOINT flushes DuckDB's WAL to the file before we open() it.
        db.flush()?;
        // Drop the connection before rename so DuckDB releases the file lock.
        drop(db);
        // sync_all forces the file's data and metadata to disk before the
        // rename, so a crash between rename and the rename being durable
        // doesn't leave the cache file pointing at unwritten data
        // (macOS APFS is the classic offender; Linux ext4 + Windows NTFS
        // benefit too). The handle must be opened with write access on
        // Windows — `FlushFileBuffers` requires `GENERIC_WRITE` and
        // rejects a read-only handle with `ERROR_ACCESS_DENIED`.
        // `File::open` (read-only) would work on Unix but break on Windows.
        {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&tmp)
                .map_err(|e| CodeLoreError::Analysis(format!("sync_all open .tmp: {e}")))?;
            f.sync_all()
                .map_err(|e| CodeLoreError::Analysis(format!("sync_all .tmp: {e}")))?;
        }
        std::fs::rename(&tmp, &cache_p)
            .map_err(|e| CodeLoreError::Analysis(format!("rename .tmp → .duckdb: {e}")))?;

        // LRU eviction: prune this repo's cache dir (max 5), then the global cap (2 GB).
        if let Some(repo_dir) = cache_p.parent() {
            cache::prune_repo_cache(repo_dir, 5);
            cache::prune_global_cache(cache_root, 2 * 1024 * 1024 * 1024);
        }

        Self::open_read_only_with_temp_dir(&cache_p, Some(&spill_dir))
    }

    /// Test-only: like [`new_in_memory_with_temp_dir`] but with an explicit
    /// `memory_limit` override, so a test can force `DuckDB` to spill on a
    /// tiny fixture instead of needing gigabytes of real data (the
    /// [`DEFAULT_DUCKDB_MEMORY_LIMIT`] ceiling never trips over test-sized
    /// inputs). Not exposed as a CLI flag — see the module's `--temp-dir`
    /// docs for why `--memory-limit` is YAGNI.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_in_memory_with_memory_limit(memory_limit: &str, temp_dir: &Path) -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| CodeLoreError::Analysis(format!("open in-memory duckdb: {e}")))?;
        apply_memory_pragmas_with_limit(&conn, memory_limit, temp_dir)?;
        let db = Self::from_conn(conn);
        db.create_schema()?;
        Ok(db)
    }

    fn create_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(schema::SCHEMA_V1)
            .map_err(|e| CodeLoreError::Analysis(format!("create schema: {e}")))?;
        let mut stmt = self
            .conn
            .prepare("INSERT OR REPLACE INTO provenance (key, value) VALUES (?, ?)")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare: {e}")))?;
        for (k, v) in schema::INITIAL_PROVENANCE {
            stmt.execute(duckdb::params![k, v])
                .map_err(|e| CodeLoreError::Analysis(format!("provenance insert: {e}")))?;
        }
        Ok(())
    }

    pub fn list_tables(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT table_name FROM duckdb_tables WHERE schema_name = 'main'")
            .map_err(|e| CodeLoreError::Analysis(format!("prepare: {e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| CodeLoreError::Analysis(format!("query_map: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect: {e}")))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn query_one_value(&self, sql: &str) -> Result<String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| CodeLoreError::Analysis(format!("prepare: {e}")))?;
        let v: String = stmt
            .query_row([], |r| r.get(0))
            .map_err(|e| CodeLoreError::Analysis(format!("query_row: {e}")))?;
        Ok(v)
    }

    /// Prepare a SQL statement against the underlying connection. Returns
    /// a `duckdb::Statement<'_>` whose lifetime is tied to `&self`. Use
    /// for the `prepare → query_map / query_row → collect` pattern when
    /// the caller needs multi-row iteration. Errors are wrapped in
    /// [`CodeLoreError::Analysis`] so they share the analysis-error exit
    /// code (4) the rest of the lib uses for SQL failures.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] if statement preparation fails.
    pub fn prepare<'a>(&'a self, sql: &str) -> Result<duckdb::Statement<'a>> {
        self.conn
            .prepare(sql)
            .map_err(|e| CodeLoreError::Analysis(format!("prepare: {e}")))
    }

    /// Run multiple SQL statements separated by `;`. Useful for test
    /// fixtures and one-shot DDL/DML. Single-statement SQL also works
    /// — `DuckDB`'s `execute_batch` just feeds the whole string through
    /// the parser.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] on any SQL error.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.conn
            .execute_batch(sql)
            .map_err(|e| CodeLoreError::Analysis(format!("execute_batch: {e}")))
    }

    /// Run a single SQL statement that returns exactly one row, mapping
    /// it via the caller-supplied closure. Mirrors `rusqlite`'s shape so
    /// migration from `db.conn().query_row(...)` is mechanical.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] on prepare / execute / no-rows
    /// error.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, mapper: F) -> Result<T>
    where
        P: duckdb::Params,
        F: FnOnce(&duckdb::Row<'_>) -> duckdb::Result<T>,
    {
        self.conn
            .query_row(sql, params, mapper)
            .map_err(|e| CodeLoreError::Analysis(format!("query_row: {e}")))
    }

    /// Internal raw-connection accessor. `pub(crate)` so the rest of
    /// `codelore-lib` (kamei, `quality_gates`, `output::spa`, ingest, etc.)
    /// can still reach the underlying `duckdb::Connection` for
    /// `Appender` / multi-statement transactions / etc. without
    /// re-implementing every primitive on `FactsDb`. External callers
    /// must use the narrow safe methods above (`prepare`,
    /// `execute_batch`, `query_row`, `query_one_value`, `list_tables`,
    /// `explain_sql`, `flush`) rather than reaching for the raw
    /// connection.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}
