use assert_cmd::Command;
use predicates::prelude::*;

/// Build a `codelore` command with GitHub Actions file-command env vars
/// stripped, so `check`/`gate` subprocesses never append to the CI
/// runner's real `$GITHUB_OUTPUT`/summary files (parallel test processes
/// would interleave writes and corrupt them).
fn codelore_cmd() -> Command {
    let mut cmd = Command::cargo_bin("codelore").unwrap();
    for var in [
        "GITHUB_OUTPUT",
        "GITHUB_STEP_SUMMARY",
        "GITHUB_ENV",
        "GITHUB_STATE",
        "GITHUB_PATH",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn analyze_revisions_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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

/// A reader closing our stdout early (`codelore … | head`, or a pager quit)
/// must exit 0 quietly — never erroring (exit 5) or panicking.
///
/// The assertion is deliberately lenient about *which* internal path fires:
/// on a tiny fixture the child usually finishes writing into the OS pipe
/// buffer before we drop the read end (a plain clean exit 0), whereas output
/// large enough to fill that buffer would block the child and surface a
/// `BrokenPipe` on the next write (mapped to a quiet exit 0 by the CLI's
/// central arm). Both outcomes are exit 0 with no error/panic on stderr, so the
/// test cannot flake on scheduling. The deterministic proof that the
/// `BrokenPipe` → exit-0 mapping itself fires lives in `main.rs`'s
/// `is_broken_pipe` unit tests.
#[test]
fn stdout_reader_closing_early_exits_quietly() {
    use std::io::Read as _;
    use std::process::{Command, Stdio};

    let tiny = codelore_lib::test_support::tiny_repo::build();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_codelore"));
    for var in [
        "GITHUB_OUTPUT",
        "GITHUB_STEP_SUMMARY",
        "GITHUB_ENV",
        "GITHUB_STATE",
        "GITHUB_PATH",
    ] {
        cmd.env_remove(var);
    }
    let mut child = cmd
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
            "--no-banner",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn codelore");

    // Consume a few bytes, then drop the read end — closing our side of the
    // pipe while the child may still be writing.
    {
        let mut stdout = child.stdout.take().expect("child stdout piped");
        let mut buf = [0u8; 8];
        let _ = stdout.read(&mut buf);
    }

    let output = child.wait_with_output().expect("wait for child");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "early pipe close must exit 0 quietly; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Broken pipe") && !stderr.contains("error:") && !stderr.contains("panic"),
        "stderr must stay quiet on early pipe close: {stderr}"
    );
}

#[test]
fn analyze_rejects_unknown_analysis() {
    // `--analysis` is a clap value_parser now, so a bad value is a parse error:
    // exit 2 (the documented CLI/arg-error code, unified with --format and
    // --complexity-sample) with the supported list rendered by clap.
    codelore_cmd()
        .args(["analyze", "--analysis", "not-real", "--repo", "."])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"))
        .stderr(predicate::str::contains("hotspots"));
}

#[test]
fn analyze_unknown_analysis_suggests_nearest() {
    // A near-miss typo gets clap's native did-you-mean tip.
    codelore_cmd()
        .args(["analyze", "--analysis", "hotspot", "--repo", "."])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("hotspots"));
}

#[test]
fn version_flag_works() {
    // Compare against the package version Cargo resolves at compile time, not
    // a hardcoded literal — otherwise every version bump fails CI silently
    // until someone re-reads this test file.
    codelore_cmd()
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
    let output = codelore_cmd()
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
    let output = codelore_cmd()
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

/// A `--depth=1` clone of a merge-commit HEAD ingests zero commits: the merge
/// tip is the only object present locally, and the default
/// `include_merges = false` walk filter drops it, leaving an empty fact
/// store over a real HEAD — the exact truncated-checkout signature
/// `FactsDb::ensure_ingest_witnessed` exists to catch. No `--after`/
/// `--before` filter is passed, so the hard-error branch applies (an empty
/// store from a genuine date-window skip only warns; see the `analyze`
/// witness comment).
///
/// `git clone --depth` on a *local path* source silently ignores the flag
/// (git falls back to its hardlink-based local-clone optimization, which
/// cannot produce a shallow repo) — the `file://` URL form is required to
/// force the real, depth-respecting clone transport.
#[test]
fn analyze_exits_3_on_truncated_shallow_checkout() {
    let full = codelore_lib::test_support::mainline_advance_repo::build();
    let shallow = tempfile::tempdir().unwrap();
    let source_url = format!("file://{}", full.dir.path().display());
    let status = std::process::Command::new("git")
        .args(["clone", "--quiet", "--depth=1"])
        .arg(&source_url)
        .arg(shallow.path())
        .status()
        .unwrap();
    assert!(status.success(), "shallow clone from {source_url} failed");

    let output = codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            shallow.path().to_str().unwrap(),
            "--no-cache",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // CodeLoreError::Repo → exit 3 per spec §6.6
    assert_eq!(output.status.code(), Some(3), "stderr: {stderr}");
    assert!(
        stderr.contains("truncated") && stderr.contains("shallow"),
        "expected the truncated-checkout witness message, got stderr: {stderr}"
    );
}

#[test]
fn invalid_options_exit_with_code_2() {
    // Inverted coupling range (`--min-coupling` > `--max-coupling`) is a
    // cross-field config error → `CodeLoreError::InvalidOptions` → exit 2.
    // Exit 2 (config errors) was the one bucket with no end-to-end CLI
    // coverage; a refactor dropping the typed error to a bare
    // `anyhow::bail!` would silently regress it to exit 1.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = codelore_cmd()
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
    codelore_cmd()
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
            "entity,revisions,cognitive,cognitive-health,hotspot-score",
        ));
}

#[test]
fn analyze_code_health_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
fn sarif_fingerprints_are_stable_across_repo_path_style() {
    // The SARIF fingerprint keys on `repo_root|path`; canonicalizing the repo
    // path makes `--repo .` and `--repo <absolute>` produce identical
    // fingerprints, so GitHub Code Scanning does not re-key (churn) the alerts
    // when the same repo is analysed with a different invocation style.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let abs = tiny.dir.path();

    let fingerprints = |mut cmd: Command| -> Vec<String> {
        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid SARIF");
        let mut fps: Vec<String> = sarif["runs"][0]["results"]
            .as_array()
            .expect("results array")
            .iter()
            .map(|r| {
                r["partialFingerprints"]["primaryLocationLineHash"]
                    .as_str()
                    .expect("each result carries a primaryLocationLineHash")
                    .to_string()
            })
            .collect();
        fps.sort();
        fps
    };

    let mut abs_cmd = codelore_cmd();
    abs_cmd.args([
        "analyze",
        "--analysis",
        "hotspots",
        "--repo",
        abs.to_str().unwrap(),
        "--format",
        "sarif",
        "--min-revs",
        "1",
    ]);
    let abs_fps = fingerprints(abs_cmd);

    // `--repo .` run from inside the repo — canonicalizes to the same path.
    let mut dot_cmd = codelore_cmd();
    dot_cmd.current_dir(abs).args([
        "analyze",
        "--analysis",
        "hotspots",
        "--repo",
        ".",
        "--format",
        "sarif",
        "--min-revs",
        "1",
    ]);
    let dot_fps = fingerprints(dot_cmd);

    assert!(!abs_fps.is_empty(), "expected at least one SARIF finding");
    assert_eq!(
        abs_fps, dot_fps,
        "SARIF fingerprints must match for `--repo .` and `--repo <absolute>`"
    );
}

#[test]
fn analyze_revisions_emits_json() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    let assert = codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "definitelybogus",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"))
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
    codelore_cmd()
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

    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
        codelore_cmd()
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
fn unknown_format_exits_with_arg_code() {
    // An unrecognised `--format` value is now rejected at the parser → clap arg
    // error → exit 2 (the documented CLI/arg-error code), listing the supported
    // formats. Previously it reached the analysis layer and exited 4.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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
        .code(2)
        .stderr(predicate::str::contains("invalid value"))
        .stderr(predicate::str::contains("csv"))
        .stderr(predicate::str::contains("panicked").not());
}

#[test]
fn analyze_warns_when_analysis_scoped_flag_is_ignored() {
    // `--target` is honored only by function-xray/function-coupling. Passing it
    // to another analysis is not an error (scripts may share a flag set), but it
    // must surface a stderr advisory naming the honoring analyses — while the run
    // still succeeds and emits normal output.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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
            "--no-banner",
            "--target",
            "src/main.rs",
        ])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not())
        .stderr(predicate::str::contains("--target"))
        .stderr(predicate::str::contains("function-xray"));
}

#[test]
fn complexity_sample_rejects_unimplemented_values() {
    // `--complexity-sample` advertises only `head` now (its sole implemented
    // strategy). `adaptive`/`full` are rejected honestly at the parser (exit 2)
    // rather than accepted-then-errored with a "not yet available" message.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "hotspots",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--complexity-sample",
            "adaptive",
            "--min-revs",
            "1",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid value"))
        .stderr(predicate::str::contains("head"));
}

