//! End-to-end coverage for the Martin/Lakos structural metrics
//! (`instability` + `architecture-metrics`) over a fixture with a known
//! shape: `a ↔ b` form a 2-cycle, `c` imports `a`. The graph algorithms
//! are unit-tested in `analyses::import_graph`; this drives the full
//! ingest → metric path and pins the exact numbers.
//!
//! Also covers `architecture-metrics`' corpus-relative percentile rows: the
//! additivity contract (no active `repo_metrics` pool ⇒ byte-identical to
//! the seven base rows), the three extra rows once a synthetic calibration
//! artifact with a `repo_metrics` section is passed via `--calibration`, and
//! the default path where the embedded world artifact's pools activate the
//! rows with no configuration.

use codelore_lib::analyses::architecture_metrics::run_architecture_metrics;
use codelore_lib::analyses::instability::run_instability;
use codelore_lib::calibration::{
    CALIBRATION_FORMAT_VERSION, CalibrationArtifact, RepoMetrics, attach_repo_metrics,
};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Build the `a ↔ b` cycle + `c → a` fixture and ingest it.
fn ingested_cycle_repo() -> (tempfile::TempDir, FactsDb, codelore_lib::Options) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "m@example.com"]);
    git(p, &["config", "user.name", "M"]);
    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"m\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(p, "src/lib.rs", "pub mod a;\npub mod b;\npub mod c;\n");
    write(p, "src/a.rs", "use crate::b;\npub fn a() { b::b(); }\n");
    write(p, "src/b.rs", "use crate::a;\npub fn b() { a::a(); }\n");
    write(p, "src/c.rs", "use crate::a;\npub fn c() { a::a(); }\n");
    git(p, &["add", "."]);
    let status = Command::new("git")
        .arg("-C")
        .arg(p)
        .args(["commit", "-m", "init", "--quiet"])
        .env("GIT_AUTHOR_DATE", "2026-01-01T10:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T10:00:00Z")
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit failed");

    let repo = GixRepo::open(p).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest repo");
    (dir, db, opts)
}

#[test]
fn instability_matches_martin_metrics() {
    let (_dir, db, opts) = ingested_cycle_repo();
    let rows = run_instability(&db, &opts).expect("run instability");
    let by: HashMap<&str, &_> = rows.iter().map(|r| (r.path.as_str(), r)).collect();

    // a: imported by b and c (Ca=2); imports b (Ce=1) → I = 1/3.
    let a = by.get("src/a.rs").expect("a present");
    assert_eq!((a.ca, a.ce), (2, 1), "a: {a:?}");
    assert!((a.instability - 1.0 / 3.0).abs() < 1e-9, "a I=1/3: {a:?}");

    // b: imported by a (Ca=1); imports a (Ce=1) → I = 1/2.
    let b = by.get("src/b.rs").expect("b present");
    assert_eq!((b.ca, b.ce), (1, 1), "b: {b:?}");
    assert!((b.instability - 0.5).abs() < 1e-9, "b I=1/2: {b:?}");

    // c: imported by nothing (Ca=0); imports a (Ce=1) → I = 1 (unstable).
    let c = by.get("src/c.rs").expect("c present");
    assert_eq!((c.ca, c.ce), (0, 1), "c: {c:?}");
    assert!((c.instability - 1.0).abs() < 1e-9, "c I=1: {c:?}");
}

