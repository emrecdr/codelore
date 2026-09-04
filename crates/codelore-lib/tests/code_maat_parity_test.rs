//! Differential parity tests: `CodeLore`'s output must match `code-maat`'s for
//! the analyses that are spec'd to be byte-equivalent. See `fixtures/golden/
//! code-maat/README.md` for the parity matrix (which analyses match, which
//! deliberately diverge, and why).
//!
//! These tests shell out to `lein run` against a checkout of code-maat. They
//! are gated by env vars and `#[ignore]`d by default so CI doesn't break when
//! Leiningen + code-maat aren't available:
//!
//! ```bash
//! # one-time setup
//! brew install leiningen
//! git clone --depth=1 https://github.com/adamtornhill/code-maat /tmp/code-maat
//! (cd /tmp/code-maat && lein deps)
//!
//! # run the parity suite
//! CODE_MAAT_PATH=/tmp/code-maat \
//!     cargo test -p codelore-lib --test code_maat_parity_test --all-features -- --ignored
//! ```

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use codelore_lib::Options;
use codelore_lib::analyses::revisions::run_revisions;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn code_maat_path() -> Option<String> {
    std::env::var("CODE_MAAT_PATH")
        .ok()
        .filter(|p| Path::new(p).join("project.clj").exists())
}

/// Run `git log` in `repo_path` to produce a code-maat `git2`-format log.
fn dump_git2_log(repo_path: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args([
            "log",
            "--pretty=format:--%h--%ad--%aN",
            "--date=short",
            "--numstat",
            "--no-renames",
        ])
        .output()
        .expect("git log");
    assert!(output.status.success(), "git log failed");
    String::from_utf8(output.stdout).expect("git log utf-8")
}

/// Run code-maat against `log_path` for `analysis`, return its CSV stdout.
fn run_code_maat(code_maat: &str, log_path: &Path, analysis: &str) -> String {
    let output = Command::new("lein")
        .arg("run")
        .args([
            "--",
            "-l",
            log_path.to_str().unwrap(),
            "-c",
            "git2",
            "-a",
            analysis,
        ])
        .current_dir(code_maat)
        .output()
        .expect("lein run");
    assert!(
        output.status.success(),
        "code-maat {analysis} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("code-maat utf-8")
}

/// Normalize CSV output for comparison: drop header (first line), sort
/// remaining rows, trim trailing whitespace per row. Returns a `BTreeSet`
/// so set-equality semantics are explicit.
fn normalize(csv: &str) -> BTreeSet<String> {
    csv.lines()
        .skip(1)
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
#[ignore = "needs CODE_MAAT_PATH env var pointing at a code-maat checkout"]
fn revisions_matches_code_maat_on_tiny_repo() {
    let Some(code_maat) = code_maat_path() else {
        eprintln!("CODE_MAAT_PATH unset or invalid — skipping");
        return;
    };

    let tiny = codelore_lib::test_support::tiny_repo::build();
    let log_path = tiny.dir.path().join("tiny.log");
    std::fs::write(&log_path, dump_git2_log(tiny.dir.path())).expect("write log");

    let cmaat_csv = run_code_maat(&code_maat, &log_path, "revisions");

    // CodeLore equivalent
    let repo = GixRepo::open(tiny.dir.path()).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_revisions(&db, &opts).expect("revisions");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_revisions_csv(&rows, &mut buf).expect("csv");
    let codelore_csv = String::from_utf8(buf).expect("utf-8");

    let cmaat = normalize(&cmaat_csv);
    let codelore = normalize(&codelore_csv);
    assert_eq!(
        codelore, cmaat,
        "revisions CSV diverged.\ncode-maat:\n{cmaat_csv}\ncodelore:\n{codelore_csv}"
    );
}

#[test]
#[ignore = "needs CODE_MAAT_PATH env var pointing at a code-maat checkout"]
fn summary_matches_code_maat_on_tiny_repo() {
    let Some(code_maat) = code_maat_path() else {
        eprintln!("CODE_MAAT_PATH unset or invalid — skipping");
        return;
    };

    let tiny = codelore_lib::test_support::tiny_repo::build();
    let log_path = tiny.dir.path().join("tiny.log");
    std::fs::write(&log_path, dump_git2_log(tiny.dir.path())).expect("write log");

    let cmaat_csv = run_code_maat(&code_maat, &log_path, "summary");

    let repo = GixRepo::open(tiny.dir.path()).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = codelore_lib::analyses::summary::run_summary(&db, &opts).expect("summary");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_summary_csv(&rows, &mut buf, false).expect("csv");
    let codelore_csv = String::from_utf8(buf).expect("utf-8");

    // code-maat summary emits `statistic,value`; codelore emits `metric,value`.
    // The metric *names* differ slightly but the underlying counts must match.
    // Build a lookup map and compare just the counts that are 1:1.
    let cmaat_map = parse_summary_csv(&cmaat_csv);
    let codelore_map = parse_summary_csv(&codelore_csv);

    // Both sides go through `parse_summary_csv`, so a parser break is a SHARED
    // failure: every lookup below would miss, both sides would fall to the same
    // sentinel, and the comparison would report parity having compared nothing.
    // Assert the parse produced rows before trusting anything derived from it.
    assert!(
        !cmaat_map.is_empty() && !codelore_map.is_empty(),
        "parse_summary_csv produced an empty map (code-maat {}, codelore {}) — \
         the parser stopped matching one of the two CSV shapes, which would \
         otherwise make every comparison below vacuously true",
        cmaat_map.len(),
        codelore_map.len()
    );

    // Compare commit count + author count. The entity count deliberately
    // diverges: code-maat is file-level (2 entities for tiny_repo); codelore
    // is function-level (function-scope extraction, so tiny_repo
    // reports 4 entities = 2 files + 2 functions). This is a methodological
    // divergence, not a bug.
    let pairs = [
        ("number-of-commits", "commits"),
        ("number-of-authors", "authors"),
    ];
    for (cmaat_key, codelore_key) in pairs {
        // A missing key must fail as a missing key. Defaulting both sides to a
        // shared sentinel lets an absent row match its opposite number's
        // absence and pass.
        let cmaat_val = cmaat_map.get(cmaat_key).copied().unwrap_or_else(|| {
            panic!("code-maat summary has no `{cmaat_key}` row; parsed keys: {cmaat_map:?}")
        });
        let codelore_val = codelore_map.get(codelore_key).copied().unwrap_or_else(|| {
            panic!("codelore summary has no `{codelore_key}` row; parsed keys: {codelore_map:?}")
        });
        assert_eq!(
            codelore_val, cmaat_val,
            "summary value mismatch for {cmaat_key} / {codelore_key}: code-maat={cmaat_val} codelore={codelore_val}"
        );
    }
}

fn parse_summary_csv(csv: &str) -> std::collections::HashMap<String, i64> {
    csv.lines()
        .skip(1)
        .filter_map(|l| {
            let mut parts = l.splitn(2, ',');
            let k = parts.next()?.trim().to_string();
            let v: i64 = parts.next()?.trim().parse().ok()?;
            Some((k, v))
        })
        .collect()
}
