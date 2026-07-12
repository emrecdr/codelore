use std::io::Write;

use codelore_lib::Options;
use codelore_lib::calibration::{
    CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MetricQuantiles,
    QUANTILE_POINTS, Stratum,
};
use codelore_lib::facts::FactsDb;
use codelore_lib::provenance::Manifest;
use codelore_lib::repo::GixRepo;

// ─── calibration fixture helpers ─────────────────────────────────────────────

/// A minimal valid quantile vector (all zeros) to satisfy length + monotonicity.
fn zero_quantiles() -> Vec<f64> {
    vec![0.0; QUANTILE_POINTS]
}

/// One-language artifact with `corpus_vintage = "test-vintage-2026-07"` and
/// enough sample functions to be above the floor.
fn test_calib_artifact() -> CalibrationArtifact {
    CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-vintage-2026-07".to_string(),
        generated_at: "2026-07-12T00:00:00Z".to_string(),
        repos_included: 1,
        repos_attempted: 1,
        languages: vec![LanguageTable {
            language: "rust".to_string(),
            sample_functions: 4_000,
            strata: vec![Stratum {
                sloc_min: 0,
                sloc_max: u64::MAX,
                metrics: vec![MetricQuantiles {
                    metric: "cyclomatic".to_string(),
                    quantiles: zero_quantiles(),
                }],
            }],
        }],
    }
}

fn write_temp_artifact(art: &CalibrationArtifact) -> tempfile::TempPath {
    let mut f = tempfile::Builder::new()
        .prefix("provenance_test_calib")
        .suffix(".calib.json")
        .tempfile()
        .expect("create temp artifact");
    let bytes = serde_json::to_vec(art).expect("serialize artifact");
    f.write_all(&bytes).expect("write artifact");
    f.into_temp_path()
}

#[test]
fn manifest_captures_basic_fields() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let manifest = Manifest::capture(&db, &opts, "revisions").expect("capture");
    assert!(
        !manifest.codelore_version.is_empty(),
        "codelore_version should be populated"
    );
    assert_eq!(manifest.analysis, "revisions");
    assert_eq!(manifest.min_revs, 1);

    let json = manifest.to_json().expect("json");
    assert!(json.contains("\"analysis\""));
    assert!(json.contains("\"codelore_version\""));
}

/// Schema v2 reproducibility-critical fields: `head_sha`,
/// `cache_key_hash`, `rust_version`, `target_triple`, and
/// `grammars`. SLSA L3 verifiers depend on these — a regression
/// that dropped them would surface only when an auditor tried to
/// reproduce a build and discovered the manifest carries
/// insufficient state.
#[test]
fn manifest_captures_reproducibility_fields() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let m = Manifest::capture(&db, &opts, "revisions").expect("capture");

    // Schema version pinned at 2. Bump again when
    // future field changes break consumer compatibility.
    assert_eq!(m.schema_version, 2);

    // head_sha must round-trip through the fact store. The
    // 40-char check pins the contract: a degenerate empty value
    // here means the ingest didn't populate `commits` (test broken)
    // OR the query rolled back (regression). Both surface clearly.
    assert_eq!(
        m.head_sha.len(),
        40,
        "head_sha must be the 40-char SHA, got {} chars: {:?}",
        m.head_sha.len(),
        m.head_sha,
    );

    // Cache key hash is the SHA-256 of the cache key — 64 hex chars.
    assert_eq!(
        m.cache_key_hash.len(),
        64,
        "cache_key_hash must be 64 hex chars (SHA-256), got {} chars",
        m.cache_key_hash.len(),
    );
    assert!(
        m.cache_key_hash.chars().all(|c| c.is_ascii_hexdigit()),
        "cache_key_hash must be hex-encoded",
    );

    // Rust version + target triple. We don't assert specific values
    // (a future toolchain bump shouldn't break this test) — just
    // that they're populated.
    assert!(!m.rust_version.is_empty());
    assert_ne!(
        m.target_triple, "unknown",
        "target_triple unknown — extend the cfg!() ladder in provenance/mod.rs",
    );

    // Grammar map must include every tree-sitter dep we ship. The
    // exact pin values can drift over time (tree-sitter bumps are
    // coordinated per CLAUDE.md) but every entry must be present and
    // non-empty.
    for crate_name in [
        "tree-sitter",
        "tree-sitter-rust",
        "tree-sitter-python",
        "tree-sitter-java",
        "tree-sitter-javascript",
        "tree-sitter-typescript",
    ] {
        let pin = m
            .grammars
            .get(crate_name)
            .unwrap_or_else(|| panic!("grammar pin missing: {crate_name}"));
        assert!(
            !pin.is_empty(),
            "grammar pin for {crate_name} must not be empty",
        );
    }

    // JSON serialization must include every new field — downstream
    // consumers grep on the field names.
    let json = m.to_json().expect("json");
    for field in [
        "\"schema_version\"",
        "\"head_sha\"",
        "\"cache_key_hash\"",
        "\"rust_version\"",
        "\"target_triple\"",
        "\"grammars\"",
    ] {
        assert!(
            json.contains(field),
            "serialized manifest missing field {field}",
        );
    }
}

