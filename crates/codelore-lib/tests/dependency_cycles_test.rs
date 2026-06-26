//! End-to-end coverage for `dependency-cycles` over a repo with a known
//! import cycle. `a` and `b` `use crate::` each other (a 2-cycle); `c`
//! imports `a` but is not part of the cycle. Exercises the full
//! `ingest → build_import_graph → tarjan_scc` path (the algorithm
//! itself is unit-tested in `analyses::import_graph`).

use codelore_lib::analyses::dependency_cycles::run_dependency_cycles;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::HashSet;
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
fn dependency_cycles_finds_the_mutual_import_tangle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "c@example.com"]);
    git(p, &["config", "user.name", "C"]);

    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"cyc\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(p, "src/lib.rs", "pub mod a;\npub mod b;\npub mod c;\n");
    // a ↔ b: a imports b, b imports a — the cycle.
    write(p, "src/a.rs", "use crate::b;\npub fn a() { b::b(); }\n");
    write(p, "src/b.rs", "use crate::a;\npub fn b() { a::a(); }\n");
    // c imports a but nothing imports c — a dependent, not a cycle member.
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

    let repo = GixRepo::open(p).expect("open cycle repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest cycle repo");

    let rows = run_dependency_cycles(&db, &opts).expect("run dependency-cycles");

    // Exactly one cycle.
    let cycle_ids: HashSet<u32> = rows.iter().map(|r| r.cycle_id).collect();
    assert_eq!(
        cycle_ids.len(),
        1,
        "exactly one cycle expected; got {rows:?}"
    );

    let members: HashSet<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    assert!(
        members.contains("src/a.rs") && members.contains("src/b.rs"),
        "the cycle must contain a and b; got {members:?}"
    );
    assert!(
        !members.contains("src/c.rs"),
        "c imports a but is NOT part of the cycle; got {members:?}"
    );
    for r in &rows {
        assert_eq!(r.size, 2, "the a↔b tangle has size 2");
    }
}
