//! `delta-health` — change-level health verdict for `codelore diff`.
//!
//! Judges the CHANGE, not the snapshot: each function added, removed, or
//! modified between base and head is classified low/medium/high risk from
//! absolute thresholds, given an outcome (good/neutral/bad) from its
//! before→after direction, and aggregated into a 0–100 ratio with an
//! explicit low-signal middle verdict. Snapshot scores are provably
//! insensitive to individual commits; this is the per-change complement.
//!
//! Thresholds are FIXED constants, not TOML-configurable: the gate cannot
//! be quietly loosened, and verdicts stay stable across PRs.

use serde::{Deserialize, Serialize};

use crate::facts::FactsDb;
use crate::{CodeLoreError, Result};

/// Function LOC at or above this is medium risk (SIG unit-size bands).
pub const LOC_MEDIUM_FROM: u32 = 31;
/// Function LOC at or above this is high risk (SIG bands / Large Method > 70).
pub const LOC_HIGH_FROM: u32 = 71;
/// Cyclomatic complexity at or above this is medium risk (SIG bands).
pub const CYCLOMATIC_MEDIUM_FROM: f64 = 6.0;
/// Cyclomatic complexity at or above this is high risk (SIG bands / CC > 10).
pub const CYCLOMATIC_HIGH_FROM: f64 = 11.0;
/// Ratio strictly below this ⇒ `degrading` verdict.
pub const RATIO_DEGRADING_BELOW: f64 = 40.0;
/// Ratio strictly above this ⇒ `improving` verdict.
pub const RATIO_IMPROVING_ABOVE: f64 = 70.0;
/// Good/bad weight multiplier for functions in base-red-band files.
pub const RED_FILE_WEIGHT_MULTIPLIER: f64 = 1.5;

/// Risk class of a single function. Derive order matters: `Low < Medium
/// < High` powers the improved/degraded direction comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskClass {
    Low,
    Medium,
    High,
}

impl RiskClass {
    /// Lowercase display form, matching the serde encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Outcome of one changed function within the scored change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Good,
    Neutral,
    Bad,
}

impl Outcome {
    /// Lowercase display form, matching the serde encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Neutral => "neutral",
            Self::Bad => "bad",
        }
    }
}

/// Classify one function from its persisted metrics. Worst triggered
/// property wins; clone membership forces High (the copy/paste penalty —
/// AI-pasted duplicates cannot score low-risk).
#[must_use]
pub fn classify(loc: u32, cyclomatic: f64, clone_member: bool) -> RiskClass {
    if clone_member || loc >= LOC_HIGH_FROM || cyclomatic >= CYCLOMATIC_HIGH_FROM {
        return RiskClass::High;
    }
    if loc >= LOC_MEDIUM_FROM || cyclomatic >= CYCLOMATIC_MEDIUM_FROM {
        return RiskClass::Medium;
    }
    RiskClass::Low
}

/// Outcome matrix per the design: added = ∅→class, removed = class→∅.
/// Good — ends Low, strictly improves, or removes a High-risk function.
/// Bad — ends High or strictly degrades. Neutral — everything else.
#[must_use]
pub fn outcome_for(before: Option<RiskClass>, after: Option<RiskClass>) -> Outcome {
    match (before, after) {
        (None, Some(a)) => match a {
            RiskClass::Low => Outcome::Good,
            RiskClass::Medium => Outcome::Neutral,
            RiskClass::High => Outcome::Bad,
        },
        (Some(b), None) => {
            if b == RiskClass::High {
                Outcome::Good
            } else {
                Outcome::Neutral
            }
        }
        (Some(b), Some(a)) => {
            if a == RiskClass::Low || a < b {
                Outcome::Good
            } else if a == RiskClass::High || a > b {
                Outcome::Bad
            } else {
                Outcome::Neutral
            }
        }
        // A function neither present at base nor head is never a change
        // candidate; keep the match total without panicking.
        (None, None) => Outcome::Neutral,
    }
}

/// Verdict from the ratio. The middle band is deliberately labeled
/// `indeterminate` — the design's honest replacement for a binary cut.
#[must_use]
pub fn verdict_for(ratio: f64) -> &'static str {
    if ratio < RATIO_DEGRADING_BELOW {
        "degrading"
    } else if ratio > RATIO_IMPROVING_ABOVE {
        "improving"
    } else {
        "indeterminate"
    }
}

