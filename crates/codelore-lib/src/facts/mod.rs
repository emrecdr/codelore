//! `DuckDB`-backed fact store. See spec §3.2 + §3.2.1 invariants.

pub mod groups;
pub mod ingest;
pub mod schema;

pub use groups::{GroupMap, GroupParseError, GroupRule};
pub use ingest::IngestStats;

use std::path::Path;

use duckdb::{AccessMode, Config, Connection};

use crate::cache;
use crate::repo::Repo;
use crate::{CodeLoreError, Options, Result};

pub struct FactsDb {
    conn: Connection,
}

impl FactsDb {
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| CodeLoreError::Analysis(format!("open in-memory duckdb: {e}")))?;
        let db = Self { conn };
        db.create_schema()?;
        Ok(db)
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| CodeLoreError::Analysis(format!("open duckdb: {e}")))?;
        let db = Self { conn };
        db.create_schema()?;
        Ok(db)
    }

    /// Open (or create) a read-write `DuckDB` file at `path`.
    /// Unlike `open()`, this does NOT call `create_schema` — the caller is
    /// responsible for schema initialisation (used internally by `open_or_ingest`).
    pub fn open_file(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| CodeLoreError::Analysis(format!("open_file duckdb: {e}")))?;
        Ok(Self { conn })
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
        let config = Config::default()
            .access_mode(AccessMode::ReadOnly)
            .map_err(|e| CodeLoreError::Analysis(format!("duckdb config read-only: {e}")))?;
        let conn = Connection::open_with_flags(path, config)
            .map_err(|e| CodeLoreError::Analysis(format!("open_read_only duckdb: {e}")))?;
        let db = Self { conn };
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
            return Self::open_read_only(&cache_p);
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
            let mem = Self::new_in_memory()?;
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

        let db = Self::open_file(&tmp)?;
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

        Self::open_read_only(&cache_p)
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
