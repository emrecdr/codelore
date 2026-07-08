//! End-to-end coverage of `run_bus_factor` against an ingested `FactsDb`.
//!
//! The audit cycle called out that bus-factor had a row struct,
//! a unit test, but no integration test against a real ingested
//! repository — so SQL typos or schema renames would surface only at
//! customer runtime. This test exercises the full path through
//! `run_bus_factor`, asserting both shape invariants and the
//! Filatov-2010 bus-factor semantic (smaller = more concentrated).

use codelore_lib::Options;
use codelore_lib::analyses::bus_factor::run_bus_factor;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn bus_factor_on_tiny_repo_concentrates_to_one_author() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_bus_factor(&db, &opts).expect("run bus-factor");

    // `tiny_repo` has a single author across all commits, so every
    // module's bus_factor must be exactly 1 (one author covers
    // ≥80% of commits by construction). The presence of at least
    // one row also proves the SQL didn't return an empty result
    // set silently.
    assert!(!rows.is_empty(), "expected at least one module row");
    for row in &rows {
        assert_eq!(
            row.bus_factor, 1,
            "single-author `tiny_repo` must yield bus_factor=1 for module {}",
            row.module,
        );
        assert!(
            row.total_commits > 0,
            "module {} reported total_commits=0",
            row.module,
        );
    }
}

/// Build a repo containing a repo-root file (no directory prefix)
/// alongside a nested file. The root file must still be counted.
fn build_root_file_repo() -> tempfile::TempDir {
    use std::process::Command;
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
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    run(path, "2026-01-02T12:00:00Z", &["add", "README.md"]);
    run(
        path,
        "2026-01-02T12:00:00Z",
        &["commit", "-m", "readme", "--quiet"],
    );

    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src/main.rs"), "fn main() {}\n").unwrap();
    run(path, "2026-01-03T12:00:00Z", &["add", "src/main.rs"]);
    run(
        path,
        "2026-01-03T12:00:00Z",
        &["commit", "-m", "src", "--quiet"],
    );
    dir
}

#[test]
fn bus_factor_includes_repo_root_files() {
    let fixture = build_root_file_repo();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_bus_factor(&db, &opts).expect("run bus-factor");

    // `README.md` lives at the repo root (no `/`), so it must be
    // counted under a root bucket rather than silently dropped.
    let root = rows
        .iter()
        .find(|r| r.module == "<root>")
        .expect("repo-root files must be counted under a <root> bucket");
    assert!(
        root.total_commits >= 1,
        "the <root> bucket must count the README.md commit; got {}",
        root.total_commits,
    );

    // The nested file's top-level directory must still bucket normally.
    assert!(
        rows.iter().any(|r| r.module == "src"),
        "nested files must still bucket by top-level directory",
    );
}

/// Verify that the commits-mode rows carry `model = "commits"` and that the
/// DOE-mode rows carry `model = "doe"`. Both modes must return ≥1 row
/// against `delivery_repo`, which has multiple authors and sufficient
/// commit depth for the DOE formula to produce experts.
#[test]
fn bus_factor_model_field_reflects_mode() {
    let delivery = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(delivery.dir.path()).expect("open delivery_repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts_base = Options {
        repo_path: delivery.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts_base).expect("ingest");

    // Commits mode (default): every row must have model = "commits".
    let commits_rows = run_bus_factor(&db, &opts_base).expect("commits mode");
    assert!(
        !commits_rows.is_empty(),
        "commits mode must return ≥1 row on delivery_repo"
    );
    for row in &commits_rows {
        assert_eq!(
            row.model, "commits",
            "commits mode row for module '{}' has wrong model field",
            row.module
        );
        assert!(
            row.bus_factor >= 1,
            "bus_factor must be ≥1 in commits mode for module '{}'",
            row.module
        );
    }

    // DOE mode: every row must have model = "doe".
    let opts_doe = Options {
        knowledge_model: "doe".to_string(),
        ..opts_base.clone()
    };
    let doe_rows = run_bus_factor(&db, &opts_doe).expect("doe mode");
    // delivery_repo has enough complexity for DOE experts to exist, so
    // we expect at least one row.
    assert!(
        !doe_rows.is_empty(),
        "doe mode must return ≥1 row on delivery_repo"
    );
    for row in &doe_rows {
        assert_eq!(
            row.model, "doe",
            "doe mode row for module '{}' has wrong model field",
            row.module
        );
        assert!(
            row.bus_factor >= 1,
            "bus_factor must be ≥1 in doe mode for module '{}'",
            row.module
        );
    }
}

