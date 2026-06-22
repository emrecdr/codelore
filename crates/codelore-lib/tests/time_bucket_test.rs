//! `--time-bucket` integration tests.
//!
//! Validates that `--time-bucket day|week|month` collapses commits within
//! the same bucket into a single logical commit for pair-counting purposes
//! (the coupling-family analyses: `coupling` + `soc` + `clone-coupling`).

use codelore_lib::Options;
use codelore_lib::analyses::soc::run_soc;
use codelore_lib::facts::FactsDb;
use codelore_lib::options::TimeBucket;
use codelore_lib::repo::GixRepo;

fn run_git(path: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(p: std::path::PathBuf, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Commit `foo` alone in one commit, then `bar` alone in another commit,
/// both on the SAME day. Without bucketing, `SoC` = 0 for both (each is a
/// solo commit). With `--time-bucket day`, they collapse into one logical
/// commit that touches `{foo, bar}` → both get `SoC` = 1.
#[test]
fn time_bucket_day_collapses_same_day_solo_commits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Both commits on the same fixed date so they fall in the same day-bucket.
    let fixed_date = "2024-01-15T10:00:00";

    write(path.join("foo.rs"), "v1\n");
    run_git(path, &["add", "."]);
    let env = format!("GIT_AUTHOR_DATE={fixed_date} GIT_COMMITTER_DATE={fixed_date}");
    // Use env vars via Command directly for the commit
    let out = std::process::Command::new("git")
        .args(["commit", "-m", "foo solo", "--quiet"])
        .env("GIT_AUTHOR_DATE", fixed_date)
        .env("GIT_COMMITTER_DATE", fixed_date)
        .current_dir(path)
        .output()
        .expect("commit");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    write(path.join("bar.rs"), "v1\n");
    let out = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()
        .expect("git add");
    assert!(out.status.success());

    let out = std::process::Command::new("git")
        .args(["commit", "-m", "bar solo", "--quiet"])
        .env("GIT_AUTHOR_DATE", fixed_date)
        .env("GIT_COMMITTER_DATE", fixed_date)
        .current_dir(path)
        .output()
        .expect("commit");
    assert!(out.status.success());
    let _ = env; // silence unused

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");

    // Without bucketing: each commit is a solo commit → SoC = 0 → all
    // dropped (default min_soc = 1).
    let opts_raw = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        time_bucket: None,
        ..Options::default()
    };
    db.ingest(&repo, &opts_raw).expect("ingest");
    let rows_raw = run_soc(&db, &opts_raw).expect("soc raw");
    assert!(
        rows_raw.is_empty() || rows_raw.iter().all(|r| r.soc == 0),
        "without bucketing: solo commits → SoC=0 → all dropped or all zero. Got: {rows_raw:?}"
    );

    // With --time-bucket day: same-day commits collapse → bucket touches
    // both foo and bar → each contributes SoC=1.
    let opts_bucket = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        min_soc: Some(1),
        time_bucket: Some(TimeBucket::Day),
        ..Options::default()
    };
    let rows_bucket = run_soc(&db, &opts_bucket).expect("soc bucket");
    let entities: Vec<&str> = rows_bucket.iter().map(|r| r.entity.as_str()).collect();
    assert!(
        entities.contains(&"foo.rs"),
        "day-bucket: foo collapsed with bar → SoC>=1. Got: {entities:?}"
    );
    assert!(
        entities.contains(&"bar.rs"),
        "day-bucket: bar collapsed with foo → SoC>=1. Got: {entities:?}"
    );
}

/// `TimeBucket::as_sql_unit` returns the lowercase strings `DuckDB`'s
/// `date_trunc` accepts. Sanity check the contract.
#[test]
fn time_bucket_sql_unit_strings() {
    assert_eq!(TimeBucket::Day.as_sql_unit(), "day");
    assert_eq!(TimeBucket::Week.as_sql_unit(), "week");
    assert_eq!(TimeBucket::Month.as_sql_unit(), "month");
}

/// Previously, `--time-bucket day` + `--max-changeset-size 5`
/// would silently drop an active day whose total distinct files exceeds
/// 5, even if every individual commit only touched 1-2 files. Post-fix,
/// the filter applies to PHYSICAL commits via `MAX(files) <= ?` so the
/// day survives as long as it contains no giant commit.
#[test]
fn time_bucket_with_max_changeset_keeps_active_day_with_many_small_commits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    let fixed_date = "2024-01-15T10:00:00";
    let commit_at_date = |msg: &str| {
        let out = std::process::Command::new("git")
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", fixed_date)
            .env("GIT_COMMITTER_DATE", fixed_date)
            .current_dir(path)
            .output()
            .expect("commit");
        assert!(
            out.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    // 8 commits on the same day, each touching exactly 1 small file.
    // Per-commit size = 1; per-bucket distinct files = 8.
    // Previously with `max_changeset_size = 5`: bucket dropped (8 > 5).
    // Now: bucket kept (MAX per-commit = 1 ≤ 5).
    for i in 0..8 {
        write(path.join(format!("f{i}.rs")), &format!("v1 for f{i}\n"));
        run_git(path, &["add", "."]);
        commit_at_date(&format!("add f{i}"));
    }

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        min_soc: Some(1),
        max_changeset_size: 5,
        time_bucket: Some(TimeBucket::Day),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_soc(&db, &opts).expect("soc");

    // Bucket survives → each of the 8 files should appear with SoC ≥ 1
    // (they all "co-occur" in the collapsed day bucket).
    assert!(
        !rows.is_empty(),
        "F29 regression: --time-bucket day + --max-changeset-size 5 dropped \
         the entire day-bucket even though every physical commit was tiny. \
         Pre-fix: empty result. Post-fix: 8 entities with SoC>=1. Got: {rows:?}"
    );
    let entities: Vec<&str> = rows.iter().map(|r| r.entity.as_str()).collect();
    for i in 0..8 {
        let name = format!("f{i}.rs");
        assert!(
            entities.iter().any(|e| *e == name),
            "F29: f{i}.rs missing from bucket-collapsed SoC. Got entities: {entities:?}"
        );
    }
}

/// Control case — a single giant commit in the bucket SHOULD still
/// drop the whole bucket under the conservative MAX(files) semantic.
/// This confirms the anti-monorepo-sweep intent survives the per-commit change.
#[test]
fn time_bucket_with_max_changeset_drops_bucket_containing_giant_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    let fixed_date = "2024-02-20T10:00:00";
    let commit_at_date = |msg: &str| {
        let out = std::process::Command::new("git")
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", fixed_date)
            .env("GIT_COMMITTER_DATE", fixed_date)
            .current_dir(path)
            .output()
            .expect("commit");
        assert!(out.status.success());
    };

    // One giant commit (10 files in one shot) + one tiny commit.
    // With max_changeset_size = 5, the giant commit's bucket should be
    // dropped under the conservative MAX(files) semantic.
    for i in 0..10 {
        write(path.join(format!("g{i}.rs")), "v\n");
    }
    run_git(path, &["add", "."]);
    commit_at_date("giant sweep");

    write(path.join("small.rs"), "v\n");
    run_git(path, &["add", "."]);
    commit_at_date("small");

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        min_soc: Some(1),
        max_changeset_size: 5,
        time_bucket: Some(TimeBucket::Day),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_soc(&db, &opts).expect("soc");
    assert!(
        rows.is_empty(),
        "F29 control: bucket containing a 10-file giant commit must be \
         dropped under max_changeset_size=5. Got: {rows:?}"
    );
}
