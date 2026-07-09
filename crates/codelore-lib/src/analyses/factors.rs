//! Zero-config four-factor dashboard header tiles.
//!
//! Each [`FactorTile`] summarises one named quality dimension into a
//! (headline 0–100, historical series, XmR-gated attention flag) triple.
//! Tiles are assembled in `codelore-cli/src/main.rs::build_spa_dashboard`
//! from whichever analyses were already run during the current invocation.
//!
//! ## Factor sources
//!
//! | Factor | Primary source | Fallback |
//! |---|---|---|
//! | Code | `health_trend` `code_health` | — |
//! | Architecture | `health_trend` `arch_health` | — |
//! | Knowledge | `code_familiarity` `familiarity_pct` + `islands_pct` | `knowledge_islands` departed share |
//! | Delivery | `delivery_metrics` `rework_pct` + `branch_duration_hours`; `release_cadence` summary | hidden (no tile) when all sources absent |
//!
//! ## `XmR` attention rule
//!
//! Uses the Shewhart individuals chart (2.66 = 3/d₂, where d₂ = 1.128
//! for n=2 moving ranges). Attention is signalled when either:
//! - The last point is outside `mean ± 2.66 × mean(|xᵢ−xᵢ₋₁|)` (natural
//!   process limit excursion), OR
//! - The last 8 consecutive points are on the same side of the mean
//!   (Western Electric rule 4 — sustained drift).
//!
//! Series shorter than 4 points return `false` (insufficient data for
//! reliable limit estimation).

use crate::analyses::delivery_metrics::DeliveryMetricsRow;
use crate::analyses::health_trend::HealthTrendRow;
use crate::analyses::knowledge_islands::KnowledgeIslandRow;
use crate::analyses::release_cadence::ReleaseCadenceRow;

/// One KPI dimension in the four-factor dashboard header.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FactorTile {
    /// Dimension name: `"Code"`, `"Knowledge"`, `"Architecture"`, or `"Delivery"`.
    pub name: String,
    /// Current headline score 0–100 (higher = healthier).
    ///
    /// `None` for the Delivery tile, which instead uses [`numbers`] to surface
    /// the three proxy values directly rather than collapsing them into a
    /// composite that would imply DORA-level measurement precision.
    pub headline: Option<f64>,
    /// Health band of the headline: `"red"`, `"yellow"`, or `"green"`.
    /// Empty string when `headline` is `None`.
    pub band: String,
    /// Historical series of headline values, oldest-first (may be empty →
    /// JS hides the sparkline).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<f64>,
    /// `true` when the `XmR` chart signals a statistical excursion or
    /// sustained run. See [`xmr_attention`].
    pub attention: bool,
    /// One-line human summary shown beneath the headline.
    pub detail: String,
    /// Key–value pairs rendered in place of the bullet bar when
    /// `headline` is `None`.  Each entry is `(label, formatted_value)`,
    /// e.g. `("rework %", "7.2")` or `("cadence median d", "14")`.
    /// Empty for all tiles that carry a `headline`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub numbers: Vec<(String, String)>,
}

/// Returns `true` when the series shows a statistically significant signal
/// by the Shewhart individuals (`XmR`) chart rules.
///
/// Two conditions are checked in order:
/// 1. **Limit excursion** — the last value is outside
///    `mean ± 2.66 × mean(|xᵢ−xᵢ₋₁|)`.  The constant 2.66 = 3/d₂
///    (d₂ = 1.128 for a moving-range span of 2).
/// 2. **Eight-point run** — the last 8 consecutive points are all on the
///    same side of the mean (Western Electric rule 4).
///
/// Returns `false` for series shorter than 4 points (insufficient data for
/// reliable natural-process-limit estimation).
#[must_use]
pub fn xmr_attention(series: &[f64]) -> bool {
    if series.len() < 4 {
        return false;
    }

    let n = series.len();
    #[allow(clippy::cast_precision_loss)]
    let mean = series.iter().sum::<f64>() / n as f64;

    // Moving-range mean: mean of |xᵢ − xᵢ₋₁| for i = 1..n.
    let mr_mean = {
        let sum: f64 = series.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        #[allow(clippy::cast_precision_loss)]
        let denom = (n - 1) as f64;
        sum / denom
    };

    let limit = 2.66 * mr_mean;
    let last = series[n - 1];

    // Rule 1: last point outside natural process limits.
    if (last - mean).abs() > limit {
        return true;
    }

    // Rule 2: last 8 consecutive points on the same side of the mean.
    if n >= 8 {
        let run = &series[n - 8..];
        let all_above = run.iter().all(|&v| v > mean);
        let all_below = run.iter().all(|&v| v < mean);
        if all_above || all_below {
            return true;
        }
    }

    false
}