/// Hand-verified DOE bus-factor for `delivery_repo`.
///
/// The DOE formula produces expert assignments for the single `src` module:
///
/// ```text
/// File            Experts
/// src/branch1.rs  bob, carol        (alice never touched it)
/// src/branch2.rs  carol             (only author)
/// src/core.rs     alice, carol
/// src/rework.rs   alice, bob
/// src/stable.rs   alice, bob
/// ```
///
/// All 9 `(file, author)` pairs in `doe_scores` have `is_expert = true`
/// (DOE scores are all well above 1.0 and above 0.75 × `max_doe` for each file).
///
/// Greedy removal trace (stop when uncovered * 2 > 5):
///   Start:   covered = {branch1, branch2, core, rework, stable} (5), uncovered = 0
///   Round 1: alice covers 3 (core, rework, stable)
///            bob   covers 3 (branch1, rework, stable)
///            carol covers 3 (branch1, branch2, core)
///            Three-way tie → alphabetical descending → alice wins.
///            Remove alice. Rework & stable still covered by bob.
///            covered still = 5, uncovered = 0. removed = 1.
///   Round 2: bob covers 3 (branch1, rework, stable)
///            carol covers 3 (branch1, branch2, core)
///            Tie → bob wins (b > c descending).
///            Remove bob. rework & stable now uncovered (carol ∉ experts).
///            covered = {branch1, branch2, core}, uncovered = 2. removed = 2.
///   Round 3: uncovered * 2 = 4 ≤ 5 → continue.
///            carol covers 3 (branch1, branch2, core).
///            Remove carol. covered = {}, uncovered = 5. removed = 3.
///   Round 4: uncovered * 2 = 10 > 5 → stop.
///   `bus_factor` = `removed_count.max(1)` = 3.
///   `top_contributor` = alice (first removed, 3/5 expert files → share = 0.6).
#[test]
fn doe_bus_factor_hand_verified() {
    let delivery = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(delivery.dir.path()).expect("open delivery_repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: delivery.dir.path().to_path_buf(),
        min_revs: 1,
        knowledge_model: "doe".to_string(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_bus_factor(&db, &opts).expect("run_bus_factor doe");

    // delivery_repo has one top-level module: "src".
    assert_eq!(rows.len(), 1, "expected exactly one module row (src)");
    let row = &rows[0];
    assert_eq!(row.module, "src");
    assert_eq!(row.model, "doe");

    // Greedy removal requires 3 authors before >50% of 5 files are uncovered.
    assert_eq!(
        row.bus_factor, 3,
        "hand-computed: 3 authors must be removed before >50% of 5 files are uncovered"
    );

    // First removed author is alice (tie-break: highest alphabetically descending).
    assert_eq!(
        row.top_contributor, "alice@example.com",
        "alice is removed first (3-way tie broken alphabetically descending)"
    );

    // alice is expert on 3 of 5 files → share = 0.6.
    assert!(
        (row.top_contributor_share - 0.6).abs() < 1e-9,
        "alice covers 3/5 expert files → share = 0.600, got {:.4}",
        row.top_contributor_share
    );
}

/// Build a repo where a file is renamed across directories so its
/// commit history spans two top-level modules.
fn build_renamed_file_repo() -> tempfile::TempDir {
    use std::process::Command;
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
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    std::fs::create_dir_all(path.join("old")).unwrap();
    std::fs::write(path.join("old/a.txt"), "one\n").unwrap();
    run(path, "2026-01-02T12:00:00Z", &["add", "old/a.txt"]);
    run(
        path,
        "2026-01-02T12:00:00Z",
        &["commit", "-m", "c1", "--quiet"],
    );
    std::fs::write(path.join("old/a.txt"), "one\ntwo\n").unwrap();
    run(
        path,
        "2026-01-03T12:00:00Z",
        &["commit", "-am", "c2", "--quiet"],
    );

    std::fs::create_dir_all(path.join("new")).unwrap();
    run(
        path,
        "2026-01-04T12:00:00Z",
        &["mv", "old/a.txt", "new/a.txt"],
    );
    run(
        path,
        "2026-01-04T12:00:00Z",
        &["commit", "-m", "rename", "--quiet"],
    );
    std::fs::write(path.join("new/a.txt"), "one\ntwo\nthree\n").unwrap();
    run(
        path,
        "2026-01-05T12:00:00Z",
        &["commit", "-am", "c3", "--quiet"],
    );
    dir
}

#[test]
fn bus_factor_attributes_renamed_file_to_one_module_under_lineage() {
    let fixture = build_renamed_file_repo();
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Under canonical lineage the renamed file's history resolves to a
    // single canonical path, so its commits attribute to ONE module —
    // not split across `old` and `new`.
    let rows = run_bus_factor(&db, &opts).expect("run bus-factor under lineage");
    let modules: Vec<&str> = rows.iter().map(|r| r.module.as_str()).collect();
    assert!(
        !(modules.contains(&"old") && modules.contains(&"new")),
        "renamed file should not split across `old` and `new` under \
         canonical lineage; got modules {modules:?}",
    );
    assert!(
        !rows.is_empty(),
        "lineage-routed bus-factor must still produce rows",
    );
}
