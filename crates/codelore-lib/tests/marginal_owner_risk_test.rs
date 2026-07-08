//! Tests for the `marginal-owner-risk` analysis.
//!
//! # Unit tests — pure risk classification
//!
//! [`classify_risk`] is a pure function; every rule branch is covered
//! without any database fixture.
//!
//! # Integration tests — `delivery_repo` invariants
//!
//! The `delivery_repo` fixture (Alice/Bob/Carol, all active within the
//! default 90-day window) produces knowledge shares that sum to 1.0 per
//! path across all three authors. With three active authors on every path,
//! the top active share is typically in the range 0.33–0.60. This is above
//! the "high" threshold (< 0.10) and above the "elevated" threshold for
//! yellow (< 0.10), so whether any rows appear at all depends on which
//! paths happen to fall in the yellow/red band with a share below 0.30 (red)
//! or 0.10 (yellow).
//!
//! Pinning an exact row count would couple the tests to the health
//! metric formula — a threshold change in code-health would silently
//! break a count assertion. Instead the integration tests assert structural
//! invariants that must hold regardless of which paths surface: every
//! emitted row must satisfy the risk-classification rule, and the
//! band/risk fields must be in their defined value sets.

use codelore_lib::analyses::marginal_owner_risk::{classify_risk, run_marginal_owner_risk};

// ── Unit tests (no DB) ────────────────────────────────────────────────────────

#[test]
fn red_below_0_10_is_high() {
    assert_eq!(classify_risk("red", 0.05), Some("high"));
    assert_eq!(classify_risk("red", 0.09), Some("high"));
    // Boundary: exactly 0.10 is NOT high (strict <)
    assert_eq!(classify_risk("red", 0.10), Some("elevated"));
}

#[test]
fn red_between_0_10_and_0_30_is_elevated() {
    assert_eq!(classify_risk("red", 0.10), Some("elevated"));
    assert_eq!(classify_risk("red", 0.20), Some("elevated"));
    // Boundary: exactly 0.30 is excluded
    assert_eq!(classify_risk("red", 0.30), None);
    assert_eq!(classify_risk("red", 0.50), None);
}

#[test]
fn yellow_below_0_10_is_elevated() {
    assert_eq!(classify_risk("yellow", 0.05), Some("elevated"));
    assert_eq!(classify_risk("yellow", 0.09), Some("elevated"));
    // Boundary: exactly 0.10 is excluded
    assert_eq!(classify_risk("yellow", 0.10), None);
    assert_eq!(classify_risk("yellow", 0.50), None);
}

#[test]
fn green_is_always_excluded() {
    assert_eq!(classify_risk("green", 0.0), None);
    assert_eq!(classify_risk("green", 0.05), None);
    assert_eq!(classify_risk("green", 0.80), None);
}

#[test]
fn unknown_band_is_excluded() {
    assert_eq!(classify_risk("unknown", 0.05), None);
    assert_eq!(classify_risk("", 0.0), None);
}

#[test]
fn zero_share_red_is_high() {
    assert_eq!(classify_risk("red", 0.0), Some("high"));
}

#[test]
fn full_share_any_band_is_excluded() {
    assert_eq!(classify_risk("red", 1.0), None);
    assert_eq!(classify_risk("yellow", 1.0), None);
}

// ── Integration tests (delivery_repo fixture) ─────────────────────────────────

#[cfg(feature = "test-support")]
mod integration {
    use super::*;
    use codelore_lib::Options;
    use codelore_lib::facts::FactsDb;
    use codelore_lib::repo::gix_repo::GixRepo;

    fn ingest_delivery() -> (
        FactsDb,
        Options,
        codelore_lib::test_support::delivery_repo::DeliveryRepo,
    ) {
        let fixture = codelore_lib::test_support::delivery_repo::build();
        let db = FactsDb::new_in_memory().expect("in-memory db");
        let repo = GixRepo::open(fixture.dir.path()).expect("open repo");
        let opts = Options {
            repo_path: fixture.dir.path().to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");
        (db, opts, fixture)
    }

    #[test]
    fn delivery_repo_runs_without_error() {
        let (db, opts, _fixture) = ingest_delivery();
        run_marginal_owner_risk(&db, &opts).expect("run should succeed");
    }

    #[test]
    fn delivery_repo_every_row_satisfies_risk_invariants() {
        let (db, opts, _fixture) = ingest_delivery();
        let rows = run_marginal_owner_risk(&db, &opts).expect("run");
        for row in &rows {
            // Band is always yellow or red (green rows are excluded).
            assert!(
                row.band == "yellow" || row.band == "red",
                "unexpected band {:?} for path {:?}",
                row.band,
                row.path,
            );
            // Risk is always high or elevated.
            assert!(
                row.risk == "high" || row.risk == "elevated",
                "unexpected risk {:?} for path {:?}",
                row.risk,
                row.path,
            );
            // Top-active-share is in [0, 1].
            assert!(
                (0.0..=1.0).contains(&row.top_active_share),
                "top_active_share out of range: {} for {:?}",
                row.top_active_share,
                row.path,
            );
            // The classify_risk rule must be satisfied.
            let expected = classify_risk(&row.band, row.top_active_share);
            assert_eq!(
                expected,
                Some(row.risk.as_str()),
                "row {:?} does not satisfy classify_risk rule",
                row.path,
            );
            // Note field is non-empty.
            assert!(
                !row.note.is_empty(),
                "note must be non-empty for {:?}",
                row.path
            );
        }
    }
}