/// Build the Code and Architecture factor tiles from `health_trend` output.
///
/// Both tiles share the same historical sample set so the series lengths
/// are identical. When `rows` is empty both tiles are omitted (returns
/// empty vec).
#[must_use]
pub fn health_trend_factors(rows: &[HealthTrendRow]) -> Vec<FactorTile> {
    if rows.is_empty() {
        return Vec::new();
    }
    let last = &rows[rows.len() - 1];

    let code_series: Vec<f64> = rows.iter().map(|r| r.code_health).collect();
    let arch_series: Vec<f64> = rows.iter().map(|r| r.arch_health).collect();

    let code_score = last.code_health;
    let arch_score = last.arch_health;

    vec![
        FactorTile {
            name: "Code".into(),
            headline: Some(code_score),
            band: crate::bands::health_band(code_score).to_string(),
            attention: xmr_attention(&code_series),
            detail: format!(
                "Code health {:.1} ({}) — averaged over all files at latest sample",
                code_score,
                crate::bands::health_band(code_score),
            ),
            series: code_series,
            numbers: Vec::new(),
        },
        FactorTile {
            name: "Architecture".into(),
            headline: Some(arch_score),
            band: crate::bands::health_band(arch_score).to_string(),
            attention: xmr_attention(&arch_series),
            detail: format!(
                "Architecture health {:.1} ({}) — propagation cost and cycle exposure",
                arch_score,
                crate::bands::health_band(arch_score),
            ),
            series: arch_series,
            numbers: Vec::new(),
        },
    ]
}

/// Build the Knowledge factor tile from `code_familiarity` output.
///
/// Headline = `0.5 × familiarity_pct + 0.5 × (100 − islands_pct)`.
/// Returns `None` when `rows` is empty.
///
/// # Parameters
///
/// - `familiarity_pct`: percentage of active SLOC known by current team.
/// - `islands_pct`: percentage of SLOC in knowledge islands (single-expert
///   or departed-expert files). Both from `run_code_familiarity`.
#[must_use]
pub fn knowledge_factor_from_familiarity(familiarity_pct: f64, islands_pct: f64) -> FactorTile {
    let headline = 0.5 * familiarity_pct + 0.5 * (100.0 - islands_pct);
    FactorTile {
        name: "Knowledge".into(),
        headline: Some(headline),
        band: crate::bands::health_band(headline).to_string(),
        series: Vec::new(),
        attention: false,
        detail: format!(
            "Team familiarity {familiarity_pct:.1}%, knowledge islands {islands_pct:.1}% of SLOC",
        ),
        numbers: Vec::new(),
    }
}

/// Build the Knowledge factor tile from `knowledge_islands` output as a
/// fallback when `code_familiarity` data is unavailable.
///
/// Headline = `100 × (1 − departed_island_share)` where
/// `departed_island_share` = fraction of island files whose main author has
/// departed (i.e. `days_since_main_active ≥ departed_threshold`).
///
/// This is a conservative proxy: it only penalises for *departed* knowledge
/// concentration, not for active-but-siloed experts. The `code_familiarity`
/// source is preferred when available.
#[must_use]
pub fn knowledge_factor_from_islands(
    rows: &[KnowledgeIslandRow],
    departed_threshold_days: i32,
) -> Option<FactorTile> {
    if rows.is_empty() {
        return None;
    }
    let total = rows.len();
    let departed = rows
        .iter()
        .filter(|r| r.days_since_main_active >= departed_threshold_days)
        .count();
    #[allow(clippy::cast_precision_loss)]
    let departed_share = departed as f64 / total as f64;
    let headline = 100.0 * (1.0 - departed_share);
    Some(FactorTile {
        name: "Knowledge".into(),
        headline: Some(headline),
        band: crate::bands::health_band(headline).to_string(),
        series: Vec::new(),
        attention: departed_share > 0.2,
        detail: format!("{departed} of {total} knowledge-island files have departed main authors"),
        numbers: Vec::new(),
    })
}

