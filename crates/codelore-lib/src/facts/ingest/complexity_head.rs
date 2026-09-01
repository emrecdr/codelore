//! HEAD-time complexity scan. Reads each live-at-HEAD Tier-1 source blob,
//! runs the tree-sitter complexity analyzer in parallel, de-duplicates the
//! resulting entities, then drains them into the `entities` and
//! `complexity_metrics` tables on the connection-owning thread.

use super::FactsDb;
use super::consumer::{append_entity_row, append_metric_row, dedup_entities};
use super::coverage::{REASON_BLOB_READ, REASON_PARSE_ERROR, ScanCoverage, ScanOutcome};
use crate::{CodeLoreError, Options, Result};

impl FactsDb {
    pub(super) fn ingest_complexity_at_head<R: crate::repo::Repo>(
        &self,
        repo: &R,
        // Because the complexity pass reads blobs instead of disk, it
        // no longer needs `opts.repo_path` — every file is sourced via
        // a `Repo::blob_reader_at("HEAD")` reader. Kept on the signature
        // for forward compatibility (future per-language flags may need it).
        _opts: &Options,
        live_paths: &[String],
        head_rev: &str,
    ) -> Result<()> {
        use crate::complexity::{Tier1Language, compute_for_file};
        use rayon::prelude::*;

        // Caller hoisted `query_live_paths` + `current_head_rev` to compute-once;
        // clone the slice into an owned `Vec` so the existing `into_par_iter`
        // shape below is unchanged. The slice clone is sub-ms vs ~10-100ms per
        // skipped SQL execution.
        let live_paths: Vec<String> = live_paths.to_vec();

        // ── Parallel pass ────────────────────────────────────────────────────────
        // Each worker thread builds one `BlobReader` via `map_init` (resolves
        // HEAD's root tree once, then reuses a warm object-decode cache for
        // every file that worker reads) and uses it to read the blob,
        // dispatch the tree-sitter parser, and de-duplicate entities.
        // Per-file failures are logged via `tracing::warn!` but do NOT abort the
        // parallel scan; they surface as [`ScanOutcome::Lost`] and are tallied
        // below so a scan that loses most of its files is disclosed rather than
        // silently thin.
        let outcomes: Vec<ScanOutcome<(String, Vec<crate::complexity::ComplexityEntity>)>> =
            live_paths
                .into_par_iter()
                .map_init(
                    || repo.blob_reader_at("HEAD"),
                    |reader, path| {
                        let Some(lang) = Tier1Language::from_path(&path) else {
                            return ScanOutcome::NotCounted;
                        };
                        // Prefer the blob at HEAD (works on bare repos, ignores
                        // dirty-tree edits). Fall back to disk if the Repo
                        // backend doesn't implement blob reads OR if the path
                        // exists on disk but isn't tracked at HEAD (a freshly
                        // ingested commit may have added paths not yet in any
                        // tree the backend has cached).
                        let source = match reader.read(&path) {
                            Ok(Some(b)) => b,
                            Ok(None) => {
                                // Path not tracked at HEAD; skip (matches the
                                // HEAD-time scan semantic of "current files only").
                                tracing::debug!("complexity: {path} not tracked at HEAD; skipping");
                                return ScanOutcome::NotCounted;
                            }
                            Err(e) => {
                                // Object-database error (corrupted pack, missing
                                // shallow object, or a blobless partial clone whose
                                // promisor blobs were never fetched). Surface as a
                                // warning and skip — the rest of the scan can still
                                // complete — but count it, because a scan that
                                // loses most of its files this way must not be
                                // indistinguishable from a small repository.
                                tracing::warn!("complexity: blob read failed for {path}: {e}");
                                return ScanOutcome::Lost(REASON_BLOB_READ);
                            }
                        };
                        // Skip oversized files before handing to tree-sitter.
                        // Without this guard, deeply-nested generated/minified
                        // files (sqlite3.c, .pb.cc, minified .js) can OOM or
                        // stack-overflow the AST walker. Log at debug — minified
                        // bundles in `node_modules`-style layouts are the common
                        // case and we'd otherwise drown the console.
                        if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
                            tracing::debug!(
                                "complexity: skipping {path} ({size} bytes > {cap}-byte AST cap; \
                             likely generated/minified; excluded from complexity metrics)",
                                size = source.len(),
                                cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
                            );
                            return ScanOutcome::SkippedOversize;
                        }
                        // Path only used for error reporting in compute_for_file;
                        // the repo-relative form is more useful than the absolute
                        // working-tree path anyway.
                        let synth_path = std::path::Path::new(&path);
                        let entities = match compute_for_file(synth_path, source, lang) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("complexity: parse error {path}: {e}");
                                return ScanOutcome::Lost(REASON_PARSE_ERROR);
                            }
                        };
                        let deduped = dedup_entities(entities);
                        ScanOutcome::Scored((path, deduped))
                    },
                )
                .collect();

        // ── Coverage disclosure ──────────────────────────────────────────────────
        // Every eligible file the scan could not score is counted, not just
        // logged per-file. A per-file `warn!` is invisible at the default
        // `EnvFilter` level on a big repo and says nothing about proportion;
        // the aggregate is what distinguishes "two generated files were
        // skipped" from "this scan went blind". Routine outcomes — non-Tier-1
        // files, paths history carries that HEAD no longer tracks, and files
        // past the AST size cap — are excluded from the denominator; counting
        // them put this repository at 86% on a scan that lost nothing.
        let coverage = ScanCoverage::tally(&outcomes);
        coverage.warn_if_degraded("complexity", "complexity_metrics");
        coverage.warn_if_mostly_oversize("complexity", "complexity_metrics");

        // Collapse back to the shape the serial drain already consumes. The
        // drain is unchanged; only the classification above is new.
        let batches: Vec<Option<(String, Vec<crate::complexity::ComplexityEntity>)>> = outcomes
            .into_iter()
            .map(|o| match o {
                ScanOutcome::Scored((path, entities)) => Some((path, entities)),
                ScanOutcome::NotCounted | ScanOutcome::SkippedOversize | ScanOutcome::Lost(_) => {
                    None
                }
            })
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
            let Some((path, entities)) = batch else {
                continue;
            };
            for ent in &entities {
                append_entity_row(&mut entities_app, &path, ent, head_rev)?;
                append_metric_row(&mut metrics_app, &path, ent, head_rev)?;
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
}
