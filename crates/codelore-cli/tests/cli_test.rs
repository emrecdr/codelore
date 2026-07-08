use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn analyze_revisions_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,n-revs"))
        .stdout(predicate::str::contains("src/main.rs,4"))
        .stdout(predicate::str::contains("src/lib.rs,1"));
}

#[test]
fn analyze_rejects_unknown_analysis() {
    Command::cargo_bin("codelore")
        .unwrap()
        .args(["analyze", "--analysis", "not-real", "--repo", "."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown analysis"));
}

#[test]
fn version_flag_works() {
    // Compare against the package version Cargo resolves at compile time, not
    // a hardcoded literal — otherwise every version bump fails CI silently
    // until someone re-reads this test file.
    Command::cargo_bin("codelore")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn diff_rejects_base_equals_head() {
    // A `--range` whose base resolves to the same SHA as head
    // used to run two identical analyses and emit an empty diff with
    // no signal. Now the entry point bails early with a typed error.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "HEAD..HEAD",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success(), "HEAD..HEAD should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("base and head resolve to the same commit")
            && stderr.contains("nothing to diff"),
        "expected base==head error, got stderr: {stderr}"
    );
}

#[test]
fn invalid_repo_exits_with_code_3() {
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            "/tmp/definitely-does-not-exist-codelore-test",
        ])
        .output()
        .unwrap();
    // CodeLoreError::Repo → exit 3 per spec §6.6
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn invalid_options_exit_with_code_2() {
    // Inverted coupling range (`--min-coupling` > `--max-coupling`) is a
    // cross-field config error → `CodeLoreError::InvalidOptions` → exit 2.
    // Exit 2 (config errors) was the one bucket with no end-to-end CLI
    // coverage; a refactor dropping the typed error to a bare
    // `anyhow::bail!` would silently regress it to exit 1.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--min-coupling",
            "80",
            "--max-coupling",
            "30",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn analyze_hotspots_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "entity,revisions,cognitive,code-health,hotspot-score",
        ));
}

#[test]
fn analyze_code_health_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "code-health",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,cognitive,score"));
}

#[test]
fn analyze_code_age_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "code-age",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "entity,age_months,age_days,last_modified",
        ));
}

#[test]
fn analyze_abs_churn_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "abs-churn",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("date,added,deleted,commits"));
}

#[test]
fn analyze_author_churn_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "author-churn",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("author,added,deleted,commits"));
}

#[test]
fn analyze_entity_churn_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "entity-churn",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,added,deleted,commits"));
}

#[test]
fn analyze_communication_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "communication",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "author-a,author-b,shared,average,strength",
        ));
}

#[test]
fn analyze_ownership_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "ownership",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "entity,main-author,total-revs,fractal-value",
        ));
}

#[test]
fn analyze_coupling_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "coupling",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "entity-a,entity-b,shared,revs-a,revs-b,average-revs,degree,fisher-p",
        ));
}

#[test]
fn analyze_summary_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "summary",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("metric,value"));
}

#[test]
fn analyze_hotspots_emits_sarif() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "sarif",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("CODELORE-HOTSPOT"))
        .stdout(predicate::str::contains(
            "json.schemastore.org/sarif-2.1.0.json",
        ));
}

#[test]
fn analyze_revisions_emits_json() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("[").or(predicate::str::contains("\"entity\"")));
}

#[test]
fn analyze_hotspots_emits_markdown() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "markdown",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# CodeLore hotspots"));
}

#[test]
fn analyze_hotspots_emits_parquet() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hotspots.parquet");
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "parquet",
            "--min-revs",
            "1",
            "--output",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(path.exists(), "parquet file should be written");
    assert!(
        path.metadata().unwrap().len() > 0,
        "parquet file should be non-empty"
    );
}

#[test]
fn analyze_emits_sqlite_dump() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dump.db");
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "sqlite",
            "--min-revs",
            "1",
            "--output",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(path.exists(), "sqlite file should be written");
}

#[test]
fn analyze_emits_provenance_sidecar_for_csv_output() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hotspots.csv");
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
            "--output",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let sidecar = dir.path().join("hotspots.csv.provenance.json");
    assert!(sidecar.exists(), "provenance sidecar should be written");
    let body = std::fs::read_to_string(&sidecar).unwrap();
    assert!(
        body.contains("\"codelore_version\""),
        "manifest should include codelore_version"
    );
    assert!(
        body.contains("\"analysis\""),
        "manifest should include analysis"
    );
    assert!(
        body.contains("hotspots"),
        "manifest should record the analysis name"
    );
}

