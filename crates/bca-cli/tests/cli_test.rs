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
