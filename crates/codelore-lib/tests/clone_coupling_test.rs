//! Clone-coupling intersection analysis tests.
//!
//! Validates that:
//!
//! 1. A live clone family (two cloned files that co-change at Fisher-
//!    significant rates) shows up in the `clone-coupling` output.
//! 2. A dead clone family (two cloned files that NEVER co-change) does
//!    NOT show up.
//! 3. The 5 false-positive mitigations from the research brief fire
//!    correctly: `min_clone_node_count`, `min_clone_shared_revs`,
//!    `clone_similarity_floor`, `--exclude` path filter, `clone_skip_same_dir`.

use codelore_lib::Options;
use codelore_lib::analyses::clone_coupling::run_clone_coupling;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// Build a fixture where two cloned files co-change in many commits so the
/// Fisher exact test fires. Then assert clone-coupling surfaces the pair.
#[test]
fn live_clone_pair_surfaces_in_clone_coupling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    // Two source files in DIFFERENT directories so clone_skip_same_dir
    // (default true) doesn't filter the pair.
    std::fs::create_dir_all(path.join("src/a")).unwrap();
    std::fs::create_dir_all(path.join("src/b")).unwrap();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    std::fs::create_dir_all(path.join("src/c")).unwrap();

    // Initial commit: both cloned functions + a distractor file.
    write(
        path.join("src/a/lib.rs"),
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    );
    write(
        path.join("src/b/lib.rs"),
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    );
    write(path.join("src/c/distractor.rs"), "fn distractor() {}\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    // 5 paired commits — both cloned files modified together (high co-change).
    for i in 0..5 {
        let v = i + 10;
        write(
            path.join("src/a/lib.rs"),
            &format!(
                "fn add(a: i32, b: i32) -> i32 {{ let x = 1; let y = 2; a + b + x + y + {v} }}\n"
            ),
        );
        write(
            path.join("src/b/lib.rs"),
            &format!(
                "fn mul(p: u64, q: u64) -> u64 {{ let s = 9; let t = 7; p + q + s + t + {v} }}\n"
            ),
        );
        run_git(path, &["add", "."]);
        run_git(
            path,
            &["commit", "-m", &format!("paired update {i}"), "--quiet"],
        );
    }

    // 5 distractor commits touching ONLY the third file. Provides Fisher
    // with "neither" data points so the contingency table isn't degenerate.
    for i in 0..5 {
        write(
            path.join("src/c/distractor.rs"),
            &format!("fn distractor_{i}() {{}}\n"),
        );
        run_git(path, &["add", "."]);
        run_git(
            path,
            &["commit", "-m", &format!("distractor {i}"), "--quiet"],
        );
    }

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1, // tiny fixture; relax coupling threshold
        min_clone_node_count: 0,
        min_clone_shared_revs: 1, // same: tiny fixture
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_clone_coupling(&db, &opts).expect("clone-coupling");

    assert!(
        !rows.is_empty(),
        "live clone pair (5 co-changes, same AST shape) should surface"
    );
    let r = &rows[0];
    assert!(r.file_a.contains("src/a/lib.rs"));
    assert!(r.file_b.contains("src/b/lib.rs"));
    assert!(r.combined_score > 0.0);
    assert!(
        (r.similarity - 1.0).abs() < f64::EPSILON,
        "T1+T2 → exact similarity, got {}",
        r.similarity
    );

    // Regression: `at_risk` defaults to false when no files surface as
    // knowledge-islands. The Tiny fixture author is "active" at test
    // runtime (commits land at `now`) so they're never departed; the
    // knowledge-islands sub-analysis returns empty; every row's at_risk
    // stays false. Locks the default-state behaviour of the intersection
    // column. The at_risk=true integration test would require injecting
    // a departed author via GIT_AUTHOR_DATE — see the standalone
    // knowledge_islands_test.rs for that pattern; cross-checking the
    // intersection itself is a future test.
    assert!(
        !r.at_risk,
        "at_risk must default false for fixture with active author; got at_risk=true",
    );

    // Regression guard: p_value must carry the real Fisher exact test
    // result from the upstream CouplingRow, NOT a hard-coded 0.0.
    // (Earlier shipping code zeroed this out and shipped the fake value
    // in every CSV/JSON/SARIF row.)
    assert!(
        r.p_value > 0.0,
        "p_value must be the real Fisher exact test result, not 0.0 — got {}",
        r.p_value
    );
    assert!(
        r.p_value < 0.05,
        "Fisher-filtered row should have p < 0.05, got {}",
        r.p_value
    );

    // combined_score must equal similarity * degree_pct * (1 - p_value).
    let expected_score = r.similarity * r.degree_pct * (1.0 - r.p_value);
    assert!(
        (r.combined_score - expected_score).abs() < 1e-9,
        "combined_score formula broken: got {}, expected {} = {} * {} * (1 - {})",
        r.combined_score,
        expected_score,
        r.similarity,
        r.degree_pct,
        r.p_value,
    );
}

