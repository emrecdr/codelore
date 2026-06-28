//! End-to-end coverage for `hotspot-velocity`. Fixture spans both
//! windows relative to a fixed anchor: `heating.rs` is quiet in the
//! baseline window and busy in the recent window (positive
//! acceleration); `cooling.rs` is the reverse (negative). Anchoring is
//! `MAX(commits.date)`, so dates are chosen relative to a fixed final
//! commit rather than wall-clock.

use codelore_lib::analyses::hotspot_velocity::run_hotspot_velocity;
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

/// Commit every currently-staged change with author+committer date set
/// to `YYYY-MM-DDT12:00:00Z`.
fn commit_at(dir: &Path, date: &str, msg: &str) {
    git(dir, &["add", "."]);
    let stamp = format!("{date}T12:00:00Z");
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
fn velocity_separates_heating_from_cooling() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "v@example.com"]);
    git(p, &["config", "user.name", "V"]);

    // Anchor (latest commit) is 2026-04-10. Recent window = last 30d
    // (≈ 2026-03-11 .. 2026-04-10); baseline = the 90d before that
    // (≈ 2025-12-11 .. 2026-03-11).
    //
    // cooling.rs: 3 commits in the baseline window, 0 in recent.
    for (i, day) in ["2026-01-05", "2026-01-20", "2026-02-10"]
        .iter()
        .enumerate()
    {
        write(
            p,
            "src/cooling.rs",
            &format!("pub fn c() {{ let _ = {i}; }}\n"),
        );
        commit_at(p, day, "cooling churn");
    }
    // heating.rs: 0 in baseline, 4 commits in the recent window.
    for (i, day) in ["2026-03-15", "2026-03-22", "2026-03-29", "2026-04-05"]
        .iter()
        .enumerate()
    {
        write(
            p,
            "src/heating.rs",
            &format!("pub fn h() {{ let _ = {i}; }}\n"),
        );
        commit_at(p, day, "heating churn");
    }
    // Final commit fixes the anchor date.
    write(p, "src/anchor.rs", "pub fn a() {}\n");
    commit_at(p, "2026-04-10", "anchor");

    let repo = GixRepo::open(p).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let opts = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest repo");

    let rows = run_hotspot_velocity(&db, &opts).expect("run hotspot-velocity");
    let by: HashMap<&str, &_> = rows.iter().map(|r| (r.path.as_str(), r)).collect();

    // heating.rs: all churn in the recent window → positive acceleration,
    // and it must rank above cooling.rs.
    let heating = by.get("src/heating.rs").expect("heating present");
    assert_eq!(heating.revs_recent, 4, "{heating:?}");
    assert_eq!(heating.revs_baseline, 0, "{heating:?}");
    assert!(
        heating.acceleration > 0.0,
        "heating accelerates: {heating:?}"
    );

    // cooling.rs: churned only in the baseline window. It has 0 recent
    // revisions, so by design it is not reported at all (cold files are
    // stale-code's job, not velocity's).
    assert!(
        !by.contains_key("src/cooling.rs"),
        "a file with 0 recent revs is not a velocity row: {rows:?}"
    );

    // Ranking: heating.rs is the top (most-accelerating) row.
    assert_eq!(
        rows.first().map(|r| r.path.as_str()),
        Some("src/heating.rs")
    );
}