/// One function's persisted metrics at a single rev. Serialized into the
/// CLI's `--base-cache` JSON, so field changes need the same
/// back-compat care as the cache struct itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionMetricRow {
    pub path: String,
    pub name: String,
    pub loc: u32,
    pub cyclomatic: f64,
}

/// Extract per-function metric rows from an ingested fact store.
///
/// `complexity_metrics` also stores class- and file-level rows; the join
/// on `entities.kind` keeps only real functions/methods so nothing
/// file-shaped is ever classified as a changed function.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on SQL failures.
pub fn run_function_metrics(db: &FactsDb) -> Result<Vec<FunctionMetricRow>> {
    const SQL: &str = "
        SELECT DISTINCT cm.path, cm.name, cm.loc, CAST(cm.cyclomatic AS DOUBLE)
        FROM complexity_metrics cm
        JOIN entities e ON e.path = cm.path AND e.name = cm.name
        WHERE e.kind IN ('function', 'method')
        ORDER BY cm.path, cm.name";
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare delta-health metrics: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FunctionMetricRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                loc: r.get::<_, u32>(2)?,
                cyclomatic: r.get::<_, f64>(3)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query delta-health metrics: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("read delta-health metrics: {e}")))?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_loc_boundaries() {
        assert_eq!(classify(30, 1.0, false), RiskClass::Low);
        assert_eq!(classify(31, 1.0, false), RiskClass::Medium);
        assert_eq!(classify(70, 1.0, false), RiskClass::Medium);
        assert_eq!(classify(71, 1.0, false), RiskClass::High);
    }

    #[test]
    fn classify_cyclomatic_boundaries() {
        assert_eq!(classify(10, 5.0, false), RiskClass::Low);
        assert_eq!(classify(10, 6.0, false), RiskClass::Medium);
        assert_eq!(classify(10, 10.0, false), RiskClass::Medium);
        assert_eq!(classify(10, 11.0, false), RiskClass::High);
    }

    #[test]
    fn classify_clone_membership_forces_high() {
        assert_eq!(classify(5, 1.0, true), RiskClass::High);
    }

    #[test]
    fn classify_worst_property_wins() {
        // Low LOC but high cyclomatic ⇒ High.
        assert_eq!(classify(10, 20.0, false), RiskClass::High);
        // High LOC but trivial cyclomatic ⇒ High.
        assert_eq!(classify(100, 1.0, false), RiskClass::High);
    }

    #[test]
    fn outcome_added() {
        assert_eq!(outcome_for(None, Some(RiskClass::Low)), Outcome::Good);
        assert_eq!(outcome_for(None, Some(RiskClass::Medium)), Outcome::Neutral);
        assert_eq!(outcome_for(None, Some(RiskClass::High)), Outcome::Bad);
    }

    #[test]
    fn outcome_removed() {
        assert_eq!(outcome_for(Some(RiskClass::High), None), Outcome::Good);
        assert_eq!(outcome_for(Some(RiskClass::Medium), None), Outcome::Neutral);
        assert_eq!(outcome_for(Some(RiskClass::Low), None), Outcome::Neutral);
    }

    #[test]
    fn outcome_modified_matrix() {
        use RiskClass::{High, Low, Medium};
        // Stayed low ⇒ good; improved ⇒ good (even High→Medium).
        assert_eq!(outcome_for(Some(Low), Some(Low)), Outcome::Good);
        assert_eq!(outcome_for(Some(High), Some(Medium)), Outcome::Good);
        assert_eq!(outcome_for(Some(Medium), Some(Low)), Outcome::Good);
        // Ends high or degrades ⇒ bad.
        assert_eq!(outcome_for(Some(High), Some(High)), Outcome::Bad);
        assert_eq!(outcome_for(Some(Low), Some(Medium)), Outcome::Bad);
        assert_eq!(outcome_for(Some(Medium), Some(High)), Outcome::Bad);
        // Stayed medium ⇒ neutral.
        assert_eq!(outcome_for(Some(Medium), Some(Medium)), Outcome::Neutral);
    }

    #[test]
    fn verdict_cut_points() {
        assert_eq!(verdict_for(39.9), "degrading");
        assert_eq!(verdict_for(40.0), "indeterminate");
        assert_eq!(verdict_for(70.0), "indeterminate");
        assert_eq!(verdict_for(70.1), "improving");
    }
}
