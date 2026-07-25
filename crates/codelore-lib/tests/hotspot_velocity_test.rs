//! End-to-end coverage for `hotspot-velocity`. Fixture spans both
//! windows relative to a fixed anchor: `heating.rs` is quiet in the
//! baseline window and busy in the recent window (positive
//! acceleration); `cooling.rs` is the reverse (negative). Anchoring is
//! `MAX(commits.date)`, so dates are chosen relative to a fixed final
//! commit rather than wall-clock.

use codelore_lib::Options;
use codelore_lib::analyses::hotspot_velocity::run_hotspot_velocity;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use time::macros::date;

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

/// A file renamed *inside* the baseline-to-recent span must fold its
/// pre-rename churn into the canonical path when `--use-canonical-lineage`
/// is on. Without lineage the renamed file sees baseline 0 (its baseline
/// commits lived under the old name) and reports a falsely maximal
/// acceleration; with lineage the pre-rename commits count against the
/// canonical path so the baseline is non-zero.
#[test]
fn velocity_baseline_folds_pre_rename_churn_under_canonical_lineage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "v@example.com"]);
    git(p, &["config", "user.name", "V"]);

    // Anchor = 2026-04-10. recent = (≈2026-03-11, 2026-04-10];
    // baseline = the 90d before (≈2025-12-11, 2026-03-11].
    //
    // old.rs churns twice in the baseline window, is renamed to new.rs in
    // the recent window, then edited once more (recent).
    write(p, "src/old.rs", "pub fn f() { let _ = 1; }\n");
    commit_at(p, "2026-01-15", "seed old");
    write(p, "src/old.rs", "pub fn f() { let _ = 2; }\n");
    commit_at(p, "2026-02-01", "edit old");

    git(p, &["mv", "src/old.rs", "src/new.rs"]);
    commit_at(p, "2026-03-20", "rename old to new");
    write(p, "src/new.rs", "pub fn f() { let _ = 3; }\n");
    commit_at(p, "2026-03-28", "edit new");

    // Final commit fixes the anchor date.
    write(p, "src/anchor.rs", "pub fn a() {}\n");
    commit_at(p, "2026-04-10", "anchor");

    let repo = GixRepo::open(p).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let base = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &base).expect("ingest repo");

    // Lineage OFF: new.rs's baseline commits lived under old.rs, so the
    // renamed file sees baseline 0 (the false-max the fix targets).
    let off = Options {
        use_canonical_lineage: false,
        ..base.clone()
    };
    let rows_off = run_hotspot_velocity(&db, &off).expect("run lineage off");
    let new_off = rows_off
        .iter()
        .find(|r| r.path == "src/new.rs")
        .expect("new.rs present under lineage off");
    assert_eq!(
        new_off.revs_baseline, 0,
        "without lineage the pre-rename baseline churn is invisible: {new_off:?}"
    );

    // Lineage ON: the two pre-rename commits fold into the canonical
    // new.rs, so its baseline reflects the real history.
    let on = Options {
        use_canonical_lineage: true,
        ..base
    };
    let rows_on = run_hotspot_velocity(&db, &on).expect("run lineage on");
    assert!(
        !rows_on.iter().any(|r| r.path == "src/old.rs"),
        "old.rs folds into src/new.rs under canonical lineage: {rows_on:?}"
    );
    let new_on = rows_on
        .iter()
        .find(|r| r.path == "src/new.rs")
        .expect("new.rs present under lineage on");
    assert!(
        new_on.revs_baseline > new_off.revs_baseline,
        "pre-rename baseline churn must fold into the canonical path: \
         on={new_on:?} off={new_off:?}"
    );
}

/// `--age-time-now` re-anchors both velocity windows to a historical
/// instant instead of the latest commit, so a back-test reproduces the
/// velocity the repo showed on a past date. With no flag the anchor stays
/// at `MAX(commits.date)` via the `COALESCE` fallback — byte-identical to
/// the un-anchored behavior — so the two runs surface opposite files.
#[test]
fn velocity_age_time_now_reanchors_windows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "v@example.com"]);
    git(p, &["config", "user.name", "V"]);

    // early.rs churns in Jan/Feb; late.rs churns in Mar/Apr; anchor fixes
    // MAX(date) = 2026-04-10.
    for day in ["2026-01-20", "2026-02-05", "2026-02-18"] {
        write(
            p,
            "src/early.rs",
            &format!("pub fn e() {{ let _ = \"{day}\"; }}\n"),
        );
        commit_at(p, day, "early churn");
    }
    for day in ["2026-03-15", "2026-03-25", "2026-04-05"] {
        write(
            p,
            "src/late.rs",
            &format!("pub fn l() {{ let _ = \"{day}\"; }}\n"),
        );
        commit_at(p, day, "late churn");
    }
    write(p, "src/anchor.rs", "pub fn a() {}\n");
    commit_at(p, "2026-04-10", "anchor");

    let repo = GixRepo::open(p).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let base = permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &base).expect("ingest repo");

    // Default anchor = MAX(date) = 2026-04-10: only the Mar/Apr churn sits
    // in the recent window, so late.rs is a velocity row and early.rs
    // (all-baseline, zero recent) is not reported.
    let default_run = run_hotspot_velocity(&db, &base).expect("default anchor run");
    assert!(
        default_run.iter().any(|r| r.path == "src/late.rs"),
        "the MAX(date) anchor reports the Mar/Apr file: {default_run:?}"
    );
    assert!(
        !default_run.iter().any(|r| r.path == "src/early.rs"),
        "early.rs has no recent churn under the MAX(date) anchor: {default_run:?}"
    );

    // Back-test anchor = 2026-02-20: the recent window now covers the
    // Jan/Feb churn (early.rs) and every Mar/Apr commit is *after* the
    // anchor, so late.rs is excluded entirely.
    let backtest = Options {
        age_time_now: Some(date!(2026 - 02 - 20)),
        ..base
    };
    let back_run = run_hotspot_velocity(&db, &backtest).expect("back-test anchor run");
    assert!(
        back_run.iter().any(|r| r.path == "src/early.rs"),
        "the back-test anchor surfaces the Jan/Feb file: {back_run:?}"
    );
    assert!(
        !back_run.iter().any(|r| r.path == "src/late.rs"),
        "commits after the --age-time-now anchor are excluded: {back_run:?}"
    );
}
