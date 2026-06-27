//! End-to-end coverage for the Martin/Lakos structural metrics
//! (`instability` + `architecture-metrics`) over a fixture with a known
//! shape: `a ↔ b` form a 2-cycle, `c` imports `a`. The graph algorithms
//! are unit-tested in `analyses::import_graph`; this drives the full
//! ingest → metric path and pins the exact numbers.

use codelore_lib::analyses::architecture_metrics::run_architecture_metrics;
use codelore_lib::analyses::instability::run_instability;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::HashMap;
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
