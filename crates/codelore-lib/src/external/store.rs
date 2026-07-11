//! Sidecar `DuckDB` store for external scanner findings.
//!
//! The store lives at
//! `<cache_root>/codelore/<repo_hash_8>/external-findings.duckdb-ext`
//! alongside the main `.duckdb` cache entries. The `.duckdb-ext` extension
//! is intentional: the LRU pruner in `cache.rs` matches files whose
//! `.extension()` equals `"duckdb"` exactly, so `.duckdb-ext` is never
//! touched by automatic eviction.
//!
//! Each [`ExternalStore`] owns its own `duckdb::Connection`. The connection
//! is `!Send + !Sync` — same constraint as `FactsDb`. Callers must keep the
//! store on the thread that created it.
//!
//! ## Read-only readers, single read-write writer
//!
//! `DuckDB` is single-writer: a read-write connection takes an exclusive file
//! lock, so a second opener (a concurrent `check`, MCP call, or `analyze` on the
//! same repo) can fail to open while the first holds it. The read paths
//! ([`ExternalStore::open_existing`], [`ExternalStore::open_nonempty`]) therefore
//! open with [`AccessMode::ReadOnly`], which takes a shared lock and lets
//! multiple readers coexist. [`ExternalStore::open_or_create`] is the sole
//! read-write opener and the sole writer/healer: it creates the parent
//! directory and the table schema. Readers never run DDL — a sidecar missing
//! the `external_findings` table (truncated or corrupt) cannot be healed under a
//! read-only lock, so [`ExternalStore::open_nonempty`] treats it as unusable and
//! maps it to `None` (absent, empty, and unreadable are one "nothing to read"
//! state for a reader).
//!
//! ## Replace semantics
//!
//! [`ExternalStore::replace_engine`] removes all existing rows for the given
//! engine before inserting the new batch. Re-ingesting the same SARIF file
//! produces an identical row count — findings are idempotent per engine.
//!
//! ## Absolute paths
//!
//! `CodeQL` and similar tools emit `file://` URIs with absolute host paths
//! (e.g. `file:///home/runner/work/repo/src/Foo.java`). After scheme
//! stripping the `path` column stores the absolute form
//! (`/home/runner/work/repo/src/Foo.java`). The overlap join on
//! repo-relative hotspot paths will simply not match these rows, which is
//! the honest outcome — no silent rewriting that could produce false matches.

use std::fs;
use std::path::{Path, PathBuf};

use duckdb::{AccessMode, Config, Connection};

use crate::cache::repo_cache_dir;
use crate::quality_gates::ledger::now_utc_ts;
use crate::{CodeLoreError, Result};

use super::sarif_parse::ExternalFinding;

/// Filename of the sidecar store within the per-repo cache directory.
const STORE_FILENAME: &str = "external-findings.duckdb-ext";

/// DDL for the external findings table.
const CREATE_TABLE: &str = "
CREATE TABLE IF NOT EXISTS external_findings (
    engine          TEXT    NOT NULL,
    engine_version  TEXT    NOT NULL,
    rule_id         TEXT    NOT NULL,
    path            TEXT    NOT NULL,
    start_line      INTEGER,
    end_line        INTEGER,
    level           TEXT    NOT NULL,
    fingerprint     TEXT    NOT NULL,
    message         TEXT    NOT NULL,
    ingested_at     TEXT    NOT NULL,
    PRIMARY KEY (engine, fingerprint)
);
";

/// Sidecar `DuckDB` store owning its own `!Send + !Sync` `Connection`.
pub struct ExternalStore {
    conn: Connection,
    /// Path to the `.duckdb-ext` file (used in error messages).
    path: PathBuf,
}