#[test]
fn schema_lists_every_registered_analysis() {
    // The schema row-type catalogue must cover every analysis the
    // registry knows about. `delivery-friction`, `main-dev-by-revs`,
    // and `main-dev-by-deletions` are registered analyses that were
    // missing from the hardcoded catalogue.
    for name in codelore_lib::analysis::AnalysisName::all() {
        codelore_cmd()
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
    // Newly added analysis: explain topic not yet wired
    "finding-hotspot-overlap",
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
        let assert = codelore_cmd().args(["explain", name.as_str()]).assert();
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
fn explain_unknown_topic_suggests_nearest() {
    // A free-string topic argument (not a clap enum) gets a hand-rolled
    // nearest-match suggestion. `hotspot` is an abbreviation of the real
    // `hotspots` topic.
    codelore_cmd()
        .args(["explain", "hotspot"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown topic"))
        .stderr(predicate::str::contains("did you mean"))
        .stderr(predicate::str::contains("hotspots"));
}

#[test]
fn health_trend_csv_has_header_and_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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
fn defect_validation_without_artifact_emits_header_only_and_stderr_hint() {
    // No --defect-calibration configured: honest absence, not an error. The
    // CSV header is still written (so downstream tooling gets a valid empty
    // table) with zero data rows, and a one-line hint points at
    // `codelore calibrate-defects` on stderr.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "defect-validation",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("metric,value"))
        // Header only — no data rows without an artifact.
        .stdout(predicate::function(|out: &str| {
            out.lines().filter(|l| !l.trim().is_empty()).count() == 1
        }))
        .stderr(predicate::str::contains("calibrate-defects"));
}

#[test]
fn effort_exposure_csv_has_header_and_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
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
            "band,files,loc-share-pct,commit-share-pct,churn-share-pct,commit-share-ci-low,commit-share-ci-high,churn-share-improving-pct,churn-share-degrading-pct",
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
    codelore_cmd()
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
    let out = codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
        // The `__summary__` carrier row is not emitted as a CSV data row.
        .stdout(predicate::str::contains("__summary__").not())
        // At least one real per-author row is present — every author falls in
        // one of the three tenure buckets.
        .stdout(
            predicate::str::contains("onboarded")
                .or(predicate::str::contains("experienced"))
                .or(predicate::str::contains("veteran")),
        );
}

#[cfg(feature = "spa")]
#[test]
fn spa_without_output_defaults_to_dot_codelore() {
    // `--format spa` no longer requires --output; it defaults to
    // `.codelore/spa.html` under the current working directory.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let cwd = tempfile::tempdir().unwrap();
    codelore_cmd()
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

/// The Architecture factor tile's detail line carries the corpus-relative
/// propagation-cost annotation (`, P<nn> of <n> corpus repos`) exactly when
/// the active calibration artifact has repo-level pools: present on the
/// default path (the embedded world artifact carries `repo_metrics`), absent
/// when `--calibration` points at an artifact without the section.
#[cfg(feature = "spa")]
#[test]
fn spa_architecture_tile_corpus_detail_follows_repo_metrics_presence() {
    // The biomarker fixture carries one resolvable HEAD-time import edge
    // (`src/importer.rs → src/trivial.rs`) and enough dated commits for the
    // health-trend series, so the Architecture tile exists and
    // architecture-metrics has a non-empty import graph to rank.
    let fx = codelore_lib::test_support::biomarker_repo::build();

    let arch_detail = |extra_args: &[&str]| -> String {
        let cwd = tempfile::tempdir().unwrap();
        let mut args = vec![
            "analyze",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--format",
            "spa",
            "--no-banner",
            "--min-revs",
            "1",
        ];
        args.extend_from_slice(extra_args);
        codelore_cmd()
            .current_dir(cwd.path())
            .args(&args)
            .assert()
            .success();
        let html = std::fs::read_to_string(cwd.path().join(".codelore").join("spa.html"))
            .expect("spa.html emitted");

        let start_tag = "<script type=\"application/json\" id=\"codelore-data\">";
        let start = html.find(start_tag).expect("embedded data script") + start_tag.len();
        let end = html[start..].find("</script>").expect("script close");
        let payload: serde_json::Value =
            serde_json::from_str(&html[start..start + end].replace(r"<\/", "</"))
                .expect("payload parses");
        payload["factors"]
            .as_array()
            .expect("factors array present")
            .iter()
            .find(|t| t["name"] == "Architecture")
            .expect("Architecture tile present")["detail"]
            .as_str()
            .expect("detail is a string")
            .to_owned()
    };

    // Default path: the embedded world artifact carries repo_metrics.
    let detail = arch_detail(&[]);
    assert!(
        detail.contains("corpus"),
        "embedded artifact has repo_metrics -> detail must carry the corpus annotation: {detail:?}"
    );

    // Override with an artifact that has no repo_metrics section: the
    // annotation must degrade to absent.
    let artifact = codelore_lib::calibration::CalibrationArtifact {
        format_version: codelore_lib::calibration::CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-corpus-no-pools".to_string(),
        generated_at: "2026-07-14T00:00:00Z".to_string(),
        repos_included: 1,
        repos_attempted: 1,
        languages: vec![],
        repo_metrics: None,
    };
    let work = tempfile::tempdir().unwrap();
    let calib_path = work.path().join("no-pools.calib.json");
    std::fs::write(&calib_path, serde_json::to_vec(&artifact).unwrap()).unwrap();

    let detail = arch_detail(&["--calibration", calib_path.to_str().unwrap()]);
    assert!(
        !detail.contains("corpus"),
        "artifact without repo_metrics -> detail must not carry the corpus annotation: {detail:?}"
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
    let output = codelore_cmd()
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
    let output = codelore_cmd()
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
fn diff_degenerate_thresholds_file_exits_with_config_code() {
    // A thresholds file with an out-of-range value is a configuration error →
    // CodeLoreError::InvalidOptions → exit 2, the same as `check`/`gate`. The
    // diff path used to flatten the typed error through `anyhow!` and exit 1.
    let (dir, base, head) = delta_health_fixture();
    let thresholds = dir.path().join("bad-thresholds.toml");
    std::fs::write(&thresholds, "[gates]\ncode_health_min = 200.0\n").unwrap();
    codelore_cmd()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            &format!("{base}..{head}"),
        ])
        .assert()
        .code(2);
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

    let output = codelore_cmd()
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

/// Two-commit repo where `src/lib.rs` is identical at base and head (only
/// `README.md` changes between them) — the "populated-unchanged" fixture:
/// real commit history, real (non-empty) hotspot rows at both revisions,
/// zero code delta. Distinct from a blind ingest, which empties the hotspot
/// row SET itself rather than just the delta between two populated sets.
fn unchanged_code_fixture() -> (tempfile::TempDir, String, String) {
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
    (dir, base, head)
}

#[test]
fn diff_gate_passes_on_populated_unchanged_range() {
    // A genuinely unchanged range with REAL (non-empty) hotspot rows at both
    // revisions must keep today's verdict byte-identical: no violations, and
    // — the case this fix must not regress — no skip disclosure either, since
    // real data was measured on both sides.
    let (dir, base, head) = unchanged_code_fixture();
    let thresholds = dir.path().join("gates.toml");
    std::fs::write(&thresholds, "[diff]\nno_new_cycles = true\n").unwrap();
    let output = codelore_cmd()
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
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // `gate_violations` is omitted from the JSON document entirely when
    // empty (`skip_serializing_if = "Vec::is_empty"`), so its absence IS the
    // empty-violations case.
    assert!(
        json.get("gate_violations").is_none(),
        "unchanged code ⇒ no violations: {json}"
    );
    assert!(
        json.get("gate_skip_reason").is_none(),
        "real rows on both sides ⇒ never a skip: {json}"
    );
}

#[test]
fn diff_gate_skipped_when_neither_revision_measures_any_hotspot_row() {
    // A `--min-revs` floor above every file's revision count empties the
    // hotspot row set at BOTH revisions — the same shape a blind ingest (a
    // shallow checkout) produces. Every scalar evaluate_diff_gate would see
    // (new_hotspot_count, delta_code_health, cycle counts) reads identically
    // to a genuinely unchanged repo; the gate must disclose a skip instead of
    // a silent pass, and must not fail the run (exit code unaffected).
    let (dir, base, head) = unchanged_code_fixture();
    let thresholds = dir.path().join("gates.toml");
    std::fs::write(&thresholds, "[diff]\nno_new_cycles = true\n").unwrap();
    let output = codelore_cmd()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "50",
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "a skip must not fail the run — stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // Omitted entirely when empty — see the sibling test's comment.
    assert!(
        json.get("gate_violations").is_none(),
        "nothing measured ⇒ no violations either: {json}"
    );
    let reason = json["gate_skip_reason"]
        .as_str()
        .expect("gate_skip_reason must be a disclosed string, not null");
    assert!(
        reason.contains("blind ingest"),
        "reason must name the cause: {reason}"
    );

    // The text format must surface the same skip, not silence.
    let text_output = codelore_cmd()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "50",
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(text_output.status.success());
    let stdout = String::from_utf8(text_output.stdout).unwrap();
    assert!(
        stdout.contains("SKIPPED"),
        "text format must disclose the skip: {stdout}"
    );
    assert!(
        !stdout.contains("VIOLATION"),
        "a skip is not a violation: {stdout}"
    );
}

#[test]
fn diff_sarif_schema_url_and_info_uri_use_canonical_constants() {
    // The diff SARIF schema URL and informationUri must use the constants from
    // codelore_lib::output::sarif, and degrading delta-health results must carry
    // codeFlows evidence chains (the monster function has one head commit).
    let (dir, base, head) = delta_health_fixture();
    let output = codelore_cmd()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "sarif",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid SARIF");

    // (a) schema URL must match the canonical constant from sarif.rs
    assert_eq!(
        sarif["$schema"], "https://json.schemastore.org/sarif-2.1.0.json",
        "wrong $schema"
    );

    // (b) informationUri must match the canonical constant from sarif.rs
    assert_eq!(
        sarif["runs"][0]["tool"]["driver"]["informationUri"], "https://github.com/emrecdr/codelore",
        "wrong informationUri"
    );

    // (c) The degrading delta-health result for src/lib.rs (monster function)
    // must carry at least one codeFlow with a threadFlow containing locations.
    let results = sarif["runs"][0]["results"]
        .as_array()
        .expect("results array");
    let degrading: Vec<_> = results
        .iter()
        .filter(|r| r["ruleId"] == "CODELORE-DELTA-HEALTH")
        .collect();
    assert!(
        !degrading.is_empty(),
        "expected at least one CODELORE-DELTA-HEALTH result (monster function)"
    );
    let r = degrading[0];
    let code_flows = r["codeFlows"]
        .as_array()
        .expect("codeFlows array on degrading result");
    assert!(
        !code_flows.is_empty(),
        "degrading result must carry at least one codeFlow"
    );
    let thread_flows = code_flows[0]["threadFlows"]
        .as_array()
        .expect("threadFlows array");
    assert!(
        !thread_flows.is_empty(),
        "codeFlow must have at least one threadFlow"
    );
    let locations = thread_flows[0]["locations"]
        .as_array()
        .expect("locations array");
    assert!(
        !locations.is_empty(),
        "threadFlow must have at least one location (evidence commit)"
    );

    // (d) The degrading result must also carry relatedLocations (plain location
    // array — the GitHub inline annotation panel source, distinct from codeFlows).
    let related = r["relatedLocations"]
        .as_array()
        .expect("relatedLocations array on degrading result");
    assert!(
        !related.is_empty(),
        "degrading result must carry at least one relatedLocation"
    );
    // relatedLocations entries are plain location objects (no "location" wrapper).
    assert!(
        related[0].get("physicalLocation").is_some(),
        "relatedLocations entry must have physicalLocation directly (no wrapper)"
    );
}

#[test]
fn diff_sarif_hotspot_rank_entrant_carries_code_flows_and_related_locations() {
    // Build a fixture where the base has no Rust files (no hotspots at base)
    // and the head introduces a Rust file that was changed twice — guaranteeing
    // it enters the hotspot list as a rank_entrant with ≥1 evidence commit.
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

    // base: no Rust files → no hotspots at base revision
    git(&["init", "-q"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base: no rust"]);
    let base = git(&["rev-parse", "HEAD"]);

    // head: src/hot.rs added and then changed — 2 revisions, enters hotspot list
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/hot.rs"), "pub fn first() -> u32 { 1 }\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "feat: add hot file"]);

    std::fs::write(
        repo.join("src/hot.rs"),
        "pub fn first() -> u32 { 2 }\npub fn second() -> u32 { 3 }\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "feat: extend hot file"]);
    let head = git(&["rev-parse", "HEAD"]);

    let output = codelore_cmd()
        .args([
            "diff",
            "--repo",
            repo.to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "sarif",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let sarif: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid SARIF");
    let results = sarif["runs"][0]["results"]
        .as_array()
        .expect("results array");

    // There must be at least one CODELORE-HOTSPOT rank-entrant result.
    let hotspot_results: Vec<_> = results
        .iter()
        .filter(|r| r["ruleId"] == "CODELORE-HOTSPOT")
        .collect();
    assert!(
        !hotspot_results.is_empty(),
        "expected ≥1 CODELORE-HOTSPOT rank-entrant result for src/hot.rs"
    );

    let r = hotspot_results[0];

    // Must carry codeFlows with at least one evidence location.
    let code_flows = r["codeFlows"]
        .as_array()
        .expect("codeFlows must be present on hotspot rank-entrant");
    assert!(!code_flows.is_empty(), "codeFlows must be non-empty");
    let tfl = code_flows[0]["threadFlows"][0]["locations"]
        .as_array()
        .expect("threadFlows[0].locations");
    assert!(
        !tfl.is_empty(),
        "hotspot result must carry at least one evidence commit in codeFlows"
    );

    // Must carry relatedLocations — plain location objects, no "location" wrapper.
    let related = r["relatedLocations"]
        .as_array()
        .expect("relatedLocations must be present on hotspot rank-entrant");
    assert!(
        !related.is_empty(),
        "hotspot result must carry at least one relatedLocation"
    );
    assert!(
        related[0].get("physicalLocation").is_some(),
        "relatedLocations entry must have physicalLocation directly (no wrapper)"
    );

    // Sanity: no stray "module" key on threadFlowLocations (was a spec error).
    assert!(
        tfl[0].get("module").is_none(),
        "threadFlowLocation must not carry 'module' (message_head goes in location.message.text)"
    );

    // partialFingerprints: both dedup keys must match the shared recipes.
    // primaryLocationLineHash must equal the check recipe for the same path.
    assert_diff_fingerprints(r, repo, "CODELORE-HOTSPOT", "src/hot.rs", "rank-entrant");
}

/// Assert a diff SARIF `result` carries both dedup fingerprint keys and that
/// each matches its shared recipe: `primaryLocationLineHash` = the check recipe
/// `sha256(canonical_repo_root|path)`, `diffFinding/v1` =
/// `sha256(rule|path|discriminant)`.
fn assert_diff_fingerprints(
    result: &serde_json::Value,
    repo: &std::path::Path,
    rule: &str,
    path: &str,
    discriminant: &str,
) {
    let fps = result["partialFingerprints"]
        .as_object()
        .expect("diff result must carry partialFingerprints");

    let canonical_root = repo.canonicalize().unwrap();
    let expected_primary = codelore_lib::output::sarif::primary_location_line_hash(
        &canonical_root.to_string_lossy(),
        path,
    );
    assert_eq!(
        fps.get("primaryLocationLineHash").and_then(|v| v.as_str()),
        Some(expected_primary.as_str()),
        "diff primaryLocationLineHash must match the check recipe sha256(repo_root|path)"
    );

    let expected_diff = codelore_lib::output::sarif::diff_finding_hash(rule, path, discriminant);
    assert_eq!(
        fps.get("diffFinding/v1").and_then(|v| v.as_str()),
        Some(expected_diff.as_str()),
        "diffFinding/v1 must be sha256(rule|path|discriminant)"
    );
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

    let output = codelore_cmd()
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
    codelore_cmd()
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
            "path,authors,fragmentation,interleave,cochange-entropy,tier,health-band,total-commits",
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
        .args(["check", "--repo", tiny.dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("vacuously passing"));
}

#[test]
fn function_xray_emits_markdown_header() {
    let repo = codelore_lib::test_support::function_xray_repo::build();
    codelore_cmd()
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
    codelore_cmd()
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
    codelore_cmd()
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

#[test]
fn check_format_sarif_emits_valid_sarif_and_exits_1() {
    // `code_health_min = 100.0` is impossibly high so the gate always fires
    // against biomarker_repo, producing at least one per-file violation.
    // With --format sarif:
    //   - exit code must still be 1 (violations are present)
    //   - stdout must be a valid SARIF document with ≥1 result
    //   - the FAIL verdict goes to stderr (not stdout)
    // Note: a PASS produces a zero-result SARIF document on stdout (valid;
    // stderr gets the PASS verdict). This is intentional — the caller decides
    // whether an empty result set is interesting.
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let thresholds = repo.dir.path().join(".codelore-thresholds.toml");
    std::fs::write(&thresholds, "[gates]\ncode_health_min = 100.0\n").unwrap();

    let output = codelore_cmd()
        .args([
            "check",
            "--repo",
            repo.dir.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .expect("run codelore check --format sarif");

    // Exit code 1 — gate violation semantics unchanged by format.
    assert!(
        !output.status.success(),
        "expected exit 1 for gate violation, got {}",
        output.status
    );

    // stdout is valid SARIF with ≥1 result.
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON SARIF");
    assert_eq!(
        parsed["version"].as_str().unwrap(),
        "2.1.0",
        "SARIF version must be 2.1.0"
    );
    let results = parsed["runs"][0]["results"].as_array().unwrap();
    assert!(
        !results.is_empty(),
        "expected ≥1 SARIF result for code_health_min violation"
    );

    // The FAIL verdict line goes to stderr (stdout stays clean JSON).
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(
        stderr.contains("FAIL"),
        "FAIL verdict must appear on stderr even with --format sarif"
    );
}

#[test]
fn check_default_format_is_text_not_json() {
    // Omitting --format must yield text output (the PASS/FAIL verdict on
    // stdout/stderr), not a JSON/SARIF document. Verifies that the
    // default_value_t = CheckFormat::Text contract holds end-to-end.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = codelore_cmd()
        .args(["check", "--repo", tiny.dir.path().to_str().unwrap()])
        .output()
        .expect("run codelore check without --format");

    // Exit 0 — tiny_repo has no thresholds file → vacuous pass.
    assert!(output.status.success(), "expected exit 0 for vacuous pass");

    // stdout must not be JSON (text mode doesn't print SARIF to stdout).
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_err(),
        "stdout must not be a JSON document in text mode, got: {stdout}"
    );
}

/// A monster function: nested loops + match + boolean conditionals so its
/// cyclomatic / cognitive / nesting / bool-op counts dominate the fixture,
/// making the appended-to file's projected code-health score strictly worse
/// than its HEAD baseline.
const GATE_MONSTER_FN: &str = r"
fn monster(x: i32) -> i32 {
    let mut acc = 0;
    for a in 0..x {
        if a % 2 == 0 && a % 3 == 0 || a % 5 == 0 {
            for b in 0..a {
                if b > 1 {
                    match b % 4 {
                        0 => { if b > 10 { acc += 1; } else { acc += 2; } }
                        1 => { while acc < 100 { acc += 1; if acc % 7 == 0 { break; } } }
                        2 => { for c in 0..b { if c > 3 && c < 9 || c == 5 { acc += c; } } }
                        _ => { if a > b { acc -= 1; } else { acc += 1; } }
                    }
                }
            }
        }
    }
    acc
}
";

/// Append `text` to the file at `path`.
fn append_to_file(path: &std::path::Path, text: &str) {
    let mut content = std::fs::read_to_string(path).expect("read file");
    content.push_str(text);
    std::fs::write(path, content).expect("write file");
}

/// Write a scratch thresholds file with `body` into its own tempdir and
/// return the guard plus the file path (the guard keeps the dir alive).
fn scratch_thresholds(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("thresholds tempdir");
    let path = dir.path().join("gate-thresholds.toml");
    std::fs::write(&path, body).expect("write thresholds");
    (dir, path)
}

#[test]
fn gate_vacuous_passes_without_thresholds() {
    // Without a thresholds file the gate vacuously passes with the same
    // diagnostic contract as `check` (wording substitutes "gate"); exit 0.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    codelore_cmd()
        .args(["gate", "--repo", tiny.dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "codelore gate: no thresholds configured",
        ))
        .stderr(predicate::str::contains("vacuously passing"));
}

#[test]
fn gate_passes_on_clean_tree_with_thresholds() {
    // A fresh clone has no working-tree changes: with gates configured the
    // run still passes (exit 0) and says so explicitly — a clean tree is a
    // pass, not a skipped evaluation.
    let fx = codelore_lib::test_support::differential_repo::build();
    let (_guard, thresholds) = scratch_thresholds("[diff]\ndelta_code_health_min_per_file = 0.0\n");
    let cache = tempfile::tempdir().expect("cache tempdir");
    codelore_cmd()
        .args([
            "gate",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no working-tree changes"));
}

#[test]
fn gate_fails_on_per_file_floor() {
    // Appending a deeply-nested high-complexity function to a tracked file
    // makes its projected score strictly worse than its HEAD baseline, so a
    // per-file floor of 0.0 (no file may lower its own health) must fail the
    // gate with check's exit contract (1) and name the offending file.
    let fx = codelore_lib::test_support::differential_repo::build();
    append_to_file(&fx.dir.path().join("src/main.rs"), GATE_MONSTER_FN);
    let (_guard, thresholds) = scratch_thresholds("[diff]\ndelta_code_health_min_per_file = 0.0\n");
    let cache = tempfile::tempdir().expect("cache tempdir");
    codelore_cmd()
        .args([
            "gate",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("codelore gate: FAIL"))
        .stderr(predicate::str::contains("delta_code_health_min_per_file"))
        .stderr(predicate::str::contains("src/main.rs"));
}

#[test]
fn gate_json_shape() {
    // --format json puts the full change-set report plus the evaluated
    // violations on stdout as one JSON document: `changes`, `findings`, and
    // `violations` are the contract keys downstream consumers read.
    let fx = codelore_lib::test_support::differential_repo::build();
    append_to_file(&fx.dir.path().join("src/main.rs"), GATE_MONSTER_FN);
    // no_new_cycles is configured (non-empty thresholds) but the append
    // introduces no import edge, so the run passes: violations = [].
    let (_guard, thresholds) = scratch_thresholds("[diff]\nno_new_cycles = true\n");
    let cache = tempfile::tempdir().expect("cache tempdir");
    let output = codelore_cmd()
        .args([
            "gate",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run codelore gate --format json");
    assert!(
        output.status.success(),
        "no cycle introduced ⇒ pass, got {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let doc: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be one JSON document");
    let changes = doc["changes"].as_array().expect("changes array");
    assert_eq!(changes.len(), 1, "one modified file: {changes:?}");
    assert!(doc["findings"].is_array(), "findings key present: {doc}");
    let violations = doc["violations"].as_array().expect("violations array");
    assert!(violations.is_empty(), "clean pass: {violations:?}");
    // The verdict line is emitted regardless of format; in JSON it goes to
    // stderr so stdout stays a pure document.
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(
        stderr.contains("codelore gate: PASS"),
        "JSON PASS must still print a verdict line to stderr: {stderr}",
    );
}

#[test]
fn gate_findings_render_capped_with_more_tail() {
    // Regression: the findings RENDER must cap at a fixed row count with
    // a "(+n more findings)" tail, mirroring the delta-table cap — otherwise
    // a big change set (13 newly-added files here, each producing its own
    // "new-file" finding) blows the text render's token budget even though
    // the underlying `ChangeSetReport.findings` stays unbounded by design.
    const ADDED: usize = 13; // > the render cap (10) — a tail is guaranteed.
    let fx = codelore_lib::test_support::differential_repo::build();
    for i in 0..ADDED {
        std::fs::write(
            fx.dir.path().join(format!("src/gate_extra_{i}.rs")),
            format!("pub fn extra_{i}() -> u32 {{ {i} }}\n"),
        )
        .expect("write extra file");
    }
    let add = std::process::Command::new("git")
        .args(["-C", fx.dir.path().to_str().unwrap(), "add", "-A"])
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed: {add:?}");
    // A real (non-empty) thresholds file: the vacuous "no thresholds
    // configured" path returns before `build_change_set_report` ever runs, so
    // it never reaches the findings render this test is pinning. `no_new_cycles`
    // is a threshold none of these additions can violate (no import edges).
    let (_guard, thresholds) = scratch_thresholds("[diff]\nno_new_cycles = true\n");
    let cache = tempfile::tempdir().expect("cache tempdir");

    let output = codelore_cmd()
        .args([
            "gate",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .output()
        .expect("run codelore gate");
    assert!(
        output.status.success(),
        "no cycle introduced ⇒ pass: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");

    let finding_lines = stdout
        .lines()
        .filter(|l| l.starts_with("[new-file]"))
        .count();
    assert_eq!(
        finding_lines, 10,
        "the findings render must cap at 10 rows: {stdout}"
    );
    assert!(
        stdout.contains(&format!("(+{} more findings)", ADDED - 10)),
        "a '(+n more findings)' tail must disclose the hidden rows: {stdout}"
    );
}

#[test]
fn gate_vacuous_json_emits_contract_document() {
    // With no thresholds configured, `--format json` must still put one
    // contract document on stdout so an agent hook that always parses JSON
    // never special-cases a repo without a thresholds file.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = codelore_cmd()
        .args([
            "gate",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .expect("run codelore gate --format json");
    assert!(output.status.success(), "vacuous pass exits 0");
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let doc: serde_json::Value =
        serde_json::from_str(&stdout).expect("vacuous JSON must be one parseable document");
    assert!(doc["changes"].is_array(), "changes key present: {doc}");
    assert!(doc["findings"].is_array(), "findings key present: {doc}");
    assert!(
        doc["violations"].is_array(),
        "violations key present: {doc}"
    );
}

#[test]
fn check_max_findings_gate_skips_gracefully_when_no_sidecar() {
    // Gate configured, but no prior `ingest-sarif` run → sidecar absent.
    // Expected contract:
    //   - exit code unaffected (0 — only the overlap gate is configured here)
    //   - ledger records a `verdict="skipped"` entry for max_findings_in_hot_files
    //   - the sidecar file is NOT created as a side-effect of the check
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo_path = tiny.dir.path();
    let thresholds = repo_path.join(".codelore-thresholds.toml");
    std::fs::write(&thresholds, "[gates]\nmax_findings_in_hot_files = 0\n").unwrap();

    // Compute the sidecar path the binary would use — same logic as the CLI.
    let cache_root = codelore_lib::cli_api::cache::default_cache_root();
    let sidecar_path = codelore_lib::cli_api::cache::repo_cache_dir(&cache_root, repo_path)
        .join("external-findings.duckdb-ext");

    // Pre-condition: sidecar must not exist before the run.
    assert!(
        !sidecar_path.exists(),
        "pre-condition: sidecar must not exist before check"
    );

    // Run check — should pass (no other gates configured) and skip the overlap gate.
    codelore_cmd()
        .args(["check", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success();

    // Post-condition: sidecar must NOT have been created by the check run.
    assert!(
        !sidecar_path.exists(),
        "check must not create the sidecar as a side-effect when ingest-sarif was never run"
    );

    // The ledger must record a skipped verdict for this gate.
    let records =
        codelore_lib::cli_api::quality_gates::ledger::read_gate_runs(&cache_root, repo_path)
            .expect("read ledger");
    let overlap_rec = records
        .iter()
        .rev()
        .find(|r| r.gate == "max_findings_in_hot_files")
        .expect("ledger must contain a max_findings_in_hot_files record");
    assert_eq!(
        overlap_rec.verdict, "skipped",
        "overlap gate must record verdict=skipped when sidecar is absent"
    );
}

#[test]
fn check_corpus_percentile_gate_skips_when_no_health_rows() {
    // Gate configured, but the tiny repo's files fall below the code-health churn
    // floor (`min_revs`), so the health scan yields no rows — there is nothing for
    // the corpus lens to populate, so no row carries `corpus_percentile` and the
    // gate skips (not pass, not fail). This holds regardless of whether a
    // calibration artifact is active. Expected contract:
    //   - exit code unaffected (0 — only the corpus gate is configured here)
    //   - ledger records a `verdict="skipped"` entry for corpus_percentile_max
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo_path = tiny.dir.path();
    let thresholds = repo_path.join(".codelore-thresholds.toml");
    std::fs::write(&thresholds, "[gates]\ncorpus_percentile_max = 0.9\n").unwrap();

    // Run check — should pass (no rows → gate skipped, no other gates).
    codelore_cmd()
        .args(["check", "--repo", repo_path.to_str().unwrap()])
        .assert()
        .success();

    // The ledger must record a skipped verdict for this gate.
    let cache_root = codelore_lib::cli_api::cache::default_cache_root();
    let records =
        codelore_lib::cli_api::quality_gates::ledger::read_gate_runs(&cache_root, repo_path)
            .expect("read ledger");
    let corpus_rec = records
        .iter()
        .rev()
        .find(|r| r.gate == "corpus_percentile_max")
        .expect("ledger must contain a corpus_percentile_max record");
    assert_eq!(
        corpus_rec.verdict, "skipped",
        "corpus gate must record verdict=skipped when the health scan yields no rows"
    );
}

/// SARIF 2.1.0 document that reports one finding for `engine` on `path`.
fn sarif_one_finding(engine: &str, path: &str) -> String {
    format!(
        r#"{{
            "version": "2.1.0",
            "runs": [{{
                "tool": {{ "driver": {{ "name": "{engine}", "version": "1.0" }} }},
                "results": [{{
                    "ruleId": "rule/one",
                    "level": "warning",
                    "message": {{ "text": "a finding" }},
                    "locations": [{{
                        "physicalLocation": {{
                            "artifactLocation": {{ "uri": "{path}" }},
                            "region": {{ "startLine": 1 }}
                        }}
                    }}]
                }}]
            }}]
        }}"#
    )
}

/// SARIF 2.1.0 document from `engine` that reports zero findings (a clean run).
fn sarif_zero_findings(engine: &str) -> String {
    format!(
        r#"{{
            "version": "2.1.0",
            "runs": [{{
                "tool": {{ "driver": {{ "name": "{engine}", "version": "1.0" }} }},
                "results": []
            }}]
        }}"#
    )
}

/// Ingest a zero-finding SARIF so the sidecar exists but holds no rows, then run
/// `check` with the overlap gate configured. The empty sidecar must take the
/// same skip path as an absent one: exit 0 and a ledger verdict of `skipped`.
#[test]
fn check_max_findings_gate_skips_when_sidecar_present_but_empty() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo_path = tiny.dir.path();
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let cache_root = cache_dir.path();

    let thresholds = repo_path.join(".codelore-thresholds.toml");
    std::fs::write(&thresholds, "[gates]\nmax_findings_in_hot_files = 0\n").unwrap();

    // Create an EMPTY sidecar via ingest-sarif with a zero-finding SARIF.
    let empty_sarif = cache_dir.path().join("empty.sarif.json");
    std::fs::write(&empty_sarif, sarif_zero_findings("semgrep")).unwrap();
    codelore_cmd()
        .args([
            "ingest-sarif",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            empty_sarif.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The sidecar file now exists but holds zero rows.
    let store =
        codelore_lib::cli_api::external::ExternalStore::open_existing(cache_root, repo_path)
            .expect("open_existing")
            .expect("sidecar must exist after ingest-sarif");
    assert_eq!(store.count().expect("count"), 0, "sidecar must be empty");
    drop(store);

    // check must exit 0 — the empty sidecar is skipped, not an error.
    codelore_cmd()
        .args([
            "check",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The ledger must record a skipped verdict for the overlap gate.
    let records =
        codelore_lib::cli_api::quality_gates::ledger::read_gate_runs(cache_root, repo_path)
            .expect("read ledger");
    let overlap_rec = records
        .iter()
        .rev()
        .find(|r| r.gate == "max_findings_in_hot_files")
        .expect("ledger must contain a max_findings_in_hot_files record");
    assert_eq!(
        overlap_rec.verdict, "skipped",
        "overlap gate must record verdict=skipped when sidecar is present but empty"
    );
}

/// `analyze --analysis finding-hotspot-overlap --cache-dir X` must read the
/// sidecar under the SAME custom cache root that `ingest-sarif --cache-dir X`
/// wrote it to — not the default XDG root.
#[test]
fn analyze_finding_overlap_respects_cache_dir() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo_path = tiny.dir.path();
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let cache_root = cache_dir.path();

    // Ingest one finding into the sidecar under the custom cache root.
    let sarif = cache_dir.path().join("one.sarif.json");
    std::fs::write(&sarif, sarif_one_finding("semgrep", "src/lib.rs")).unwrap();
    codelore_cmd()
        .args([
            "ingest-sarif",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            sarif.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The overlap analysis under the same cache-dir must FIND the ingested
    // finding (emit ≥1 row), not report the missing-sidecar pre-condition error.
    let output = codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "finding-hotspot-overlap",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            "--format",
            "json",
            "--min-revs",
            "1",
        ])
        .output()
        .expect("run finding-hotspot-overlap");
    assert!(
        output.status.success(),
        "overlap analysis must succeed when the sidecar lives under --cache-dir; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("overlap output must be valid JSON");
    let rows = parsed.as_array().expect("overlap JSON is an array");
    assert!(
        rows.iter().any(|r| r["path"] == "src/lib.rs"),
        "overlap must include the ingested finding's path, got: {stdout}"
    );
}

/// `analyze --analysis finding-hotspot-overlap` against an existing-but-EMPTY
/// sidecar must surface the same "requires prior ingest-sarif" pre-condition
/// error as an absent sidecar — an empty sidecar carries no findings to read.
#[test]
fn analyze_finding_overlap_empty_sidecar_reports_precondition_error() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo_path = tiny.dir.path();
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let cache_root = cache_dir.path();

    // Create an EMPTY sidecar via ingest-sarif with a zero-finding SARIF.
    let empty_sarif = cache_dir.path().join("empty.sarif.json");
    std::fs::write(&empty_sarif, sarif_zero_findings("semgrep")).unwrap();
    codelore_cmd()
        .args([
            "ingest-sarif",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            empty_sarif.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The sidecar file now exists but holds zero rows.
    let store =
        codelore_lib::cli_api::external::ExternalStore::open_existing(cache_root, repo_path)
            .expect("open_existing")
            .expect("sidecar must exist after ingest-sarif");
    assert_eq!(store.count().expect("count"), 0, "sidecar must be empty");
    drop(store);

    // The overlap analysis must FAIL with the pre-condition error, not succeed
    // with an empty table.
    let output = codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "finding-hotspot-overlap",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            "--format",
            "json",
            "--min-revs",
            "1",
        ])
        .output()
        .expect("run finding-hotspot-overlap");
    assert!(
        !output.status.success(),
        "overlap analysis must fail on an empty sidecar"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ingest-sarif"),
        "error must mention the ingest-sarif pre-condition; got: {stderr}"
    );
}

/// Re-ingesting a clean (zero-result) scan for an engine must clear that
/// engine's stale rows. The stored count must reflect the current scanner run,
/// never an accumulation of a prior run's findings.
#[test]
fn ingest_sarif_clean_rescan_clears_stale_engine_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo_path = tiny.dir.path();
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let cache_root = cache_dir.path();

    // First scan: one finding for engine "semgrep".
    let with_finding = cache_dir.path().join("with_finding.sarif.json");
    std::fs::write(&with_finding, sarif_one_finding("semgrep", "src/lib.rs")).unwrap();
    codelore_cmd()
        .args([
            "ingest-sarif",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            with_finding.to_str().unwrap(),
        ])
        .assert()
        .success();

    let store =
        codelore_lib::cli_api::external::ExternalStore::open_existing(cache_root, repo_path)
            .expect("open_existing")
            .expect("sidecar must exist");
    assert_eq!(
        store.count().expect("count"),
        1,
        "first scan must store one finding"
    );
    drop(store);

    // Second scan for the SAME engine reports zero findings (issue fixed).
    let clean = cache_dir.path().join("clean.sarif.json");
    std::fs::write(&clean, sarif_zero_findings("semgrep")).unwrap();
    codelore_cmd()
        .args([
            "ingest-sarif",
            "--repo",
            repo_path.to_str().unwrap(),
            "--cache-dir",
            cache_root.to_str().unwrap(),
            clean.to_str().unwrap(),
        ])
        .assert()
        .success();

    // The stale finding must be gone — the clean re-scan cleared the engine.
    let store =
        codelore_lib::cli_api::external::ExternalStore::open_existing(cache_root, repo_path)
            .expect("open_existing")
            .expect("sidecar must still exist");
    assert_eq!(
        store.count().expect("count"),
        0,
        "clean re-scan must clear the engine's stale rows"
    );
}

/// `check --format sarif` on a repo with no thresholds file must still emit a
/// valid zero-result SARIF document to stdout — the documented upload-sarif
/// pipeline breaks if a vacuous pass prints nothing.
#[test]
fn check_format_sarif_vacuous_pass_emits_zero_result_document() {
    // tiny_repo has no `.codelore-thresholds.toml` → vacuous pass.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = codelore_cmd()
        .args([
            "check",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .expect("run codelore check --format sarif with no thresholds");

    // Exit 0 — vacuous pass.
    assert!(
        output.status.success(),
        "vacuous pass must exit 0, got {}",
        output.status
    );

    // stdout must be a valid SARIF document with an empty results array.
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("vacuous pass stdout must be valid SARIF JSON: {e}; got: {stdout}")
    });
    assert_eq!(parsed["version"].as_str().unwrap(), "2.1.0");
    let results = parsed["runs"][0]["results"]
        .as_array()
        .expect("runs[0].results must be an array");
    assert!(
        results.is_empty(),
        "vacuous pass must emit runs[0].results == [], got: {results:?}"
    );
}

/// `check --ratchet --format sarif` on a first run (no snapshot yet → the
/// ratchet-init exit path) must still emit a valid SARIF document to stdout,
/// exactly like every non-ratchet check path. Before the fix the init path
/// returned before the SARIF emission, so the flag combination silently emitted
/// nothing.
#[test]
fn check_ratchet_format_sarif_init_emits_valid_document() {
    // Fresh clone → no `.codelore-ratchet.toml`, so --ratchet takes the init
    // path. No thresholds file, but --ratchet bypasses the vacuous-pass guard.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let output = codelore_cmd()
        .args([
            "check",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--ratchet",
            "--format",
            "sarif",
        ])
        .output()
        .expect("run codelore check --ratchet --format sarif");

    // Exit 0 — ratchet initialization is not a failure.
    assert!(
        output.status.success(),
        "ratchet init must exit 0, got {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    // stdout must parse as a SARIF 2.1.0 document — not be empty.
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("ratchet+sarif stdout must be valid SARIF JSON: {e}; got: {stdout:?}")
    });
    assert_eq!(parsed["version"].as_str().unwrap(), "2.1.0");
    parsed["runs"][0]["results"]
        .as_array()
        .expect("runs[0].results must be an array");
}

// ── codelore calibrate ───────────────────────────────────────────────────────

/// Write a corpus manifest pointing at one or more local `(path, sha)` repos and
/// return the manifest path (kept alive by the caller-owned `dir`).
fn write_calibrate_manifest(dir: &std::path::Path, repos: &[(&str, &str)]) -> std::path::PathBuf {
    use std::fmt::Write as _;
    let mut toml = String::new();
    for (source, sha) in repos {
        let _ = write!(
            toml,
            "[[repos]]\nsource = {source:?}\nsha = {sha:?}\nlanguages = [\"rust\"]\n\n"
        );
    }
    let path = dir.join("corpus.toml");
    std::fs::write(&path, toml).expect("write manifest");
    path
}

/// A manifest of two local fixture repos builds an artifact that parses through
/// the library's own load/validate path, with a non-empty, monotone rust table.
#[test]
fn calibrate_builds_artifact_from_local_fixtures() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let work = tempfile::tempdir().expect("tempdir");
    let cache = tempfile::tempdir().expect("cache tempdir");

    let manifest = write_calibrate_manifest(
        work.path(),
        &[
            (tiny.dir.path().to_str().unwrap(), &tiny.head_sha),
            (bio.dir.path().to_str().unwrap(), &bio.head_sha),
        ],
    );
    let out = work.path().join("world.calib.json");

    codelore_cmd()
        .args([
            "calibrate",
            "--repos",
            manifest.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(out.exists(), "artifact file must be written");
    let art = codelore_lib::calibration::load(&out).expect("artifact parses + validates");
    assert_eq!(art.repos_attempted, 2);
    assert_eq!(art.repos_included, 2);

    let rust = art
        .languages
        .iter()
        .find(|l| l.language == "rust")
        .expect("rust table present");
    assert!(
        rust.sample_functions > 0,
        "rust must have pooled at least one function"
    );
    // Every metric's breakpoint vector is full-length and non-decreasing — the
    // same invariant `load` already enforced, re-asserted here as the test's
    // own contract on the built artifact.
    for stratum in &rust.strata {
        for metric in &stratum.metrics {
            assert_eq!(
                metric.quantiles.len(),
                codelore_lib::calibration::QUANTILE_POINTS
            );
            assert!(
                metric.quantiles.windows(2).all(|w| w[1] >= w[0]),
                "metric {:?} quantiles must be monotone",
                metric.metric
            );
        }
    }

    // Repo-level architecture metrics: `tiny_repo` has no resolvable HEAD-time
    // imports (empty import graph → skipped entirely per the pooling
    // contract), while `biomarker_repo` carries one resolvable
    // `src/importer.rs → src/trivial.rs` edge, so at most one of the two
    // repos contributes an observation to each pool.
    let rm = art
        .repo_metrics
        .expect("repo_metrics must be populated when at least one repo has a non-empty graph");
    let propagation_cost = rm
        .values
        .get("propagation_cost")
        .expect("propagation_cost pool present");
    let cycle_file_share = rm
        .values
        .get("cycle_file_share")
        .expect("cycle_file_share pool present");
    assert!(
        !propagation_cost.is_empty() && propagation_cost.len() <= 2,
        "propagation_cost must have between 1 and repos_included entries, got {}",
        propagation_cost.len()
    );
    assert!(
        !cycle_file_share.is_empty() && cycle_file_share.len() <= 2,
        "cycle_file_share must have between 1 and repos_included entries, got {}",
        cycle_file_share.len()
    );
    for &v in propagation_cost.iter().chain(cycle_file_share.iter()) {
        assert!(
            (0.0..=1.0).contains(&v),
            "repo-level metric value {v} must be in [0,1]"
        );
    }
}

/// A manifest with one good repo and one nonexistent path: the bad repo is
/// warned about and skipped, the run still exits 0, and the artifact records
/// `attempted == 2`, `included == 1`.
#[test]
fn calibrate_skips_unreachable_repo_and_exits_zero() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let work = tempfile::tempdir().expect("tempdir");
    let cache = tempfile::tempdir().expect("cache tempdir");

    let missing = work.path().join("does-not-exist");
    let manifest = write_calibrate_manifest(
        work.path(),
        &[
            (tiny.dir.path().to_str().unwrap(), &tiny.head_sha),
            (missing.to_str().unwrap(), "deadbeef"),
        ],
    );
    let out = work.path().join("world.calib.json");

    codelore_cmd()
        .args([
            "calibrate",
            "--repos",
            manifest.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("skip"));

    let art = codelore_lib::calibration::load(&out).expect("artifact parses");
    assert_eq!(art.repos_attempted, 2);
    assert_eq!(art.repos_included, 1);
}

/// A manifest where EVERY repo is unreachable (0-of-N included): `calibrate.rs`
/// hard-errors via `CodeLoreError::Analysis` rather than silently writing a
/// data-free artifact — spec §6.6 exit 4 — and no output file lands at all
/// (the atomic-publish write never runs). Locks in the existing total-failure
/// guard (`calibrate.rs`'s `attempted > 0 && included == 0` check).
#[test]
fn calibrate_all_repos_unreachable_exits_analysis_error() {
    let work = tempfile::tempdir().expect("tempdir");
    let cache = tempfile::tempdir().expect("cache tempdir");

    let missing_one = work.path().join("does-not-exist-1");
    let missing_two = work.path().join("does-not-exist-2");
    let manifest = write_calibrate_manifest(
        work.path(),
        &[
            (missing_one.to_str().unwrap(), "deadbeef"),
            (missing_two.to_str().unwrap(), "deadbeef"),
        ],
    );
    let out = work.path().join("world.calib.json");

    codelore_cmd()
        .args([
            "calibrate",
            "--repos",
            manifest.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("skip"))
        .stderr(predicate::str::contains(
            "failed to fetch or ingest — no calibration data pooled",
        ));

    assert!(
        !out.exists(),
        "a total-failure run must not write an artifact file"
    );
}

/// `--merge` folds a prior artifact into a fresh build over the same repo, so
/// the pooled rust sample count doubles versus the standalone build.
#[test]
fn calibrate_merge_doubles_sample_counts() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let work = tempfile::tempdir().expect("tempdir");
    let cache = tempfile::tempdir().expect("cache tempdir");

    let manifest = write_calibrate_manifest(
        work.path(),
        &[(tiny.dir.path().to_str().unwrap(), &tiny.head_sha)],
    );

    // Base build.
    let base = work.path().join("base.calib.json");
    codelore_cmd()
        .args([
            "calibrate",
            "--repos",
            manifest.to_str().unwrap(),
            "--output",
            base.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let base_art = codelore_lib::calibration::load(&base).expect("base parses");
    let base_rust = base_art
        .languages
        .iter()
        .find(|l| l.language == "rust")
        .expect("rust in base")
        .sample_functions;

    // Merge the base artifact into a rebuild over the same repo.
    let merged = work.path().join("merged.calib.json");
    codelore_cmd()
        .args([
            "calibrate",
            "--repos",
            manifest.to_str().unwrap(),
            "--output",
            merged.to_str().unwrap(),
            "--merge",
            base.to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let merged_art = codelore_lib::calibration::load(&merged).expect("merged parses");
    let merged_rust = merged_art
        .languages
        .iter()
        .find(|l| l.language == "rust")
        .expect("rust in merged")
        .sample_functions;
    assert_eq!(merged_rust, base_rust * 2, "merge must sum sample counts");
    assert_eq!(merged_art.repos_included, 2, "merge sums repos_included");
}

/// A planted, dated fixture for `calibrate-defects`:
///   A — introduces `src/lib.rs` with a "buggy" line
///   B — unrelated churn in a different file
///   C — `fix: remove buggy line`, deleting A's buggy line (must link to A)
///   D — a comment-only reformat (adds a `//` line)
///   E — `fix: tidy`, deleting D's comment line (must be AG-filtered — the
///       deleted line is cosmetic, so it must yield NO link)
fn defect_calibration_fixture() -> tempfile::TempDir {
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
    };
    let commit_at = |msg: &str, date: &str| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", msg])
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(out.status.success(), "git commit {msg:?}: {out:?}");
    };

    git(&["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();

    // A: introduces src/lib.rs with a "buggy" line (x + 999).
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    let result = x + 999;\n    result\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    commit_at("feat: add compute helper", "2026-01-01T10:00:00Z");

    // B: unrelated churn in a different file.
    std::fs::write(
        repo.join("src/other.rs"),
        "pub fn other() -> i32 {\n    42\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    commit_at(
        "chore: unrelated churn in another module",
        "2026-01-02T10:00:00Z",
    );

    // C: fix removing A's buggy line (x + 999 -> x + 1).
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    let result = x + 1;\n    result\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    commit_at("fix: remove buggy line", "2026-01-03T10:00:00Z");

    // D: comment-only reformat (pure addition).
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    let result = x + 1;\n    \
         // TODO: revisit this computation\n    result\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    commit_at("docs: annotate compute with a TODO", "2026-01-04T10:00:00Z");

    // E: "fix" that only deletes D's cosmetic comment line — must be
    // AG-filtered, yielding no link.
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn compute(x: i32) -> i32 {\n    let result = x + 1;\n    result\n}\n",
    )
    .unwrap();
    git(&["add", "."]);
    commit_at("fix: tidy", "2026-01-05T10:00:00Z");

    dir
}

#[test]
fn calibrate_defects_links_planted_defect_and_ag_filters_cosmetic_fix() {
    let repo = defect_calibration_fixture();
    let out_dir = tempfile::tempdir().unwrap();
    let output = out_dir.path().join("defects.calib.json");

    codelore_cmd()
        .args([
            "calibrate-defects",
            "--repo",
            repo.path().to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let bytes = std::fs::read(&output).expect("artifact written");
    let artifact: serde_json::Value =
        serde_json::from_slice(&bytes).expect("artifact parses as JSON");

    assert_eq!(
        artifact["mining"]["fixes_found"], 2,
        "C and E both classify as fixes"
    );
    assert_eq!(
        artifact["mining"]["links_found"], 1,
        "only C -> A must survive; E's cosmetic candidate must be AG-filtered"
    );
    // The persisted artifact never carries raw (defect, fix, path) triples
    // (DefectArtifact only stores aggregated MiningStats/ValidationMetrics) —
    // the aggregate counts below uniquely pin down the one surviving link as
    // (defect=A, fix=C, path=src/lib.rs), the only pair the fixture can
    // possibly produce.
    assert_eq!(
        artifact["validation"]["linked_defects"], 1,
        "exactly one distinct defect-introducing commit (A) must be linked"
    );
    assert_eq!(
        artifact["validation"]["implicated_files"], 1,
        "exactly one file (src/lib.rs) must be defect-implicated"
    );
    let band_table = artifact["validation"]["band_table"]
        .as_array()
        .expect("band_table present");
    assert_eq!(
        band_table.len(),
        3,
        "band_table always carries red/yellow/green"
    );

    assert_eq!(
        artifact["tuning"]["outcome"], "DefaultsKept",
        "1 linked defect is far below the 30-defect honesty floor"
    );
    assert_eq!(
        artifact["tuning"]["reason"], "fewer than 30 linked defect-changes",
        "the linked-defect-changes floor, not the implicated-file or margin branch, must fire"
    );
}

#[test]
fn cycle_health_csv_has_header() {
    // Build a minimal inline repo with an `a ↔ b` import cycle so
    // `cycle-health` has something to report. The smoke test only checks
    // the CSV header and exit 0; correctness is covered by the lib-level
    // cycle_health_test integration tests.
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
            .env("GIT_AUTHOR_DATE", "2026-06-01T10:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-06-01T10:00:00Z")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname=\"cyc\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub mod a;\npub mod b;\n").unwrap();
    std::fs::write(
        repo.join("src/a.rs"),
        "use crate::b;\npub fn a() { b::b(); }\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("src/b.rs"),
        "use crate::a;\npub fn b() { a::a(); }\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);

    codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "cycle-health",
            "--repo",
            repo.to_str().unwrap(),
            "--format",
            "csv",
            "--min-revs",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "cycle-id,size,members,heat-pct,verdict,extract-candidate,predicted-pc-drop",
        ));
}

/// End-to-end coverage for `explain <path>` — the deterministic per-file
/// evidence dossier and its opt-in `--llm` advisory narrative. The dossier
/// branch needs no network; the `--llm` cases point the client at a
/// test-local one-shot HTTP server so nothing touches an external endpoint.
mod explain_path {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::thread;

    use crate::codelore_cmd;
    use predicates::prelude::*;

    /// The LLM environment variables the dossier surface reads. Cleared on every
    /// spawned CLI so an ambient developer configuration can never leak into a
    /// test's resolution. Shared with the `diff --llm` tests.
    pub(crate) const LLM_ENV_VARS: &[&str] = &[
        "CODELORE_LLM_PROVIDER",
        "CODELORE_LLM_BASE_URL",
        "CODELORE_LLM_API_KEY",
        "CODELORE_LLM_MODEL",
        "ANTHROPIC_API_KEY",
    ];

    /// Run `analyze code-health` (with `--min-revs 1`, matching the dossier
    /// branch) over the fixture and return the worst-scoring file's repo-relative
    /// path and code-health band. Deriving the target from the same engine the
    /// dossier uses keeps the assertions robust to fixture regeneration.
    fn code_health_worst_row(repo: &Path, cache: &Path) -> (String, String) {
        let out = codelore_cmd()
            .args([
                "analyze",
                "--analysis",
                "code-health",
                "--repo",
                repo.to_str().unwrap(),
                "--cache-dir",
                cache.to_str().unwrap(),
                "--min-revs",
                "1",
                "--format",
                "csv",
            ])
            .output()
            .expect("run code-health");
        assert!(
            out.status.success(),
            "code-health failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 code-health output");
        let row = stdout
            .lines()
            .nth(1)
            .expect("code-health yields at least one row for the fixture");
        let fields: Vec<&str> = row.split(',').collect();
        // Header: entity,cognitive,score,structural_risk,percentile,band,corpus-pct
        (fields[0].to_string(), fields[5].to_string())
    }

    /// Every code-health entity path (one row per file), in engine order, for
    /// tests that need two distinct files from the fixture.
    fn code_health_entity_paths(repo: &Path, cache: &Path) -> Vec<String> {
        let out = codelore_cmd()
            .args([
                "analyze",
                "--analysis",
                "code-health",
                "--repo",
                repo.to_str().unwrap(),
                "--cache-dir",
                cache.to_str().unwrap(),
                "--min-revs",
                "1",
                "--format",
                "csv",
            ])
            .output()
            .expect("run code-health");
        assert!(
            out.status.success(),
            "code-health failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 code-health output");
        stdout
            .lines()
            .skip(1)
            .filter_map(|line| line.split(',').next())
            .map(str::to_string)
            .collect()
    }

    /// Spawn a one-shot HTTP server on an ephemeral localhost port that answers a
    /// single OpenAI-compatible `/chat/completions` request with `narrative` as
    /// the assistant message, then exits. Returns the bound base URL. `narrative`
    /// must be free of `"`, `\`, and newlines so it embeds directly in the JSON.
    pub(crate) fn serve_one_completion(narrative: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let base = format!("http://{}", listener.local_addr().expect("local addr"));
        let body = format!(
            "{{\"choices\":[{{\"message\":{{\"role\":\"assistant\",\"content\":\"{narrative}\"}}}}]}}"
        );
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            drain_request(&mut stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
            stream.flush().ok();
        });
        base
    }

    /// Read the request up to the end of its headers, then consume any declared
    /// body, so the client's `POST` fully completes before we reply.
    fn drain_request(stream: &mut TcpStream) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let n = stream.read(&mut chunk).expect("read request headers");
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let content_length = head
            .split("\r\n")
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut body_read = buf.len() - header_end;
        while body_read < content_length {
            let n = stream.read(&mut chunk).expect("read request body");
            if n == 0 {
                break;
            }
            body_read += n;
        }
    }

    #[test]
    fn explain_known_topic_still_prints_topic_text() {
        // Contract 1: a known topic is looked up first and prints byte-for-byte
        // what it always did — the new file-path branch never runs.
        codelore_cmd()
            .args(["explain", "hotspots"])
            .assert()
            .success()
            .stdout(predicate::str::contains("# hotspots"))
            .stdout(predicate::str::contains("**Citation**"))
            .stdout(predicate::str::contains("**Formula**"));
    }

    #[test]
    fn explain_file_prints_dossier_without_network() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let (target, band) = code_health_worst_row(fx.dir.path(), cache.path());

        let mut cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        cmd.args([
            "explain",
            &target,
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("code-health"))
        .stdout(predicate::str::contains(band.as_str()))
        .stdout(predicate::str::contains(target.as_str()));
    }

    /// A syntactically valid defect-calibration artifact with a deliberately
    /// foreign `repo_identity` — proves `--allow-foreign-calibration` is what
    /// lets it apply, not merely that the flag parses. `weights` are the
    /// built-in smell defaults in canonical order, matching what
    /// `active_weights` (consulted by the dossier's code-health section)
    /// requires of a well-formed artifact.
    fn write_foreign_defect_artifact(dir: &Path) -> std::path::PathBuf {
        use codelore_lib::defect_calibration::{
            DEFECT_FORMAT_VERSION, DefectArtifact, MiningStats, OracleConfig, TuningDecision,
            ValidationMetrics, save, validate::default_weights,
        };
        let artifact = DefectArtifact {
            format_version: DEFECT_FORMAT_VERSION,
            repo_identity: "0".repeat(64),
            head_at_mining: "0".repeat(40),
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
            weights: default_weights(),
            tuning: TuningDecision::DefaultsKept {
                reason: "insufficient evidence for weight tuning".to_string(),
                auc_validation_default: None,
                auc_validation_tuned: None,
            },
        };
        let path = dir.join("defects.calib.json");
        save(&artifact, &path).expect("save artifact");
        path
    }

    #[test]
    fn explain_file_defect_calibration_adds_defect_evidence_section() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let (target, _band) = code_health_worst_row(fx.dir.path(), cache.path());
        let artifact_dir = tempfile::tempdir().expect("artifact dir");
        let artifact_path = write_foreign_defect_artifact(artifact_dir.path());

        codelore_cmd()
            .args([
                "explain",
                &target,
                "--repo",
                fx.dir.path().to_str().unwrap(),
                "--cache-dir",
                cache.path().to_str().unwrap(),
                "--defect-calibration",
                artifact_path.to_str().unwrap(),
                "--allow-foreign-calibration",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("defect-evidence"))
            .stdout(predicate::str::contains("defects-2026-07-17"));
    }

    #[test]
    fn explain_file_without_defect_calibration_has_no_defect_evidence_section() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let (target, _band) = code_health_worst_row(fx.dir.path(), cache.path());

        codelore_cmd()
            .args([
                "explain",
                &target,
                "--repo",
                fx.dir.path().to_str().unwrap(),
                "--cache-dir",
                cache.path().to_str().unwrap(),
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("defect-evidence").not());
    }

    #[test]
    fn explain_file_bad_defect_calibration_path_errors_naming_the_path() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let (target, _band) = code_health_worst_row(fx.dir.path(), cache.path());
        let bad_path = cache.path().join("does-not-exist.calib.json");

        codelore_cmd()
            .args([
                "explain",
                &target,
                "--repo",
                fx.dir.path().to_str().unwrap(),
                "--cache-dir",
                cache.path().to_str().unwrap(),
                "--defect-calibration",
                bad_path.to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(bad_path.to_str().unwrap()));
    }

    #[test]
    fn explain_unknown_arg_errors_naming_topics_and_files() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        codelore_cmd()
            .args([
                "explain",
                "definitely-not-a-topic-or-file",
                "--repo",
                fx.dir.path().to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("topic"))
            .stderr(predicate::str::contains("file"));
    }

    #[test]
    fn explain_file_llm_prints_narrative_and_stamp() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let (target, _band) = code_health_worst_row(fx.dir.path(), cache.path());

        let narrative = "Diagnosis: the evidence indicates this file is structurally healthy.";
        let base = serve_one_completion(narrative);

        let mut cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        // Force the OpenAI-compatible dialect at the local test server so
        // resolution is deterministic regardless of the developer's environment.
        cmd.env("CODELORE_LLM_PROVIDER", "openai-compat")
            .env("CODELORE_LLM_BASE_URL", &base)
            .env("CODELORE_LLM_MODEL", "test-model")
            .args([
                "explain",
                &target,
                "--llm",
                "--repo",
                fx.dir.path().to_str().unwrap(),
                "--cache-dir",
                cache.path().to_str().unwrap(),
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains(narrative))
            .stdout(predicate::str::contains("advisory — model"))
            .stdout(predicate::str::contains("test-model"));
    }

    #[test]
    fn explain_file_llm_without_config_errors_naming_setup_vars() {
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let (target, _band) = code_health_worst_row(fx.dir.path(), cache.path());

        let mut cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        cmd.args([
            "explain",
            &target,
            "--llm",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("CODELORE_LLM_MODEL"));
    }

    #[test]
    fn explain_file_staleness_note_is_scoped_to_the_explained_file() {
        // Regression: narrating file B must not make explaining a never-narrated
        // file A print a staleness note. The note is scoped to A's own subject,
        // and A has no narrative of its own, so no note may appear.
        let fx = codelore_lib::test_support::biomarker_repo::build();
        let cache = tempfile::tempdir().expect("cache dir");
        let paths = code_health_entity_paths(fx.dir.path(), cache.path());
        let file_b = &paths[0];
        let file_a = paths
            .iter()
            .find(|p| *p != file_b)
            .expect("fixture yields at least two distinct files");

        // Narrate file B through the local test server so a narrative is cached
        // for B's subject in this cache root.
        let base = serve_one_completion("Diagnosis: file B looks structurally healthy.");
        let mut narrate_b = codelore_cmd();
        for var in LLM_ENV_VARS {
            narrate_b.env_remove(var);
        }
        narrate_b
            .env("CODELORE_LLM_PROVIDER", "openai-compat")
            .env("CODELORE_LLM_BASE_URL", &base)
            .env("CODELORE_LLM_MODEL", "test-model")
            .args([
                "explain",
                file_b,
                "--llm",
                "--repo",
                fx.dir.path().to_str().unwrap(),
                "--cache-dir",
                cache.path().to_str().unwrap(),
            ])
            .assert()
            .success();

        // Explain file A without --llm over the same cache root: it has no
        // narrative of its own, so the staleness note must not appear.
        let mut explain_a = codelore_cmd();
        for var in LLM_ENV_VARS {
            explain_a.env_remove(var);
        }
        explain_a
            .args([
                "explain",
                file_a,
                "--repo",
                fx.dir.path().to_str().unwrap(),
                "--cache-dir",
                cache.path().to_str().unwrap(),
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("stale").not());
    }
}

/// End-to-end coverage for `diff --llm` — the opt-in, degrade-gracefully
/// advisory PR narrative. The deterministic diff output, its gate verdict, and
/// its exit code must be identical with or without the flag; the narrative is
/// appended only as a delimited advisory block. The `--llm` cases point the
/// client at the same test-local one-shot HTTP server the `explain` tests use so
/// nothing touches an external endpoint.
mod diff_llm {
    use crate::codelore_cmd;

    use crate::explain_path::{LLM_ENV_VARS, serve_one_completion};

    #[test]
    fn diff_without_llm_has_no_advisory_block() {
        let (dir, base, head) = super::delta_health_fixture();
        let mut cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        let out = cmd
            .args([
                "diff",
                "--repo",
                dir.path().to_str().unwrap(),
                "--min-revs",
                "1",
                "--format",
                "text",
                &format!("{base}..{head}"),
            ])
            .output()
            .expect("run diff without --llm");
        assert!(
            out.status.success(),
            "diff without --llm should succeed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 diff output");
        assert!(
            !stdout.contains("LLM narrative"),
            "no advisory block without --llm: {stdout}"
        );
    }

    #[test]
    fn diff_llm_appends_advisory_block_and_preserves_exit_code() {
        let (dir, base, head) = super::delta_health_fixture();
        let repo = dir.path().to_str().unwrap().to_string();
        let range = format!("{base}..{head}");

        // Baseline: the no-flag run establishes the exit code the --llm run must
        // reproduce (the narrative is advisory and must not move it).
        let mut baseline_cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            baseline_cmd.env_remove(var);
        }
        let baseline = baseline_cmd
            .args([
                "diff",
                "--repo",
                &repo,
                "--min-revs",
                "1",
                "--format",
                "text",
                &range,
            ])
            .output()
            .expect("baseline diff");
        assert!(baseline.status.success());

        let narrative = "This change adds a large branchy function that degrades change health.";
        let base_url = serve_one_completion(narrative);

        // --llm-refresh forces the server round-trip: diff has no --cache-dir, so
        // it shares the default narrative cache; refreshing keeps the assertion
        // hermetic against any pre-existing cached narrative for this fact sheet.
        let mut cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        let out = cmd
            .env("CODELORE_LLM_PROVIDER", "openai-compat")
            .env("CODELORE_LLM_BASE_URL", &base_url)
            .env("CODELORE_LLM_MODEL", "test-model")
            .args([
                "diff",
                "--repo",
                &repo,
                "--min-revs",
                "1",
                "--format",
                "text",
                "--llm",
                "--llm-refresh",
                &range,
            ])
            .output()
            .expect("run diff --llm");
        assert!(
            out.status.success(),
            "diff --llm should succeed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.status.code(),
            baseline.status.code(),
            "the advisory narrative must not change the exit code"
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 diff output");
        assert!(
            stdout.contains("LLM narrative (advisory)"),
            "advisory block present: {stdout}"
        );
        assert!(
            stdout.contains(narrative),
            "the served narrative is rendered: {stdout}"
        );
        assert!(
            stdout.contains("advisory — model"),
            "the citation-check stamp is rendered: {stdout}"
        );
        assert!(
            stdout.contains("test-model"),
            "the stamp names the model: {stdout}"
        );
    }

    #[test]
    fn diff_llm_without_config_warns_and_leaves_output_identical() {
        let (dir, base, head) = super::delta_health_fixture();
        let repo = dir.path().to_str().unwrap().to_string();
        let range = format!("{base}..{head}");

        let mut baseline_cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            baseline_cmd.env_remove(var);
        }
        let baseline = baseline_cmd
            .args([
                "diff",
                "--repo",
                &repo,
                "--min-revs",
                "1",
                "--format",
                "text",
                &range,
            ])
            .output()
            .expect("baseline diff");
        assert!(baseline.status.success());

        // --llm with no LLM environment: resolution fails, the failure is a
        // stderr warning, and stdout + exit code are byte-identical to the
        // no-flag run.
        let mut cmd = codelore_cmd();
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        let out = cmd
            .args([
                "diff",
                "--repo",
                &repo,
                "--min-revs",
                "1",
                "--format",
                "text",
                "--llm",
                &range,
            ])
            .output()
            .expect("run diff --llm without config");
        assert_eq!(
            out.status.code(),
            baseline.status.code(),
            "an unavailable narrative must not change the exit code"
        );
        assert_eq!(
            out.stdout, baseline.stdout,
            "an unavailable narrative must leave stdout identical to the no-flag run"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("llm narrative unavailable"),
            "the degrade-gracefully warning is on stderr: {stderr}"
        );
    }
}

/// Scope guard for the advisory `--llm` flag: it exists only on the surfaces
/// that render narratives (`explain`, `diff`). The scored surfaces (`analyze`,
/// `check`) must reject it at the parser, so the flag can never even be spelled
/// on a command whose output feeds gates or CI.
mod llm_flag_scope {
    use crate::codelore_cmd;
    use predicates::prelude::*;

    #[test]
    fn analyze_rejects_the_llm_flag_at_the_parser() {
        codelore_cmd()
            .args(["analyze", "--analysis", "hotspots", "--llm", "--repo", "."])
            .assert()
            .failure()
            .stderr(predicate::str::contains("unexpected argument"))
            .stderr(predicate::str::contains("--llm"));
    }

    #[test]
    fn check_rejects_the_llm_flag_at_the_parser() {
        codelore_cmd()
            .args(["check", "--repo", ".", "--llm"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("unexpected argument"))
            .stderr(predicate::str::contains("--llm"));
    }
}

/// Manual-only live check against a local ollama. Run with:
///
/// ```text
/// CODELORE_LLM_MODEL=<model from `ollama list`> \
///   cargo test -p codelore --test cli_test -- --ignored explain_file_llm_live
/// ```
///
/// Ignored by default: CI performs no live network calls, and the assertion
/// depends on a developer-local model server at the default base URL.
#[test]
#[ignore = "requires a running local ollama and CODELORE_LLM_MODEL set"]
fn explain_file_llm_live_against_local_ollama() {
    let model = std::env::var("CODELORE_LLM_MODEL")
        .expect("set CODELORE_LLM_MODEL to a model name from `ollama list` for the live check");
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let cache = tempfile::tempdir().expect("cache dir");

    // Resolve a real dossier target the same way the hermetic explain tests do.
    let out = codelore_cmd()
        .args([
            "analyze",
            "--analysis",
            "code-health",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "csv",
        ])
        .output()
        .expect("run code-health");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8 code-health output");
    let target = stdout
        .lines()
        .nth(1)
        .and_then(|row| row.split(',').next())
        .expect("code-health yields at least one row")
        .to_string();

    let mut cmd = codelore_cmd();
    for var in explain_path::LLM_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd.env("CODELORE_LLM_PROVIDER", "openai-compat")
        .env("CODELORE_LLM_MODEL", &model)
        .args([
            "explain",
            &target,
            "--llm",
            "--llm-refresh",
            "--repo",
            fx.dir.path().to_str().unwrap(),
            "--cache-dir",
            cache.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("advisory — model"))
        .stdout(predicate::str::contains(model.as_str()));
}
