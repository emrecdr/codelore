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
//! (`/home/runner/work/repo/src/Foo.java`). The B3 overlap join on
//! repo-relative hotspot paths will simply not match these rows, which is
//! the honest outcome — no silent rewriting that could produce false matches.

use std::fs;
use std::path::{Path, PathBuf};

use duckdb::Connection;

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
    /// Deletes all existing rows where `engine = ?`, then inserts each
    /// finding in `findings`. The operation is idempotent: re-ingesting the
    /// same file produces an identical row count.
    ///
    /// Returns the count of inserted rows.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] on any `DuckDB` error.
    pub fn replace_engine(&self, engine: &str, findings: &[ExternalFinding]) -> Result<usize> {
        self.conn
            .execute("DELETE FROM external_findings WHERE engine = ?", [engine])
            .map_err(|e| {
                CodeLoreError::Analysis(format!(
                    "external store: delete engine {engine} in {}: {e}",
                    self.path.display()
                ))
            })?;

        if findings.is_empty() {
            return Ok(0);
        }

        let ingested_at = now_utc_ts();
        let mut stmt = self
            .conn
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
    /// analyses (hotspots, code-health) — keeping the two `!Send + !Sync`
    /// `DuckDB` connections separate, per R7.
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