#[test]
fn analyze_emits_provenance_sidecar_for_parquet_output() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hotspots.parquet");
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "parquet",
            "--min-revs",
            "1",
            "--output",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let sidecar = dir.path().join("hotspots.parquet.provenance.json");
    assert!(
        sidecar.exists(),
        "provenance sidecar should be written next to parquet"
    );
}

#[test]
fn analyze_skips_sidecar_for_sqlite_output() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dump.db");
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "sqlite",
            "--min-revs",
            "1",
            "--output",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Provenance is inside the .db (via ATTACH); no sidecar required.
    let sidecar = dir.path().join("dump.db.provenance.json");
    assert!(
        !sidecar.exists(),
        "no sidecar for sqlite — provenance lives in the DB"
    );
}

#[test]
fn analyze_skips_sidecar_for_stdout() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let dir = tempfile::tempdir().unwrap();
    // Run from inside the tempdir so any accidental relative-path sidecar shows up.
    let assert = Command::cargo_bin("codelore")
        .unwrap()
        .current_dir(dir.path())
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
            // no --output
        ])
        .assert()
        .success();
    drop(assert);

    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    let names: Vec<String> = entries
        .into_iter()
        .filter_map(|e| e.ok().and_then(|de| de.file_name().into_string().ok()))
        .collect();
    let has_sidecar = names.iter().any(|n| n.ends_with(".provenance.json"));
    assert!(
        !has_sidecar,
        "stdout output should not create a sidecar: found {names:?}"
    );
}

#[test]
fn parquet_requires_output_flag() {
    // A binary format with no --output is an output-side usage error →
    // CodeLoreError::Output → spec §6.6 exit 5 (not the generic 1).
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "parquet",
            "--min-revs",
            "1",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("requires --output"));
}

#[test]
fn sarif_rejects_unsupported_analysis() {
    // SARIF support covers {hotspots, clones}.
    // clone-coupling is also covered.
    // `revisions` is still unsupported and must bail with a helpful
    // message naming the supported analyses.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "sarif",
            "--min-revs",
            "1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "hotspots, clones, and clone-coupling",
        ))
        .stderr(predicate::str::contains("clones"));
}

#[test]
fn unknown_analysis_lists_supported_names() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "definitelybogus",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown analysis"))
        .stderr(predicate::str::contains("hotspots"))
        .stderr(predicate::str::contains("clones"));
}

// ---------------------------------------------------------------------------
// --no-cache + --cache-dir
// ---------------------------------------------------------------------------

/// `--no-cache` must succeed and produce the same CSV output as the default path.
#[test]
fn no_cache_flag_produces_valid_output() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
            "--no-cache",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,n-revs"))
        .stdout(predicate::str::contains("src/main.rs"));
}

/// `--cache-dir` must succeed and write the cache file under the given dir.
#[test]
fn cache_dir_flag_writes_cache_to_custom_location() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let cache_dir = tempfile::tempdir().expect("tempdir");

    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,n-revs"));

    // At least one .duckdb file must have been created under the custom cache dir.
    let count = walkdir::WalkDir::new(cache_dir.path())
        .into_iter()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x == "duckdb")
        })
        .count();

    assert!(
        count >= 1,
        "expected at least 1 .duckdb file under cache_dir, got {count}"
    );
}

/// `--time-bucket` on an incompatible analysis
/// must be rejected at the CLI boundary with a descriptive error,
/// pointing the user at the supported analyses (coupling, soc,
/// hotspots, code-health). Previously this either crashed with
/// `Catalog Error: changes_bucketed does not exist` or silently
/// returned empty rows.
#[test]
fn time_bucket_rejected_for_incompatible_analysis() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--no-banner",
            "--no-cache",
            "--time-bucket",
            "week",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--time-bucket is not supported"))
        .stderr(predicate::str::contains(
            "coupling, soc, hotspots, code-health",
        ));
}

