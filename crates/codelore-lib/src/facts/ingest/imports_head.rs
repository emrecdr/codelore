//! HEAD-time import-edge extraction and resolution. The first pass tree-sitter
//! parses every live-at-HEAD Tier-1 source file for import statements and
//! inserts one unresolved row per edge into the `imports` table; the companion
//! resolver pass maps the raw targets to repo-relative tracked paths where the
//! per-language resolver succeeds.

use super::FactsDb;
use crate::{CodeLoreError, Options, Result};

impl FactsDb {
    /// Walk Tier-1 source files at HEAD, tree-sitter-parse each for
    /// import statements, and bulk-insert one row per import edge
    /// into the `imports` table. Returns the total row count for
    /// diagnostics.
    ///
    /// Mirrors `populate_clones_at_head`'s rayon-then-serial-drain
    /// shape: parallel blob-read + extraction, then a single Appender
    /// drain on the connection-owning thread (`DuckDB` Connection is
    /// `!Send + !Sync`). Per-file duplicates (same raw target listed
    /// twice in one file) are deduped before drain to honor the
    /// `(rev, src_path, target)` PRIMARY KEY.
    ///
    /// Every row lands with `resolved=false` / `target_path=NULL`;
    /// the companion `resolve_imports_at_head` UPDATE pass fills in
    /// resolvable targets immediately after.
    pub(super) fn populate_imports_at_head<R: crate::repo::Repo>(
        &self,
        repo: &R,
        _opts: &Options,
        live_paths: &[String],
        head_rev: &str,
    ) -> Result<usize> {
        use crate::imports::{ImportLanguage, RawImport, extract_imports};
        use rayon::prelude::*;
        use std::collections::HashSet;

        // Phase 1 (serial): caller-supplied path set, filtered to Tier-1
        // extensions. Same source-of-truth pattern as the complexity +
        // clones passes (works on bare repos via the gix ODB;
        // `PathsFilter` already ran at ingest time).
        let candidates: Vec<(String, ImportLanguage)> = live_paths
            .iter()
            .filter_map(|rel| {
                let lang = ImportLanguage::from_path(std::path::Path::new(rel))?;
                Some((rel.clone(), lang))
            })
            .collect();

        // Phase 2 (parallel): read blob + extract imports per file.
        // Errors are logged at warn / debug and the file skipped —
        // a single malformed file shouldn't fail the whole ingest.
        let per_file: Vec<(String, Vec<RawImport>)> = candidates
            .into_par_iter()
            .filter_map(|(rel, lang)| {
                let Ok(Some(code)) = repo.read_blob_at_head(&rel) else {
                    tracing::debug!("imports: {rel} not tracked at HEAD; skipping");
                    return None;
                };
                if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                    tracing::debug!(
                        "imports: skipping {rel} ({size} bytes > {cap}-byte AST cap)",
                        size = code.len(),
                        cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                    );
                    return None;
                }
                let imports = match extract_imports(&code, lang) {
                    Ok(v) if !v.is_empty() => v,
                    Ok(_) => return None,
                    Err(e) => {
                        tracing::warn!("imports: extract failed for {rel}: {e}");
                        return None;
                    }
                };
                Some((rel, imports))
            })
            .collect();

