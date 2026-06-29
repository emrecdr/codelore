//! End-to-end coverage for the architecture quality gate
//! (`[gates] max_dependency_cycles` / `max_propagation_cost`). Ingests a
//! fixture with a known `a ↔ b` cycle and asserts the gate fires when
//! the cycle budget is exceeded and clears when it isn't.

use codelore_lib::facts::FactsDb;
use codelore_lib::quality_gates::{Thresholds, evaluate_architecture_gate};
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
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

/// Ingest an `a ↔ b` 2-cycle (+ `c → a`) repo. Exactly one dependency
/// cycle, of size 2.
fn ingested_cyclic_repo() -> (tempfile::TempDir, FactsDb, codelore_lib::Options) {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "g@example.com"]);
    git(p, &["config", "user.name", "G"]);
    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"g\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
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
fn architecture_gate_fails_on_a_cycle_when_budget_is_zero() {
    let (_dir, db, _opts) = ingested_cyclic_repo();

    // max_dependency_cycles = 0 → the one cycle violates.
    let mut t = Thresholds::default();
    t.gates.max_dependency_cycles = Some(0);
    let v = evaluate_architecture_gate(&t, &db).expect("evaluate gate");
    assert_eq!(v.len(), 1, "exactly one cycle violation: {v:?}");
    assert_eq!(v[0].gate, "max_dependency_cycles");
    assert_eq!(v[0].actual, "1");
    assert_eq!(v[0].threshold, "0");
}

#[test]
fn architecture_gate_passes_when_budget_covers_the_cycle() {
    let (_dir, db, _opts) = ingested_cyclic_repo();

    // The gate is `> max`, so a budget of 1 admits the single cycle.
    let mut t = Thresholds::default();
    t.gates.max_dependency_cycles = Some(1);
    assert!(
        evaluate_architecture_gate(&t, &db)
            .expect("evaluate gate")
            .is_empty(),
        "one cycle within a budget of 1 must pass"
    );

    // No architecture gate configured → noop (no graph build, no rows).
    let empty = Thresholds::default();
    assert!(
        evaluate_architecture_gate(&empty, &db)
            .expect("evaluate gate")
            .is_empty()
    );
}
