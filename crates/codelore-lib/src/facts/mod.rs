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
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let config = Config::default()
            .access_mode(AccessMode::ReadOnly)
            .map_err(|e| CodeLoreError::Analysis(format!("duckdb config read-only: {e}")))?;
        let conn = Connection::open_with_flags(path, config)
            .map_err(|e| CodeLoreError::Analysis(format!("open_read_only duckdb: {e}")))?;
        Ok(Self { conn })
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
            return Self::open_read_only(&cache_p);
        }

        tracing::info!("cache miss: ingesting to {}", cache_p.display());

        // Create the parent directory if it doesn't exist yet.
        if let Some(parent) = cache_p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CodeLoreError::Analysis(format!("create cache dir: {e}")))?;
        }

        // Write to a .tmp file first; atomic-rename on success.
        let tmp = cache_p.with_extension("duckdb.tmp");
        // Remove any leftover .tmp from a prior aborted run.
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

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