/// Control case: `--time-bucket` on a compatible analysis
/// (coupling) must succeed.
#[test]
fn time_bucket_accepted_for_coupling() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "coupling",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--no-banner",
            "--no-cache",
            "--time-bucket",
            "day",
            "--min-revs",
            "1",
            "--min-shared-revs",
            "1",
        ])
        .assert()
        .success();
}

#[test]
fn unsupported_format_bails_cleanly_instead_of_panicking() {
    // `--format ndjson`/`gha` pass top-level format validation but are only
    // wired for a few analyses. For the rest, the dispatch must bail with a
    // clean, descriptive error — NOT panic through a reachable
    // `unreachable!` (exit 101). The per-analysis format mismatch carries
    // CodeLoreError::Analysis → spec §6.6 exit 4 (was the generic 1 before
    // the dispatch bails grew typed error buckets). Cover ndjson and gha.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    for fmt in ["ndjson", "gha"] {
        Command::cargo_bin("codelore")
            .unwrap()
            .args([
                "analyze",
                "--analysis",
                "abs-churn",
                "--repo",
                tiny.dir.path().to_str().unwrap(),
                "--format",
                fmt,
                "--no-banner",
                "--min-revs",
                "1",
            ])
            .assert()
            .code(4)
            .stderr(predicate::str::contains("abs-churn"))
            .stderr(predicate::str::contains("panicked").not())
            .stderr(predicate::str::contains("unreachable").not());
    }
}

#[test]
fn unknown_format_exits_with_analysis_code() {
    // An unrecognised `--format` value is an analysis-selection error →
    // CodeLoreError::Analysis → spec §6.6 exit 4, never a panic or a bare 1.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "bogusfmt",
            "--no-banner",
            "--min-revs",
            "1",
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("unknown --format"))
        .stderr(predicate::str::contains("panicked").not());
}

#[test]
fn schema_lists_every_registered_analysis() {
    // The schema row-type catalogue must cover every analysis the
    // registry knows about. `delivery-friction`, `main-dev-by-revs`,
    // and `main-dev-by-deletions` are registered analyses that were
    // missing from the hardcoded catalogue.
    for name in codelore_lib::analysis::AnalysisName::all() {
        Command::cargo_bin("codelore")
            .unwrap()
            .args(["schema", name.as_str()])
            .assert()
            .success()
            .stderr(predicate::str::contains("unknown row type").not());
    }
}

/// Analyses that intentionally have NO `codelore explain` topic yet —
/// either the formula is too involved to state accurately in one line, or
/// the analysis is low-value for the explain surface. Anti-drift contract:
/// to add a new analysis you must EITHER add an explain entry in
/// `run_explain_cmd` OR add the name here (and document why). A name listed
/// here that later gains an explain topic flips the assertion below, forcing
/// the stale allowlist entry to be removed.
const EXPLAIN_UNCOVERED: &[&str] = &[
    "coupling",
    "author-churn",
    "entity-churn",
    "communication",
    "summary",
    "clones",
    "clone-coupling",
    "messages",
    "main-dev",
    "main-dev-by-revs",
    "main-dev-by-deletions",
    "entity-effort",
    "entity-ownership",
    "top-committers",
    "delivery-friction",
];

#[test]
fn explain_covers_every_registered_analysis_or_allowlists_it() {
    // Every registered analysis must either resolve to an `explain` topic
    // (exit 0) or be on the explicit uncovered allowlist (exit non-zero
    // with an "unknown topic" message). This stops a newly-added analysis
    // from silently shipping with no explain coverage and no decision
    // recorded about it.
    for name in codelore_lib::analysis::AnalysisName::all() {
        let allowlisted = EXPLAIN_UNCOVERED.contains(&name.as_str());
        let assert = Command::cargo_bin("codelore")
            .unwrap()
            .args(["explain", name.as_str()])
            .assert();
        if allowlisted {
            assert
                .failure()
                .stderr(predicate::str::contains("unknown topic"));
        } else {
            assert.success();
        }
    }
}

#[test]
fn health_trend_csv_has_header_and_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "health-trend",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "date,rev,files,arch-health,code-health,combined-health,arch-band,code-band,combined-band",
        ))
        // Header alone would pass on empty output — require at least one data row.
        .stdout(predicate::function(|out: &str| {
            out.lines().filter(|l| !l.trim().is_empty()).count() >= 2
        }));
}

