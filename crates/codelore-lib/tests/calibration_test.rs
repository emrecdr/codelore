//! Unit tests for the corpus-calibration artifact: serde round-trip, the
//! quantile-breakpoint interpolation contract of `percentile`, `load`
//! validation (format version + quantile monotonicity), and the
//! sample-count-weighted `merge` approximation.

use std::io::Write;
use std::path::Path;

use codelore_lib::calibration::{
    self, CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MIN_LANG_SAMPLE,
    MetricQuantiles, QUANTILE_POINTS, Stratum,
};

// ─── fixtures ────────────────────────────────────────────────────────────────

/// A `QUANTILE_POINTS`-long quantile vector rising linearly from `min` to
/// `max`, so `q[i] == min + (max - min) * i / (QUANTILE_POINTS - 1)`. With
/// `min = 0.0`, `max = 1000.0` the breakpoint index equals the value, making
/// the interpolation arithmetic checkable by hand.
///
/// Indices are bounded by `QUANTILE_POINTS` (~1e3), far below `2^53`, so the
/// `usize` → `f64` casts are exact.
#[allow(clippy::cast_precision_loss)]
fn linear_quantiles(min: f64, max: f64) -> Vec<f64> {
    let last = (QUANTILE_POINTS - 1) as f64;
    (0..QUANTILE_POINTS)
        .map(|i| min + (max - min) * (i as f64) / last)
        .collect()
}

/// One-language, one-stratum artifact whose single `cyclomatic` metric has the
/// hand-checkable `0..=1000` linear ramp and a sample count above the floor.
fn ramp_artifact() -> CalibrationArtifact {
    CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-ramp".to_string(),
        generated_at: "2026-07-12T00:00:00Z".to_string(),
        repos_included: 3,
        repos_attempted: 3,
        languages: vec![LanguageTable {
            language: "rust".to_string(),
            sample_functions: 4_000,
            strata: vec![Stratum {
                sloc_min: 0,
                sloc_max: u64::MAX,
                metrics: vec![MetricQuantiles {
                    metric: "cyclomatic".to_string(),
                    quantiles: linear_quantiles(0.0, 1000.0),
                }],
            }],
        }],
    }
}

fn write_temp_json(name: &str, art: &CalibrationArtifact) -> tempfile::TempPath {
    let mut f = tempfile::Builder::new()
        .prefix(name)
        .suffix(".calib.json")
        .tempfile()
        .expect("create temp artifact");
    let bytes = serde_json::to_vec(art).expect("serialize artifact");
    f.write_all(&bytes).expect("write artifact bytes");
    f.into_temp_path()
}

// ─── serde round-trip ────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_the_artifact() {
    let art = ramp_artifact();
    let json = serde_json::to_vec(&art).expect("serialize");
    let back: CalibrationArtifact = serde_json::from_slice(&json).expect("deserialize");

    assert_eq!(back.format_version, art.format_version);
    assert_eq!(back.corpus_vintage, art.corpus_vintage);
    assert_eq!(back.generated_at, art.generated_at);
    assert_eq!(back.repos_included, art.repos_included);
    assert_eq!(back.repos_attempted, art.repos_attempted);
    assert_eq!(back.languages.len(), 1);
    assert_eq!(back.languages[0].language, "rust");
    assert_eq!(back.languages[0].sample_functions, 4_000);
    assert_eq!(
        back.languages[0].strata[0].metrics[0].quantiles.len(),
        QUANTILE_POINTS
    );
    assert_eq!(
        back.languages[0].strata[0].metrics[0].quantiles,
        art.languages[0].strata[0].metrics[0].quantiles
    );
}

#[test]
fn load_reads_a_written_artifact() {
    let art = ramp_artifact();
    let path = write_temp_json("load-ok", &art);
    let loaded = calibration::load(Path::new(&path)).expect("load valid artifact");
    assert_eq!(loaded.corpus_vintage, "test-ramp");
    assert_eq!(loaded.languages[0].sample_functions, 4_000);
}

// ─── percentile: interpolation contract ──────────────────────────────────────

