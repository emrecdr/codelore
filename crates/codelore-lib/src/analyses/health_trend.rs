//! Repo Health Timeline: architectural, code, and combined health (each 0–100,
//! higher = healthier) across evenly-spaced historical revisions. Reuses the
//! `architecture_trend` sampler for the rev set and the rev-parameterizable
//! `code_health` engine for the per-rev code score. On-demand, never cached.

use std::collections::{HashMap, HashSet};

use crate::analyses::architecture_trend::{
    import_graph_from_live_paths, live_paths_at, sampled_commits,
};
use crate::analyses::code_health::{CodeHealthRow, HealthScanCtx, run_code_health_scoped};
use crate::analyses::import_graph::{GraphMetrics, graph_metrics};
use crate::facts::FactsDb;
use crate::facts::ingest::at_rev::{ingest_complexity_at_rev, materialize_imports_at_rev};
use crate::repo::Repo;
use crate::{CodeLoreError, Options, Result};

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

/// Re-exported from [`crate::bands`] (the single source of band
/// thresholds) so callers can use either path.
pub use crate::bands::health_band;

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

/// One per-file health data point captured at a sampled historical revision.
/// Only emitted for paths that rank in the top-50 hotspots at HEAD (computed
/// once before the loop) to keep the SPA JSON payload small.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileHealthPoint {
    /// Repo-relative file path.
    pub path: String,
    /// `YYYY-MM-DD` prefix of the sampled commit timestamp.
    pub date: String,
    /// Composite code-health score 0–100 (higher = healthier).
    pub score: f64,
    /// Health band: `"red"`, `"yellow"`, or `"green"`.
    pub band: String,
}

/// A signal-bearing band transition for one file between two consecutive
/// sampled revisions.
///
/// Only two directions are emitted:
/// - `"regressed"` — file **entered** the red band (prev band was not red,
///   current band is red).
/// - `"improved"` — file **left** red or **entered** green (prev was red and
///   current is not, OR prev was not green and current is green).
///
/// Transitions that stay within the same band, or move between yellow↔green
/// without crossing a meaningful threshold, are not emitted (signal noise
/// without actionable meaning).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthTransitionRow {
    /// Repo-relative file path.
    pub path: String,
    /// `YYYY-MM-DD` of the sampled commit where the transition was detected.
    pub date: String,
    /// Band at the previous sampled revision.
    pub from_band: String,
    /// Band at this sampled revision.
    pub to_band: String,
    /// `"improved"` or `"regressed"`.
    pub direction: String,
}

/// Full output of the health-trend detail scan.
///
/// `trend` is byte-identical to [`run_health_trend`]'s output so the
/// existing CSV/Markdown emitters remain unchanged. `file_series` and
/// `transitions` are the new per-file layers added for the SPA.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthTrendDetail {
    /// Repo-level three-score timeline (same as [`run_health_trend`]).
    pub trend: Vec<HealthTrendRow>,
    /// Per-file health points for the top-50 hotspot paths across all
    /// sampled revisions. Empty when no hotspot data is available.
    pub file_series: Vec<FileHealthPoint>,
    /// Signal-bearing band transitions (regressions + improvements) across
    /// all paths at all sampled revisions. Newest first.
    pub transitions: Vec<HealthTransitionRow>,
}

/// Session-scoped temp-table names the rev-scoped `HealthScanCtx` points at.
/// `CREATE OR REPLACE` inside the helpers means reusing them across samples is
/// safe — each iteration replaces the prior rev's contents.
const CM_AT_REV: &str = "cm_at_rev";
const IMPORTS_AT_REV: &str = "imports_at_rev";

