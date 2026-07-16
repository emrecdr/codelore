//! Integration tests for the defect-calibration artifact model: serde
//! round-trip, byte-determinism across two saves, `load` version-mismatch
//! rejection, and `check_repo_identity` pass/fail/override behavior. Also
//! covers the parts of Units C/D that need a real repo + `FactsDb`
//! connection — `band_history`'s uncapped at-rev scan and the
//! biomarker-intensity-capture / Rust-side risk-formula parity against a
//! real SQL `code-health` scan.
//!
//! The oracle's table-driven classification tests, and `validate`'s /
//! `tune_weights`'s hand-computed-value unit tests, live in-module
//! (`codelore-lib/src/defect_calibration/{mod,validate}.rs`) since they need
//! no filesystem I/O or repo fixture; this file covers the parts that do
//! (temp files, path canonicalization, a real ingested repo).

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

// ─── defect-validation analysis: reads the artifact through opts ─────────────

#[test]
fn run_defect_validation_reads_artifact_via_opts_and_returns_evidence_rows() {
    use codelore_lib::Options;
    use codelore_lib::analyses::defect_validation::run_defect_validation;

    let art = sample_artifact(&"1".repeat(64));
    // Keep the TempPath alive for the whole test so the file is not deleted
    // before `run_defect_validation` reads it.
    let path = write_temp_json("dv-run", &art);
    let opts = Options {
        defect_calibration: Some(Path::new(&path).to_path_buf()),
        // Synthetic identity → bypass the repo-identity guard the same way
        // `--allow-foreign-calibration` does.
        allow_foreign_calibration: true,
        ..Options::default()
    };

    let rows = run_defect_validation(&opts).expect("run defect-validation over a real artifact");
    let value = |metric: &str| {
        rows.iter()
            .find(|r| r.metric == metric)
            .unwrap_or_else(|| panic!("row {metric:?} must be present"))
            .value
            .as_str()
    };
    assert_eq!(value("vintage"), art.vintage);
    assert_eq!(value("weights_source"), "tuned (applied)");
    assert_eq!(value("band:red"), "18/30 defect-changes (60.0%)");
    // The Applied validation AUCs are surfaced plainly.
    assert_eq!(value("tuning_auc_validation_default"), "0.710");
    assert_eq!(value("tuning_auc_validation_tuned"), "0.740");
}

#[test]
fn run_defect_validation_without_artifact_returns_zero_rows() {
    use codelore_lib::Options;
    use codelore_lib::analyses::defect_validation::run_defect_validation;

    let opts = Options {
        defect_calibration: None,
        ..Options::default()
    };
    let rows = run_defect_validation(&opts).expect("honest absence must not be an error");
    assert!(rows.is_empty(), "no artifact configured -> zero rows");
}

#[test]
fn run_defect_validation_rejects_a_foreign_artifact_without_override() {
    use codelore_lib::Options;
    use codelore_lib::analyses::defect_validation::run_defect_validation;

    let art = sample_artifact(&"2".repeat(64)); // identity never matches a real dir
    let path = write_temp_json("dv-foreign", &art);
    let foreign = tempfile::tempdir().expect("tempdir");
    let opts = Options {
        defect_calibration: Some(Path::new(&path).to_path_buf()),
        repo_path: foreign.path().to_path_buf(),
        allow_foreign_calibration: false,
        ..Options::default()
    };
    let err = run_defect_validation(&opts)
        .expect_err("a foreign artifact must be rejected without --allow-foreign-calibration");
    assert!(matches!(err, CodeLoreError::Analysis(_)));
}

// ─── band_history: uncapped, at-rev, oldest-first ────────────────────────────

