//! Maintainability Index banding — **repo-relative**, not absolute.
//!
//! `complexity_metrics.mi` stores the SEI-variant MI computed by the
//! vendored `codelore-rca` fork of Mozilla `rust-code-analysis`. The
//! formula (Coleman et al. 1994 + SEI 1997) is:
//!
//! ```text
//!   mi_sei = 171 − 5.2·log₂(V) − 0.23·CC − 16.2·log₂(SLOC)
//!          + 50·sin(√(2.4·comments%))
//! ```
//!
//! where `V` is Halstead volume, `CC` is cyclomatic complexity, `SLOC`
//! is source lines of code, and `comments%` is the comment-line ratio.
//!
//! **Why we don't use the literature's absolute thresholds.** The Coleman/SEI
//! convention (`≥85` high, `65–85` moderate, `<65` low) was calibrated on
//! 1990s-era embedded-software modules typically `<200` SLOC each. Modern
//! source files at 500–5000 SLOC produce much lower MI because the
//! `−16.2·log₂(SLOC)` term grows fast. Empirically validated on `CodeLore`'s
//! own Rust codebase (see `docs/research-foundations.md`): MI values range
//! `[−137, +104]` with median `≈2.7` — applying the literature thresholds
//! verbatim would classify 100% of well-maintained files as "low
//! maintainability". `CHM` ships the literature thresholds because its `JS`/`TS`
//! sample files are small; ours aren't.
//!
//! **`CodeLore`'s choice**: repo-relative percentile bands. The hotspots
//! SQL computes `PERCENT_RANK() OVER (ORDER BY mi)` and the band is
//! derived from that rank. This matches the existing relative-ranking
//! convention used by `hotspot_score` (which is built on
//! `PERCENT_RANK(revs) × PERCENT_RANK(cognitive)`).
//!
//! **Bands**:
//!
//! | Band       | Percentile rank within repo |
//! |------------|-----------------------------|
//! | `High`     | top 25% (rank ≥ 0.75)       |
//! | `Moderate` | middle 50% (0.25 ≤ rank < 0.75) |
//! | `Low`      | bottom 25% (rank < 0.25)    |
//! | (unknown)  | file has no `kind='unit'` complexity entry — language not yet supported by `codelore-rca`, or file skipped at ingest |
//!
//! **Trade-off**: bands aren't comparable across repos (a "moderate" file
//! in a tightly-maintained repo could be a "high" file in a sprawling one).
//! The raw `mi` value and `mi_rank` percentile are surfaced alongside the
//! band on every emitter so users can see the absolute context too.

use serde::{Deserialize, Serialize};

use crate::analyses::hotspots::HotspotRow;

/// Lower bound of the `High` band on the `[0, 1]` percentile-rank scale.
pub const MI_BAND_HIGH_PERCENTILE: f64 = 0.75;

/// Lower bound of the `Moderate` band on the `[0, 1]` percentile-rank
/// scale. Anything below this is `Low`.
pub const MI_BAND_MODERATE_PERCENTILE: f64 = 0.25;

/// Repo-relative MI band. See module docs for the rationale on percentile
/// bands vs absolute Coleman/SEI thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiBand {
    /// File is in the bottom quartile of MI within the analyzed repo.
    /// Highest refactoring priority among files with known MI.
    Low,
    /// File is in the middle 50% of MI within the analyzed repo.
    Moderate,
    /// File is in the top quartile of MI within the analyzed repo.
    /// Easiest to maintain among files with known MI.
    High,
}

impl MiBand {
    /// Classify a repo-relative percentile rank into a band. `rank` must
    /// be in `[0, 1]`; the standard `DuckDB` `PERCENT_RANK()` window
    /// function emits exactly that range.
    #[must_use]
    pub fn from_rank(rank: f64) -> Self {
        if rank >= MI_BAND_HIGH_PERCENTILE {
            Self::High
        } else if rank >= MI_BAND_MODERATE_PERCENTILE {
            Self::Moderate
        } else {
            Self::Low
        }
    }

    /// Lowercased single-word identifier for the band. Used by emitters
    /// (CSV cell, SARIF property, JSON serialization label).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }
}

/// Per-repo aggregate count of files per MI band. Feeds the SPA dashboard's
/// MI-band KPI tile and serializable summaries.
///
/// `unknown` covers files whose MI is `None` — typically because the file
/// is in a language the vendored `codelore-rca` doesn't yet support, or
/// the file was skipped at ingest (binary, oversized, etc).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct MiRollup {
    pub low: usize,
    pub moderate: usize,
    pub high: usize,
    pub unknown: usize,
}

impl MiRollup {
    /// Roll a slice of hotspot rows up into per-band counts.
    #[must_use]
    pub fn from_hotspots(rows: &[HotspotRow]) -> Self {
        let mut r = Self::default();
        for row in rows {
            match (row.mi, row.mi_rank) {
                (Some(_), Some(rank)) if rank.is_finite() => match MiBand::from_rank(rank) {
                    MiBand::Low => r.low += 1,
                    MiBand::Moderate => r.moderate += 1,
                    MiBand::High => r.high += 1,
                },
                _ => r.unknown += 1,
            }
        }
        r
    }

    /// Total count of files with a known MI (excludes `unknown`).
    #[must_use]
    pub fn known(self) -> usize {
        self.low + self.moderate + self.high
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_rank_boundary_values() {
        assert_eq!(MiBand::from_rank(0.0), MiBand::Low);
        assert_eq!(MiBand::from_rank(0.24), MiBand::Low);
        assert_eq!(MiBand::from_rank(0.25), MiBand::Moderate);
        assert_eq!(MiBand::from_rank(0.50), MiBand::Moderate);
        assert_eq!(MiBand::from_rank(0.74), MiBand::Moderate);
        assert_eq!(MiBand::from_rank(0.75), MiBand::High);
        assert_eq!(MiBand::from_rank(1.00), MiBand::High);
    }

    #[test]
    fn rollup_counts_each_band_and_unknown() {
        let make = |path: &str, mi: Option<f64>, rank: Option<f64>| HotspotRow {
            path: path.to_owned(),
            revisions: 1,
            cognitive: 0.0,
            code_health: 100.0,
            hotspot_score: 0.0,
            mi,
            mi_rank: rank,
        };
        let rows = vec![
            make("a.rs", Some(80.0), Some(0.90)), // High
            make("b.rs", Some(50.0), Some(0.50)), // Moderate
            make("c.rs", Some(50.0), Some(0.30)), // Moderate
            make("d.rs", Some(10.0), Some(0.10)), // Low
            make("e.rs", None, None),             // Unknown
            make("f.rs", Some(0.0), None),        // Unknown — value but no rank
        ];
        let r = MiRollup::from_hotspots(&rows);
        assert_eq!(r.high, 1);
        assert_eq!(r.moderate, 2);
        assert_eq!(r.low, 1);
        assert_eq!(r.unknown, 2);
        assert_eq!(r.known(), 4);
    }

    #[test]
    fn rollup_ignores_non_finite_ranks() {
        let row = HotspotRow {
            path: "x.rs".into(),
            revisions: 1,
            cognitive: 0.0,
            code_health: 100.0,
            hotspot_score: 0.0,
            mi: Some(50.0),
            mi_rank: Some(f64::NAN),
        };
        let r = MiRollup::from_hotspots(std::slice::from_ref(&row));
        assert_eq!(r.unknown, 1);
        assert_eq!(r.known(), 0);
    }
}
