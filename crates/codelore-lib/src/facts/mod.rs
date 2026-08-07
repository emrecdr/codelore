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
    /// Type-erased, per-`FactsDb` slot map for the analyses layer's process-
    /// local memos (coupling graph, import graph, clones walk, code-health).
    /// Those caches are keyed on `analyses` row/graph types, so storing them
    /// here concretely would make the `facts` layer depend on `analyses` — a
    /// module cycle. Instead each memo is a `T` the analyses layer defines
    /// (in its `memo` module) and reaches through [`Self::analysis_memo`],
    /// which lazily inserts one `T` per connection and hands back a shared
    /// `Rc<T>`. `RefCell` + `Rc<dyn Any>` (not a `Mutex`/`Arc`) because the
    /// `DuckDB` `Connection` is `!Send + !Sync` and every analysis runs on the
    /// single connection-owning thread. This field names no `analyses` type.
    analysis_memos: std::cell::RefCell<
        std::collections::HashMap<std::any::TypeId, std::rc::Rc<dyn std::any::Any>>,
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
    /// Idempotence guard for the knowledge-share temp tables (set on first
    /// materialise, checked at every entry to skip redundant rebuilds within
    /// a single run). Mirrors `changes_lineage_built`; a plain `bool`, so it
    /// names no `analyses` type and stays here with the other SQL-level guards.
    knowledge_shares_built: std::cell::Cell<bool>,
}

impl FactsDb {
    /// Wrap an open `DuckDB` connection with an empty analysis-memo map. The
    /// single point where the non-`conn` fields are initialised so the four
    /// public constructors stay in lockstep.
    fn from_conn(conn: Connection) -> Self {
        Self {
            conn,
            analysis_memos: std::cell::RefCell::new(std::collections::HashMap::new()),
            changes_lineage_built: std::cell::Cell::new(false),
            knowledge_shares_built: std::cell::Cell::new(false),
        }
    }