#[test]
fn effort_exposure_csv_has_header_and_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "effort-exposure",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "band,files,loc-share-pct,commit-share-pct,churn-share-pct,commit-share-ci-low,commit-share-ci-high",
        ))
        // Header alone would pass on empty output — require at least one data row.
        .stdout(predicate::function(|out: &str| {
            out.lines().filter(|l| !l.trim().is_empty()).count() >= 2
        }));
}

#[test]
fn code_familiarity_csv_has_header() {
    // tiny_repo has no recognized source files → complexity_metrics is empty
    // → no familiarity rows. This test only verifies the CSV header is present.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "code-familiarity",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "scope,familiarity-pct,active-authors,total-authors,islands-pct,verdict",
        ));
}

#[test]
fn code_familiarity_csv_has_header_and_rows() {
    // delivery_repo has src/*.rs files (Rust, Tier-1) → complexity_metrics
    // populated → knowledge_shares materialised → one familiarity row emitted.
    let delivery = codelore_lib::test_support::delivery_repo::build();
    let out = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "code-familiarity",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines.len() >= 2,
        "expected header + at least one data row, got:\n{text}"
    );
    assert!(
        lines[0].contains("scope") && lines[0].contains("familiarity-pct"),
        "first line must be the CSV header: {}",
        lines[0]
    );
    // Data row: scope=repo, verdict is good or risky, familiarity in [0,100].
    assert!(
        lines[1].starts_with("repo,"),
        "data row must start with 'repo,': {}",
        lines[1]
    );
}

#[test]
fn bus_factor_csv_contains_model_column() {
    // Verify the `model` column is present in both commits and doe mode output.
    let delivery = codelore_lib::test_support::delivery_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "bus-factor",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "module,total_commits,bus_factor,top_contributor,top_contributor_share,model",
        ));
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "bus-factor",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
            "--knowledge-model",
            "doe",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "module,total_commits,bus_factor,top_contributor,top_contributor_share,model",
        ))
        .stdout(predicate::str::contains(",doe"));
}

#[test]
fn team_composition_csv_has_header_and_rows() {
    // Verify CSV header columns and that delivery_repo produces author data.
    let delivery = codelore_lib::test_support::delivery_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "team-composition",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "author,tenure-days,bucket,veteran-breadth-ok,active,commits,files-touched,onboarding-weeks",
        ))
        .stdout(predicate::str::contains("__summary__"));
}

#[cfg(feature = "spa")]
#[test]
fn spa_without_output_defaults_to_dot_codelore() {
    // `--format spa` no longer requires --output; it defaults to
    // `.codelore/spa.html` under the current working directory.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let cwd = tempfile::tempdir().unwrap();
    Command::cargo_bin("codelore")
        .unwrap()
        .current_dir(cwd.path())
        .args([
            "analyze",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "spa",
            "--no-banner",
            "--min-revs",
            "1",
        ])
        .assert()
        .success();
    assert!(
        cwd.path().join(".codelore").join("spa.html").is_file(),
        "spa without --output should create .codelore/spa.html in the cwd"
    );
}

// ---------------------------------------------------------------------------
// Delta health end-to-end tests
// ---------------------------------------------------------------------------

/// Build a two-commit repo: commit 1 has a trivial function, commit 2
/// adds a large, branchy function. Returns `(dir, base_sha, head_sha)`.
fn delta_health_fixture() -> (tempfile::TempDir, String, String) {
    use std::fmt::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn tiny() -> i32 {\n    1\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);

    // A >70-line, CC>10 function: 12 sequential if-blocks + filler lets
    // both the LOC and cyclomatic High thresholds trigger.
    let mut monster = String::from("pub fn monster(x: i32) -> i32 {\n    let mut acc = 0;\n");
    for i in 0..12 {
        let _ = write!(monster, "    if x > {i} {{\n        acc += {i};\n    }}\n");
    }
    for i in 0..40 {
        let _ = writeln!(monster, "    acc += {i};");
    }
    monster.push_str("    acc\n}\n");
    std::fs::write(
        repo.join("src/lib.rs"),
        format!("pub fn tiny() -> i32 {{\n    1\n}}\n\n{monster}"),
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "add monster"]);
    let head = git(&["rev-parse", "HEAD"]);
    (dir, base, head)
}

