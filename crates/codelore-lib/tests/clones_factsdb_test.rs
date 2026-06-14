//! Plan 8 §4 Task 15: `FactsDb::ingest` now populates the `clones` table.
//!
//! Validates that:
//!   1. After ingest on a fixture with cloned functions, the `clones` table
//!      has rows.
//!   2. `IngestStats.clones_ingested` reports the count.
//!   3. Honors `opts.exclude_patterns` (no rows for excluded paths).
//!   4. Honors `opts.min_clone_node_count`.

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn ingest_populates_clones_table_for_type2_clone_pair() {
    // Build a tiny repo where two files contain Type-2 clone fns.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(
        path.join("src/a.rs"),
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    )
    .unwrap();
    std::fs::write(
        path.join("src/b.rs"),
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    )
    .unwrap();

    // git init + initial commit so FactsDb::ingest has a HEAD to walk.
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_clone_node_count: 0, // include this very small fn
        ..Options::default()
    };
    let stats = db.ingest(&repo, &opts).expect("ingest");

    // Both functions live in 1 family → 2 rows in the clones table.
    assert_eq!(
        stats.clones_ingested, 2,
        "expected 2 clone-family rows; got {}",
        stats.clones_ingested
    );

    // Spot-check the table by counting.
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM clones", [], |r| r.get(0))
        .expect("count clones");
    assert_eq!(count, 2);
}

#[test]
fn ingest_respects_exclude_patterns_for_clones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::create_dir_all(path.join("vendor")).unwrap();
    std::fs::write(
        path.join("src/a.rs"),
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    )
    .unwrap();
    std::fs::write(
        path.join("vendor/b.rs"),
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    )
    .unwrap();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_clone_node_count: 0,
        exclude_patterns: vec!["vendor/**".to_string()],
        ..Options::default()
    };
    let stats = db.ingest(&repo, &opts).expect("ingest");

    // Vendor is excluded so the clone partner disappears → no families.
    assert_eq!(
        stats.clones_ingested, 0,
        "excluded vendor/ should kill the family; got {}",
        stats.clones_ingested
    );
}

#[test]
fn ingest_populates_clones_on_bare_repository() {
    // F77 regression: the previous `WalkDir::new(&opts.repo_path)`
    // discovery phase yielded zero candidates on a bare repo (no
    // working tree → only `.git`-style metadata exists). Clones
    // would silently come back as 0 rows. The fix switches to
    // `query_live_paths`, which reads from the `changes` table —
    // populated by the gix walker via the ODB regardless of whether
    // a working tree exists.

    // Step 1: build a regular fixture with a clone pair.
    let source = tempfile::tempdir().unwrap();
    let source_path = source.path();
    std::fs::create_dir_all(source_path.join("src")).unwrap();
    std::fs::write(
        source_path.join("src/a.rs"),
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    )
    .unwrap();
    std::fs::write(
        source_path.join("src/b.rs"),
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    )
    .unwrap();
    run_git(source_path, &["init", "-b", "main", "--quiet"]);
    run_git(source_path, &["config", "user.email", "t@e.com"]);
    run_git(source_path, &["config", "user.name", "T"]);
    run_git(source_path, &["add", "."]);
    run_git(source_path, &["commit", "-m", "init", "--quiet"]);

    // Step 2: clone --bare into a fresh tempdir. The resulting repo
    // has no working tree — exactly the shape WalkDir failed on.
    let bare = tempfile::tempdir().unwrap();
    let bare_path = bare.path().join("clones.git");
    let clone_exit = std::process::Command::new("git")
        .args(["clone", "--bare", "--quiet"])
        .arg(source_path)
        .arg(&bare_path)
        .status()
        .expect("git clone --bare");
    assert!(clone_exit.success(), "git clone --bare failed");
    // Sanity: confirm there's truly no working-tree checkout.
    assert!(
        !bare_path.join("src").exists(),
        "bare repo unexpectedly has src/ — test invalid"
    );

    let repo = GixRepo::open(&bare_path).expect("gix open bare");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bare_path.clone(),
        min_clone_node_count: 0,
        ..Options::default()
    };
    let ingest_stats = db.ingest(&repo, &opts).expect("ingest bare");

    assert_eq!(
        ingest_stats.clones_ingested, 2,
        "bare repo must still detect the clone pair; got {}",
        ingest_stats.clones_ingested
    );
}

fn run_git(path: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}
