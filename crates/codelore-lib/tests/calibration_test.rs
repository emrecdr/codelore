//! Unit tests for the corpus-calibration artifact: serde round-trip, the
//! quantile-breakpoint interpolation contract of `percentile`, `load`
//! validation (format version + quantile monotonicity), the
//! sample-count-weighted `merge` approximation, the golden fixture
//! artifact byte-determinism guard, and the `repo_metrics` optional section
//! with `raw_percentile` / `attach_repo_metrics`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use codelore_lib::calibration::{
    self, CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MIN_LANG_SAMPLE,
    MetricQuantiles, QUANTILE_POINTS, RepoMetrics, Stratum,
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
        repo_metrics: None,
    }
}

/// Wrap a hand-built `nargs` quantile vector (an integer-style metric, prone to
/// plateaus) into a one-language, above-floor artifact for plateau tests.
fn nargs_artifact(quantiles: Vec<f64>) -> CalibrationArtifact {
    CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-plateau".to_string(),
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
                    metric: "nargs".to_string(),
                    quantiles,
                }],
            }],
        }],
        repo_metrics: None,
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

// Plateau-vector indices are bounded by `QUANTILE_POINTS` (~1e3), far below
// `2^53`, so the `usize` → `f64` casts building the fixture are exact.
#[test]
#[allow(clippy::cast_precision_loss)]
fn percentile_on_an_interior_plateau_returns_the_upper_edge() {
    // Integer-style metric: q[200..=700] all equal 3.0 (a plateau), rising on
    // either side. Looking up 3.0 must resolve to the run's UPPER edge, index
    // 700 → p = 0.700 — the P(X ≤ value) CDF reading, not the lower edge.
    let mut q = vec![0.0; QUANTILE_POINTS];
    for (i, slot) in q.iter_mut().enumerate() {
        *slot = if i <= 200 {
            3.0 * (i as f64) / 200.0
        } else if i <= 700 {
            3.0
        } else {
            3.0 + 7.0 * ((i - 700) as f64) / ((QUANTILE_POINTS - 1 - 700) as f64)
        };
    }
    let art = nargs_artifact(q);
    let cp = calibration::percentile(&art, "rust", "nargs", 3.0).expect("in-corpus lookup");
    assert!(
        (cp.p - 0.700).abs() < 1e-9,
        "interior plateau must resolve to its upper edge (p≈0.700), got {}",
        cp.p
    );
    assert!(!cp.beyond_corpus);
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn percentile_on_a_minimum_plateau_is_zero() {
    // q[0..=300] all equal 5.0 (a plateau touching the minimum), rising after.
    // A lookup of 5.0 hits the `value <= q[0]` short-circuit → p = 0.0.
    let mut q = vec![5.0; QUANTILE_POINTS];
    for (i, slot) in q.iter_mut().enumerate() {
        if i > 300 {
            *slot = 5.0 + 5.0 * ((i - 300) as f64) / ((QUANTILE_POINTS - 1 - 300) as f64);
        }
    }
    let art = nargs_artifact(q);
    let cp = calibration::percentile(&art, "rust", "nargs", 5.0).expect("in-corpus lookup");
    assert!(
        cp.p.abs() < 1e-9,
        "a plateau at the minimum floors to p≈0.0, got {}",
        cp.p
    );
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

// ─── golden fixture artifact + byte-determinism guard ────────────────────────

/// Fixed RFC 3339 timestamp injected into the golden artifact so the build is
/// byte-reproducible regardless of wall clock. The value is arbitrary.
const GOLDEN_GENERATED_AT: &str = "2026-07-12T00:00:00Z";

/// Fixed vintage for the golden test artifact. The `golden-` prefix is
/// distinct from `placeholder-` (suppressed) and `world-` (real corpus) so
/// `embedded_world()` correctly returns `None` for this artifact, which is
/// committed under `tests/` not `src/`.
const GOLDEN_VINTAGE: &str = "golden-fixtures";

/// The committed golden artifact path, relative to the crate root.
const GOLDEN_ARTIFACT_PATH: &str = "tests/fixtures/calibration/test.calib.json";

/// Pool per-function raw metrics from `tiny_repo`, `biomarker_repo`, and
/// `coupling_repo` into a single `LangObservations`.
///
/// Each fixture is materialized from its embedded bundle into a fresh tempdir,
/// ingested into an in-memory `FactsDb`, and its `complexity_metrics` rows are
/// pooled. The fixture repos are all Rust-only, so the returned observations
/// carry only the `rust` language.
///
/// # Panics
///
/// Panics on any fixture-build, ingest, or query error — all indicate a
/// broken test environment.
fn build_fixture_observations() -> calibration::LangObservations {
    use codelore_lib::Options;
    use codelore_lib::complexity::Tier1Language;
    use codelore_lib::facts::FactsDb;
    use codelore_lib::repo::GixRepo;

    // Hold all three TempDirs alive for the duration of the function so the
    // fixture paths remain valid. Each fixture is named for error messages only.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let biomarker = codelore_lib::test_support::biomarker_repo::build();
    let coupling = codelore_lib::test_support::coupling_repo::build();

    let fixture_paths: [(&str, &std::path::Path); 3] = [
        ("tiny", tiny.dir.path()),
        ("biomarker", biomarker.dir.path()),
        ("coupling", coupling.dir.path()),
    ];

    let mut obs = calibration::LangObservations::new();

    for (name, repo_path) in &fixture_paths {
        let repo = GixRepo::open(repo_path).unwrap_or_else(|e| panic!("open {name} repo: {e}"));
        let opts = Options {
            repo_path: repo_path.to_path_buf(),
            min_revs: 1,
            ..Options::default()
        };
        let db = FactsDb::new_in_memory().expect("new_in_memory");
        db.ingest(&repo, &opts)
            .unwrap_or_else(|e| panic!("ingest {name}: {e}"));
        let mut stmt = db
            .prepare(
                "SELECT path, cyclomatic, cognitive, sloc, nargs, max_nesting \
                 FROM complexity_metrics",
            )
            .unwrap_or_else(|e| panic!("prepare complexity query for {name}: {e}"));
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })
            .unwrap_or_else(|e| panic!("query complexity for {name}: {e}"));

        for row in rows {
            let (path, cyclomatic, cognitive, sloc, nargs, max_nesting) =
                row.unwrap_or_else(|e| panic!("read row from {name}: {e}"));
            let Some(lang) = Tier1Language::from_path(&path) else {
                continue;
            };
            let lang = lang.as_str();
            for (metric, value) in [
                ("cyclomatic", cyclomatic),
                ("cognitive", cognitive),
                ("sloc", sloc),
                ("nargs", nargs),
                ("max_nesting", max_nesting),
            ] {
                if let Some(v) = value {
                    // Lossless i64 → f64: per-function metric values are bounded
                    // small integers far below 2^53.
                    #[allow(clippy::cast_precision_loss)]
                    obs.observe(lang, metric, v as f64);
                }
            }
        }
    }

    obs
}

