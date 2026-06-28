//! End-to-end coverage for `architecture-trend`. The fixture introduces
//! a dependency cycle partway through history: the trend must show
//! `cycle_count` rising from 0 (acyclic early) to 1 (after the back-edge
//! lands), proving the historical re-scan reads structure at past revs
//! rather than just reporting HEAD everywhere.

use codelore_lib::analyses::architecture_trend::run_architecture_trend;
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
    let stamp = format!("2026-02-{day:02}T12:00:00Z");
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
fn trend_captures_cycle_introduction_over_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "t@example.com"]);
    git(p, &["config", "user.name", "T"]);

    write(
        p,
        "Cargo.toml",
        "[package]\nname=\"t\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write(p, "src/lib.rs", "pub mod a;\npub mod b;\n");
    // C1: acyclic — a imports b, b imports nothing.
    write(p, "src/a.rs", "use crate::b;\npub fn a() { b::b(); }\n");
    write(p, "src/b.rs", "pub fn b() {}\n");
    commit_at(p, 1, "acyclic");

    // Pad the early (acyclic) era with several commits so the even
    // sampler lands at least one point before the cycle is introduced.
    for (i, day) in [2u32, 3, 4, 5].iter().enumerate() {
        write(
            p,
            "src/a.rs",
            &format!("use crate::b;\npub fn a() {{ let _ = {i}; b::b(); }}\n"),
        );
        commit_at(p, *day, "edit a (still acyclic)");
    }

    // C6: introduce the cycle — b now imports a (a ↔ b).
    write(p, "src/b.rs", "use crate::a;\npub fn b() { a::a(); }\n");
    commit_at(p, 6, "introduce a<->b cycle");
    // Pad the post-cycle era too.
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

    let rows = run_architecture_trend(&db, &repo, &opts).expect("run architecture-trend");
    assert!(rows.len() >= 4, "expected several sample points: {rows:?}");

    // Chronological order (oldest first).
    let dates: Vec<&str> = rows.iter().map(|r| r.date.as_str()).collect();
    let mut sorted = dates.clone();
    sorted.sort_unstable();
    assert_eq!(dates, sorted, "rows must be oldest-first: {dates:?}");

    // At least one early sample is acyclic; the final sample is cyclic.
    assert!(
        rows.iter().any(|r| r.cycle_count == 0),
        "an early sample must be acyclic: {rows:?}"
    );
    let last = rows.last().expect("non-empty");
    assert_eq!(
        last.cycle_count, 1,
        "final state has the a<->b cycle: {last:?}"
    );
    assert_eq!(last.largest_cycle, 2, "the cycle is size 2: {last:?}");

    // cycle_count is monotonic-ish here: once introduced it never leaves,
    // so the first cyclic sample comes after the last acyclic one.
    let first_cyclic = rows.iter().position(|r| r.cycle_count >= 1);
    let last_acyclic = rows.iter().rposition(|r| r.cycle_count == 0);
    if let (Some(fc), Some(la)) = (first_cyclic, last_acyclic) {
        assert!(
            fc > la,
            "cycle introduction must be a clean transition: {rows:?}"
        );
    }
}