#[test]
fn percentile_at_an_exact_breakpoint_returns_that_quantile() {
    let art = ramp_artifact();
    // q[750] == 750.0 on the 0..=1000 ramp; percentile there is 750/1000 = 0.75.
    let cp = calibration::percentile(&art, "rust", "cyclomatic", 750.0).expect("in-corpus lookup");
    assert!((cp.p - 0.75).abs() < 1e-9, "expected p≈0.75, got {}", cp.p);
    assert!(!cp.beyond_corpus);
}

#[test]
fn percentile_between_breakpoints_interpolates_linearly() {
    let art = ramp_artifact();
    // 750.5 sits halfway between q[750]=750.0 and q[751]=751.0, so the
    // percentile is halfway between 0.750 and 0.751 → 0.7505.
    let cp = calibration::percentile(&art, "rust", "cyclomatic", 750.5).expect("in-corpus lookup");
    assert!(
        (cp.p - 0.7505).abs() < 1e-9,
        "expected p≈0.7505, got {}",
        cp.p
    );
    assert!(!cp.beyond_corpus);
}

#[test]
fn percentile_below_the_minimum_is_zero() {
    let art = ramp_artifact();
    // q[0] == 0.0; anything strictly below floors to p=0.0, not beyond-corpus.
    let cp = calibration::percentile(&art, "rust", "cyclomatic", -5.0).expect("in-corpus lookup");
    assert!(cp.p.abs() < 1e-9, "expected p≈0.0, got {}", cp.p);
    assert!(!cp.beyond_corpus);
}

#[test]
fn percentile_at_the_minimum_breakpoint_is_zero() {
    let art = ramp_artifact();
    let cp = calibration::percentile(&art, "rust", "cyclomatic", 0.0).expect("in-corpus lookup");
    assert!(cp.p.abs() < 1e-9, "expected p≈0.0, got {}", cp.p);
    assert!(!cp.beyond_corpus);
}

#[test]
fn percentile_beyond_the_maximum_is_one_and_flags_beyond_corpus() {
    let art = ramp_artifact();
    // q[last] == 1000.0; strictly above → saturates to 1.0 with the flag set.
    let cp =
        calibration::percentile(&art, "rust", "cyclomatic", 5_000.0).expect("in-corpus lookup");
    assert!((cp.p - 1.0).abs() < 1e-9, "expected p≈1.0, got {}", cp.p);
    assert!(cp.beyond_corpus);
}

#[test]
fn percentile_at_the_maximum_breakpoint_is_one_without_beyond_flag() {
    let art = ramp_artifact();
    let cp = calibration::percentile(&art, "rust", "cyclomatic", 1000.0).expect("in-corpus lookup");
    assert!((cp.p - 1.0).abs() < 1e-9, "expected p≈1.0, got {}", cp.p);
    assert!(!cp.beyond_corpus);
}

#[test]
fn percentile_for_an_unknown_language_is_none() {
    let art = ramp_artifact();
    assert!(calibration::percentile(&art, "haskell", "cyclomatic", 100.0).is_none());
}

#[test]
fn percentile_for_an_unknown_metric_is_none() {
    let art = ramp_artifact();
    assert!(calibration::percentile(&art, "rust", "halstead", 100.0).is_none());
}

#[test]
fn percentile_for_a_language_below_the_sample_floor_is_none() {
    let mut art = ramp_artifact();
    art.languages[0].sample_functions = MIN_LANG_SAMPLE - 1;
    assert!(
        calibration::percentile(&art, "rust", "cyclomatic", 100.0).is_none(),
        "under-sampled language must be treated as absent"
    );
}

#[test]
fn percentile_at_exactly_the_sample_floor_is_present() {
    let mut art = ramp_artifact();
    art.languages[0].sample_functions = MIN_LANG_SAMPLE;
    assert!(
        calibration::percentile(&art, "rust", "cyclomatic", 100.0).is_some(),
        "a language at exactly the floor is in-corpus"
    );
}

// ─── load: validation ────────────────────────────────────────────────────────

#[test]
fn load_rejects_an_unknown_format_version() {
    let mut art = ramp_artifact();
    art.format_version = CALIBRATION_FORMAT_VERSION + 1;
    let path = write_temp_json("bad-version", &art);
    let err = calibration::load(Path::new(&path)).expect_err("unknown version must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("format") || msg.contains("version"),
        "error should mention the format version: {msg}"
    );
}

