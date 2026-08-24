//! HEAD-time complexity scan. Reads each live-at-HEAD Tier-1 source blob,
//! runs the tree-sitter complexity analyzer in parallel, de-duplicates the
//! resulting entities, then drains them into the `entities` and
//! `complexity_metrics` tables on the connection-owning thread.

use super::FactsDb;
use super::consumer::{append_entity_row, append_metric_row, dedup_entities};
use crate::{CodeLoreError, Options, Result};

/// Proportion of eligible files the scan must score before the run is
/// considered healthy. Below this, the fact store is thin enough that every
/// analysis reading `complexity_metrics` — and every gate reading those
/// analyses — is drawing conclusions from a minority of the codebase.
const MIN_SCAN_COVERAGE: f64 = 0.9;

const REASON_NOT_TRACKED: &str = "not tracked at HEAD";
const REASON_BLOB_READ: &str = "blob read failed";
const REASON_SIZE_CAP: &str = "exceeds the AST size cap";
const REASON_PARSE_ERROR: &str = "parse error";

/// What the HEAD scan did with one file.
///
/// The distinction that matters is [`Ineligible`](ScanOutcome::Ineligible) vs
/// [`Skipped`](ScanOutcome::Skipped). Both produce no `complexity_metrics` row,
/// but only the second is a coverage loss: a `README.md` is *supposed* to score
/// nothing, whereas a `.rs` file whose blob would not read is a file the scan
/// owed the user and could not deliver. Collapsing the two — which a bare
/// `Option` does — is what lets a scan that reached 200 of 5,200 files look
/// exactly like a small repository.
enum ScanOutcome {
    /// Not a Tier-1 source file. Expected; excluded from the denominator.
    Ineligible,
    /// Eligible, but the scan could not score it. Carries the reason so the
    /// aggregate can say *why* coverage was lost rather than only that it was.
    Skipped(&'static str),
    /// Scored: `(path, deduped_entities)`.
    Scored(String, Vec<crate::complexity::ComplexityEntity>),
}

/// Aggregate coverage of one HEAD scan.
struct ScanCoverage {
    eligible: usize,
    scored: usize,
    /// Skip counts by reason, most frequent first when rendered.
    by_reason: Vec<(&'static str, usize)>,
}

impl ScanCoverage {
    fn tally(outcomes: &[ScanOutcome]) -> Self {
        let mut scored = 0usize;
        let mut counts: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for o in outcomes {
            match o {
                ScanOutcome::Scored(..) => scored += 1,
                ScanOutcome::Skipped(reason) => *counts.entry(reason).or_default() += 1,
                ScanOutcome::Ineligible => {}
            }
        }
        let skipped: usize = counts.values().sum();
        let mut by_reason: Vec<(&'static str, usize)> = counts.into_iter().collect();
        by_reason.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        Self {
            eligible: scored + skipped,
            scored,
            by_reason,
        }
    }

    /// Fraction of eligible files that produced metrics. Vacuously 1.0 when the
    /// repository carries no Tier-1 source at all — a docs-only tree is honestly
    /// complete, not degraded.
    #[allow(clippy::cast_precision_loss)]
    fn ratio(&self) -> f64 {
        if self.eligible == 0 {
            1.0
        } else {
            self.scored as f64 / self.eligible as f64
        }
    }

    /// Emit one aggregate warning when coverage falls below the floor.
    ///
    /// Deliberately `warn!` and not `debug!`: the default `EnvFilter` is `warn`,
    /// so this is the one level at which the message reaches a user who did not
    /// opt into logging. The per-file messages stay where they are — they say
    /// *which* file, this says *how much of the repository is missing*.
    fn warn_if_degraded(&self) {
        if self.eligible == 0 || self.ratio() >= MIN_SCAN_COVERAGE {
            return;
        }
        let detail = self
            .by_reason
            .iter()
            .map(|(reason, n)| format!("{n} {reason}"))
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            "complexity scan covered {scored}/{eligible} eligible source files \
             ({pct:.0}%); {detail}. Analyses and quality gates that read complexity \
             are drawing on a minority of this repository. A blobless partial clone \
             (`git clone --filter=blob:none`, or `actions/checkout` with a filter) is \
             the usual cause and is not detected by the shallow-clone check, because \
             such a clone has complete commit history.",
            scored = self.scored,
            eligible = self.eligible,
            pct = self.ratio() * 100.0,
        );
    }
}

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
        // parallel scan; they surface as [`ScanOutcome::Skipped`] and are tallied
        // below so a scan that loses most of its files is disclosed rather than
        // silently thin.
        let outcomes: Vec<ScanOutcome> = live_paths
            .into_par_iter()
            .map_init(
                || repo.blob_reader_at("HEAD"),
                |reader, path| {
                    let Some(lang) = Tier1Language::from_path(&path) else {
                        return ScanOutcome::Ineligible;
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
                            return ScanOutcome::Skipped(REASON_NOT_TRACKED);
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
                            return ScanOutcome::Skipped(REASON_BLOB_READ);
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
                        return ScanOutcome::Skipped(REASON_SIZE_CAP);
                    }
                    // Path only used for error reporting in compute_for_file;
                    // the repo-relative form is more useful than the absolute
                    // working-tree path anyway.
                    let synth_path = std::path::Path::new(&path);
                    let entities = match compute_for_file(synth_path, source, lang) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("complexity: parse error {path}: {e}");
                            return ScanOutcome::Skipped(REASON_PARSE_ERROR);
                        }
                    };
                    let deduped = dedup_entities(entities);
                    ScanOutcome::Scored(path, deduped)
                },
            )
            .collect();

        // ── Coverage disclosure ──────────────────────────────────────────────────
        // Every eligible file the scan could not score is counted, not just
        // logged per-file. A per-file `warn!` is invisible at the default
        // `EnvFilter` level on a big repo and says nothing about proportion;
        // the aggregate is what distinguishes "two generated files were
        // skipped" from "this scan went blind". Ineligible (non-Tier-1) files
        // are excluded from the denominator — counting them would mark every
        // repository with a README as degraded.
        let coverage = ScanCoverage::tally(&outcomes);
        coverage.warn_if_degraded();

        // Collapse back to the shape the serial drain already consumes. The
        // drain is unchanged; only the classification above is new.
        let batches: Vec<Option<(String, Vec<crate::complexity::ComplexityEntity>)>> = outcomes
            .into_iter()
            .map(|o| match o {
                ScanOutcome::Scored(path, entities) => Some((path, entities)),
                ScanOutcome::Ineligible | ScanOutcome::Skipped(_) => None,
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

#[cfg(test)]
mod tests {
    use super::{MIN_SCAN_COVERAGE, REASON_BLOB_READ, REASON_PARSE_ERROR, ScanCoverage, ScanOutcome};

    fn scored(path: &str) -> ScanOutcome {
        ScanOutcome::Scored(path.to_string(), Vec::new())
    }

    #[test]
    fn ineligible_files_are_not_a_coverage_loss() {
        // A docs-only tree is honestly complete, not degraded. If `Ineligible`
        // counted toward the denominator, every repository with a README would
        // report a thin scan — which is the false positive that makes a
        // coverage sentinel unusable.
        let outcomes = vec![
            ScanOutcome::Ineligible,
            ScanOutcome::Ineligible,
            scored("src/lib.rs"),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(cov.eligible, 1, "only the Tier-1 file is eligible");
        assert_eq!(cov.scored, 1);
        assert!((cov.ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_source_less_tree_is_vacuously_complete() {
        let cov = ScanCoverage::tally(&[ScanOutcome::Ineligible]);
        assert_eq!(cov.eligible, 0);
        assert!(
            (cov.ratio() - 1.0).abs() < f64::EPSILON,
            "no eligible files must not read as 0% coverage"
        );
    }

    #[test]
    fn skips_lower_the_ratio_and_are_attributed_by_reason() {
        // The defect this guards: a scan that reached a minority of its files
        // must not be arithmetically indistinguishable from a small repository.
        let mut outcomes = vec![scored("a.rs")];
        for _ in 0..9 {
            outcomes.push(ScanOutcome::Skipped(REASON_BLOB_READ));
        }
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(cov.eligible, 10);
        assert_eq!(cov.scored, 1);
        assert!(
            (cov.ratio() - 0.1).abs() < 1e-9,
            "1 of 10 eligible files is 10% coverage, got {}",
            cov.ratio()
        );
        assert!(
            cov.ratio() < MIN_SCAN_COVERAGE,
            "10% coverage must fall below the floor that triggers disclosure"
        );
        assert_eq!(cov.by_reason, vec![(REASON_BLOB_READ, 9)]);
    }

    #[test]
    fn reasons_are_ranked_most_frequent_first() {
        let outcomes = vec![
            ScanOutcome::Skipped(REASON_PARSE_ERROR),
            ScanOutcome::Skipped(REASON_BLOB_READ),
            ScanOutcome::Skipped(REASON_BLOB_READ),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(
            cov.by_reason,
            vec![(REASON_BLOB_READ, 2), (REASON_PARSE_ERROR, 1)],
            "the dominant failure mode must be named first so the message leads with it"
        );
    }

    #[test]
    fn a_healthy_scan_stays_above_the_floor() {
        let outcomes = vec![
            scored("a.rs"),
            scored("b.rs"),
            scored("c.rs"),
            scored("d.rs"),
            scored("e.rs"),
            scored("f.rs"),
            scored("g.rs"),
            scored("h.rs"),
            scored("i.rs"),
            scored("j.rs"),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert!(cov.ratio() >= MIN_SCAN_COVERAGE);
    }
}
