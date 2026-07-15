//! Integration tests for the defect-calibration artifact model: serde
//! round-trip, byte-determinism across two saves, `load` version-mismatch
//! rejection, and `check_repo_identity` pass/fail/override behavior.
//!
//! The oracle's table-driven classification tests live in-module
//! (`codelore-lib/src/defect_calibration.rs`) since they need no filesystem
//! I/O; this file covers the parts that do (temp files, path canonicalization).

use std::io::Write;
use std::path::Path;

use codelore_lib::CodeLoreError;
use codelore_lib::defect_calibration::{
    self, DEFECT_FORMAT_VERSION, DefectArtifact, MiningStats, OracleConfig, TuningDecision,
    ValidationMetrics,
};

/// A fully-populated, hand-built artifact exercising every field — including
/// a non-trivial `TuningDecision::Applied` branch — so round-trip tests catch
/// any field silently dropped by serde.
fn sample_artifact(repo_identity: &str) -> DefectArtifact {
    DefectArtifact {
        format_version: DEFECT_FORMAT_VERSION,
        repo_identity: repo_identity.to_string(),
        head_at_mining: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
        vintage: "defects-2026-07-15".to_string(),
        generated_at: "2026-07-15T00:00:00Z".to_string(),
        oracle: OracleConfig {
            extra_patterns: vec!["JIRA-\\d+".to_string()],
        },
        mining: MiningStats {
            fixes_found: 42,
            links_found: 30,
            files_blamed: 55,
            lines_considered: 900,
            lines_dropped_cosmetic: 60,
            blame_failures: 2,
            pure_addition_fixes: 5,
        },
        validation: ValidationMetrics {
            band_table: vec![
                ("red".to_string(), 18, 0.6),
                ("yellow".to_string(), 9, 0.3),
                ("green".to_string(), 3, 0.1),
            ],
            auc_default: Some(0.71),
            precision_at_10: Some(0.5),
            precision_at_red: Some(0.65),
            implicated_files: 25,
            linked_defects: 30,
            sample_dates: vec!["2026-01-01".to_string(), "2026-04-01".to_string()],
            excluded_no_data: 2,
        },
        weights: vec![
            ("complexity".to_string(), 0.2),
            ("churn".to_string(), 0.15),
            ("ownership".to_string(), 0.1),
            ("coupling".to_string(), 0.15),
            ("clones".to_string(), 0.1),
            ("hotspot".to_string(), 0.1),
            ("god_class".to_string(), 0.1),
            ("age".to_string(), 0.1),
        ],
        tuning: TuningDecision::Applied {
            auc_train: 0.80,
            auc_validation_default: 0.71,
            auc_validation_tuned: 0.74,
        },
    }
}

fn write_temp_json(name: &str, art: &DefectArtifact) -> tempfile::TempPath {
    let f = tempfile::Builder::new()
        .prefix(name)
        .suffix(".calib.json")
        .tempfile()
        .expect("create temp artifact");
    let path = f.into_temp_path();
    // Reserve the path via the tempfile handle (avoids collisions with other
    // parallel test runs), then let `save` write the actual bytes through its
    // own file handle — exercising the function under test end-to-end.
    defect_calibration::save(art, Path::new(&path)).expect("save artifact");
    path
}

// ─── serde round-trip ────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_every_field() {
    let art = sample_artifact("a".repeat(64).as_str());
    let json = serde_json::to_vec(&art).expect("serialize");
    let back: DefectArtifact = serde_json::from_slice(&json).expect("deserialize");

    assert_eq!(back.format_version, art.format_version);
    assert_eq!(back.repo_identity, art.repo_identity);
    assert_eq!(back.head_at_mining, art.head_at_mining);
    assert_eq!(back.vintage, art.vintage);
    assert_eq!(back.generated_at, art.generated_at);
    assert_eq!(back.oracle, art.oracle);
    assert_eq!(back.mining, art.mining);
    assert_eq!(back.validation, art.validation);
    assert_eq!(back.weights, art.weights);
    assert_eq!(back.tuning, art.tuning);
}

#[test]
fn serde_round_trip_preserves_defaults_kept_variant() {
    let mut art = sample_artifact("b".repeat(64).as_str());
    art.tuning = TuningDecision::DefaultsKept {
        reason: "fewer than 30 linked defects".to_string(),
        auc_validation_default: None,
        auc_validation_tuned: None,
    };
    let json = serde_json::to_vec(&art).expect("serialize");
    let back: DefectArtifact = serde_json::from_slice(&json).expect("deserialize");
    assert_eq!(back.tuning, art.tuning);
}

// ─── save / load round-trip ──────────────────────────────────────────────────

#[test]
fn save_then_load_round_trips() {
    let art = sample_artifact("c".repeat(64).as_str());
    let path = write_temp_json("roundtrip", &art);
    let loaded = defect_calibration::load(Path::new(&path)).expect("load valid artifact");
    assert_eq!(loaded.vintage, art.vintage);
    assert_eq!(loaded.mining, art.mining);
    assert_eq!(loaded.tuning, art.tuning);
}

