use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn analyze_revisions_emits_csv() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    Command::cargo_bin("bca")
        .unwrap()
        .args(["analyze", "--analysis", "not-real", "--repo", "."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown analysis"));
}

#[test]
fn version_flag_works() {
    Command::cargo_bin("bca")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("0.1.0-alpha.1"));
}

#[test]
fn invalid_repo_exits_with_code_3() {
    let output = Command::cargo_bin("bca")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            "/tmp/definitely-does-not-exist-bca-test",
        ])
        .output()
        .unwrap();
    // BcaError::Repo → exit 3 per spec §6.6
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn analyze_hotspots_emits_csv() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
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
