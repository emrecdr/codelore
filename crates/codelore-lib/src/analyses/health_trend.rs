//! Repo Health Timeline: architectural, code, and combined health (each 0–100,
//! higher = healthier) across evenly-spaced historical revisions. Reuses the
//! `architecture_trend` sampler for the rev set and the rev-parameterizable
//! `code_health` engine for the per-rev code score. On-demand, never cached.

use crate::analyses::architecture_trend::{
    import_graph_from_live_paths, live_paths_at, sampled_commits,
};
use crate::analyses::code_health::{CodeHealthRow, HealthScanCtx, run_code_health_scoped};
use crate::analyses::import_graph::{GraphMetrics, graph_metrics};
use crate::facts::FactsDb;
use crate::facts::ingest::at_rev::{ingest_complexity_at_rev, materialize_imports_at_rev};
use crate::repo::Repo;
use crate::{Options, Result};

/// One sampled revision's three health scores + bands. Emitted oldest-first.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthTrendRow {
    /// `YYYY-MM-DD` prefix of the commit timestamp.
    pub date: String,
    /// First 12 chars of the commit SHA.
    pub rev: String,
    /// Nodes in the resolved import graph at this rev.
    pub files: u32,
    /// Architectural health 0..=100 (structural only — no complexity).
    pub arch_health: f64,
    /// Code health 0..=100 (mean of per-file code-health scores, DRY excluded).
    pub code_health: f64,
    /// Combined health 0..=100 = mean of arch + code.
    pub combined_health: f64,
    pub arch_band: String,
    pub code_band: String,
    pub combined_band: String,
}

/// Shared band for all three scores: green ≥ 70, yellow ≥ 40, else red.
#[must_use]
pub fn health_band(score: f64) -> &'static str {
    if score >= 70.0 {
        "green"
    } else if score >= 40.0 {
        "yellow"
    } else {
        "red"
    }
}

/// Architectural health from the per-rev import-graph metrics. Purely
/// structural: propagation cost (dominant) plus the fraction of the codebase
/// tangled in cycles and the span of the single largest tangle. An empty graph
/// (`n == 0`) is trivially healthy (nothing to be unhealthy about).
#[must_use]
pub fn arch_health(m: &GraphMetrics) -> f64 {
    if m.n == 0 {
        return 100.0;
    }
    let n = f64::from(u32::try_from(m.n).unwrap_or(u32::MAX));
    let arch_risk = 0.5 * m.propagation_cost
        + 0.3 * (f64::from(m.cyclic_nodes) / n)
        + 0.2 * (f64::from(m.largest_cycle) / n);
    100.0 * (1.0 - arch_risk.min(1.0))
}

/// Repo-level code health for one rev: the arithmetic mean of the per-file
/// code-health scores (all files, un-truncated). No files scored ⇒ 100.
#[must_use]
pub(crate) fn repo_code_health(rows: &[CodeHealthRow]) -> f64 {
    if rows.is_empty() {
        return 100.0;
    }
    let sum: f64 = rows.iter().map(|r| r.score).sum();
    let count = f64::from(u32::try_from(rows.len()).unwrap_or(u32::MAX));
    sum / count
}

/// Combined health: equal blend of systemic (architecture) and local (code).
#[must_use]
pub(crate) fn combined_health(arch: f64, code: f64) -> f64 {
    0.5 * arch + 0.5 * code
}

/// Session-scoped temp-table names the rev-scoped `HealthScanCtx` points at.
/// `CREATE OR REPLACE` inside the helpers means reusing them across samples is
/// safe — each iteration replaces the prior rev's contents.
const CM_AT_REV: &str = "cm_at_rev";
const IMPORTS_AT_REV: &str = "imports_at_rev";