// ─── determinism ─────────────────────────────────────────────────────────────

#[test]
fn two_saves_of_the_same_artifact_are_byte_identical() {
    let art = sample_artifact("d".repeat(64).as_str());

    let path_a = tempfile::Builder::new()
        .prefix("det-a")
        .suffix(".calib.json")
        .tempfile()
        .expect("temp a")
        .into_temp_path();
    let path_b = tempfile::Builder::new()
        .prefix("det-b")
        .suffix(".calib.json")
        .tempfile()
        .expect("temp b")
        .into_temp_path();

    defect_calibration::save(&art, &path_a).expect("save a");
    defect_calibration::save(&art, &path_b).expect("save b");

    let bytes_a = std::fs::read(&path_a).expect("read a");
    let bytes_b = std::fs::read(&path_b).expect("read b");
    assert_eq!(
        bytes_a, bytes_b,
        "two saves of an identical artifact must be byte-identical"
    );
}

#[test]
fn save_writes_compact_not_pretty_json() {
    // Mirrors the `calibrate` command precedent: serde_json::to_vec (compact),
    // never to_vec_pretty. A pretty writer would introduce newlines/indentation.
    let art = sample_artifact("e".repeat(64).as_str());
    let path = tempfile::Builder::new()
        .prefix("compact")
        .suffix(".calib.json")
        .tempfile()
        .expect("temp")
        .into_temp_path();
    defect_calibration::save(&art, &path).expect("save");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        !text.contains('\n'),
        "compact JSON must not contain newlines, got: {text}"
    );
}

#[test]
fn save_creates_missing_parent_directories() {
    let base = tempfile::tempdir().expect("tempdir");
    let nested = base
        .path()
        .join("nested")
        .join("dirs")
        .join("defects.calib.json");
    let art = sample_artifact("f".repeat(64).as_str());
    defect_calibration::save(&art, &nested).expect("save into nested missing dirs");
    assert!(nested.exists());
}

// ─── load: version mismatch ───────────────────────────────────────────────────

#[test]
fn load_rejects_an_unknown_format_version() {
    let mut art = sample_artifact("0".repeat(64).as_str());
    art.format_version = DEFECT_FORMAT_VERSION + 1;
    let path = write_temp_json("bad-version", &art);
    let err = defect_calibration::load(Path::new(&path)).expect_err("unknown version must fail");
    let msg = err.to_string();
    assert!(
        msg.contains(&DEFECT_FORMAT_VERSION.to_string()),
        "error should name the supported version: {msg}"
    );
    assert!(
        msg.contains(&(DEFECT_FORMAT_VERSION + 1).to_string()),
        "error should name the artifact's (unsupported) version: {msg}"
    );
}

#[test]
fn load_rejects_malformed_json() {
    let mut f = tempfile::Builder::new()
        .prefix("garbage")
        .suffix(".calib.json")
        .tempfile()
        .expect("temp");
    f.write_all(b"{not valid json").expect("write");
    let path = f.into_temp_path();
    let err = defect_calibration::load(Path::new(&path)).expect_err("malformed json must fail");
    assert!(matches!(err, CodeLoreError::Analysis(_)));
}

#[test]
fn load_rejects_an_unreadable_path() {
    let missing = Path::new("/nonexistent/path/defects.calib.json");
    let err = defect_calibration::load(missing).expect_err("missing file must fail");
    assert!(
        matches!(err, CodeLoreError::RepoIo(_)),
        "unreadable path must surface as RepoIo, got: {err:?}"
    );
}

// ─── check_repo_identity: pass / fail / override ─────────────────────────────

#[test]
fn check_repo_identity_passes_for_the_mining_repo() {
    let repo_dir = std::env::temp_dir();
    let identity = defect_calibration::repo_identity(&repo_dir);
    let art = sample_artifact(&identity);
    defect_calibration::check_repo_identity(&art, &repo_dir, false)
        .expect("identity must match the repo it was mined from");
}

#[test]
fn check_repo_identity_fails_for_a_foreign_repo() {
    let mined_from = std::env::temp_dir();
    let identity = defect_calibration::repo_identity(&mined_from);
    let art = sample_artifact(&identity);

    // A different, real directory guarantees a distinct canonicalized path
    // (and thus a distinct identity) from `mined_from`.
    let foreign = tempfile::tempdir().expect("tempdir");
    let err = defect_calibration::check_repo_identity(&art, foreign.path(), false)
        .expect_err("foreign repo must be rejected");
    assert!(matches!(err, CodeLoreError::Analysis(_)));
}

#[test]
fn check_repo_identity_override_allows_a_foreign_repo() {
    let art = sample_artifact(&"0".repeat(64));
    let foreign = tempfile::tempdir().expect("tempdir");
    defect_calibration::check_repo_identity(&art, foreign.path(), true)
        .expect("allow_foreign must bypass the identity check");
}
