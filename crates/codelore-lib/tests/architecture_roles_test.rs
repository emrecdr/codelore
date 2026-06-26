//! End-to-end coverage for `architecture-roles`. Fixture: `a ↔ b` form
//! a 2-cycle (the Core); `c` imports `a` but nothing imports `c`. So
//! `a`/`b` are `core` (in a cycle), and `c` is `control` (depends on the
//! Core, nothing depends on it). The reachability math is unit-tested in
//! `analyses::import_graph`; this drives the full ingest → classify path.

use codelore_lib::analyses::architecture_roles::run_architecture_roles;
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

#[test]
fn architecture_roles_classifies_core_and_control() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "r@example.com"]);
    git(p, &["config", "user.name", "R"]);

    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"roles\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(p, "src/lib.rs", "pub mod a;\npub mod b;\npub mod c;\n");
    // a ↔ b: the Core (a 2-cycle).
    write(p, "src/a.rs", "use crate::b;\npub fn a() { b::b(); }\n");
    write(p, "src/b.rs", "use crate::a;\npub fn b() { a::a(); }\n");
    // c depends on the Core; nothing depends on c → Control.
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

    let repo = GixRepo::open(p).expect("open roles repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest roles repo");

    let rows = run_architecture_roles(&db, &opts).expect("run architecture-roles");
    let by_path: HashMap<&str, &_> = rows.iter().map(|r| (r.path.as_str(), r)).collect();

    let a = by_path.get("src/a.rs").expect("a present");
    let b = by_path.get("src/b.rs").expect("b present");
    let c = by_path.get("src/c.rs").expect("c present");

    // a and b are the Core: in a cycle, classified `core`.
    assert_eq!(a.role, "core", "a is in the a↔b cycle: {a:?}");
    assert_eq!(b.role, "core", "b is in the a↔b cycle: {b:?}");
    assert!(a.in_cycle && b.in_cycle, "a and b are cycle members");

    // c depends on the Core but nothing depends on it → control, no cycle.
    assert_eq!(c.role, "control", "c depends on much, nothing on it: {c:?}");
    assert!(!c.in_cycle, "c is not in a cycle");
    // c reaches a and b downstream (vfo includes the Core), but only
    // itself reaches c (vfi = 1).
    assert_eq!(c.vfi, 1, "nothing imports c");
    assert!(c.vfo >= 3, "c reaches itself + the a↔b core: {c:?}");

    // Layering: c imports the core, nothing imports c → c is the source
    // (level 0); the a↔b core sits one layer down (level 1).
    assert_eq!(c.level, 0, "c is a source (nothing imports it): {c:?}");
    assert_eq!(a.level, 1, "the core sits below c: {a:?}");
    assert_eq!(b.level, 1, "the core sits below c: {b:?}");
}
