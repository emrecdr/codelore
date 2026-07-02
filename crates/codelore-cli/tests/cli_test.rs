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
    // Plan 8 §2 Task 10 widened SARIF support to {hotspots, clones}.
    // Plan 8 §6 Task 21 added clone-coupling.
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
// Plan 8 §3 Task 14 — --no-cache + --cache-dir
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