/// Build the golden calibration artifact from the bundled fixture repos with a
/// constant timestamp and the `golden-fixtures` vintage.
///
/// `repos_attempted` and `repos_included` are set to 3 (tiny + biomarker +
/// coupling); they are metadata fields not used in quantile computation.
///
/// # Regenerating the committed artifact
///
/// If the fixture bundles or the metrics extraction change, regenerate via:
///
/// ```text
/// cargo test --features test-support \
///     -p codelore-lib --test calibration_test \
///     -- --nocapture generate_golden_fixture_artifact
/// ```
///
/// The test will print the updated JSON to stdout. Pipe or copy it to:
/// `crates/codelore-lib/tests/fixtures/calibration/test.calib.json` and commit.
fn build_golden_artifact() -> (Vec<u8>, codelore_lib::calibration::CalibrationArtifact) {
    let obs = build_fixture_observations();
    let mut art = calibration::build_from_observations(GOLDEN_VINTAGE, GOLDEN_GENERATED_AT, &obs);
    art.repos_attempted = 3;
    art.repos_included = 3;
    let bytes = serde_json::to_vec(&art).expect("serialize golden artifact");
    (bytes, art)
}

/// Byte-determinism guard: building the artifact twice from identical fixture
/// repos and an identical fixed timestamp must produce byte-identical JSON.
///
/// This guards against any source of non-determinism in
/// `build_from_observations` (floating-point sort stability, map iteration
/// order, etc.).
#[test]
fn golden_artifact_is_byte_deterministic() {
    let (a, _) = build_golden_artifact();
    let (b, _) = build_golden_artifact();
    assert_eq!(
        a, b,
        "golden artifact must be byte-identical across two builds with identical inputs"
    );
}

/// Drift guard: the freshly-built golden artifact must match the committed
/// `tests/fixtures/calibration/test.calib.json`.
///
/// If this test fails the fixtures or metrics layer changed and the committed
/// artifact is stale. Regenerate it (see the `build_golden_artifact` doc
/// comment for the exact command) and commit the updated file.
#[test]
fn golden_artifact_matches_committed_file() {
    let committed_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_ARTIFACT_PATH);
    let committed = std::fs::read(&committed_path).unwrap_or_else(|e| {
        panic!(
            "could not read committed artifact {}: {e}\n\
             Run `cargo test ... -- --nocapture generate_golden_fixture_artifact` to regenerate.",
            committed_path.display()
        )
    });
    let (fresh, _) = build_golden_artifact();
    assert_eq!(
        fresh, committed,
        "committed artifact is stale — fixtures or metrics changed; \
         regenerate via `cargo test --features test-support -p codelore-lib \
         --test calibration_test -- --nocapture generate_golden_fixture_artifact`"
    );
}

