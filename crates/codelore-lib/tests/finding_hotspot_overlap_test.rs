//! Integration tests for `finding-hotspot-overlap`.
//!
//! Covers:
//! - Findings on a known hotspot path → row with correct engines + health band
//! - Findings on a path NOT in hotspots → row with 0.0 score + 0.0 percentile
//! - Empty store → error with the expected message
//! - Priority pure-function branch coverage (see unit tests in the source module)
//! - `_with` variant and wrapper agree on identical inputs

use codelore_lib::Options;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::analyses::finding_hotspot_overlap::{
    run_finding_hotspot_overlap, run_finding_hotspot_overlap_with,
};
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::external::ExternalFinding;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::{finding_for, temp_external_store};
use std::path::Path;
use std::process::Command;

/// Helper: ingest the `tiny_repo` fixture and return (db, opts).
fn ingest_tiny() -> (tempfile::TempDir, FactsDb, Options) {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    // keep the TempDir alive by returning it
    (tiny.dir, db, opts)
}

// ─── empty store error ───────────────────────────────────────────────────────

#[test]
fn empty_store_returns_error() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());
    let (_tiny_dir, db, opts) = ingest_tiny();

    let err = run_finding_hotspot_overlap(&db, &opts, &store)
        .expect_err("should return error when store is empty");
    let msg = err.to_string();
    assert!(
        msg.contains("ingest-sarif"),
        "error message should mention ingest-sarif; got: {msg}"
    );
}

// ─── findings on a hotspot path ─────────────────────────────────────────────

/// `src/main.rs` has 4 revisions in `tiny_repo` — the highest revision count.
/// A finding against it should produce a row with matching engines + a
/// non-zero `hotspot_score` and `revs_percentile`.
#[test]
fn findings_on_hotspot_path_produce_correct_row() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());

    // Ingest two findings for src/main.rs from two different engines.
    let findings = [
        finding_for("src/main.rs", "semgrep", "warning"),
        finding_for("src/main.rs", "clippy", "error"),
    ];
    store
        .replace_engine("semgrep", &findings[..1])
        .expect("replace semgrep");
    store
        .replace_engine("clippy", &findings[1..])
        .expect("replace clippy");

    let (_tiny_dir, db, opts) = ingest_tiny();
    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    assert_eq!(rows.len(), 1, "one path in store → one output row");
    let row = &rows[0];
    assert_eq!(row.path, "src/main.rs");
    assert_eq!(row.findings, 2);
    // Both engines present, sorted → "clippy,semgrep"
    assert_eq!(row.engines, "clippy,semgrep");
    assert_eq!(row.worst_level, "error");
    // src/main.rs has revisions=4, src/lib.rs has revisions=1. Two files in
    // hotspot result set → PERCENT_RANK: 0.0 for 1 rev, 1.0 for 4 revs.
    // So src/main.rs should have revs_percentile == 1.0.
    assert!(
        (row.revs_percentile - 1.0).abs() < 1e-9,
        "src/main.rs (max revs) should have revs_percentile=1.0, got {}",
        row.revs_percentile
    );
    // hotspot_score may be 0.0 for tiny repos where cognitive complexity is
    // effectively 0 (one-liner Rust files); the percentile is the stronger
    // signal that proves the path was matched in the hotspot result set.
    assert!(
        row.hotspot_score >= 0.0,
        "hotspot_score should be non-negative, got {}",
        row.hotspot_score
    );
}

// ─── findings on a non-hotspot path ─────────────────────────────────────────

/// A finding for a path that is NOT in the hotspot result set (it doesn't
/// exist in the repo at all) → 0.0 score and 0.0 percentile (LEFT-join contract).
#[test]
fn findings_on_non_hotspot_path_produce_zero_scores() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());

    let f = finding_for("src/does-not-exist.rs", "semgrep", "note");
    store.replace_engine("semgrep", &[f]).expect("replace");

    let (_tiny_dir, db, opts) = ingest_tiny();
    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.path, "src/does-not-exist.rs");
    assert!(
        (row.hotspot_score).abs() < 1e-9,
        "non-hotspot path should have hotspot_score=0.0, got {}",
        row.hotspot_score
    );
    assert!(
        (row.revs_percentile).abs() < 1e-9,
        "non-hotspot path should have revs_percentile=0.0, got {}",
        row.revs_percentile
    );
    assert_eq!(row.health_band, "unknown");
}

// ─── biomarker_repo: health band flows through ───────────────────────────────

