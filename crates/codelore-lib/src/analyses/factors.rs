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
//! | Delivery | WS-C composite (not yet implemented) | hidden (empty tile list) |
//!
//! ## XmR attention rule
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

use crate::analyses::health_trend::HealthTrendRow;
use crate::analyses::knowledge_islands::KnowledgeIslandRow;

/// One KPI dimension in the four-factor dashboard header.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FactorTile {
    /// Dimension name: `"Code"`, `"Knowledge"`, `"Architecture"`, or `"Delivery"`.
    pub name: String,
    /// Current headline score 0–100 (higher = healthier).
    /// `None` for the Delivery tile when no delivery data is available.
    pub headline: Option<f64>,
    /// Health band of the headline: `"red"`, `"yellow"`, or `"green"`.
    /// Empty string when `headline` is `None`.
    pub band: String,
    /// Historical series of headline values, oldest-first (may be empty →
    /// JS hides the sparkline).
    pub series: Vec<f64>,
    /// `true` when the XmR chart signals a statistical excursion or
    /// sustained run. See [`xmr_attention`].
    pub attention: bool,
    /// One-line human summary shown beneath the headline.
    pub detail: String,
}

/// Returns `true` when the series shows a statistically significant signal
/// by the Shewhart individuals (XmR) chart rules.
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
    let mean = series.iter().sum::<f64>() / n as f64;

    // Moving-range mean: mean of |xᵢ − xᵢ₋₁| for i = 1..n.
    let mr_mean = {
        let sum: f64 = series.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        sum / (n - 1) as f64
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
            "Team familiarity {:.1}%, knowledge islands {:.1}% of SLOC",
            familiarity_pct, islands_pct,
        ),
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
/// source is preferred when available — this fallback will be swapped out
/// once Task B.2 lands on the branch.
#[must_use]
pub fn knowledge_factor_from_islands(
    rows: &[KnowledgeIslandRow],
    departed_threshold_days: i32,
) -> Option<FactorTile> {
    if rows.is_empty() {
        return None;
    }
    let total = rows.len() as f64;
    let departed = rows
        .iter()
        .filter(|r| r.days_since_main_active >= departed_threshold_days)
        .count() as f64;
    let departed_share = departed / total;
    let headline = 100.0 * (1.0 - departed_share);
    Some(FactorTile {
        name: "Knowledge".into(),
        headline: Some(headline),
        band: crate::bands::health_band(headline).to_string(),
        series: Vec::new(),
        attention: departed_share > 0.2,
        detail: format!(
            "{} of {} knowledge-island files have departed main authors",
            departed as u32, total as u32,
        ),
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
}
