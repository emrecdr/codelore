//! Repo Health Timeline: architectural, code, and combined health (each 0–100,
//! higher = healthier) across evenly-spaced historical revisions. Reuses the
//! `architecture_trend` sampler for the rev set and piece-1's rev-parameterizable
//! `code_health` engine for the per-rev code score. On-demand, never cached.

use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::import_graph::GraphMetrics;

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
    let n = m.n as f64;
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
    sum / rows.len() as f64
}

/// Combined health: equal blend of systemic (architecture) and local (code).
#[must_use]
pub(crate) fn combined_health(arch: f64, code: f64) -> f64 {
    0.5 * arch + 0.5 * code
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
