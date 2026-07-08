//! Tests for the `release-cadence` analysis.
//!
//! Fixture legend:
//!
//! - **`delivery_repo`** — Alice / Bob / Carol with annotated tags:
//!   `v0.1.0` (2026-01-11), `v0.2.0` (2026-02-20), `nightly-1`, `v1.0.0`
//!   (2026-04-21), and `light-1` (lightweight, same commit as `v1.0.0`).
//!
//!   With glob `v*`: 3 matched tags → 2 gaps (v0.1.0→v0.2.0 = 40 d,
//!   v0.2.0→v1.0.0 = 60 d); median = 50.0, IQR = 10.0 (linear
//!   interpolation: P25=45.0, P75=55.0).
//!
//!   With glob `*`: all 5 tags included.
use codelore_lib::Options;
use codelore_lib::analyses::release_cadence::run_release_cadence;
use codelore_lib::repo::gix_repo::GixRepo;

// ── helpers ──────────────────────────────────────────────────────────────────

fn open_delivery_repo() -> (GixRepo, std::path::PathBuf) {
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let path = fixture.dir.path().to_path_buf();
    // Keep fixture alive by leaking (it holds the TempDir).
    let repo = GixRepo::open(&path).expect("open repo");
    // leak the fixture so TempDir is not dropped while the test runs
    std::mem::forget(fixture);
    (repo, path)
}

// ── 1. v* glob tests ─────────────────────────────────────────────────────────

#[test]
fn v_star_glob_returns_three_tags_plus_summary() {
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let tag_rows: Vec<_> = rows.iter().filter(|r| r.tag != "__summary__").collect();
    assert_eq!(
        tag_rows.len(),
        3,
        "v* glob must match exactly v0.1.0, v0.2.0, v1.0.0; got {} rows: {:?}",
        tag_rows.len(),
        tag_rows.iter().map(|r| &r.tag).collect::<Vec<_>>(),
    );
    assert!(
        rows.iter().any(|r| r.tag == "__summary__"),
        "summary row must be present"
    );
}

#[test]
fn v_star_first_tag_has_no_gap() {
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let first = rows
        .iter()
        .find(|r| r.tag == "v0.1.0")
        .expect("v0.1.0 must be present");
    assert!(
        first.days_since_prev.is_none(),
        "first tag has no predecessor; days_since_prev must be None, got {:?}",
        first.days_since_prev,
    );
}

#[test]
fn v_star_gaps_are_exact() {
    // v0.1.0 tagger date 2026-01-11 → v0.2.0 tagger date 2026-02-20 = 40 d
    // v0.2.0 tagger date 2026-02-20 → v1.0.0 tagger date 2026-04-21 = 60 d
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let v020 = rows
        .iter()
        .find(|r| r.tag == "v0.2.0")
        .expect("v0.2.0 must be present");
    let gap1 = v020.days_since_prev.expect("v0.2.0 must have a gap");
    assert!(
        (gap1 - 40.0).abs() < 0.1,
        "v0.1.0 → v0.2.0 gap must be 40 d; got {gap1:.2}"
    );

    let v100 = rows
        .iter()
        .find(|r| r.tag == "v1.0.0")
        .expect("v1.0.0 must be present");
    let gap2 = v100.days_since_prev.expect("v1.0.0 must have a gap");
    assert!(
        (gap2 - 60.0).abs() < 0.1,
        "v0.2.0 → v1.0.0 gap must be 60 d; got {gap2:.2}"
    );
}

#[test]
fn v_star_summary_median_is_50() {
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let summary = rows
        .iter()
        .find(|r| r.tag == "__summary__")
        .expect("summary row must be present");
    let median = summary
        .days_since_prev
        .expect("summary days_since_prev carries median");
    assert!(
        (median - 50.0).abs() < 0.1,
        "median of [40, 60] must be 50.0; got {median:.2}"
    );
}

#[test]
fn v_star_summary_iqr_encoded_in_date() {
    // 2-gap series [40, 60], linear-interpolation percentiles:
    //   P25: idx = 0.25 × 1 = 0.25 → 40×0.75 + 60×0.25 = 45.0
    //   P75: idx = 0.75 × 1 = 0.75 → 40×0.25 + 60×0.75 = 55.0
    //   IQR = 55.0 − 45.0 = 10.0
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let summary = rows
        .iter()
        .find(|r| r.tag == "__summary__")
        .expect("summary row must be present");
    // date field carries "iqr=N.NNd"
    assert!(
        summary.date.starts_with("iqr="),
        "summary date must start with 'iqr='; got {:?}",
        summary.date,
    );
    let iqr_str = summary
        .date
        .trim_start_matches("iqr=")
        .trim_end_matches('d');
    let iqr: f64 = iqr_str.parse().expect("iqr must be a number");
    assert!(
        (iqr - 10.0).abs() < 0.1,
        "IQR of [40, 60] (linear interpolation) must be 10.0; got {iqr:.2}"
    );
}

#[test]
fn v_star_trend_is_slowing() {
    // gaps [40, 60] → slope = 20 > 0.1 → slowing
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let summary = rows
        .iter()
        .find(|r| r.tag == "__summary__")
        .expect("summary row");
    assert_eq!(
        summary.trend, "slowing",
        "gaps [40, 60] have positive slope → slowing; got {:?}",
        summary.trend,
    );
}

// ── 2. Glob exclusion ────────────────────────────────────────────────────────

#[test]
fn nightly_tag_excluded_by_v_star_glob() {
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "v*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    assert!(
        !rows.iter().any(|r| r.tag == "nightly-1"),
        "nightly-1 must be excluded by v* glob"
    );
    assert!(
        !rows.iter().any(|r| r.tag == "light-1"),
        "light-1 must be excluded by v* glob"
    );
}

#[test]
fn star_glob_includes_all_tags() {
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    let tag_rows: Vec<_> = rows.iter().filter(|r| r.tag != "__summary__").collect();
    assert_eq!(
        tag_rows.len(),
        5,
        "glob '*' must include all 5 tags; got {} rows: {:?}",
        tag_rows.len(),
        tag_rows.iter().map(|r| &r.tag).collect::<Vec<_>>(),
    );
}

// ── 3. Edge cases ────────────────────────────────────────────────────────────

#[test]
fn unmatched_glob_returns_empty() {
    let (repo, path) = open_delivery_repo();
    let opts = Options {
        repo_path: path,
        release_tag_glob: "release/*".to_string(),
        ..Options::default()
    };
    let rows = run_release_cadence(&repo, &opts).expect("run release-cadence");
    assert!(
        rows.is_empty(),
        "no tags match 'release/*'; must return empty vec"
    );
}