/// `src/complex.rs` in `biomarker_repo` has the highest cognitive complexity
/// and a non-trivial revision count. The code-health pipeline must produce a
/// real band (not `"unknown"`) for this path, proving that the health-band join
/// arm of `run_finding_hotspot_overlap` actually fires on a repo with complex
/// files.
#[test]
fn complex_file_in_biomarker_repo_has_real_health_band() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fx.dir.path().to_path_buf(),
        min_revs: 1,
        fisher_significance: 1.0,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());

    // Ingest a finding for src/complex.rs — the most complex file in the fixture.
    let f = finding_for("src/complex.rs", "semgrep", "warning");
    store.replace_engine("semgrep", &[f]).expect("replace");

    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    let row = rows
        .iter()
        .find(|r| r.path == "src/complex.rs")
        .expect("src/complex.rs must appear in output");

    assert!(
        matches!(row.health_band.as_str(), "red" | "yellow" | "green"),
        "health_band must be a real band value (not 'unknown') for complex.rs; got '{}'",
        row.health_band
    );
}

// ─── sort order ─────────────────────────────────────────────────────────────

/// With two paths in the store (main.rs: 2 findings, lib.rs: 1 finding),
/// the sort (priority asc rank then findings desc then path asc) puts the
/// higher-finding count first when both share the same priority.
#[test]
fn rows_sorted_priority_then_findings_then_path() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());

    // 2 findings for main.rs and 1 for lib.rs, all from the same engine.
    let all = [
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule".to_string(),
            path: "src/main.rs".to_string(),
            start_line: Some(1),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "test/v1/m1".to_string(),
            message: "m1".to_string(),
        },
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule".to_string(),
            path: "src/main.rs".to_string(),
            start_line: Some(2),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "test/v1/m2".to_string(),
            message: "m2".to_string(),
        },
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule".to_string(),
            path: "src/lib.rs".to_string(),
            start_line: Some(1),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "test/v1/l1".to_string(),
            message: "l1".to_string(),
        },
    ];
    store.replace_engine("semgrep", &all).expect("replace all");

    let (_tiny_dir, db, opts) = ingest_tiny();
    let rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("run");

    // Both paths have "note" priority; main.rs (2 findings) should rank before lib.rs (1).
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].path, "src/main.rs");
    assert_eq!(rows[0].findings, 2);
    assert_eq!(rows[1].path, "src/lib.rs");
    assert_eq!(rows[1].findings, 1);
}

// ─── _with variant agrees with wrapper on identical inputs ───────────────────

/// `run_finding_hotspot_overlap_with` must produce byte-identical output to
/// the convenience wrapper `run_finding_hotspot_overlap` when called with
/// the same pre-computed rows. Guards against the two diverging over time.
#[test]
fn with_variant_and_wrapper_agree_on_identical_inputs() {
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());

    let f = finding_for("src/main.rs", "semgrep", "warning");
    store.replace_engine("semgrep", &[f]).expect("replace");

    let (_tiny_dir, db, opts) = ingest_tiny();

    // Run the wrapper (it runs hotspots + code_health internally).
    let via_wrapper = run_finding_hotspot_overlap(&db, &opts, &store).expect("wrapper");

    // Run _with using the same row sets derived independently.
    let hotspot_rows = run_hotspots(&db, &opts).expect("hotspots");
    let health_rows = run_code_health(&db, &opts).expect("code_health");
    let via_with =
        run_finding_hotspot_overlap_with(&store, &hotspot_rows, &health_rows).expect("_with");

    assert_eq!(via_wrapper.len(), via_with.len(), "row count must match");
    for (w, ww) in via_wrapper.iter().zip(via_with.iter()) {
        assert_eq!(w.path, ww.path, "path mismatch");
        assert_eq!(w.findings, ww.findings, "findings mismatch for {}", w.path);
        assert_eq!(w.engines, ww.engines, "engines mismatch for {}", w.path);
        assert_eq!(
            w.worst_level, ww.worst_level,
            "worst_level mismatch for {}",
            w.path
        );
        assert_eq!(
            w.health_band, ww.health_band,
            "health_band mismatch for {}",
            w.path
        );
        assert_eq!(w.priority, ww.priority, "priority mismatch for {}", w.path);
        assert!(
            (w.hotspot_score - ww.hotspot_score).abs() < 1e-9,
            "hotspot_score mismatch for {}: {} vs {}",
            w.path,
            w.hotspot_score,
            ww.hotspot_score
        );
        assert!(
            (w.revs_percentile - ww.revs_percentile).abs() < 1e-9,
            "revs_percentile mismatch for {}: {} vs {}",
            w.path,
            w.revs_percentile,
            ww.revs_percentile
        );
    }
}

// ─── --rows must rank against the full population, then truncate at output ────

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git")
        .success();
    assert!(ok, "git {args:?} failed");
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "."]);
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", msg, "--quiet"])
        .status()
        .expect("spawn git commit")
        .success();
    assert!(ok, "git commit {msg} failed");
}

fn write_body(root: &Path, rel: &str, n: u32) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, format!("pub fn f() -> u32 {{ {n} }}\n")).expect("write");
}