#[test]
fn architecture_metrics_report_the_cycle_and_type() {
    let (_dir, db, opts) = ingested_cycle_repo();
    let rows = run_architecture_metrics(&db, &opts).expect("run architecture-metrics");
    let m: HashMap<&str, &str> = rows
        .iter()
        .map(|r| (r.metric.as_str(), r.value.as_str()))
        .collect();

    assert_eq!(
        m.get("files"),
        Some(&"3"),
        "3 files in the import graph: {m:?}"
    );
    assert_eq!(m.get("dependency_cycles"), Some(&"1"), "one cycle: {m:?}");
    assert_eq!(
        m.get("largest_cycle"),
        Some(&"2"),
        "the a↔b tangle is size 2: {m:?}"
    );
    assert_eq!(
        m.get("architecture_type"),
        Some(&"core-periphery"),
        "one dominant cycle → core-periphery: {m:?}"
    );
    // propagation_cost is present and a parseable ratio in [0, 1].
    let pc: f64 = m
        .get("propagation_cost")
        .expect("propagation_cost present")
        .parse()
        .expect("propagation_cost parses");
    assert!((0.0..=1.0).contains(&pc), "propagation cost in [0,1]: {pc}");
}

// ─── corpus-relative percentile rows ─────────────────────────────────────────

/// Serialize a synthetic [`CalibrationArtifact`] to a temp `.calib.json` file
/// and return its path, so it can be passed via `opts.calibration`. Mirrors
/// `calibration_test.rs`'s `write_temp_json` helper (each integration test
/// binary is compiled separately, so it isn't shared directly).
fn write_temp_calibration_artifact(art: &CalibrationArtifact) -> tempfile::TempPath {
    let mut f = tempfile::Builder::new()
        .prefix("arch-metrics-corpus")
        .suffix(".calib.json")
        .tempfile()
        .expect("create temp artifact");
    let bytes = serde_json::to_vec(art).expect("serialize artifact");
    f.write_all(&bytes).expect("write artifact bytes");
    f.into_temp_path()
}

/// Additivity contract: an artifact that is active but carries no
/// `repo_metrics` section (as every artifact built before the section
/// existed does) must leave the row set byte-identical to the seven base
/// metric names architecture-metrics has always emitted — the
/// corpus-percentile rows stay silent.
#[test]
fn architecture_metrics_additivity_without_repo_metrics_pool() {
    let (_dir, db, mut opts) = ingested_cycle_repo();

    // A valid artifact with no `repo_metrics` key at all — serialization
    // omits the `None` field, matching pre-section artifacts on disk.
    let artifact = CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-corpus-no-pools".to_string(),
        generated_at: "2026-07-14T00:00:00Z".to_string(),
        repos_included: 1,
        repos_attempted: 1,
        languages: vec![],
        repo_metrics: None,
    };
    let path = write_temp_calibration_artifact(&artifact);
    opts.calibration = Some(path.to_path_buf());

    let rows = run_architecture_metrics(&db, &opts).expect("run architecture-metrics");
    let names: Vec<&str> = rows.iter().map(|r| r.metric.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "propagation_cost",
            "acd",
            "nccd",
            "dependency_cycles",
            "largest_cycle",
            "files",
            "architecture_type",
        ],
        "no active repo_metrics pool -> exactly the seven base rows: {names:?}"
    );
}

/// The default (no `--calibration` override) path resolves to the embedded
/// world artifact, which carries repo-level corpus pools — so the three
/// corpus rows appear after the seven base rows, with a percentile in
/// `[0, 1]` and `corpus_n` matching the embedded pool length.
#[test]
fn architecture_metrics_default_embedded_artifact_emits_corpus_rows() {
    let (_dir, db, opts) = ingested_cycle_repo();
    assert!(
        opts.calibration.is_none(),
        "fixture must exercise the default (embedded-artifact) path"
    );
    let rows = run_architecture_metrics(&db, &opts).expect("run architecture-metrics");
    let m: HashMap<&str, &str> = rows
        .iter()
        .map(|r| (r.metric.as_str(), r.value.as_str()))
        .collect();

    let p: f64 = m
        .get("corpus_percentile:propagation_cost")
        .expect("embedded world artifact must carry a propagation_cost pool")
        .parse()
        .expect("percentile parses");
    assert!(
        (0.0..=1.0).contains(&p),
        "percentile must be in [0, 1], got {p}"
    );

    let embedded =
        codelore_lib::calibration::embedded_world().expect("embedded world artifact resolves");
    let pool_len = embedded
        .repo_metrics
        .as_ref()
        .expect("embedded world artifact must carry repo_metrics")
        .values
        .get("propagation_cost")
        .expect("propagation_cost pool present")
        .len();
    assert_eq!(
        m.get("corpus_n").copied(),
        Some(pool_len.to_string().as_str()),
        "corpus_n must state the embedded pool size"
    );
}