/// Run `git` in `dir` with a fixed author/committer date, panicking on
/// failure — the same shell-out idiom the other repo-building integration
/// tests use.
fn run_git_at(dir: &Path, date: &str, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

#[test]
fn band_history_is_oldest_first_and_uncapped() {
    let fx = codelore_lib::test_support::biomarker_repo::build();

    // Grow the fixture past `health_trend::file_series`'s top-50 hotspot
    // cap: one extra commit (dated after the fixture's newest, so it becomes
    // HEAD and the newest sample) adding 55 trivial one-function files. A
    // capped `band_history` implementation could pass every assertion below
    // on a sub-50-file tree; against 65 files it cannot.
    for i in 0..55 {
        std::fs::write(
            fx.dir.path().join(format!("src/filler_{i:02}.rs")),
            format!("pub fn filler_{i:02}() -> i32 {{\n    {i}\n}}\n"),
        )
        .expect("write filler file");
    }
    let date = "2026-06-07T10:00:00Z";
    run_git_at(
        fx.dir.path(),
        date,
        &["config", "user.email", "bio@example.com"],
    );
    run_git_at(fx.dir.path(), date, &["config", "user.name", "Bio"]);
    run_git_at(fx.dir.path(), date, &["add", "."]);
    run_git_at(
        fx.dir.path(),
        date,
        &["commit", "-m", "add filler files", "--quiet"],
    );

    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let samples =
        defect_calibration::validate::band_history(&db, &repo, &opts).expect("band_history");
    assert!(
        !samples.is_empty(),
        "fixture with several commits must yield samples"
    );

    // Oldest-first by date (non-decreasing) — `validate`'s
    // nearest-sample-at-or-before lookup assumes this ordering.
    for w in samples.windows(2) {
        assert!(
            w[0].0.as_str() <= w[1].0.as_str(),
            "samples must be oldest-first by date: {} then {}",
            w[0].0,
            w[1].0
        );
    }
    for (date, _) in &samples {
        assert!(
            date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-',
            "date must be zero-padded YYYY-MM-DD for lexicographic comparison: {date}"
        );
    }

    // The newest sample covers the same commit as a full-history HEAD scan
    // (the sampler always includes the newest commit) — every file HEAD
    // scores must have a band in that last sample. `health_trend::file_series`
    // caps this to the top 50 hotspot paths; `band_history` must NOT, since
    // `validate` needs a band for whichever file a mined defect touched, not
    // just the busiest ones.
    let head_rows =
        codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("head scan");
    assert!(
        head_rows.len() > 50,
        "fixture must outgrow a top-50 cap for this test to bite (got {} files)",
        head_rows.len()
    );
    let (_, last_bands) = samples.last().expect("at least one sample");
    assert!(
        last_bands.len() > 50,
        "band_history's newest sample must carry more entries than a top-50 cap would \
         allow (got {})",
        last_bands.len()
    );
    assert_eq!(
        last_bands.len(),
        head_rows.len(),
        "band_history's newest sample must score every file HEAD scores, not a top-N subset"
    );
    for row in &head_rows {
        assert!(
            last_bands.contains_key(&row.path),
            "missing band for {} at the newest sample — band_history must be uncapped",
            row.path
        );
    }
}

// ─── Unit D parity: Rust-side risk formula vs a real SQL scan ────────────────

#[test]
fn structural_risk_from_intensities_matches_a_real_sql_scan_on_the_biomarker_fixture() {
    use codelore_lib::analyses::code_health::{HealthScanCtx, run_code_health_scoped};
    use codelore_lib::defect_calibration::validate::{
        capture_intensities, default_weights, structural_risk_from_intensities,
    };

    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    // A single HEAD (`include_clones = true`) scan, then capture the
    // `code_health_biomarkers_v1` rows it leaves behind on the SAME
    // connection — exactly the recipe `capture_intensities`'s rustdoc
    // prescribes and `calibrate-defects` will follow.
    let head_rows = run_code_health_scoped(&db, &opts, &HealthScanCtx::head()).expect("head scan");
    let intensities = capture_intensities(&db).expect("capture intensities");

    assert!(
        !head_rows.is_empty(),
        "biomarker_repo must yield scored rows for the parity loop to bite"
    );

    let weights = default_weights();
    for row in &head_rows {
        let ints = intensities
            .get(&row.path)
            .unwrap_or_else(|| panic!("no captured intensities for {}", row.path));
        let rust_risk = structural_risk_from_intensities(&weights, ints);
        assert!(
            (rust_risk - row.structural_risk).abs() < 1e-9,
            "parity mismatch for {}: rust-side={rust_risk} sql structural_risk={}",
            row.path,
            row.structural_risk
        );
    }
}