#[test]
fn diff_emits_degrading_delta_health_for_added_monster() {
    let (dir, base, head) = delta_health_fixture();
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let dh = &json["delta_health"];
    assert_eq!(dh["verdict"], "degrading", "delta_health: {dh}");
    assert_eq!(dh["counts"]["added"].as_u64(), Some(1));
    let f = &dh["functions"][0];
    assert_eq!(f["function"], "monster");
    assert_eq!(f["after"], "high");
    assert_eq!(f["outcome"], "bad");
}

#[test]
fn diff_delta_health_gate_fails_the_run() {
    let (dir, base, head) = delta_health_fixture();
    let thresholds = dir.path().join("gates.toml");
    std::fs::write(&thresholds, "[diff]\ndeny_degrading_verdict = true\n").unwrap();
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "deny_degrading_verdict should fail the run"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["gate_violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["gate"] == "deny_degrading_verdict"),
        "violations: {}",
        json["gate_violations"]
    );
}

#[test]
fn diff_docs_only_change_is_no_code_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn tiny() -> i32 {\n    1\n}\n",
    )
    .unwrap();
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.join("README.md"), "hello world\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "docs"]);
    let head = git(&["rev-parse", "HEAD"]);

    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            repo.to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["delta_health"]["verdict"], "no-code-change");
    assert!(json["delta_health"]["ratio"].is_null());
}

const CLONE_ORIGINAL_SRC: &str = "\
pub fn original(x: i32) -> i32 {
    let mut acc = 0;
    if x > 0 {
        acc += 1;
    } else {
        acc -= 1;
    }
    if x > 10 {
        acc += 2;
    } else {
        acc -= 2;
    }
    if x > 20 {
        acc += 3;
    } else {
        acc -= 3;
    }
    if x > 30 {
        acc += 4;
    } else {
        acc -= 4;
    }
    if x > 40 {
        acc += 5;
    } else {
        acc -= 5;
    }
    acc
}
";
const CLONE_PASTED_COPY_SRC: &str = "\
pub fn pasted_copy(y: i64) -> i64 {
    let mut total = 0;
    if y > 0 {
        total += 1;
    } else {
        total -= 1;
    }
    if y > 10 {
        total += 2;
    } else {
        total -= 2;
    }
    if y > 20 {
        total += 3;
    } else {
        total -= 3;
    }
    if y > 30 {
        total += 4;
    } else {
        total -= 4;
    }
    if y > 40 {
        total += 5;
    } else {
        total -= 5;
    }
    total
}
";

/// Validates the clone→high-risk penalty through the real ingest pipeline.
///
/// Base commit: `src/lib.rs` with one named function `original`.
/// Head commit: add `src/copy.rs` with `pasted_copy` — a structural
/// Type-2 clone (same AST shape, different identifiers/types). The body
/// has five if/else blocks, giving it well over 30 structural nodes so it
/// clears the default `min_clone_node_count` filter.
///
/// The assertion proves that the clone extractor's function name (`pasted_copy`,
/// from the first identifier child of `function_item`) matches the complexity
/// name (stripped of the `@start-end` span by `run_function_metrics`) — the
/// alignment invariant that makes the clone penalty reachable in practice.
#[test]
fn diff_delta_health_flags_pasted_clone_as_high_risk() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // Base commit: one small function in src/lib.rs.  The original is also
    // the template whose structure will be duplicated in the head commit.
    git(&["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    // Five if/else blocks — structurally rich enough to produce > 30
    // fingerprint nodes (the default min_clone_node_count).
    std::fs::write(repo.join("src/lib.rs"), CLONE_ORIGINAL_SRC).unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);

    // Head commit: add src/copy.rs with pasted_copy — same structure,
    // different name and types (Type-2 clone).
    std::fs::write(repo.join("src/copy.rs"), CLONE_PASTED_COPY_SRC).unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "paste copy"]);
    let head = git(&["rev-parse", "HEAD"]);

    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            repo.to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let dh = &json["delta_health"];

    // pasted_copy is added in src/copy.rs — find it in the functions list.
    let fns = dh["functions"].as_array().expect("functions array");
    let pasted = fns
        .iter()
        .find(|f| f["function"] == "pasted_copy")
        .unwrap_or_else(|| panic!("pasted_copy not found in delta_health.functions; got: {fns:?}"));

    // Clone membership forces High regardless of LOC/cyclomatic.
    assert_eq!(
        pasted["after"], "high",
        "pasted_copy must be classified high-risk (clone penalty); row: {pasted}"
    );

    // reasons must mention the clone group.
    let reasons = pasted["reasons"].as_array().expect("reasons array");
    assert!(
        reasons
            .iter()
            .any(|r| r.as_str().unwrap_or("").contains("clone")),
        "reasons must mention clone membership; got: {reasons:?}"
    );
}