// ─── corpus_vintage stamp ─────────────────────────────────────────────────────

/// No `--calibration` override → the stamp falls through to the embedded world
/// corpus, so `corpus_vintage` is the embedded vintage and present in JSON.
#[test]
fn manifest_corpus_vintage_is_embedded_world_without_calibration() {
    let embedded = codelore_lib::calibration::embedded_world()
        .expect("the embedded world corpus must be active for this test");
    let embedded_vintage = embedded.corpus_vintage.clone();

    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        calibration: None,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let m = Manifest::capture(&db, &opts, "code-health").expect("capture");
    assert_eq!(
        m.corpus_vintage.as_deref(),
        Some(embedded_vintage.as_str()),
        "corpus_vintage must be the embedded world vintage when no --calibration is passed"
    );

    let json = m.to_json().expect("json");
    assert!(
        json.contains("\"corpus_vintage\""),
        "corpus_vintage must be present in JSON when the embedded world is active"
    );
    assert!(
        json.contains(&embedded_vintage),
        "JSON must carry the embedded world vintage string"
    );
}

/// Explicit `--calibration` path → `corpus_vintage` matches the artifact's own vintage.
#[test]
fn manifest_corpus_vintage_present_with_calibration_file() {
    let art = test_calib_artifact();
    let artifact_path = write_temp_artifact(&art);

    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        calibration: Some(artifact_path.to_path_buf()),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let m = Manifest::capture(&db, &opts, "code-health").expect("capture");
    assert_eq!(
        m.corpus_vintage.as_deref(),
        Some("test-vintage-2026-07"),
        "corpus_vintage must match the artifact's corpus_vintage field"
    );

    let json = m.to_json().expect("json");
    assert!(
        json.contains("\"corpus_vintage\""),
        "corpus_vintage must be present in JSON when Some"
    );
    assert!(
        json.contains("test-vintage-2026-07"),
        "JSON must carry the vintage string"
    );
}

/// Precedence branch 2 (embedded world) and the shared-seam guarantee.
///
/// `active_vintage` and the corpus-percentile lens both resolve through the one
/// `load_active_artifact` home, so the vintage stamped into provenance is always
/// the vintage of the artifact the lens actually applied. This asserts that
/// invariant on the seam directly: for a real-vintage `--calibration` file both
/// entry points agree, and with no override both fall through to the embedded
/// world (branch 2), which now carries a real corpus.
#[test]
fn active_vintage_and_lens_share_one_resolution_seam() {
    // File override present: load_active_artifact yields the artifact the lens
    // consumes, and active_vintage yields exactly that artifact's vintage.
    let art = test_calib_artifact();
    let artifact_path = write_temp_artifact(&art);
    let with_file = Options {
        calibration: Some(artifact_path.to_path_buf()),
        ..Options::default()
    };
    let resolved = codelore_lib::calibration::load_active_artifact(&with_file)
        .expect("resolve")
        .expect("a --calibration file must resolve to an artifact");
    assert_eq!(
        resolved.corpus_vintage, "test-vintage-2026-07",
        "the lens resolves the artifact whose vintage the stamp records"
    );
    assert_eq!(
        codelore_lib::calibration::active_vintage(&with_file).expect("vintage"),
        Some("test-vintage-2026-07".to_string()),
        "active_vintage must be the same artifact's vintage, via the shared seam"
    );

    // No override: both entry points fall through to the embedded world corpus.
    // Both resolve to the same real artifact — the vintage the lens applied is
    // exactly the vintage the stamp records.
    let embedded_vintage = codelore_lib::calibration::embedded_world()
        .expect("embedded world corpus must be active")
        .corpus_vintage
        .clone();
    let no_override = Options::default();
    let resolved = codelore_lib::calibration::load_active_artifact(&no_override)
        .expect("resolve")
        .expect("with no --calibration the seam resolves to the embedded world");
    assert_eq!(
        resolved.corpus_vintage, embedded_vintage,
        "branch 2 resolves the embedded world corpus"
    );
    assert_eq!(
        codelore_lib::calibration::active_vintage(&no_override).expect("vintage"),
        Some(embedded_vintage),
        "active_vintage agrees with the seam on the embedded world vintage"
    );
}
