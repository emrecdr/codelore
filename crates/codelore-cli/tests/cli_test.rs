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
    Command::cargo_bin("codelore")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("0.1.0-alpha.1"));
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
            "entity,name,revisions,cognitive,code-health,hotspot-score",
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
        .stdout(predicate::str::contains("entity,name,cognitive,score"));
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
        .stdout(predicate::str::contains("entity,age-months"));
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
            "entity,main-dev,total-revs,fractal-value",
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
        .stdout(predicate::str::contains("schemastore.azurewebsites.net"));
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
        .failure()
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
        .stderr(predicate::str::contains("hotspots, clones, and clone-coupling"))
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
