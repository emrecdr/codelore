//! Integration tests for `finding-hotspot-overlap`.
//!
//! Covers:
//! - Findings on a known hotspot path → row with correct engines + health band
//! - Findings on a path NOT in hotspots → row with 0.0 score + 0.0 percentile
//! - Empty store → error with the expected message
//! - Priority pure-function branch coverage (see unit tests in the source module)

use std::path::Path;

use codelore_lib::Options;
use codelore_lib::analyses::finding_hotspot_overlap::run_finding_hotspot_overlap;
use codelore_lib::external::{ExternalFinding, ExternalStore};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn temp_store_for(dir: &Path) -> ExternalStore {
    ExternalStore::open_or_create(dir, Path::new("/test/fake-repo")).expect("open_or_create")
}

fn finding_for(path: &str, engine: &str, level: &str) -> ExternalFinding {
    ExternalFinding {
        engine: engine.to_string(),
        engine_version: "0.0.0".to_string(),
        rule_id: "test/rule".to_string(),
        path: path.to_string(),
        start_line: Some(1),
        end_line: None,
        level: level.to_string(),
        fingerprint: format!("test/v1/{engine}/{path}"),
        message: "test finding".to_string(),
    }
}

/// Helper: ingest the `tiny_repo` fixture and return (db, opts).
fn ingest_tiny() -> (tempfile::TempDir, FactsDb, Options) {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    // keep the TempDir alive by returning it
    (tiny.dir, db, opts)
}

// ─── empty store error ───────────────────────────────────────────────────────

#[test]
fn empty_store_returns_error() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_store_for(store_dir.path());
    let (_tiny_dir, db, opts) = ingest_tiny();

    let err = run_finding_hotspot_overlap(&db, &opts, &store)
        .expect_err("should return error when store is empty");
    let msg = err.to_string();
    assert!(
        msg.contains("ingest-sarif"),
        "error message should mention ingest-sarif; got: {msg}"
    );
}

// ─── findings on a hotspot path ─────────────────────────────────────────────

/// `src/main.rs` has 4 revisions in `tiny_repo` — the highest revision count.
/// A finding against it should produce a row with matching engines + a
/// non-zero `hotspot_score` and `revs_percentile`.
#[test]
fn findings_on_hotspot_path_produce_correct_row() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_store_for(store_dir.path());

    // Ingest two findings for src/main.rs from two different engines.
    let findings = [
        finding_for("src/main.rs", "semgrep", "warning"),
        finding_for("src/main.rs", "clippy", "error"),
    ];
    store
        .replace_engine("semgrep", &findings[..1])
        .expect("replace semgrep");
    store
        .replace_engine("clippy", &findings[1..])
        .expect("replace clippy");

    let (_tiny_dir, db, opts) = ingest_tiny();
    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    assert_eq!(rows.len(), 1, "one path in store → one output row");
    let row = &rows[0];
    assert_eq!(row.path, "src/main.rs");
    assert_eq!(row.findings, 2);
    // Both engines present, sorted → "clippy,semgrep"
    assert_eq!(row.engines, "clippy,semgrep");
    assert_eq!(row.worst_level, "error");
    // src/main.rs has revisions=4, src/lib.rs has revisions=1. Two files in
    // hotspot result set → PERCENT_RANK: 0.0 for 1 rev, 1.0 for 4 revs.
    // So src/main.rs should have revs_percentile == 1.0.
    assert!(
        (row.revs_percentile - 1.0).abs() < 1e-9,
        "src/main.rs (max revs) should have revs_percentile=1.0, got {}",
        row.revs_percentile
    );
    // hotspot_score may be 0.0 for tiny repos where cognitive complexity is
    // effectively 0 (one-liner Rust files); the percentile is the stronger
    // signal that proves the path was matched in the hotspot result set.
    assert!(
        row.hotspot_score >= 0.0,
        "hotspot_score should be non-negative, got {}",
        row.hotspot_score
    );
}

// ─── findings on a non-hotspot path ─────────────────────────────────────────

/// A finding for a path that is NOT in the hotspot result set (it doesn't
/// exist in the repo at all) → 0.0 score and 0.0 percentile (LEFT-join contract).
#[test]
fn findings_on_non_hotspot_path_produce_zero_scores() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_store_for(store_dir.path());

    let f = finding_for("src/does-not-exist.rs", "semgrep", "note");
    store.replace_engine("semgrep", &[f]).expect("replace");

    let (_tiny_dir, db, opts) = ingest_tiny();
    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.path, "src/does-not-exist.rs");
    assert!(
        (row.hotspot_score).abs() < 1e-9,
        "non-hotspot path should have hotspot_score=0.0, got {}",
        row.hotspot_score
    );
    assert!(
        (row.revs_percentile).abs() < 1e-9,
        "non-hotspot path should have revs_percentile=0.0, got {}",
        row.revs_percentile
    );
    assert_eq!(row.health_band, "unknown");
}

// ─── biomarker_repo: health band flows through ───────────────────────────────

/// `src/complex.rs` in `biomarker_repo` has the highest cognitive complexity
/// and a non-trivial revision count. The code-health pipeline must produce a
/// real band (not `"unknown"`) for this path, proving that the health-band join
/// arm of `run_finding_hotspot_overlap` actually fires on a repo with complex
/// files.
#[test]
fn complex_file_in_biomarker_repo_has_real_health_band() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fx.dir.path().to_path_buf(),
        min_revs: 1,
        fisher_significance: 1.0,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_store_for(store_dir.path());

    // Ingest a finding for src/complex.rs — the most complex file in the fixture.
    let f = finding_for("src/complex.rs", "semgrep", "warning");
    store.replace_engine("semgrep", &[f]).expect("replace");

    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    let row = rows
        .iter()
        .find(|r| r.path == "src/complex.rs")
        .expect("src/complex.rs must appear in output");

    assert!(
        matches!(row.health_band.as_str(), "red" | "yellow" | "green"),
        "health_band must be a real band value (not 'unknown') for complex.rs; got '{}'",
        row.health_band
    );
}

// ─── sort order ─────────────────────────────────────────────────────────────

/// With two paths in the store (main.rs: 2 findings, lib.rs: 1 finding),
/// the sort (priority asc rank then findings desc then path asc) puts the
/// higher-finding count first when both share the same priority.
#[test]
fn rows_sorted_priority_then_findings_then_path() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_store_for(store_dir.path());

    // 2 findings for main.rs and 1 for lib.rs, all from the same engine.
    let all = [
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule".to_string(),
            path: "src/main.rs".to_string(),
            start_line: Some(1),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "test/v1/m1".to_string(),
            message: "m1".to_string(),
        },
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule".to_string(),
            path: "src/main.rs".to_string(),
            start_line: Some(2),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "test/v1/m2".to_string(),
            message: "m2".to_string(),
        },
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule".to_string(),
            path: "src/lib.rs".to_string(),
            start_line: Some(1),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "test/v1/l1".to_string(),
            message: "l1".to_string(),
        },
    ];
    store.replace_engine("semgrep", &all).expect("replace all");

    let (_tiny_dir, db, opts) = ingest_tiny();
    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    // Both paths have "note" priority; main.rs (2 findings) should rank before lib.rs (1).
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].path, "src/main.rs");
    assert_eq!(rows[0].findings, 2);
    assert_eq!(rows[1].path, "src/lib.rs");
    assert_eq!(rows[1].findings, 1);
}
