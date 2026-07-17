//! Integration tests for the per-file enrichment fact sheet: determinism
//! across two builds on the same fact store, the mandatory-code-health error
//! path, and numeric-value extraction. Uses the biomarker fixture (a repo with
//! a deliberate complexity gradient and co-changed files) ingested through a
//! real `FactsDb`, mirroring the ingest pattern in `defect_calibration_test`.
//!
//! It also exercises the sidecar narrative cache and the `narrate` orchestrator
//! end-to-end across the crate boundary, over a real filesystem cache root and a
//! test-local `ChatClient`.

use codelore_lib::CodeLoreError;
use codelore_lib::Result;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::defect_calibration::{
    self, DEFECT_FORMAT_VERSION, DefectArtifact, MiningStats, OracleConfig, TuningDecision,
    ValidationMetrics,
};
use codelore_lib::enrichment::client::ChatClient;
use codelore_lib::enrichment::engine::SheetFacts;
use codelore_lib::enrichment::fact_sheet::FileFactSheet;
use codelore_lib::enrichment::prompt::Lens;
use codelore_lib::enrichment::{cache, engine};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::{biomarker_repo, permissive_coupling_opts};

/// A chat client with a canned reply that counts how many times it is asked to
/// complete — so a test can prove a second `narrate` served from cache never
/// reached the model.
struct CountingChatClient {
    reply: String,
    model: String,
    calls: std::cell::Cell<usize>,
}

impl ChatClient for CountingChatClient {
    fn complete(&self, _system: &str, _user: &str) -> Result<String> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.reply.clone())
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

#[test]
fn narrate_misses_then_hits_then_refresh_regenerates() {
    let cache_root = tempfile::tempdir().expect("cache root");
    let repo_path = std::path::Path::new("/tmp/some-repo");
    let sheet_text = "code-health\n  score = 87.5\n  band = green\n";
    let values = [87.5];
    let facts = SheetFacts {
        text: sheet_text,
        values: &values,
    };

    let client = CountingChatClient {
        reply: "Diagnosis: the health score is 87.5.".to_string(),
        model: "mock-model".to_string(),
        calls: std::cell::Cell::new(0),
    };

    // First call: cache miss — the client is asked, result is fresh.
    let first = engine::narrate(
        &client,
        Lens::FileDiagnosis,
        "src/health.rs",
        facts,
        cache_root.path(),
        repo_path,
        false,
    )
    .expect("first narrate");
    assert!(!first.from_cache, "a cold cache must miss");
    assert!(first.grounded, "87.5 is grounded by the fact value");
    assert_eq!(client.calls.get(), 1, "the model was consulted once");

    // Second call: cache hit — same key, the client must NOT be reached again.
    let second = engine::narrate(
        &client,
        Lens::FileDiagnosis,
        "src/health.rs",
        facts,
        cache_root.path(),
        repo_path,
        false,
    )
    .expect("second narrate");
    assert!(second.from_cache, "a warm cache must hit");
    assert_eq!(
        second.narrative, first.narrative,
        "hit returns the cached text"
    );
    assert_eq!(
        client.calls.get(),
        1,
        "a cache hit does not reach the model"
    );

    // Refresh forces regeneration regardless of the warm cache.
    let refreshed = engine::narrate(
        &client,
        Lens::FileDiagnosis,
        "src/health.rs",
        facts,
        cache_root.path(),
        repo_path,
        true,
    )
    .expect("refresh narrate");
    assert!(!refreshed.from_cache, "refresh must bypass the cache");
    assert_eq!(client.calls.get(), 2, "refresh reaches the model again");

    // The sidecar carries the fact digest for the staleness comparison, scoped to
    // this file's own subject; it equals the digest of the canonical sheet text.
    let latest = cache::latest_for_subject(cache_root.path(), repo_path, "src/health.rs")
        .expect("a cached narrative exists");
    assert_eq!(latest.model, "mock-model");
    assert_eq!(latest.subject, "src/health.rs");
    assert_eq!(latest.fact_digest, sheet_digest(sheet_text));

    // A sibling that was never narrated has no narrative of its own, so a
    // staleness check for it must not surface this file's entry.
    assert!(
        cache::latest_for_subject(cache_root.path(), repo_path, "src/other.rs").is_none(),
        "the staleness check is scoped to the subject's own narratives"
    );

    // The stamp reflects the grounded verdict.
    assert!(engine::stamp(&first).contains("grounded"));
}

/// Lowercase-hex SHA-256 of `text`, mirroring the fact-sheet digest so the test
/// can check `narrate` stored the sheet-text digest.
fn sheet_digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Ingest the biomarker fixture and return the pieces `FileFactSheet::build`
/// needs. The `TempDir` inside `BiomarkerRepo` must outlive the returned repo,
/// so the fixture is returned too.
fn ingest_biomarker_fixture() -> (
    biomarker_repo::BiomarkerRepo,
    GixRepo,
    FactsDb,
    codelore_lib::Options,
) {
    let fx = biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open fixture");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest fixture");
    (fx, repo, db, opts)
}