/// Helper test that prints the freshly-built golden artifact JSON to stdout.
///
/// Run explicitly with `--nocapture` to regenerate the committed artifact:
///
/// ```text
/// cargo test --features test-support -p codelore-lib --test calibration_test \
///     -- --nocapture generate_golden_fixture_artifact
/// ```
///
/// Pipe/copy the output to `tests/fixtures/calibration/test.calib.json`.
#[test]
fn generate_golden_fixture_artifact() {
    let (bytes, art) = build_golden_artifact();
    println!("{}", String::from_utf8(bytes).expect("valid UTF-8 JSON"));
    // Structural sanity: the artifact must have at least one language and be
    // loadable via the normal validation path.
    assert!(
        !art.languages.is_empty(),
        "golden artifact has no languages"
    );
    assert_eq!(art.corpus_vintage, GOLDEN_VINTAGE);
    assert_eq!(art.generated_at, GOLDEN_GENERATED_AT);
}

// ─── repo_metrics: serde additivity ──────────────────────────────────────────

/// An artifact JSON that lacks the `repo_metrics` field entirely (as all
/// artifacts built before this section existed do). Deserializing it must
/// yield `repo_metrics: None` — the `#[serde(default)]` path.
///
/// This also exercises the embedded-world artifact: it too lacks the field
/// and must deserialize cleanly after this serde change.
#[test]
fn old_artifact_without_repo_metrics_deserializes_with_none() {
    // Minimal valid v1 artifact JSON — no `repo_metrics` key at all.
    // Quantile vector must be exactly QUANTILE_POINTS (1001) long.
    let quantiles_json = {
        let vals: Vec<String> = (0..QUANTILE_POINTS).map(|i| i.to_string()).collect();
        format!("[{}]", vals.join(","))
    };
    let json = format!(
        r#"{{
            "format_version": 1,
            "corpus_vintage": "world-2026-01",
            "generated_at": "2026-01-01T00:00:00Z",
            "repos_included": 1,
            "repos_attempted": 1,
            "languages": [{{
                "language": "rust",
                "sample_functions": 1000,
                "strata": [{{
                    "sloc_min": 0,
                    "sloc_max": 18446744073709551615,
                    "metrics": [{{
                        "metric": "cyclomatic",
                        "quantiles": {quantiles_json}
                    }}]
                }}]
            }}]
        }}"#,
    );

    let art: CalibrationArtifact =
        serde_json::from_str(&json).expect("old artifact without repo_metrics must deserialize");
    assert!(
        art.repo_metrics.is_none(),
        "repo_metrics must be None when the JSON key is absent"
    );
    assert_eq!(art.corpus_vintage, "world-2026-01");
}

/// An artifact WITH `repo_metrics: Some(...)` serializes WITHOUT the key when
/// it is `None`, and WITH the key when it is `Some`. Both directions must
/// round-trip through serde.
#[test]
fn repo_metrics_none_omitted_from_serialization() {
    let mut art = ramp_artifact();
    // Start with None — key must be absent from the JSON.
    assert!(art.repo_metrics.is_none());
    let json = serde_json::to_string(&art).expect("serialize None repo_metrics");
    assert!(
        !json.contains("repo_metrics"),
        "repo_metrics:None must not appear in the JSON; got: {json}"
    );

    // Now set Some — key must appear.
    let mut values = BTreeMap::new();
    values.insert("propagation_cost".to_string(), vec![0.1, 0.2, 0.3]);
    art.repo_metrics = Some(RepoMetrics { values });
    let json_some = serde_json::to_string(&art).expect("serialize Some repo_metrics");
    assert!(
        json_some.contains("repo_metrics"),
        "repo_metrics:Some must appear in the JSON; got: {json_some}"
    );

    // Round-trip Some back.
    let back: CalibrationArtifact =
        serde_json::from_str(&json_some).expect("deserialize Some repo_metrics");
    let rm = back
        .repo_metrics
        .expect("repo_metrics must be Some after round-trip");
    assert_eq!(
        rm.values["propagation_cost"],
        vec![0.1, 0.2, 0.3],
        "pool values must survive the round-trip"
    );
}

// ─── raw_percentile: table-driven ────────────────────────────────────────────

/// Empty sorted slice → `None` (no observations, no rank possible).
#[test]
fn raw_percentile_empty_is_none() {
    assert!(
        calibration::raw_percentile(&[], 1.0).is_none(),
        "empty slice must return None"
    );
}