    /// Hand back this connection's process-local memo of type `T`, lazily
    /// creating an empty one on first use. The analyses layer defines each
    /// memo `T` (in its `memo` module) and drives it through the returned
    /// `Rc<T>`; keeping the concrete types out of `facts` is what breaks the
    /// facts→analyses cycle. One `T` is stored per `FactsDb` and shared for
    /// its lifetime, so the once-per-run memo semantics hold.
    pub(crate) fn analysis_memo<T: std::any::Any + Default>(&self) -> std::rc::Rc<T> {
        let mut slots = self.analysis_memos.borrow_mut();
        let any =
            std::rc::Rc::clone(slots.entry(std::any::TypeId::of::<T>()).or_insert_with(|| {
                std::rc::Rc::new(T::default()) as std::rc::Rc<dyn std::any::Any>
            }));
        // The entry under `TypeId::of::<T>()` is always `Rc::new(T::default())`,
        // so this downcast cannot fail; return a fresh detached default on the
        // unreachable branch rather than unwrap/expect.
        any.downcast::<T>()
            .unwrap_or_else(|_| std::rc::Rc::new(T::default()))
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
                // Reachable from every command that opens the cache, and most
                // of them have no `--no-cache` — that flag exists only on
                // `analyze`. Name the escape hatch that works everywhere.
                "fact store {} has schema_version={v}, this binary expects {} — \
                 point this run at a fresh cache with `--cache-dir <scratch>` \
                 (or `--no-cache` on `analyze`), or use a codelore version \
                 matching the stored schema",
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
            // matters. Gate on an interactive stderr first: `is_worktree_dirty()`
            // is an O(tracked-files) status walk, and on the non-interactive
            // agent-loop / CI path the persistent cache is built to serve — a
            // near-O(1) "open a file" fast path — nobody reads this warning, so
            // skip the scan entirely there rather than pay it on every hit. A
            // missed hint on a piped/redirected stderr is within the `Repo`
            // trait's best-effort-hint contract for `is_worktree_dirty`.
            if std::io::IsTerminal::is_terminal(&std::io::stderr()) && repo.is_worktree_dirty() {
                tracing::warn!(
                    "cache hit on a working tree with uncommitted changes; \
                     HEAD-time metrics (hotspots' complexity, clones) may be \
                     stale relative to disk. Recompute against the working \
                     tree with `--cache-dir <scratch>` (or `--no-cache` on \
                     `analyze`, which is the only surface carrying that flag)."
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
                 Commit the changes to silence this, or pass `--no-cache` on \
                 `analyze` (the only surface carrying that flag)."
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
        // Never persist a zero-commit store. The cache key is HEAD-scoped and does
        // not fold shallow/worktree state, so a later run on a repaired (unshallowed)
        // clone would hit this same file, load the empty store, and re-fail the
        // ingest witness on healthy history — a sticky failure the witness message's
        // own remedy cannot clear. Serve this run from memory instead, exactly like
        // the dirty-tree bail.
        //
        // The witness has to match the ingest mode. A head-only ingest walks
        // no commits by design, so `commit_count` is zero on a completely
        // healthy run — gating on it there fired every time, discarded the
        // store that had just been built, and re-ran the expensive HEAD
        // complexity scan into memory. `codelore calibrate` takes this path
        // once per corpus repository, so it paid the scan twice per repo and
        // could never persist an entry to reuse on the next run. For that
        // mode the meaningful floor is the table the scan actually fills.
        let witnessed = if opts.head_only_ingest {
            db.complexity_row_count()? > 0
        } else {
            db.commit_count()? > 0
        };
        if !witnessed {
            drop(db);
            let _ = std::fs::remove_file(&tmp);
            let mem = Self::new_in_memory_with_temp_dir(Some(&spill_dir))?;
            mem.create_schema()?;
            mem.ingest(repo, opts)?;
            return Ok(mem);
        }
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

        // Evict this repo's cache dir past its entry cap, then enforce the
        // global byte cap. Both bounds are named in `cache` so the values
        // enforced here and the ones `codelore profile` reports agree.
        if let Some(repo_dir) = cache_p.parent() {
            cache::prune_repo_cache(repo_dir, cache::MAX_REPO_CACHE_ENTRIES);
            cache::prune_global_cache(cache_root, cache::GLOBAL_CACHE_MAX_BYTES);
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

    /// Number of commits in the fact store — the persisted, cache-safe form of
    /// [`ingest::IngestStats::commits_ingested`] (one row per ingested commit).
    /// Unlike that in-memory counter it is readable after a cache HIT as well as
    /// a fresh ingest; and unlike `complexity_metrics` / `changes` it is the raw
    /// output of the commit walk — it does not derive from the `changes ⋈
    /// commits` join, so a blind walk that empties that join still leaves this
    /// readable (and zero). That independence is what makes it a witness.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Analysis`] on query failure.
    pub fn commit_count(&self) -> Result<i64> {
        self.query_row("SELECT COUNT(*) FROM commits", [], |r| r.get::<_, i64>(0))
    }

    /// Rows the HEAD-state scan produced, for use as a witness where
    /// [`Self::commit_count`] cannot be one.
    ///
    /// A head-only ingest deliberately leaves the history tables empty — it
    /// scans complexity and imports at HEAD and walks no commits — so
    /// `commit_count` is zero for a perfectly healthy run. Anything gating on
    /// that count therefore fires unconditionally on this path. This counts
    /// the table the head-only scan actually fills, so "did the ingest see
    /// anything?" stays answerable in both modes.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Analysis`] on query failure.
    pub fn complexity_row_count(&self) -> Result<i64> {
        self.query_row("SELECT COUNT(*) FROM complexity_metrics", [], |r| {
            r.get::<_, i64>(0)
        })
    }

    /// Fail loudly when the walk ingested no commits while HEAD names a real
    /// commit — the signature of a truncated checkout. A shallow `fetch-depth`
    /// clone whose tip is a merge commit ingests zero commits under the default
    /// merge filter, leaving an empty fact store on which every quality gate
    /// finds nothing to violate and `codelore check` reports a green pass over
    /// no data. Gating on the ingest count turns that silent pass into a hard,
    /// distinct error.
    ///
    /// An empty `head_sha` (an unborn HEAD — `git init` with nothing committed)
    /// is deliberately not this case and passes through: that is a genuinely
    /// empty repository, the province of the empty-repository preflight, not a
    /// truncated one.
    ///
    /// # Errors
    ///
    /// [`CodeLoreError::Repo`] (spec §6.6 exit 3 — the shallow/corrupted-repo
    /// bucket that [`CodeLoreError::BlobNotFound`] also occupies) when HEAD is
    /// real but no commits were ingested.
    pub fn ensure_ingest_witnessed(&self, head_sha: &str) -> Result<()> {
        if !head_sha.is_empty() && self.commit_count()? == 0 {
            return Err(CodeLoreError::Repo(
                "HEAD names a real commit but the walk ingested no history — the repository \
                 checkout is truncated. A shallow clone (git fetch-depth, e.g. \
                 actions/checkout's default fetch-depth: 1) whose tip is a merge commit ingests \
                 zero commits under the default merge filter, so every quality gate would pass \
                 over an empty fact store. Re-run against full history (fetch-depth: 0). If this \
                 keeps failing from a stale cache after the history is repaired and the command \
                 (check, gate, explain) offers no --no-cache flag, point it at a fresh cache with \
                 --cache-dir <scratch-path>."
                    .to_string(),
            ));
        }
        Ok(())
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

#[cfg(test)]
mod ingest_witness_tests {
    use super::FactsDb;

    /// Insert one inert commit row — the witness reads only the row count.
    fn seed_commit(db: &FactsDb, rev: &str) {
        db.conn()
            .execute(
                &format!(
                    "INSERT INTO commits (rev, author_email, author_name, committer_email, \
                     canonical_author, date, committer_date, message, is_merge, parent_count) \
                     VALUES ('{rev}', 'a@b.com', 'A', 'a@b.com', 'A', \
                     TIMESTAMP '2026-01-01', TIMESTAMP '2026-01-01', 'm', false, 1)"
                ),
                [],
            )
            .expect("insert commit");
    }

    #[test]
    fn commit_count_reflects_ingested_rows() {
        let db = FactsDb::new_in_memory().expect("db");
        assert_eq!(db.commit_count().expect("count"), 0);
        seed_commit(&db, "c1");
        seed_commit(&db, "c2");
        assert_eq!(db.commit_count().expect("count"), 2);
    }

    #[test]
    fn witness_errors_on_real_head_with_zero_commits() {
        // The truncated-checkout signature: HEAD resolves to a real commit but
        // the walk ingested nothing (a shallow merge-tip checkout under the
        // default merge filter). Must be a hard repo error, never a pass.
        let db = FactsDb::new_in_memory().expect("db");
        let err = db
            .ensure_ingest_witnessed("af53d17d1e3d64679d1691e75f82b65a2edb397a")
            .expect_err("zero commits + real HEAD must be a hard error");
        assert_eq!(
            err.exit_code(),
            3,
            "a truncated checkout maps to the repo-error exit bucket"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("fetch-depth"),
            "the message must name the likely cause: {msg}"
        );
        assert!(
            msg.contains("truncated"),
            "the message must name the condition: {msg}"
        );
    }

    #[test]
    fn witness_passes_when_commits_present() {
        let db = FactsDb::new_in_memory().expect("db");
        seed_commit(&db, "c1");
        db.ensure_ingest_witnessed("af53d17d")
            .expect("a store with history passes the witness");
    }

    #[test]
    fn witness_ignores_unborn_head() {
        // An empty head_sha is an unborn HEAD (git init, nothing committed) — a
        // genuinely empty repository, not a truncated one. The witness must not
        // fire so the empty-repository preflight owns that case.
        let db = FactsDb::new_in_memory().expect("db");
        db.ensure_ingest_witnessed("")
            .expect("an unborn HEAD is not a truncated checkout");
    }
}