        // Phase 3 (serial drain): bulk-insert via the DuckDB Appender
        // on the connection-owning thread. Dedup within each file's
        // target set so the (rev, src_path, target) PK isn't violated
        // by a file that lists the same raw target twice (rare but
        // legal in JS dynamic-import patterns).
        let mut app = self
            .conn()
            .appender("imports")
            .map_err(|e| CodeLoreError::Analysis(format!("appender imports: {e}")))?;
        let mut rows_inserted = 0usize;
        for (path, imports) in per_file {
            let mut seen = HashSet::new();
            for imp in &imports {
                if !seen.insert(imp.target.clone()) {
                    continue;
                }
                app.append_row(duckdb::params![
                    head_rev,
                    &path,
                    &imp.target,
                    false,
                    Option::<&str>::None,
                    imp.kind.as_str(),
                ])
                .map_err(|e| CodeLoreError::Analysis(format!("append imports row: {e}")))?;
                rows_inserted += 1;
            }
        }
        app.flush()
            .map_err(|e| CodeLoreError::Analysis(format!("flush imports appender: {e}")))?;
        Ok(rows_inserted)
    }

    /// Resolver pass: for every import row whose `target_path` is
    /// still NULL, attempt a per-language resolution against the
    /// live-at-HEAD tracked path set. On a hit, UPDATE the row to
    /// set `resolved=TRUE` and the canonical `target_path`. Returns
    /// the total number of rows successfully resolved.
    ///
    /// Covers Rust `crate::` / `self::` / `super::` paths, Python
    /// relative imports, and JS/TS `./` / `../` paths today. Java
    /// FQN → filesystem-path mapping is project-layout-specific
    /// and is not attempted here.
    pub(super) fn resolve_imports_at_head(
        &self,
        live_paths: &[String],
        head_rev: &str,
    ) -> Result<usize> {
        use crate::imports::{resolve_js_relative, resolve_python_relative, resolve_rust_path};

        // 1. Build the live-at-HEAD path set as a `HashSet` for O(1)
        //    candidate lookups inside the resolver. Caller hoisted
        //    `query_live_paths` to compute-once across all HEAD-time
        //    passes; we just lift the slice into a hash set here.
        let live_paths_set: std::collections::HashSet<&str> =
            live_paths.iter().map(String::as_str).collect();

        // 2. Pull every unresolved import row scoped to a language
        //    the multi-language resolver supports (JS/TS, Python,
        //    Rust). Java FQNs are project-layout-specific and stay
        //    deferred until a Java-specific resolver lands.
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT src_path, target FROM imports \
                 WHERE target_path IS NULL \
                   AND (
                       src_path LIKE '%.js' OR src_path LIKE '%.jsx' OR
                       src_path LIKE '%.mjs' OR src_path LIKE '%.cjs' OR
                       src_path LIKE '%.ts' OR src_path LIKE '%.tsx' OR
                       src_path LIKE '%.py' OR src_path LIKE '%.pyi' OR
                       src_path LIKE '%.rs'
                   )",
            )
            .map_err(|e| CodeLoreError::Analysis(format!("prepare imports scan: {e}")))?;
        let candidates: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| CodeLoreError::Analysis(format!("query imports scan: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect imports scan: {e}")))?;

        // 3. Dispatch to the per-language resolver by file extension.
        //    First-match wins; falls through to `None` for external
        //    targets the resolver can't map to a tracked path.
        //
        //    The resolvers want an owned-string hash set; we adapt the
        //    `&str` set above into a single shared owned copy so each
        //    resolver call gets the same `&HashSet<String>` without
        //    rebuilding it per row.
        let live_paths_owned: std::collections::HashSet<String> =
            live_paths_set.iter().map(|s| (*s).to_string()).collect();
        let mut hits: Vec<(String, String, String)> = Vec::new();
        for (src_path, target) in candidates {
            let ext = std::path::Path::new(&src_path)
                .extension()
                .and_then(std::ffi::OsStr::to_str);
            let resolved = match ext {
                Some("rs") => resolve_rust_path(&src_path, &target, &live_paths_owned),
                Some("py" | "pyi") => {
                    resolve_python_relative(&src_path, &target, &live_paths_owned)
                }
                Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx") => {
                    resolve_js_relative(&src_path, &target, &live_paths_owned)
                }
                _ => None,
            };
            if let Some(resolved_target_path) = resolved {
                hits.push((resolved_target_path, src_path, target));
            }
        }

        if hits.is_empty() {
            return Ok(0);
        }

        // 4. Apply all resolved hits via a single hash-joined UPDATE…FROM
        //    instead of N independent UPDATEs. Each per-row UPDATE in the
        //    old shape forced DuckDB to scan `imports` end-to-end (the
        //    `(rev, src_path, target)` PK is not a clustered index in
        //    DuckDB), making the pass O(N × |imports|). The temp-table
        //    join shape is one hash build + one streaming pass.
        //
        //    `CREATE OR REPLACE TEMPORARY TABLE` mirrors the lineage /
        //    grouping passes elsewhere in this file.
        self.conn()
            .execute_batch(
                "CREATE OR REPLACE TEMPORARY TABLE _resolved_imports (
                     target_path TEXT,
                     src_path    TEXT,
                     target      TEXT
                 )",
            )
            .map_err(|e| CodeLoreError::Analysis(format!("create resolved temp table: {e}")))?;
        {
            let mut app = self
                .conn()
                .appender("_resolved_imports")
                .map_err(|e| CodeLoreError::Analysis(format!("appender resolved: {e}")))?;
            for (target_path, src_path, target) in &hits {
                app.append_row(duckdb::params![target_path, src_path, target])
                    .map_err(|e| CodeLoreError::Analysis(format!("append resolved row: {e}")))?;
            }
            app.flush()
                .map_err(|e| CodeLoreError::Analysis(format!("flush resolved appender: {e}")))?;
        }

        let updated = self
            .conn()
            .execute(
                "UPDATE imports
                    SET resolved = TRUE,
                        target_path = r.target_path
                   FROM _resolved_imports r
                  WHERE imports.rev = ?
                    AND imports.src_path = r.src_path
                    AND imports.target = r.target",
                duckdb::params![head_rev],
            )
            .map_err(|e| CodeLoreError::Analysis(format!("bulk imports update: {e}")))?;

        self.conn()
            .execute_batch("DROP TABLE _resolved_imports")
            .map_err(|e| CodeLoreError::Analysis(format!("drop resolved temp table: {e}")))?;

        Ok(updated)
    }
}
