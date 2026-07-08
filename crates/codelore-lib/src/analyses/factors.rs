//! Factor tiles consumed by the SPA dashboard summary panel.
//!
//! Each `FactorTile` aggregates one named KPI dimension into a
//! (headline, trend) pair for the overview row. Tiles are assembled
//! in `codelore-cli/src/main.rs::build_spa_dashboard` from whichever
//! analyses were already run during the current invocation.

/// A single KPI tile in the factor summary panel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FactorTile {
    /// Dimension name, e.g. `"Knowledge"`, `"Health"`.
    pub name: String,
    /// Headline numeric value (0–100 scale).
    pub headline: f64,
    /// Directional trend: `"up"`, `"down"`, or `"flat"`.
    pub trend: String,
}

/// Compute the Knowledge factor tile from code-familiarity output.
///
/// Headline = `familiarity_pct × 0.5 + (100 − islands_pct) × 0.5`.
/// Returns `None` when `rows` is empty.
#[must_use]
pub fn knowledge_factor(
    rows: &[super::code_familiarity::CodeFamiliarityRow],
) -> Option<FactorTile> {
    let row = rows.first()?;
    let headline = row.familiarity_pct * 0.5 + (100.0 - row.islands_pct) * 0.5;
    Some(FactorTile {
        name: "Knowledge".into(),
        headline,
        trend: "flat".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyses::code_familiarity::CodeFamiliarityRow;

    #[test]
    fn knowledge_factor_blends_familiarity_and_islands() {
        let rows = vec![CodeFamiliarityRow {
            scope: "repo".into(),
            familiarity_pct: 80.0,
            active_authors: 2,
            total_authors: 3,
            islands_pct: 20.0,
            verdict: "good".into(),
        }];
        let tile = knowledge_factor(&rows).expect("tile");
        // 80.0 * 0.5 + (100.0 - 20.0) * 0.5 = 40.0 + 40.0 = 80.0
        assert!(
            (tile.headline - 80.0).abs() < 1e-9,
            "headline: {}",
            tile.headline
        );
        assert_eq!(tile.name, "Knowledge");
        assert_eq!(tile.trend, "flat");
    }

    #[test]
    fn knowledge_factor_empty_returns_none() {
        assert!(knowledge_factor(&[]).is_none());
    }

    #[test]
    fn knowledge_factor_high_islands_lowers_headline() {
        let rows = vec![CodeFamiliarityRow {
            scope: "repo".into(),
            familiarity_pct: 100.0,
            active_authors: 1,
            total_authors: 1,
            islands_pct: 80.0,
            verdict: "good".into(),
        }];
        let tile = knowledge_factor(&rows).expect("tile");
        // 100.0 * 0.5 + (100.0 - 80.0) * 0.5 = 50.0 + 10.0 = 60.0
        assert!(
            (tile.headline - 60.0).abs() < 1e-9,
            "headline: {}",
            tile.headline
        );
    }
}