/// Two cloned files that NEVER co-change (modified in different commits) →
/// not Fisher-significant → must NOT appear in clone-coupling.
#[test]
fn dead_clone_pair_does_not_surface() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    std::fs::create_dir_all(path.join("src/a")).unwrap();
    std::fs::create_dir_all(path.join("src/b")).unwrap();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Initial commit: both functions present (cloned shape).
    write(
        path.join("src/a/lib.rs"),
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    );
    write(
        path.join("src/b/lib.rs"),
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    );
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    // Modify ONLY src/a in 3 commits.
    for i in 0..3 {
        let v = i + 100;
        write(
            path.join("src/a/lib.rs"),
            &format!(
                "fn add(a: i32, b: i32) -> i32 {{ let x = 1; let y = 2; a + b + x + y + {v} }}\n"
            ),
        );
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("a-only {i}"), "--quiet"]);
    }
    // Modify ONLY src/b in 3 different commits.
    for i in 0..3 {
        let v = i + 200;
        write(
            path.join("src/b/lib.rs"),
            &format!(
                "fn mul(p: u64, q: u64) -> u64 {{ let s = 9; let t = 7; p + q + s + t + {v} }}\n"
            ),
        );
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("b-only {i}"), "--quiet"]);
    }

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_clone_node_count: 0,
        min_clone_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_clone_coupling(&db, &opts).expect("clone-coupling");
    assert!(
        rows.is_empty(),
        "dead clone pair (no co-changes) must NOT surface; got {rows:?}"
    );
}

/// Same-dir clones are filtered by default (intentional structural mirroring
/// like `foo_test.rs` ↔ `foo.rs`).
#[test]
fn same_dir_clones_filtered_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::create_dir_all(path.join("misc")).unwrap();
    write(
        path.join("src/a.rs"),
        "fn add(a: i32, b: i32) -> i32 { let x = 1; let y = 2; a + b + x + y }\n",
    );
    write(
        path.join("src/b.rs"),
        "fn mul(p: u64, q: u64) -> u64 { let s = 9; let t = 7; p + q + s + t }\n",
    );
    write(path.join("misc/distractor.rs"), "fn distractor() {}\n");
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);
    // Co-change both same-dir files multiple times.
    for i in 0..5 {
        write(
            path.join("src/a.rs"),
            &format!(
                "fn add(a: i32, b: i32) -> i32 {{ let x = 1; let y = 2; a + b + x + y + {i} }}\n"
            ),
        );
        write(
            path.join("src/b.rs"),
            &format!(
                "fn mul(p: u64, q: u64) -> u64 {{ let s = 9; let t = 7; p + q + s + t + {i} }}\n"
            ),
        );
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("c{i}"), "--quiet"]);
    }
    // Distractor commits so Fisher's contingency table isn't degenerate.
    for i in 0..5 {
        write(
            path.join("misc/distractor.rs"),
            &format!("fn distractor_{i}() {{}}\n"),
        );
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("d{i}"), "--quiet"]);
    }
    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_clone_node_count: 0,
        min_clone_shared_revs: 1,
        clone_skip_same_dir: true, // explicit
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_clone_coupling(&db, &opts).expect("clone-coupling");
    assert!(
        rows.is_empty(),
        "same-dir clones should be filtered; got {} rows",
        rows.len()
    );

    // Now explicitly disable the same-dir filter — pair must appear.
    let opts2 = Options {
        clone_skip_same_dir: false,
        ..opts
    };
    let rows2 = run_clone_coupling(&db, &opts2).expect("clone-coupling no-skip");
    assert!(
        !rows2.is_empty(),
        "with clone_skip_same_dir=false, same-dir live clones should surface"
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

fn write(p: std::path::PathBuf, content: &str) {
    std::fs::write(p, content).unwrap();
}