#[test]
fn load_rejects_a_non_monotonic_quantile_vector() {
    let mut art = ramp_artifact();
    // Break monotonicity: make one breakpoint dip below its predecessor.
    art.languages[0].strata[0].metrics[0].quantiles[500] = -1.0;
    let path = write_temp_json("non-monotonic", &art);
    let err = calibration::load(Path::new(&path)).expect_err("non-monotonic must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("monoton") || msg.contains("decreasing") || msg.contains("quantile"),
        "error should mention monotonicity: {msg}"
    );
}

#[test]
fn load_rejects_a_quantile_vector_of_the_wrong_length() {
    let mut art = ramp_artifact();
    art.languages[0].strata[0].metrics[0].quantiles.pop();
    let path = write_temp_json("short-vector", &art);
    let err = calibration::load(Path::new(&path)).expect_err("wrong length must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("length") || msg.contains("1001") || msg.contains("quantile"),
        "error should mention the quantile-vector length: {msg}"
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
    assert!(calibration::load(Path::new(&path)).is_err());
}

// ─── merge: weighted quantile blending ───────────────────────────────────────

#[test]
fn merging_an_artifact_with_itself_preserves_quantiles() {
    let art = ramp_artifact();
    let merged = calibration::merge(art.clone(), art.clone());

    let before = &art.languages[0].strata[0].metrics[0].quantiles;
    let after = &merged.languages[0].strata[0].metrics[0].quantiles;
    assert_eq!(after.len(), before.len());
    for (i, (a, b)) in after.iter().zip(before.iter()).enumerate() {
        assert!(
            (a - b).abs() < 1e-9,
            "quantile {i} drifted under self-merge: {a} vs {b}"
        );
    }
}

#[test]
fn merging_an_artifact_with_itself_doubles_the_sample_counts() {
    let art = ramp_artifact();
    let base_count = art.languages[0].sample_functions;
    let merged = calibration::merge(art.clone(), art.clone());
    assert_eq!(
        merged.languages[0].sample_functions,
        base_count * 2,
        "self-merge must pool the sample counts"
    );
}

#[test]
fn merged_quantiles_stay_monotonic_and_reload() {
    // A blended artifact must survive its own load validation.
    let art = ramp_artifact();
    let merged = calibration::merge(art.clone(), art);
    let path = write_temp_json("merged", &merged);
    calibration::load(Path::new(&path)).expect("merged artifact must reload");
}

// ─── build_from_observations: determinism + injectable timestamp ─────────────

#[test]
fn build_from_observations_is_deterministic_for_a_fixed_timestamp() {
    let obs = sample_observations();
    let a = calibration::build_from_observations("test-build", "2026-07-12T00:00:00Z", &obs);
    let b = calibration::build_from_observations("test-build", "2026-07-12T00:00:00Z", &obs);
    let ja = serde_json::to_vec(&a).expect("serialize a");
    let jb = serde_json::to_vec(&b).expect("serialize b");
    assert_eq!(
        ja, jb,
        "same observations + timestamp must serialize identically"
    );
}

#[test]
fn build_from_observations_stamps_the_injected_timestamp_and_vintage() {
    let obs = sample_observations();
    let art = calibration::build_from_observations("world-2099-01", "2099-01-02T03:04:05Z", &obs);
    assert_eq!(art.corpus_vintage, "world-2099-01");
    assert_eq!(art.generated_at, "2099-01-02T03:04:05Z");
    assert_eq!(art.format_version, CALIBRATION_FORMAT_VERSION);
}

#[test]
fn build_from_observations_yields_a_loadable_artifact() {
    let obs = sample_observations();
    let art = calibration::build_from_observations("test-build", "2026-07-12T00:00:00Z", &obs);
    let path = write_temp_json("built", &art);
    let loaded = calibration::load(Path::new(&path)).expect("built artifact must load");
    // The pooled sample count reflects the number of observations fed in.
    assert!(loaded.languages.iter().any(|l| l.language == "rust"));
}

/// A `LangObservations` carrying enough raw `cyclomatic` samples for one
/// language to clear the sample floor once pooled.
fn sample_observations() -> calibration::LangObservations {
    let mut obs = calibration::LangObservations::default();
    let floor = u32::try_from(MIN_LANG_SAMPLE).expect("MIN_LANG_SAMPLE fits u32");
    for v in 0..(floor + 10) {
        obs.observe("rust", "cyclomatic", f64::from(v));
    }
    obs
}