#[test]
fn fact_sheet_is_deterministic() {
    let (_fx, repo, db, opts) = ingest_biomarker_fixture();

    // Pick a real tracked path (the worst-scoring file is first, deterministic)
    // and remember its band so we can assert the canonical text carries it.
    let health = run_code_health(&db, &opts.with_no_row_limit()).expect("code-health");
    let target = health
        .first()
        .expect("fixture yields code-health rows")
        .path
        .clone();
    let band = health[0].band.clone();

    let first = FileFactSheet::build(&db, &repo, &opts, &target).expect("build 1");
    let second = FileFactSheet::build(&db, &repo, &opts, &target).expect("build 2");

    assert_eq!(
        first.to_canonical_text(),
        second.to_canonical_text(),
        "two builds over the same fact store must be byte-identical"
    );
    assert_eq!(
        first.digest(),
        second.digest(),
        "equal text -> equal digest"
    );

    let canonical = first.to_canonical_text();
    assert!(
        canonical.contains("code-health"),
        "canonical text must carry the mandatory code-health section: {canonical}"
    );
    assert!(
        canonical.contains(&band),
        "canonical text must carry the file's band ({band}): {canonical}"
    );
}

#[test]
fn fact_sheet_unknown_path_errors() {
    let (_fx, repo, db, opts) = ingest_biomarker_fixture();

    let err = FileFactSheet::build(&db, &repo, &opts, "src/does_not_exist.rs")
        .expect_err("an untracked path has no code-health data");
    assert!(
        matches!(err, CodeLoreError::Analysis(_)),
        "a missing code-health row is an analysis error, got: {err:?}"
    );
    assert!(
        err.to_string().contains("src/does_not_exist.rs"),
        "the error message must name the offending path: {err}"
    );
}

/// A minimal-but-plausible defect-calibration artifact for the
/// defect-evidence section test — every field the section reads is
/// populated; `repo_identity` is arbitrary because the test applies the
/// artifact with `allow_foreign_calibration: true`. `weights` are the
/// built-in smell defaults in canonical order — `active_weights` (consulted
/// by the code-health section that builds ahead of defect-evidence in the
/// same dossier) rejects anything else as a malformed artifact.
fn minimal_defect_artifact() -> DefectArtifact {
    DefectArtifact {
        format_version: DEFECT_FORMAT_VERSION,
        repo_identity: "test-repo-identity".to_string(),
        head_at_mining: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string(),
        vintage: "defects-2026-07-17".to_string(),
        generated_at: "2026-07-17T00:00:00Z".to_string(),
        oracle: OracleConfig::default(),
        mining: MiningStats::default(),
        validation: ValidationMetrics {
            band_table: vec![("red".to_string(), 5, 1.0)],
            auc_default: None,
            precision_at_10: None,
            precision_at_red: None,
            implicated_files: 3,
            linked_defects: 5,
            sample_dates: vec!["2026-01-01".to_string()],
            excluded_no_data: 0,
        },
        weights: defect_calibration::validate::default_weights(),
        tuning: TuningDecision::DefaultsKept {
            reason: "insufficient evidence for weight tuning".to_string(),
            auc_validation_default: None,
            auc_validation_tuned: None,
        },
    }
}

#[test]
fn fact_sheet_defect_evidence_section_is_additive() {
    let (_fx, repo, db, opts) = ingest_biomarker_fixture();
    let health = run_code_health(&db, &opts.with_no_row_limit()).expect("code-health");
    let target = health
        .first()
        .expect("fixture yields code-health rows")
        .path
        .clone();

    // Additive contract: without a configured artifact, no defect-evidence
    // section appears.
    let without = FileFactSheet::build(&db, &repo, &opts, &target).expect("build without artifact");
    assert!(
        !without.to_canonical_text().contains("defect-evidence"),
        "no defect-evidence section without a configured artifact: {}",
        without.to_canonical_text()
    );

    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let artifact_path = artifact_dir.path().join("defects.calib.json");
    defect_calibration::save(&minimal_defect_artifact(), &artifact_path).expect("save artifact");

    let mut opts_with_calibration = opts.clone();
    opts_with_calibration.defect_calibration = Some(artifact_path);
    opts_with_calibration.allow_foreign_calibration = true;

    let with = FileFactSheet::build(&db, &repo, &opts_with_calibration, &target)
        .expect("build with artifact");
    let canonical = with.to_canonical_text();
    assert!(
        canonical.contains("defect-evidence"),
        "defect-evidence section present when an artifact is configured: {canonical}"
    );
    assert!(
        canonical.contains("vintage = defects-2026-07-17"),
        "defect-evidence carries the artifact vintage: {canonical}"
    );
    assert!(
        canonical.contains("linked_defects = 5"),
        "defect-evidence carries linked_defects: {canonical}"
    );
}

#[test]
fn numeric_values_extracts_floats() {
    // A hand-built sheet: only whole-number-parseable values are extracted, in
    // section then key order; string values (band, author) are skipped.
    let sheet = FileFactSheet {
        path: "src/foo.rs".to_string(),
        sections: vec![
            (
                "code-health".to_string(),
                vec![
                    ("score".to_string(), "72.5".to_string()),
                    ("band".to_string(), "yellow".to_string()),
                    ("structural_risk".to_string(), "0.41".to_string()),
                ],
            ),
            (
                "ownership".to_string(),
                vec![
                    ("main_author".to_string(), "Bio".to_string()),
                    ("total_revs".to_string(), "6".to_string()),
                ],
            ),
        ],
    };
    assert_eq!(sheet.numeric_values(), vec![72.5, 0.41, 6.0]);
}
