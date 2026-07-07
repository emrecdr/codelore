//! Shared health and risk band constants used across analyses and the SPA.
//!
//! All band thresholds live here so a single edit propagates to every
//! consumer: `health_trend`, `code_health`, `delta_health`, the SPA JSON
//! payload (`SpaOptionsSnapshot`), and the JavaScript band helpers.
//!
//! Two independent scales:
//!
//! * **Health score** (0–100, higher = healthier): used by `health_trend` for
//!   architectural, code, and combined health.
//! * **Structural risk** (0–1, higher = worse): used by `code_health` for the
//!   `structural_risk` biomarker aggregate.
//!
//! The two scales are numerically unrelated — do NOT conflate them.

/// Minimum score (inclusive) for the **green** (healthy) health band.
/// Scores in the range `[HEALTH_GREEN_MIN, 100]` map to `"green"`.
pub const HEALTH_GREEN_MIN: f64 = 70.0;

/// Minimum score (inclusive) for the **yellow** (at-risk) health band.
/// Scores in the range `[HEALTH_YELLOW_MIN, HEALTH_GREEN_MIN)` map to
/// `"yellow"`; scores below this threshold map to `"red"`.
pub const HEALTH_YELLOW_MIN: f64 = 40.0;

/// Structural risk threshold above which a file is in the **red** band.
/// Files with `structural_risk >= RISK_RED_MIN` receive `band = "red"`.
pub const RISK_RED_MIN: f64 = 0.55;

/// Structural risk threshold above which a file is in the **yellow** band.
/// Files with `RISK_YELLOW_MIN <= structural_risk < RISK_RED_MIN` receive
/// `band = "yellow"`; files below `RISK_YELLOW_MIN` receive `band = "green"`.
pub const RISK_YELLOW_MIN: f64 = 0.28;

/// Map a health score (0–100, higher = healthier) to its color band.
///
/// Returns `"green"` (score ≥ 70), `"yellow"` (score ≥ 40), or `"red"`.
/// Used by `health_trend` for the three per-rev health scores and by any
/// future analysis that emits a 0–100 health score.
#[must_use]
pub fn health_band(score: f64) -> &'static str {
    if score >= HEALTH_GREEN_MIN {
        "green"
    } else if score >= HEALTH_YELLOW_MIN {
        "yellow"
    } else {
        "red"
    }
}

/// Map a structural risk value (0–1, higher = riskier) to its color band.
///
/// Returns `"red"` (risk ≥ 0.55), `"yellow"` (risk ≥ 0.28), or `"green"`.
/// Used by `code_health` for the composite `structural_risk` biomarker.
#[must_use]
pub fn risk_band(risk: f64) -> &'static str {
    if risk >= RISK_RED_MIN {
        "red"
    } else if risk >= RISK_YELLOW_MIN {
        "yellow"
    } else {
        "green"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── health_band boundary checks ──────────────────────────────────

    #[test]
    fn health_band_green_at_threshold() {
        assert_eq!(health_band(HEALTH_GREEN_MIN), "green");
        assert_eq!(health_band(100.0), "green");
    }

    #[test]
    fn health_band_yellow_range() {
        assert_eq!(health_band(HEALTH_YELLOW_MIN), "yellow");
        // One step below green threshold → yellow.
        assert_eq!(health_band(HEALTH_GREEN_MIN - 0.01), "yellow");
    }

    #[test]
    fn health_band_red_below_yellow() {
        // One step below yellow threshold → red.
        assert_eq!(health_band(HEALTH_YELLOW_MIN - 0.01), "red");
        assert_eq!(health_band(0.0), "red");
    }

    // ── risk_band boundary checks ────────────────────────────────────

    #[test]
    fn risk_band_red_at_threshold() {
        assert_eq!(risk_band(RISK_RED_MIN), "red");
        assert_eq!(risk_band(1.0), "red");
    }

    #[test]
    fn risk_band_yellow_range() {
        assert_eq!(risk_band(RISK_YELLOW_MIN), "yellow");
        // One step below red threshold → yellow.
        assert_eq!(risk_band(RISK_RED_MIN - 0.01), "yellow");
    }

    #[test]
    fn risk_band_green_below_yellow() {
        // One step below yellow threshold → green.
        assert_eq!(risk_band(RISK_YELLOW_MIN - 0.01), "green");
        assert_eq!(risk_band(0.0), "green");
    }
}
