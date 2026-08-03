//! HEAD-time clone detection. Fingerprints every function in every live-at-HEAD
//! Tier-1 source file, groups members by structural digest, and bulk-inserts one
//! row per clone-family member into the `clones` table for the clone-coupling
//! analysis to join against.

use super::FactsDb;
use crate::{CodeLoreError, Options, Result};

impl FactsDb {
    /// Walk the working tree at HEAD, fingerprint every
    /// function in every Tier-1 file, group by structural digest, and INSERT
    /// one row per clone-family member into the `clones` table. Returns the
    /// number of rows inserted (0 if no clones found or no Tier-1 sources).
    ///
    /// Honors `opts.min_clone_node_count` (default 30) and `opts.exclude_patterns`
    /// (built from `--exclude` flags + `.codeloreignore`).
    pub(super) fn populate_clones_at_head<R: crate::repo::Repo>(
        &self,
        repo: &R,
        opts: &Options,
        live_paths: &[String],
        head_rev: &str,
    ) -> Result<usize> {
        use crate::clones::{CloneLanguage, extract_functions, group_clones};
        use rayon::prelude::*;

        // `live_paths` + `head_rev` are computed once by the caller and shared
        // across all four HEAD-time passes. Source-of-truth pattern: paths the
        // ingest already accepted (`PathsFilter` ran before any row landed in
        // `changes`), so we don't re-apply `--exclude` / `.gitignore` /
        // `.codeloreignore` here. Bare-repo safe because the query reads from
        // the fact store, not a working tree.
        let candidates: Vec<(String, CloneLanguage)> = live_paths
            .iter()
            .filter_map(|rel| {
                let lang = CloneLanguage::from_path(std::path::Path::new(rel))?;
                Some((rel.clone(), lang))
            })
            .collect();

        // Phase 2 (parallel): read each file + run tree-sitter fingerprinting on
        // the rayon pool. Mirrors the complexity pass above. Unreadable files
        // silently yield no fingerprints; extract errors short-circuit the pass
        // via `collect::<Result<_>>`.
        let per_file: Vec<Vec<_>> = candidates
            .into_par_iter()
            .map_init(
                || repo.blob_reader_at("HEAD"),
                |reader, (rel, lang)| -> Result<Vec<_>> {
                    // Read the blob at HEAD via the Repo trait. Bare-repo
                    // safe and ignores dirty-tree edits. Backends without
                    // blob support return Ok(None) — same skip behaviour as
                    // the disk-not-found case the previous let-Ok-else
                    // handled.
                    let code = match reader.read(&rel) {
                        Ok(Some(code)) => code,
                        Ok(None) => {
                            // Path not tracked at HEAD; skip (non-fatal, the
                            // rest of the scan continues).
                            tracing::debug!("clones: {rel} not tracked at HEAD; skipping");
                            return Ok(Vec::new());
                        }
                        Err(e) => {
                            // Object-database error (corrupted pack, missing
                            // shallow object). Surface as a warning and skip
                            // — the rest of the scan can still complete.
                            tracing::warn!("clones: blob read failed for {rel}: {e}");
                            return Ok(Vec::new());
                        }
                    };
                    // Skip oversized files (generated / minified) before
                    // tree-sitter to avoid OOM / stack-overflow on deeply
                    // nested generated code. Same cap as complexity pass.
                    if code.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                        tracing::debug!(
                            "clones: skipping {rel} ({size} bytes > {cap}-byte AST cap)",
                            size = code.len(),
                            cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                        );
                        return Ok(Vec::new());
                    }
                    extract_functions(&rel, &code, lang)
                        .map_err(|e| CodeLoreError::Analysis(format!("clones: extract {rel}: {e}")))
                },
            )
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
