//! End-to-end coverage for `cycle-health` over a repo with a known import
//! cycle and dated commits. `a` and `b` `use crate::` each other (a
//! 2-cycle); `c` imports `a` but is not part of the cycle. Commit dates are
//! pinned so the trailing window either contains member churn (live) or
//! only non-member churn (fossil). The extraction-candidate scoring is
//! unit-tested in `analyses::cycle_health`; this exercises the full
//! `ingest → build_import_graph → run_cycle_health` path.

use codelore_lib::analyses::cycle_health::run_cycle_health;
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

/// Commit with author + committer dates pinned to a fixed instant so the
/// fixture's trailing-window placement is deterministic across runs.
fn commit_at(dir: &Path, message: &str, date: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", message, "--quiet"])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit '{message}' failed");
}

/// Initialise a repo whose first (2026-01-01) commit contains the
/// `a ↔ b` import cycle plus the non-member dependent `c`.
fn init_cycle_repo(p: &Path) {
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
    commit_at(p, "init", "2026-01-01T10:00:00Z");
}

#[test]
fn cycle_health_reports_live_cycle_with_heat() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init_cycle_repo(p);

    // Second commit at the repo's max date touches cycle member `a` —
    // inside the 90-day trailing window anchored to 2026-06-01.
    write(
        p,
        "src/a.rs",
        "use crate::b;\npub fn a() { b::b(); }\npub fn a2() {}\n",
    );
    git(p, &["add", "."]);
    commit_at(p, "touch a", "2026-06-01T10:00:00Z");

    let repo = GixRepo::open(p).expect("open cycle repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = codelore_lib::Options {
        window_days: 90,
        ..permissive_coupling_opts(p.to_path_buf())
    };
    db.ingest(&repo, &opts).expect("ingest cycle repo");

    let rows = run_cycle_health(&db, &opts).expect("run");
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.size, 2);
    assert_eq!(r.verdict, "live"); // a.rs modified at repo-max-date
    assert!(r.heat_pct > 0.0 && r.heat_pct <= 100.0);
    assert!(r.members_preview.contains("src/a.rs"));
    assert!(r.predicted_pc_drop.is_some()); // size 2 <= 64
}

#[test]
fn cycle_health_fossil_when_window_excludes_member_churn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init_cycle_repo(p);

    // Second commit at the repo's max date touches ONLY src/d.rs (outside
    // the cycle); the 90-day window contains no member churn.
    write(p, "src/d.rs", "pub fn d() {}\n");
    git(p, &["add", "."]);
    commit_at(p, "touch d only", "2026-06-01T10:00:00Z");

    let repo = GixRepo::open(p).expect("open cycle repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = codelore_lib::Options {
        window_days: 90,
        ..permissive_coupling_opts(p.to_path_buf())
    };
    db.ingest(&repo, &opts).expect("ingest cycle repo");

    let rows = run_cycle_health(&db, &opts).expect("run");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verdict, "fossil");
    assert!(
        rows[0].heat_pct.abs() < 1e-9,
        "fossil cycle must have zero heat, got {}",
        rows[0].heat_pct
    );
}
