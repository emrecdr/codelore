//! End-to-end coverage for `cycle-origins`. The fixture introduces an
//! `a ↔ b` cycle partway through history (b starts importing a on day 6);
//! the analysis must report exactly that cycle (size 2) and pinpoint the
//! day-6 commit as its formation point — proving the binary search reads
//! historical structure rather than just HEAD.

use codelore_lib::analyses::cycle_origins::run_cycle_origins;
use codelore_lib::facts::FactsDb;
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

fn commit_at(dir: &Path, day: u32, msg: &str) {
    git(dir, &["add", "."]);
    let stamp = format!("2026-03-{day:02}T12:00:00Z");
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit {msg} failed");
}

#[test]
fn cycle_origins_pinpoints_the_formation_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "o@example.com"]);
    git(p, &["config", "user.name", "O"]);

    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"o\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(p, "src/lib.rs", "pub mod a;\npub mod b;\n");
    // C1–C5: acyclic — a imports b, b imports nothing.
    write(p, "src/a.rs", "use crate::b;\npub fn a() { b::b(); }\n");
    write(p, "src/b.rs", "pub fn b() {}\n");
    commit_at(p, 1, "acyclic");
    for (i, day) in [2u32, 3, 4, 5].iter().enumerate() {
        write(
            p,
            "src/a.rs",
            &format!("use crate::b;\npub fn a() {{ let _ = {i}; b::b(); }}\n"),
        );
        commit_at(p, *day, "edit a (acyclic)");
    }
    // C6: the cycle forms — b now imports a.
    write(p, "src/b.rs", "use crate::a;\npub fn b() { a::a(); }\n");
    commit_at(p, 6, "introduce a<->b cycle");
    let formation_date = "2026-03-06";
    // C7–C10: post-cycle edits.
    for (i, day) in [7u32, 8, 9, 10].iter().enumerate() {
        write(
            p,
            "src/b.rs",
            &format!("use crate::a;\npub fn b() {{ let _ = {i}; a::a(); }}\n"),
        );
        commit_at(p, *day, "edit b (cyclic)");
    }

    let repo = GixRepo::open(p).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest repo");

    let rows = run_cycle_origins(&db, &repo, &opts).expect("run cycle-origins");
    assert_eq!(rows.len(), 1, "exactly one cycle at HEAD: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.size, 2, "the a<->b cycle is size 2: {row:?}");
    assert_eq!(
        row.formed_at_date, formation_date,
        "cycle formed on the day b started importing a: {row:?}"
    );
    assert!(
        row.members.contains("src/a.rs") && row.members.contains("src/b.rs"),
        "members list both files: {row:?}"
    );
}