impl ExternalStore {
    /// Open the sidecar store read-only if it already exists on disk.
    ///
    /// Returns `None` when the sidecar file is absent — no directory is created
    /// and no file is written. Use this in read paths (e.g. gate evaluation)
    /// where creating the sidecar as a side-effect would be wrong. The
    /// connection is opened with [`AccessMode::ReadOnly`], so it takes a shared
    /// lock and coexists with other readers; it never runs DDL, so a sidecar
    /// missing its table is returned as-is (queries against it will fail —
    /// prefer [`ExternalStore::open_nonempty`], which maps that case to `None`).
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] only if the file exists but cannot
    /// be opened read-only.
    pub fn open_existing(cache_root: &Path, repo_path: &Path) -> Result<Option<Self>> {
        let path = repo_cache_dir(cache_root, repo_path).join(STORE_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let conn = open_read_only(&path)?;
        Ok(Some(Self { conn, path }))
    }

    /// Open the sidecar store read-only only when it exists, is readable, **and**
    /// holds at least one finding.
    ///
    /// Returns `None` when the sidecar is absent, present-but-empty, or present
    /// but unusable (truncated/corrupt — missing the `external_findings` table).
    /// For a reader these are one "nothing to read" state: an empty sidecar
    /// carries no more information than a missing one, and a tableless sidecar
    /// cannot be healed under a read-only lock (only [`ExternalStore::open_or_create`]
    /// heals). The "absent, empty, and unreadable are equivalent" policy lives
    /// here; call sites keep their own presentation (skip notice, precondition
    /// error).
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] if the file exists but cannot be
    /// opened read-only, or if the table-presence probe itself fails for a
    /// reason other than the table being absent.
    pub fn open_nonempty(cache_root: &Path, repo_path: &Path) -> Result<Option<Self>> {
        match Self::open_existing(cache_root, repo_path)? {
            Some(store) if store.has_findings_table()? && store.count()? > 0 => Ok(Some(store)),
            _ => Ok(None),
        }
    }

    /// Whether the opened sidecar actually holds the `external_findings` table.
    ///
    /// A truncated or corrupt sidecar can open (the file exists) yet lack the
    /// table; readers open read-only and cannot heal it, so callers treat a
    /// `false` here as "unreadable, nothing to read". Probing
    /// `information_schema` keeps [`ExternalStore::count`]'s error path meaning a
    /// genuine query failure rather than a missing table.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] if the `information_schema` probe
    /// itself fails.
    fn has_findings_table(&self) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM information_schema.tables
                 WHERE table_name = 'external_findings'",
            )
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: prepare table probe in {}: {e}",
                    self.path.display()
                ))
            })?;
        let present: u64 = stmt.query_row([], |row| row.get(0)).map_err(|e| {
            CodeLoreError::Analysis(format!(
                "external store: table probe in {}: {e}",
                self.path.display()
            ))
        })?;
        Ok(present > 0)
    }

    /// Open or create the sidecar store for `repo_path` under `cache_root`.
    ///
    /// Creates the parent directory and the table schema on first call.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] if the directory cannot be created
    /// or the `DuckDB` connection/schema step fails.
    pub fn open_or_create(cache_root: &Path, repo_path: &Path) -> Result<Self> {
        let dir = repo_cache_dir(cache_root, repo_path);
        fs::create_dir_all(&dir).map_err(|e| {
            CodeLoreError::Analysis(format!("external store: create dir {}: {e}", dir.display()))
        })?;
        let path = dir.join(STORE_FILENAME);
        let conn = Connection::open(&path).map_err(|e| {
            CodeLoreError::Analysis(format!("external store: open {}: {e}", path.display()))
        })?;
        conn.execute_batch(CREATE_TABLE).map_err(|e| {
            CodeLoreError::Analysis(format!(
                "external store: create table in {}: {e}",
                path.display()
            ))
        })?;
        Ok(Self { conn, path })
    }

    /// Replace all findings for `engine` with `findings`.
    ///
    /// Wraps the DELETE + INSERT loop in a single transaction so a mid-loop
    /// process kill never leaves the engine's rows deleted without replacement.
    /// The operation is idempotent: re-ingesting the same file produces an
    /// identical row count.
    ///
    /// Returns the count of inserted rows.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] on any `DuckDB` error. On error the
    /// transaction is rolled back automatically when it is dropped.
    pub fn replace_engine(&self, engine: &str, findings: &[ExternalFinding]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction().map_err(|e| {
            CodeLoreError::Analysis(format!(
                "external store: begin transaction in {}: {e}",
                self.path.display()
            ))
        })?;

        tx.execute("DELETE FROM external_findings WHERE engine = ?", [engine])
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: delete engine {engine} in {}: {e}",
                    self.path.display()
                ))
            })?;

        if findings.is_empty() {
            tx.commit().map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: commit in {}: {e}",
                    self.path.display()
                ))
            })?;
            return Ok(0);
        }

        let ingested_at = now_utc_ts();
        let mut stmt = tx
            .prepare(
                "INSERT INTO external_findings
                 (engine, engine_version, rule_id, path, start_line, end_line,
                  level, fingerprint, message, ingested_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (engine, fingerprint) DO UPDATE SET
                   engine_version = excluded.engine_version,
                   rule_id        = excluded.rule_id,
                   path           = excluded.path,
                   start_line     = excluded.start_line,
                   end_line       = excluded.end_line,
                   level          = excluded.level,
                   message        = excluded.message,
                   ingested_at    = excluded.ingested_at",
            )
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: prepare insert in {}: {e}",
                    self.path.display()
                ))
            })?;

        for f in findings {
            stmt.execute(duckdb::params![
                f.engine,
                f.engine_version,
                f.rule_id,
                f.path,
                f.start_line,
                f.end_line,
                f.level,
                f.fingerprint,
                f.message,
                ingested_at,
            ])
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: insert finding {} in {}: {e}",
                    f.fingerprint,
                    self.path.display()
                ))
            })?;
        }

        drop(stmt);
        tx.commit().map_err(|e| {
            CodeLoreError::Analysis(format!(
                "external store: commit in {}: {e}",
                self.path.display()
            ))
        })?;

        Ok(findings.len())
    }

    /// Count all findings currently stored across all engines.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] on `DuckDB` error.
    pub fn count(&self) -> Result<u64> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM external_findings")
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: prepare count in {}: {e}",
                    self.path.display()
                ))
            })?;
        let count: u64 = stmt.query_row([], |row| row.get(0)).map_err(|e| {
            CodeLoreError::Analysis(format!(
                "external store: count in {}: {e}",
                self.path.display()
            ))
        })?;
        Ok(count)
    }

    /// Read all findings grouped by file path.
    ///
    /// Returns a map from `path` → `(engines, finding_count, worst_level)` where:
    /// - `engines` is the sorted, deduplicated list of engine names that flagged
    ///   the path
    /// - `finding_count` is the total number of findings across all engines
    /// - `worst_level` is the most severe level present (`"error"` > `"warning"` >
    ///   `"note"`)
    ///
    /// The caller uses this map for a Rust-side join against the behavioral
    /// analyses (hotspots, code-health): the sidecar and fact-store each own a
    /// `!Send + !Sync` `DuckDB` connection, so the two are never joined across a
    /// single connection — the join happens in Rust instead.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] on `DuckDB` error.
    pub fn findings_by_path(&self) -> Result<std::collections::HashMap<String, PathFindings>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, engine, level
                 FROM external_findings
                 ORDER BY path, engine",
            )
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: prepare findings_by_path in {}: {e}",
                    self.path.display()
                ))
            })?;

        let mut map: std::collections::HashMap<String, PathFindings> =
            std::collections::HashMap::new();

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: query findings_by_path in {}: {e}",
                    self.path.display()
                ))
            })?;

        for row in rows {
            let (path, engine, level) = row.map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: read row in findings_by_path: {e}"
                ))
            })?;
            let entry = map.entry(path).or_default();
            entry.count += 1;
            if !entry.engines.contains(&engine) {
                entry.engines.push(engine);
            }
            entry.worst_level = worse_level(&entry.worst_level, &level);
        }

        Ok(map)
    }

    /// Path to the sidecar store file (for printing in diagnostics).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Open an existing sidecar file read-only.