/// Compute the three health scores across ≤12 evenly-spaced historical revs.
///
/// Per sample: build the in-memory import graph → `arch_health`; materialize
/// rev-scoped complexity + imports temp tables and run the (DRY-excluded,
/// date-cut) code-health engine → mean per-file score = `code_health`;
/// `combined = mean(arch, code)`. Every sample — including the newest — is
/// computed the same reduced way, so the series is internally consistent (it
/// may sit slightly above the standalone HEAD `code-health` number, which
/// includes DRY + full external fan-out).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on any query / ingest failure.
#[tracing::instrument(name = "health-trend", skip_all)]
pub fn run_health_trend<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
) -> Result<Vec<HealthTrendRow>> {
    let samples = sampled_commits(db)?;
    // ALL files must feed the code-health mean — never the user's `--rows` cut.
    let scan_opts = opts.with_no_row_limit();
    let mut rows = Vec::with_capacity(samples.len());
    for (rev, ts) in &samples {
        // Resolve the live-at-`ts` path set once — both the import graph and the
        // rev-scoped complexity scan below consume it.
        let live = live_paths_at(db, ts)?;

        // Architectural half — purely structural, from the in-memory graph.
        let graph = import_graph_from_live_paths(repo, rev, &live);
        let m = graph_metrics(&graph);
        let files = u32::try_from(m.n).unwrap_or(u32::MAX);
        let arch = arch_health(&m);

        // Code half — rev-scoped sources into the scoped code-health engine.
        ingest_complexity_at_rev(db, repo, rev, &live, CM_AT_REV)?;
        materialize_imports_at_rev(db, &graph, IMPORTS_AT_REV)?;
        let cx = HealthScanCtx {
            complexity_source: CM_AT_REV.to_string(),
            imports_source: IMPORTS_AT_REV.to_string(),
            history_cutoff: Some(ts.clone()),
            include_clones: false,
        };
        let code_rows = run_code_health_scoped(db, &scan_opts, &cx)?;
        let code = repo_code_health(&code_rows);

        let combined = combined_health(arch, code);
        rows.push(HealthTrendRow {
            date: ts.get(..10).unwrap_or(ts).to_string(),
            rev: rev.chars().take(12).collect(),
            files,
            arch_health: arch,
            code_health: code,
            combined_health: combined,
            arch_band: health_band(arch).to_string(),
            code_band: health_band(code).to_string(),
            combined_band: health_band(combined).to_string(),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyses::import_graph::GraphMetrics;

    fn metrics(n: usize, pc: f64, cyclic: u32, largest: u32) -> GraphMetrics {
        GraphMetrics {
            n,
            ccd: 0.0,
            propagation_cost: pc,
            cycle_count: 0,
            largest_cycle: largest,
            cyclic_nodes: cyclic,
        }
    }

    #[test]
    fn band_boundaries() {
        assert_eq!(health_band(69.9), "yellow");
        assert_eq!(health_band(70.0), "green");
        assert_eq!(health_band(40.0), "yellow");
        assert_eq!(health_band(39.9), "red");
        assert_eq!(health_band(100.0), "green");
        assert_eq!(health_band(0.0), "red");
    }

    #[test]
    fn arch_health_empty_graph_is_perfect() {
        assert!((arch_health(&metrics(0, 0.0, 0, 0)) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn arch_health_acyclic_is_100_minus_half_pc() {
        // n=10, pc=0.2, no cycles → risk = 0.5*0.2 = 0.10 → health = 90.
        let h = arch_health(&metrics(10, 0.2, 0, 0));
        assert!((h - 90.0).abs() < 1e-9, "got {h}");
    }

    #[test]
    fn arch_health_fully_tangled_is_low() {
        // n=10, pc=1.0, all 10 cyclic, largest 10 → risk = 0.5 + 0.3 + 0.2 = 1.0 → health 0.
        let h = arch_health(&metrics(10, 1.0, 10, 10));
        assert!(h.abs() < 1e-9, "got {h}");
    }

    #[test]
    fn arch_risk_maxes_at_health_zero() {
        // The worst valid graph (propagation_cost 1.0, every node cyclic, largest
        // tangle spanning all of it) drives risk to its 1.0 ceiling and health to
        // 0 — never negative. The `min(1.0, ...)` clamp is defensive: with valid
        // metrics (pc <= 1, cyclic <= n, largest <= n) raw risk cannot exceed 1.0.
        let h = arch_health(&metrics(2, 1.0, 2, 2));
        assert!(h.abs() < 1e-9, "worst-case health must be 0, got {h}");
    }

    #[test]
    fn combined_is_mean_of_arch_and_code() {
        assert!((combined_health(80.0, 60.0) - 70.0).abs() < 1e-9);
    }

    #[test]
    fn repo_code_health_empty_is_100() {
        assert!((repo_code_health(&[]) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn repo_code_health_averages_scores() {
        let rows = vec![
            CodeHealthRow {
                path: "a".into(),
                cognitive: 0.0,
                score: 90.0,
                structural_risk: 0.0,
                percentile: 0.0,
                band: "green".into(),
            },
            CodeHealthRow {
                path: "b".into(),
                cognitive: 0.0,
                score: 50.0,
                structural_risk: 0.0,
                percentile: 0.0,
                band: "yellow".into(),
            },
        ];
        assert!((repo_code_health(&rows) - 70.0).abs() < 1e-9);
    }
}