#[test]
fn coordination_needs_csv_has_header_and_rows() {
    // delivery_repo has src/*.rs Rust files → complexity ingest fires →
    // knowledge_shares materialised → coordination-needs rows produced.
    let delivery = codelore_lib::test_support::delivery_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "coordination-needs",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "path,authors,fragmentation,interleave,cochange-entropy,tier,health-band",
        ))
        // Header alone would pass on empty output — require at least one data row.
        .stdout(predicate::function(|out: &str| {
            out.lines().filter(|l| !l.trim().is_empty()).count() >= 2
        }));
}

#[test]
fn release_cadence_csv_has_header_and_rows() {
    // delivery_repo has v0.1.0, v0.2.0, v1.0.0 tags → 3 rows + summary.
    let delivery = codelore_lib::test_support::delivery_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "release-cadence",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--release-tag-glob",
            "v*",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("tag,date,days-since-prev,trend"))
        // Header + 3 tag rows + 1 summary row = ≥4 non-empty lines.
        .stdout(predicate::function(|out: &str| {
            out.lines().filter(|l| !l.trim().is_empty()).count() >= 4
        }));
}

#[test]
fn delivery_metrics_markdown_exits_zero() {
    // delivery_repo has two --no-ff merges and two author→committer gaps;
    // run with include_merges so the commit_parents table is populated.
    let delivery = codelore_lib::test_support::delivery_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "delivery-metrics",
            "--repo",
            delivery.dir.path().to_str().unwrap(),
            "--format",
            "markdown",
            "--include-merges",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("delivery-metrics"))
        .stdout(predicate::str::contains("branch_duration_hours"));
}

#[test]
fn check_quiet_suppresses_vacuous_pass_noise() {
    // Without a thresholds file the check vacuously passes and prints a
    // diagnostic to stderr. With --quiet that diagnostic is suppressed;
    // exit 0 is preserved.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "check",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

#[test]
fn check_without_quiet_prints_vacuous_pass_diagnostic() {
    // Without --quiet the vacuous-pass diagnostic appears on stderr so users
    // know the check did nothing.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args(["check", "--repo", tiny.dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("vacuously passing"));
}

#[test]
fn function_xray_emits_markdown_header() {
    let repo = codelore_lib::test_support::function_xray_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "function-xray",
            "--repo",
            repo.dir.path().to_str().unwrap(),
            "--target",
            "src/target.rs",
            "--format",
            "markdown",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# CodeLore function-xray"));
}

#[test]
fn function_coupling_emits_markdown_header() {
    let repo = codelore_lib::test_support::function_xray_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "function-coupling",
            "--repo",
            repo.dir.path().to_str().unwrap(),
            "--target",
            "src/target.rs",
            "--format",
            "markdown",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# CodeLore function-coupling"));
}

#[test]
fn check_quiet_violation_path_suppresses_detail_keeps_verdict() {
    // When gates are configured and violations occur, --quiet suppresses the
    // per-violation detail lines on stderr but preserves the FAIL verdict line
    // and exits 1.
    //
    // code_health_min = 100.0 is set impossibly high so every file in the repo
    // is a violation. code-health runs regardless of --min-revs so tiny_repo
    // (whose files don't reach the default min_revs = 5 threshold used by the
    // hotspot gate) still produces evaluable rows.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let thresholds = tiny.dir.path().join(".codelore-thresholds.toml");
    std::fs::write(&thresholds, "[gates]\ncode_health_min = 100.0\n").unwrap();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "check",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--quiet",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("FAIL"))
        // Per-violation detail lines name the gate; --quiet must suppress them.
        .stderr(predicate::str::contains("code_health_min").not());
}