/// Fetch the top-N hotspot paths at HEAD (by revision count × cognitive
/// complexity), used to cap `file_series` output size.
///
/// Returns a [`HashSet`] so per-sample lookups are O(1). Uses a minimal
/// inline SQL rather than the full hotspots engine (no DRY, no lineage,
/// no `PERCENT_RANK` window functions) because we only need the path set,
/// not scores, and we want to avoid re-running the heavy grouping step.
fn top_hotspot_paths(db: &FactsDb, opts: &Options, cap: usize) -> Result<HashSet<String>> {
    let cap_i64 = i64::try_from(cap).unwrap_or(i64::MAX);
    let sql = "
        SELECT path
        FROM changes
        GROUP BY path
        HAVING COUNT(rev) >= ?
        ORDER BY COUNT(rev) DESC, path ASC
        LIMIT ?
    ";
    let mut stmt = db
        .conn()
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare top-hotspot-paths: {e}")))?;
    let rows = stmt
        .query_map(duckdb::params![opts.min_revs, cap_i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query top-hotspot-paths: {e}")))?;
    rows.collect::<std::result::Result<HashSet<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect top-hotspot-paths: {e}")))
}

/// Detect signal-bearing band transitions between consecutive samples.
///
/// `"regressed"` — file enters red (was not red, now is red).
/// `"improved"`  — file leaves red (was red, now is not red) OR
///                  file enters green (was not green, now is green).
fn detect_transitions(
    prev_bands: &HashMap<String, String>,
    code_rows: &[CodeHealthRow],
    date: &str,
) -> Vec<HealthTransitionRow> {
    let mut out = Vec::new();
    for row in code_rows {
        let Some(prev) = prev_bands.get(&row.path) else {
            continue;
        };
        let curr = &row.band;
        if prev == curr {
            continue;
        }
        let direction = if curr == "red" && prev != "red" {
            "regressed"
        } else if (prev == "red" && curr != "red") || (curr == "green" && prev != "green") {
            "improved"
        } else {
            continue;
        };
        out.push(HealthTransitionRow {
            path: row.path.clone(),
            date: date.to_string(),
            from_band: prev.clone(),
            to_band: curr.clone(),
            direction: direction.to_string(),
        });
    }
    out
}

/// Compute the three health scores plus per-file series and band transitions
/// across ≤12 evenly-spaced historical revs.
///
/// This is the full detail variant. [`run_health_trend`] is a thin wrapper
/// that returns only `.trend` so the CSV/Markdown emitters are unchanged.
///
/// **Per-file series** — for every sampled rev, the per-file score from
/// [`run_code_health_scoped`] is captured for paths in the top-50 hotspots
/// at HEAD (computed once before the loop via [`top_hotspot_paths`]). The cap
/// keeps the SPA JSON payload small; the hotspot ranking is revision-count
/// ordered so the most-active files are always included.
///
/// **Transitions** — consecutive-sample band changes across ALL paths (not
/// just top-50) where the change is signal-bearing: entering red
/// (`"regressed"`) or leaving red / entering green (`"improved"`). Returned
/// newest-first.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on any query / ingest failure.
#[tracing::instrument(name = "health-trend-detail", skip_all)]
pub fn run_health_trend_detail<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
) -> Result<HealthTrendDetail> {
    const FILE_SERIES_CAP: usize = 50;

    let samples = sampled_commits(db)?;
    // ALL files must feed the code-health mean — never the user's `--rows` cut.
    let scan_opts = opts.with_no_row_limit();

    // Compute the top-50 hotspot path set once before the loop.
    let top_paths = top_hotspot_paths(db, opts, FILE_SERIES_CAP)?;

    let mut trend = Vec::with_capacity(samples.len());
    let mut file_series: Vec<FileHealthPoint> = Vec::new();
    let mut all_transitions: Vec<HealthTransitionRow> = Vec::new();
    // Tracks the previous sample's per-file bands for transition detection.
    let mut prev_bands: HashMap<String, String> = HashMap::new();

    for (rev, ts) in &samples {
        let date = ts.get(..10).unwrap_or(ts);

        // Resolve the live-at-`ts` path set once.
        let live = live_paths_at(db, ts)?;

        // Architectural half.
        let graph = import_graph_from_live_paths(repo, rev, &live);
        let m = graph_metrics(&graph);
        let files = u32::try_from(m.n).unwrap_or(u32::MAX);
        let arch = arch_health(&m);

        // Code half — per-file rows available at zero extra scan cost.
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

        trend.push(HealthTrendRow {
            date: date.to_string(),
            rev: rev.chars().take(12).collect(),
            files,
            arch_health: arch,
            code_health: code,
            combined_health: combined,
            arch_band: health_band(arch).to_string(),
            code_band: health_band(code).to_string(),
            combined_band: health_band(combined).to_string(),
        });

        // Capture per-file points for the top-50 hotspot paths.
        for row in &code_rows {
            if top_paths.contains(&row.path) {
                file_series.push(FileHealthPoint {
                    path: row.path.clone(),
                    date: date.to_string(),
                    score: row.score,
                    band: row.band.clone(),
                });
            }
        }

        // Detect signal-bearing transitions from previous sample.
        if !prev_bands.is_empty() {
            let transitions = detect_transitions(&prev_bands, &code_rows, date);
            all_transitions.extend(transitions);
        }

        // Update prev_bands for the next iteration.
        prev_bands.clear();
        for row in &code_rows {
            prev_bands.insert(row.path.clone(), row.band.clone());
        }
    }

    // Transitions are newest-first: reverse the chronological order.
    all_transitions.reverse();

    Ok(HealthTrendDetail {
        trend,
        file_series,
        transitions: all_transitions,
    })
}

/// Compute the three health scores across ≤12 evenly-spaced historical revs.
///
/// Thin wrapper over [`run_health_trend_detail`] that returns only the
/// `trend` field for backward compatibility — CSV/Markdown emitters are
/// unchanged.
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
    run_health_trend_detail(db, repo, opts).map(|d| d.trend)
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
                corpus_percentile: None,
                beyond_corpus: false,
            },
            CodeHealthRow {
                path: "b".into(),
                cognitive: 0.0,
                score: 50.0,
                structural_risk: 0.0,
                percentile: 0.0,
                band: "yellow".into(),
                corpus_percentile: None,
                beyond_corpus: false,
            },
        ];
        assert!((repo_code_health(&rows) - 70.0).abs() < 1e-9);
    }
}