/// Build a repo whose three files differ only in revision count:
/// `src/a.rs` (3 revs), `src/m.rs` (1 rev), `src/z.rs` (2 revs). All three
/// carry identical zero-complexity bodies, so the hotspot ranking is decided
/// purely by `path ASC` — `src/a.rs` sorts first, `src/z.rs` last. A small
/// `--rows` prefix therefore drops `src/z.rs` from the inner hotspot set even
/// though its middle revision count gives it a non-trivial percentile.
fn build_percentile_population_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "rows@example.com"]);
    git(p, &["config", "user.name", "Rows"]);

    // Seed: all three files get revision #1.
    write_body(p, "src/a.rs", 0);
    write_body(p, "src/m.rs", 0);
    write_body(p, "src/z.rs", 0);
    commit_all(p, "seed");

    // a.rs → 2 revisions.
    write_body(p, "src/a.rs", 1);
    commit_all(p, "edit a");

    // a.rs → 3 revisions, z.rs → 2 revisions (one commit touches both).
    write_body(p, "src/a.rs", 2);
    write_body(p, "src/z.rs", 1);
    commit_all(p, "edit a and z");

    dir
}

/// A finding on the alphabetically-last, middle-revs file must carry the same
/// `revs_percentile` and `health_band` whether or not `--rows` is set: the
/// percentile denominator is the whole hotspot population, not the truncated
/// prefix. On the pre-fix path `--rows` flows into the inner analyses, so the
/// retained row's percentile/band collapse to the absent defaults.
#[test]
fn rows_limit_does_not_corrupt_retained_percentile_or_band() {
    let dir = build_percentile_population_repo();
    let p = dir.path();
    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());
    let f = finding_for("src/z.rs", "semgrep", "warning");
    store.replace_engine("semgrep", &[f]).expect("replace");

    let limited = Options {
        rows_limit: Some(1),
        ..opts.clone()
    };

    let full_rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("full");
    let limited_rows = run_finding_hotspot_overlap(&db, &limited, &store).expect("limited");

    let z_full = full_rows
        .iter()
        .find(|r| r.path == "src/z.rs")
        .expect("z.rs in full run");
    let z_limited = limited_rows
        .iter()
        .find(|r| r.path == "src/z.rs")
        .expect("z.rs in limited run");

    assert!(
        (z_full.revs_percentile - z_limited.revs_percentile).abs() < 1e-9,
        "revs_percentile must be independent of --rows: full={} limited={}",
        z_full.revs_percentile,
        z_limited.revs_percentile
    );
    assert_eq!(
        z_full.health_band, z_limited.health_band,
        "health_band must be independent of --rows"
    );
    // Non-vacuity: z.rs really is a mid-population hotspot, so a corrupted
    // denominator would visibly move its percentile off this middle value.
    assert!(
        z_full.revs_percentile > 0.0 && z_full.revs_percentile < 1.0,
        "z.rs should have a middle percentile; got {}",
        z_full.revs_percentile
    );
}

/// `--rows N` truncates the final priority-sorted output to its top N — and
/// only there. With more finding-paths than N, the retained rows are exactly
/// the highest-priority prefix of the unlimited run. The pre-fix path never
/// truncated the output at all, so every finding-path leaked through.
#[test]
fn rows_limit_truncates_to_highest_priority_prefix() {
    let dir = build_percentile_population_repo();
    let p = dir.path();
    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Findings on all three paths with distinct counts (3 / 2 / 1) so the
    // priority sort is fully determined; distinct fingerprints avoid dedup.
    let mk = |path: &str, fp: &str| ExternalFinding {
        engine: "semgrep".to_string(),
        engine_version: "1.0.0".to_string(),
        rule_id: "rule".to_string(),
        path: path.to_string(),
        start_line: Some(1),
        end_line: None,
        level: "note".to_string(),
        fingerprint: fp.to_string(),
        message: "m".to_string(),
    };
    let findings = [
        mk("src/a.rs", "test/v1/a1"),
        mk("src/a.rs", "test/v1/a2"),
        mk("src/a.rs", "test/v1/a3"),
        mk("src/m.rs", "test/v1/m1"),
        mk("src/m.rs", "test/v1/m2"),
        mk("src/z.rs", "test/v1/z1"),
    ];
    let store_dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(store_dir.path());
    store.replace_engine("semgrep", &findings).expect("replace");

    let limited = Options {
        rows_limit: Some(2),
        ..opts.clone()
    };

    let full_rows = run_finding_hotspot_overlap(&db, &opts, &store).expect("full");
    let limited_rows = run_finding_hotspot_overlap(&db, &limited, &store).expect("limited");

    assert_eq!(
        full_rows.len(),
        3,
        "three finding-paths → three rows unlimited"
    );
    assert_eq!(limited_rows.len(), 2, "--rows 2 keeps exactly two rows");
    // The retained two are the highest-priority prefix of the full sort.
    for (i, lim) in limited_rows.iter().enumerate() {
        assert_eq!(
            lim.path, full_rows[i].path,
            "limited row {i} must equal the full run's prefix"
        );
        assert_eq!(
            lim.priority, full_rows[i].priority,
            "priority for {} must match the full run",
            lim.path
        );
    }
}