/// Build the Delivery factor tile from `delivery-metrics` and
/// `release-cadence` output.
///
/// The Delivery tile deliberately shows NO composite score. Instead it
/// surfaces three git-proxy numbers with their own band coloring:
///
/// | Number | Source | Band rule |
/// |---|---|---|
/// | `rework %` | `delivery-metrics` `rework_pct` p50 | green <9 %, yellow 9-14 %, red ≥15 % |
/// | `branch p75 h` | `delivery-metrics` `branch_duration_hours` p75 | uncolored |
/// | `cadence median d` | `release-cadence` summary `days_since_prev` | uncolored |
///
/// The rework band thresholds are from Pluralsight Flow's published
/// benchmark ranges (vendor benchmark, correlational — not a causal
/// threshold). The other numbers have no validated benchmark and are
/// presented without coloring.
///
/// Returns `None` when both inputs are empty (all absent → tile omitted).
/// When only one source is available, the other numbers are omitted and
/// the tile still appears with whatever numbers are present.
///
/// **These are git-only proxies, not DORA metrics.** Rework detection
/// uses hunk-pair overlap (approximate — line drift between commits is not
/// tracked). Branch duration uses commit-parent topology (squash/rebase
/// workflows undercount). Lead-time uses author→committer date gap
/// (proxy only — does not include waiting time before first review).
/// Cadence counts `v*` release tags (configurable via
/// `--release-tag-glob`).
#[must_use]
pub fn delivery_factor_from_metrics(
    delivery_rows: &[DeliveryMetricsRow],
    cadence_rows: &[ReleaseCadenceRow],
) -> Option<FactorTile> {
    let mut numbers: Vec<(String, String)> = Vec::new();

    // Rework % — band-colored (Pluralsight benchmark, correlational).
    let rework_band = if let Some(r) = delivery_rows.iter().find(|r| r.metric == "rework_pct") {
        let pct = r.p50;
        let band = if pct < 9.0 {
            "green"
        } else if pct < 15.0 {
            "yellow"
        } else {
            "red"
        };
        numbers.push(("rework %".to_string(), format!("{pct:.1}")));
        band
    } else {
        ""
    };

    // Branch p75 hours — topology-based, uncolored.
    if let Some(r) = delivery_rows
        .iter()
        .find(|r| r.metric == "branch_duration_hours")
    {
        numbers.push(("branch p75 h".to_string(), format!("{:.0}", r.p75)));
    }

    // Cadence median days — from release-cadence summary row.
    if let Some(days) = cadence_rows
        .iter()
        .find(|r| r.tag == "__summary__")
        .and_then(|s| s.days_since_prev)
    {
        numbers.push(("cadence median d".to_string(), format!("{days:.0}")));
    }

    if numbers.is_empty() {
        return None;
    }

    Some(FactorTile {
        name: "Delivery".into(),
        headline: None,
        band: rework_band.to_string(),
        series: Vec::new(),
        attention: false,
        detail: "Git-only proxies — not DORA metrics. Rework band: Pluralsight benchmark (correlational).".into(),
        numbers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── xmr_attention ────────────────────────────────────────────────────────

    #[test]
    fn xmr_flat_series_no_attention() {
        // Perfectly flat: zero moving range → limits = mean ± 0 → last point
        // exactly on the mean → no excursion; not an 8-run either.
        let series = vec![70.0_f64; 10];
        assert!(!xmr_attention(&series));
    }

    #[test]
    fn xmr_step_change_signals_attention() {
        // 9 stable samples then a large drop → last point far outside limits.
        let mut series = vec![70.0_f64; 9];
        series.push(10.0);
        assert!(xmr_attention(&series));
    }

    #[test]
    fn xmr_short_series_returns_false() {
        // Series length < 4 is always false.
        assert!(!xmr_attention(&[]));
        assert!(!xmr_attention(&[50.0, 60.0]));
        assert!(!xmr_attention(&[50.0, 60.0, 55.0]));
    }

    #[test]
    fn xmr_eight_run_below_mean_signals_attention() {
        // Mean is ~72 (first 4 points above), last 8 all below → rule 2.
        let series = vec![
            90.0, 88.0, 92.0, 91.0, 50.0, 51.0, 52.0, 53.0, 54.0, 55.0, 56.0, 57.0,
        ];
        assert!(xmr_attention(&series));
    }

    #[test]
    fn xmr_eight_run_above_mean_signals_attention() {
        // Mean is ~28 (first 4 points below), last 8 all above → rule 2.
        let series = vec![
            10.0, 12.0, 8.0, 9.0, 50.0, 51.0, 52.0, 53.0, 54.0, 55.0, 56.0, 57.0,
        ];
        assert!(xmr_attention(&series));
    }

    #[test]
    fn xmr_exactly_four_points_eligible() {
        // Length == 4 is the minimum for evaluation; gentle variation → false.
        let series = vec![70.0, 71.0, 70.5, 70.8];
        assert!(!xmr_attention(&series));
    }

    // ── health_trend_factors ─────────────────────────────────────────────────

    #[test]
    fn health_trend_factors_empty_returns_empty() {
        assert!(health_trend_factors(&[]).is_empty());
    }

    #[test]
    fn health_trend_factors_returns_code_and_arch() {
        let row = HealthTrendRow {
            date: "2026-01-01".into(),
            rev: "abc123".into(),
            files: 5,
            arch_health: 85.0,
            code_health: 62.0,
            combined_health: 73.5,
            arch_band: "green".into(),
            code_band: "yellow".into(),
            combined_band: "green".into(),
        };
        let tiles = health_trend_factors(&[row]);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].name, "Code");
        assert!((tiles[0].headline.unwrap() - 62.0).abs() < 1e-9);
        assert_eq!(tiles[0].band, "yellow");
        assert_eq!(tiles[1].name, "Architecture");
        assert!((tiles[1].headline.unwrap() - 85.0).abs() < 1e-9);
        assert_eq!(tiles[1].band, "green");
    }

    // ── knowledge factors ────────────────────────────────────────────────────

    #[test]
    fn knowledge_familiarity_blends_correctly() {
        let tile = knowledge_factor_from_familiarity(80.0, 20.0);
        // 0.5 × 80 + 0.5 × 80 = 80
        assert!((tile.headline.unwrap() - 80.0).abs() < 1e-9);
        assert_eq!(tile.name, "Knowledge");
        assert_eq!(tile.band, "green");
    }

    #[test]
    fn knowledge_familiarity_high_islands_lowers_headline() {
        let tile = knowledge_factor_from_familiarity(100.0, 80.0);
        // 0.5 × 100 + 0.5 × 20 = 60
        assert!((tile.headline.unwrap() - 60.0).abs() < 1e-9);
        assert_eq!(tile.band, "yellow");
    }

    #[test]
    fn knowledge_islands_fallback_empty_returns_none() {
        assert!(knowledge_factor_from_islands(&[], 90).is_none());
    }

    #[test]
    fn knowledge_islands_fallback_all_active() {
        let rows = vec![
            KnowledgeIslandRow {
                entity: "src/a.rs".into(),
                main_author: "alice".into(),
                ownership_pct: 90.0,
                days_since_main_active: 10,
                last_main_author_commit: "abc".into(),
                n_substantial_others: 0,
            },
            KnowledgeIslandRow {
                entity: "src/b.rs".into(),
                main_author: "bob".into(),
                ownership_pct: 85.0,
                days_since_main_active: 5,
                last_main_author_commit: "def".into(),
                n_substantial_others: 1,
            },
        ];
        let tile = knowledge_factor_from_islands(&rows, 90).expect("tile");
        // 0 departed → headline = 100
        assert!((tile.headline.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(tile.band, "green");
        assert!(!tile.attention);
    }

    #[test]
    fn knowledge_islands_fallback_departed_lowers_headline() {
        let rows = vec![
            KnowledgeIslandRow {
                entity: "src/a.rs".into(),
                main_author: "alice".into(),
                ownership_pct: 90.0,
                days_since_main_active: 200, // departed
                last_main_author_commit: "abc".into(),
                n_substantial_others: 0,
            },
            KnowledgeIslandRow {
                entity: "src/b.rs".into(),
                main_author: "bob".into(),
                ownership_pct: 85.0,
                days_since_main_active: 5,
                last_main_author_commit: "def".into(),
                n_substantial_others: 1,
            },
        ];
        let tile = knowledge_factor_from_islands(&rows, 90).expect("tile");
        // 1 of 2 departed → share = 0.5 → headline = 50
        assert!((tile.headline.unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(tile.band, "yellow");
        assert!(tile.attention); // > 20% departed
    }

    // ── delivery_factor_from_metrics ─────────────────────────────────────

    fn make_delivery_row(metric: &str, p50: f64, p75: f64) -> DeliveryMetricsRow {
        DeliveryMetricsRow {
            metric: metric.to_string(),
            p50,
            p75,
            p90: 0.0,
            n: 5,
            caveat: String::new(),
        }
    }

    fn make_cadence_summary(median_days: f64) -> ReleaseCadenceRow {
        ReleaseCadenceRow {
            tag: "__summary__".to_string(),
            date: "iqr=3.0d".to_string(),
            days_since_prev: Some(median_days),
            trend: "stable".to_string(),
        }
    }

    #[test]
    fn delivery_factor_both_empty_returns_none() {
        assert!(delivery_factor_from_metrics(&[], &[]).is_none());
    }

    #[test]
    fn delivery_factor_rework_only_returns_tile() {
        let delivery = vec![make_delivery_row("rework_pct", 7.0, 7.0)];
        let tile = delivery_factor_from_metrics(&delivery, &[]).expect("tile");
        assert_eq!(tile.name, "Delivery");
        assert!(tile.headline.is_none());
        assert_eq!(tile.band, "green"); // 7.0 < 9.0
        assert_eq!(tile.numbers.len(), 1);
        assert_eq!(tile.numbers[0].0, "rework %");
        assert_eq!(tile.numbers[0].1, "7.0");
    }

    #[test]
    fn delivery_factor_rework_yellow_band() {
        // 10.0 is in [9, 15) → yellow
        let delivery = vec![make_delivery_row("rework_pct", 10.0, 10.0)];
        let tile = delivery_factor_from_metrics(&delivery, &[]).expect("tile");
        assert_eq!(tile.band, "yellow");
    }

    #[test]
    fn delivery_factor_rework_red_band() {
        // 15.0 ≥ 15 → red
        let delivery = vec![make_delivery_row("rework_pct", 15.0, 15.0)];
        let tile = delivery_factor_from_metrics(&delivery, &[]).expect("tile");
        assert_eq!(tile.band, "red");
    }

    #[test]
    fn delivery_factor_all_three_numbers_present() {
        let delivery = vec![
            make_delivery_row("rework_pct", 5.0, 5.0),
            make_delivery_row("branch_duration_hours", 12.0, 26.0),
        ];
        let cadence = vec![make_cadence_summary(14.0)];
        let tile = delivery_factor_from_metrics(&delivery, &cadence).expect("tile");
        assert_eq!(tile.numbers.len(), 3);
        // Order: rework %, branch p75 h, cadence median d
        assert_eq!(tile.numbers[0].0, "rework %");
        assert_eq!(tile.numbers[1].0, "branch p75 h");
        assert_eq!(tile.numbers[1].1, "26"); // p75 formatted as integer
        assert_eq!(tile.numbers[2].0, "cadence median d");
        assert_eq!(tile.numbers[2].1, "14");
    }

    #[test]
    fn delivery_factor_no_rework_no_band() {
        // Only cadence present — band should be empty (no rework to color)
        let cadence = vec![make_cadence_summary(7.0)];
        let tile = delivery_factor_from_metrics(&[], &cadence).expect("tile");
        assert_eq!(tile.band, "");
        assert_eq!(tile.numbers.len(), 1);
        assert_eq!(tile.numbers[0].0, "cadence median d");
    }
}