/// With a synthetic calibration artifact carrying a `repo_metrics` section
/// passed via `--calibration`, the three corpus-percentile rows appear with
/// hand-computed values.
///
/// The fixture's import graph (`a ↔ b` 2-cycle, `c → a`) has, per
/// `architecture_metrics_report_the_cycle_and_type` above and
/// `import_graph::graph_metrics`'s definition: `n = 3`, `cyclic_nodes = 2`
/// (`a`, `b`), so `propagation_cost = 7/9 ≈ 0.7778` (`ccd = vfo(a) + vfo(b) +
/// vfo(c) = 2 + 2 + 3 = 7`, `n² = 9`) and `cycle_file_share = 2/3 ≈ 0.6667`.
///
/// Pools are chosen so neither this-repo value collides with a pool member,
/// keeping the `raw_percentile` midpoint-rank arithmetic unambiguous:
/// - `propagation_cost` pool `[0.1, 0.5, 0.9]`: 0.7778 sits strictly between
///   0.5 and 0.9, so `count_less = 2`, `count_equal = 0`, `n = 3` →
///   `p = 2/3 ≈ 0.6667` → `"0.67"`.
/// - `cycle_file_share` pool `[0.0, 0.2]`: 0.6667 sits strictly above both,
///   so `count_less = 2`, `count_equal = 0`, `n = 2` → `p = 2/2 = 1.0` →
///   `"1.00"`.
/// - `corpus_n` uses the `propagation_cost` pool's length (3), per the
///   documented precedence rule.
#[test]
fn architecture_metrics_emits_corpus_percentiles_when_pool_active() {
    let (_dir, db, mut opts) = ingested_cycle_repo();

    let mut artifact = CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-corpus".to_string(),
        generated_at: "2026-07-14T00:00:00Z".to_string(),
        repos_included: 3,
        repos_attempted: 3,
        languages: vec![],
        repo_metrics: None,
    };
    let mut values = BTreeMap::new();
    values.insert("propagation_cost".to_string(), vec![0.1, 0.5, 0.9]);
    values.insert("cycle_file_share".to_string(), vec![0.0, 0.2]);
    attach_repo_metrics(&mut artifact, RepoMetrics { values });

    let path = write_temp_calibration_artifact(&artifact);
    opts.calibration = Some(path.to_path_buf());

    let rows = run_architecture_metrics(&db, &opts).expect("run architecture-metrics");
    let names: Vec<&str> = rows.iter().map(|r| r.metric.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "propagation_cost",
            "acd",
            "nccd",
            "dependency_cycles",
            "largest_cycle",
            "files",
            "architecture_type",
            "corpus_percentile:propagation_cost",
            "corpus_percentile:cycle_file_share",
            "corpus_n",
        ],
        "the three corpus rows must append, in order, after the seven base rows: {names:?}"
    );

    let m: HashMap<&str, &str> = rows
        .iter()
        .map(|r| (r.metric.as_str(), r.value.as_str()))
        .collect();
    assert_eq!(
        m.get("corpus_percentile:propagation_cost"),
        Some(&"0.67"),
        "hand-computed midpoint-rank percentile: {m:?}"
    );
    assert_eq!(
        m.get("corpus_percentile:cycle_file_share"),
        Some(&"1.00"),
        "hand-computed midpoint-rank percentile: {m:?}"
    );
    assert_eq!(
        m.get("corpus_n"),
        Some(&"3"),
        "corpus_n uses the propagation_cost pool length: {m:?}"
    );
}