///
/// Uses [`AccessMode::ReadOnly`] so the connection takes a shared lock and
/// coexists with other readers instead of taking the exclusive write lock a
/// default open would. The caller has already checked that `path` exists.
fn open_read_only(path: &Path) -> Result<Connection> {
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .map_err(|e| {
            CodeLoreError::Analysis(format!(
                "external store: configure read-only access for {}: {e}",
                path.display()
            ))
        })?;
    Connection::open_with_flags(path, config).map_err(|e| {
        CodeLoreError::Analysis(format!(
            "external store: open read-only {}: {e}",
            path.display()
        ))
    })
}

/// Per-path aggregation produced by [`ExternalStore::findings_by_path`].
#[derive(Debug, Default, Clone)]
pub struct PathFindings {
    /// All engine names that flagged this path (deduplicated, insertion order).
    pub engines: Vec<String>,
    /// Total findings across all engines.
    pub count: usize,
    /// Most severe level: `"error"` > `"warning"` > `"note"`.
    pub worst_level: String,
}

/// Returns the more severe of two level strings.
/// Severity order: `"error"` > `"warning"` > anything else (treated as `"note"`).
fn worse_level(a: &str, b: &str) -> String {
    fn rank(s: &str) -> u8 {
        match s {
            "error" => 2,
            "warning" => 1,
            _ => 0,
        }
    }
    if rank(b) > rank(a) || a.is_empty() {
        b.to_owned()
    } else {
        a.to_owned()
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    /// Sidecar path a store would use for `repo_path` under `cache_root`,
    /// created eagerly so the test can seed or corrupt the file directly.
    fn sidecar_path(cache_root: &Path, repo_path: &Path) -> PathBuf {
        let dir = repo_cache_dir(cache_root, repo_path);
        fs::create_dir_all(&dir).expect("create cache dir");
        dir.join(STORE_FILENAME)
    }

    /// Two read-only readers must open the same sidecar at once. A read-write
    /// open takes an exclusive lock; read-only takes a shared lock, so the
    /// second open here would fail if the readers were not read-only.
    #[test]
    fn concurrent_readers_coexist_on_one_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Path::new("/test/repo");

        // Seed a non-empty sidecar via the writer, then drop the writer so its
        // exclusive lock is released before the readers open.
        let writer = ExternalStore::open_or_create(dir.path(), repo).expect("open_or_create");
        writer
            .replace_engine(
                "semgrep",
                &[ExternalFinding {
                    engine: "semgrep".into(),
                    engine_version: "1.0".into(),
                    rule_id: "r/1".into(),
                    path: "src/a.rs".into(),
                    start_line: Some(1),
                    end_line: None,
                    level: "warning".into(),
                    fingerprint: "fp-1".into(),
                    message: "m".into(),
                }],
            )
            .expect("seed finding");
        drop(writer);

        let reader_a = ExternalStore::open_existing(dir.path(), repo)
            .expect("open reader a")
            .expect("sidecar exists");
        let reader_b = ExternalStore::open_existing(dir.path(), repo)
            .expect("open reader b — second read-only open must not be blocked")
            .expect("sidecar exists");

        assert_eq!(reader_a.count().expect("count a"), 1);
        assert_eq!(reader_b.count().expect("count b"), 1);
    }

    /// A sidecar file that opens but lacks the `external_findings` table
    /// (truncated/corrupt, or written by another tool) is unusable for a reader:
    /// it cannot be healed under a read-only lock. `open_nonempty` must map it to
    /// `None` rather than erroring on the count query.
    #[test]
    fn tableless_sidecar_maps_to_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Path::new("/test/repo");
        let path = sidecar_path(dir.path(), repo);

        // Create a valid DuckDB file at the sidecar path with *some* table but
        // not `external_findings` — simulating a corrupt/foreign sidecar.
        {
            let conn = Connection::open(&path).expect("create tableless duckdb file");
            conn.execute_batch("CREATE TABLE unrelated (x INTEGER)")
                .expect("create unrelated table");
        }

        // The file exists, so open_existing returns Some, but the table probe
        // must report the findings table as absent.
        let store = ExternalStore::open_existing(dir.path(), repo)
            .expect("open_existing on tableless file")
            .expect("file exists so Some");
        assert!(
            !store.has_findings_table().expect("table probe"),
            "external_findings table must be reported absent"
        );

        // For a reader the whole file collapses to None — nothing to read.
        let reader = ExternalStore::open_nonempty(dir.path(), repo)
            .expect("open_nonempty must not error on a tableless sidecar");
        assert!(
            reader.is_none(),
            "a tableless sidecar must read as None, not error"
        );
    }

    /// A properly created but empty sidecar (table present, zero rows) also reads
    /// as `None` through `open_nonempty` — the reader has nothing to act on.
    #[test]
    fn empty_but_valid_sidecar_maps_to_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Path::new("/test/repo");

        // open_or_create makes the table but inserts no rows.
        let writer = ExternalStore::open_or_create(dir.path(), repo).expect("open_or_create");
        assert_eq!(writer.count().expect("count"), 0);
        drop(writer);

        let reader = ExternalStore::open_nonempty(dir.path(), repo).expect("open_nonempty");
        assert!(reader.is_none(), "empty sidecar must read as None");
    }
}