/// `[1.0, 2.0, 3.0]`, value `2.0`:
/// `count_less` = 1 (value 1.0 < 2.0), `count_equal` = 1 (the one 2.0), n = 3.
/// p = (1 + 0.5 * 1) / 3 = 1.5 / 3 = 0.5.
#[test]
fn raw_percentile_middle_value_of_three() {
    let sorted = [1.0_f64, 2.0, 3.0];
    let p = calibration::raw_percentile(&sorted, 2.0).expect("non-empty slice");
    // Hand-verified: (1 + 0.5*1) / 3 = 0.5
    assert!((p - 0.5).abs() < 1e-12, "expected 0.5, got {p}");
}

/// `[1.0, 2.0, 3.0]`, value `0.0` (below all elements):
/// `count_less` = 0, `count_equal` = 0 (0.0 is not in the slice), n = 3.
/// p = (0 + 0.5 * 0) / 3 = 0.0.
#[test]
fn raw_percentile_below_all_is_zero() {
    let sorted = [1.0_f64, 2.0, 3.0];
    let p = calibration::raw_percentile(&sorted, 0.0).expect("non-empty slice");
    // Hand-verified: (0 + 0.5*0) / 3 = 0.0
    assert!(
        p.abs() < 1e-12,
        "value below all elements → p = 0.0, got {p}"
    );
}

/// `[1.0, 2.0, 3.0]`, value `4.0` (above all elements):
/// `count_less` = 3 (all < 4.0), `count_equal` = 0, n = 3.
/// p = (3 + 0.5 * 0) / 3 = 1.0.
#[test]
fn raw_percentile_above_all_is_one() {
    let sorted = [1.0_f64, 2.0, 3.0];
    let p = calibration::raw_percentile(&sorted, 4.0).expect("non-empty slice");
    // Hand-verified: (3 + 0.5*0) / 3 = 1.0
    assert!(
        (p - 1.0).abs() < 1e-12,
        "value above all elements → p = 1.0, got {p}"
    );
}

/// `[1.0, 2.0, 2.0, 3.0]`, value `2.0` (two-element tie):
/// `count_less` = 1 (only 1.0 < 2.0), `count_equal` = 2 (the two 2.0s), n = 4.
/// p = (1 + 0.5 * 2) / 4 = 2.0 / 4 = 0.5.
#[test]
fn raw_percentile_tie_uses_midpoint_rank() {
    let sorted = [1.0_f64, 2.0, 2.0, 3.0];
    let p = calibration::raw_percentile(&sorted, 2.0).expect("non-empty slice");
    // Hand-verified: (1 + 0.5*2) / 4 = 2/4 = 0.5
    assert!(
        (p - 0.5).abs() < 1e-12,
        "tie: midpoint rank (1 + 0.5*2)/4 = 0.5, got {p}"
    );
}

/// Single-element slice `[5.0]`, value `5.0`:
/// `count_less` = 0, `count_equal` = 1, n = 1.
/// p = (0 + 0.5 * 1) / 1 = 0.5.
#[test]
fn raw_percentile_single_element_equal_is_half() {
    let sorted = [5.0_f64];
    let p = calibration::raw_percentile(&sorted, 5.0).expect("non-empty slice");
    // Hand-verified: (0 + 0.5*1) / 1 = 0.5
    assert!(
        (p - 0.5).abs() < 1e-12,
        "single-element equal: (0 + 0.5*1)/1 = 0.5, got {p}"
    );
}

// ─── attach_repo_metrics ──────────────────────────────────────────────────────

/// `attach_repo_metrics` with non-empty pools sets `repo_metrics: Some` and
/// sorts each vec ascending.
#[test]
fn attach_repo_metrics_sets_some_and_sorts() {
    let mut art = ramp_artifact();
    let mut values = BTreeMap::new();
    // Provide out-of-order values; expect them sorted ascending after attach.
    values.insert("propagation_cost".to_string(), vec![0.5, 0.1, 0.3]);
    values.insert("cycle_file_share".to_string(), vec![0.9, 0.2]);
    let pools = RepoMetrics { values };

    calibration::attach_repo_metrics(&mut art, pools);

    let rm = art
        .repo_metrics
        .expect("repo_metrics must be Some after attach");
    assert_eq!(
        rm.values["propagation_cost"],
        vec![0.1, 0.3, 0.5],
        "propagation_cost must be sorted ascending"
    );
    assert_eq!(
        rm.values["cycle_file_share"],
        vec![0.2, 0.9],
        "cycle_file_share must be sorted ascending"
    );
}

/// `attach_repo_metrics` with an empty `values` map sets `repo_metrics: None`
/// (empty pools = no lens).
#[test]
fn attach_repo_metrics_empty_pools_sets_none() {
    let mut art = ramp_artifact();
    let pools = RepoMetrics {
        values: BTreeMap::new(),
    };
    calibration::attach_repo_metrics(&mut art, pools);
    assert!(
        art.repo_metrics.is_none(),
        "empty pools must leave repo_metrics as None"
    );
}
