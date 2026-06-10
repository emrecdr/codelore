#![allow(clippy::doc_markdown, clippy::items_after_statements)]
//! T8 — `knowledge-islands` analysis tests.
//!
//! Tests the bus-factor / knowledge-loss risk indicator: per-file
//! identification of cases where the primary author (by LoC) has
//! effectively departed (no commits in `--departed-threshold-days`)
//! AND no other contributor owns a substantial share.

use codelore_lib::Options;
use codelore_lib::analyses::knowledge_islands::run_knowledge_islands;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// Build a deliberately-aged test repo: one file authored entirely by
/// Alice in early-2024, then no Alice commits since. Bob makes a
/// trivial 1-line edit (insufficient ownership share).
///
/// With `--age-time-now 2026-06-01` and `--departed-threshold-days 30`:
///   - Alice's last commit: ~2024-03-01 → days_since ~ 730 days → departed
///   - Bob's tiny contribution: 1 LoC out of Alice's much-larger total
///     → ownership_pct(Bob) ≈ 1% → below the 10% substantial threshold
///   - `alice_owned.txt` SHOULD appear as a knowledge-island
///   - `bob_dominates.txt`: Bob owns 100% → Bob's last_at (if recent)
///     → NOT departed → SHOULD NOT appear
fn build_fixture() -> tempfile::TempDir {
    use std::process::Command;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    fn run(path: &std::path::Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git");
        assert!(status.success());
    }
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["config", "user.email", "init@x"],
    );
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["config", "user.name", "Init"],
    );

    // Alice writes a substantial file in 2024.
    std::fs::write(
        path.join("alice_owned.txt"),
        "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n",
    )
    .unwrap();
    run(path, "2024-03-01T12:00:00Z", &["add", "alice_owned.txt"]);
    run(
        path,
        "2024-03-01T12:00:00Z",
        &[
            "commit",
            "-m",
            "alice",
            "--author",
            "Alice <alice@old.com>",
            "--quiet",
        ],
    );

    // Bob writes his own small file 7 days before the anchor (well
    // within any reasonable departure threshold — he's "active") AND
    // makes a 1-line touch to alice_owned.txt (insufficient ownership).
    std::fs::write(path.join("bob_dominates.txt"), "x\n").unwrap();
    run(path, "2026-05-25T12:00:00Z", &["add", "bob_dominates.txt"]);
    run(
        path,
        "2026-05-25T12:00:00Z",
        &[
            "commit",
            "-m",
            "bob solo",
            "--author",
            "Bob <bob@active.com>",
            "--quiet",
        ],
    );
    std::fs::write(
        path.join("alice_owned.txt"),
        "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n",
    )
    .unwrap();
    run(path, "2026-05-26T12:00:00Z", &["add", "alice_owned.txt"]);
    run(
        path,
        "2026-05-26T12:00:00Z",
        &[
            "commit",
            "-m",
            "bob 1-liner",
            "--author",
            "Bob <bob@active.com>",
            "--quiet",
        ],
    );
    dir
}

#[test]
fn knowledge_islands_surfaces_departed_main_author_solo_owner() {
    let fixture = build_fixture();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // Anchor 2026-06-01 → Alice last 2024-03-01 → ~822 days departed
    //                  → Bob   last 2025-06-02 → ~365 days NOT departed
    // Threshold 30 days.
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        age_time_now: Some(time::macros::date!(2026 - 06 - 01)),
        departed_threshold_days: 30,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_knowledge_islands(&db, &opts).expect("run");
    let alice_row = rows
        .iter()
        .find(|r| r.entity == "alice_owned.txt")
        .expect("alice_owned.txt surfaces as knowledge-island");
    assert_eq!(alice_row.main_author, "alice@old.com");
    assert!(
        alice_row.ownership_pct > 80.0,
        "Alice owns ~91% (10/11 lines); got {}",
        alice_row.ownership_pct,
    );
    assert!(
        alice_row.days_since_main_active > 800,
        "should reflect ~822-day gap; got {}",
        alice_row.days_since_main_active,
    );

    // Bob's solo file should NOT appear — Bob isn't departed at the anchor.
    assert!(
        !rows.iter().any(|r| r.entity == "bob_dominates.txt"),
        "bob_dominates.txt should NOT surface; Bob's last commit is recent",
    );
}

#[test]
fn knowledge_islands_threshold_gates_inclusion() {
    let fixture = build_fixture();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // High threshold (10 years) → nobody is "departed" → empty output.
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        age_time_now: Some(time::macros::date!(2026 - 06 - 01)),
        departed_threshold_days: 365 * 10,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_knowledge_islands(&db, &opts).expect("run");
    assert!(
        rows.is_empty(),
        "10-year departure threshold → no rows; got {} rows",
        rows.len(),
    );
}

#[test]
fn knowledge_islands_excludes_deleted_files() {
    // F16 carry-over: knowledge-islands uses the same live-paths CTE
    // pattern; deleted files shouldn't surface even with departed
    // authors.
    use std::process::Command;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    fn run(path: &std::path::Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git");
        assert!(status.success());
    }
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2024-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );
    std::fs::write(path.join("gone.txt"), "x\ny\nz\n").unwrap();
    run(path, "2024-03-01T12:00:00Z", &["add", "gone.txt"]);
    run(
        path,
        "2024-03-01T12:00:00Z",
        &[
            "commit",
            "-m",
            "add",
            "--author",
            "Alice <alice@old.com>",
            "--quiet",
        ],
    );
    std::fs::remove_file(path.join("gone.txt")).unwrap();
    run(path, "2024-04-01T12:00:00Z", &["add", "-A"]);
    run(
        path,
        "2024-04-01T12:00:00Z",
        &[
            "commit",
            "-m",
            "delete",
            "--author",
            "Alice <alice@old.com>",
            "--quiet",
        ],
    );

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        age_time_now: Some(time::macros::date!(2026 - 06 - 01)),
        departed_threshold_days: 30,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_knowledge_islands(&db, &opts).expect("run");
    assert!(
        !rows.iter().any(|r| r.entity == "gone.txt"),
        "deleted files must not appear in knowledge-islands; got {rows:?}",
    );
}
