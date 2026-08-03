//! Tests for the `coordination-needs` analysis.
//!
//! Three fixture scenarios:
//!
//! 1. **`tiny_repo`** — single author, no co-change edges. All rows must carry
//!    `tier = "single"` and `fragmentation = 0.0`.
//!
//! 2. **`coupling_repo`** — single author but guaranteed co-change edges
//!    (`src/alpha/svc.rs` ↔ `src/beta/svc.rs` co-change ≥5 times). Verifies
//!    that entropy > 0 for the coupled files, 0.0 for files with no co-change
//!    edges, and that all rows still carry `tier = "single"` (single author).
//!
//! 3. **`delivery_repo`** — 3 authors (Alice / Bob / Carol) touching overlapping
//!    files. Verifies fragmentation > 0 for files touched by multiple authors
//!    and that at least one file is not in `tier = "single"`.

use codelore_lib::Options;
use codelore_lib::analyses::coordination_needs::run_coordination_needs;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::gix_repo::GixRepo;

fn ingest(repo_path: &std::path::Path) -> FactsDb {
    let db = FactsDb::new_in_memory().expect("in-memory db");
    let repo = GixRepo::open(repo_path).expect("open repo");
    let opts = Options {
        repo_path: repo_path.to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    db
}

// ── 1. tiny_repo: all single-author, no co-change edges ─────────────────────

#[test]
fn tiny_repo_all_tiers_single() {
    let fixture = codelore_lib::test_support::tiny_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 365,
        ..Options::default()
    };

    // tiny_repo has no recognised source files → knowledge_shares may be empty;
    // run must not error.
    let rows = run_coordination_needs(&db, &opts).expect("run coordination-needs");
    for row in &rows {
        assert_eq!(
            row.tier, "single",
            "tiny_repo is single-author; every file must carry tier='single', got {row:?}",
        );
        assert!(
            row.fragmentation.abs() < f64::EPSILON,
            "single-author file must have fragmentation=0"
        );
    }
}

// ── 2. coupling_repo: co-change entropy + single-author tiers ───────────────

#[test]
fn coupling_repo_entropy_positive_for_coupled_files() {
    // coupling_repo: single author "Coupling" touches all files.
    // src/alpha/svc.rs and src/beta/svc.rs co-change ≥5 times in the window
    // → both must appear as co-change edges → entropy > 0.
    let fixture = codelore_lib::test_support::coupling_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 365,
        ..Options::default()
    };

    let rows = run_coordination_needs(&db, &opts).expect("run coordination-needs");
    assert!(
        !rows.is_empty(),
        "coupling_repo must produce at least one row"
    );

    // All rows are tier="single" (single author, even though there are edges).
    for row in &rows {
        assert_eq!(
            row.tier, "single",
            "coupling_repo is single-author; tier must be 'single' for {:?}",
            row.path
        );
    }

    // alpha/svc.rs and beta/svc.rs co-change in ≥5 commits → they must have
    // co-change edges → entropy > 0.
    let alpha_svc = rows
        .iter()
        .find(|r| r.path == "src/alpha/svc.rs")
        .expect("src/alpha/svc.rs must be present");
    let beta_svc = rows
        .iter()
        .find(|r| r.path == "src/beta/svc.rs")
        .expect("src/beta/svc.rs must be present");

    assert!(
        alpha_svc.cochange_entropy > 0.0,
        "src/alpha/svc.rs co-changes with beta/svc.rs; entropy must be > 0, got {}",
        alpha_svc.cochange_entropy
    );
    assert!(
        beta_svc.cochange_entropy > 0.0,
        "src/beta/svc.rs co-changes with alpha/svc.rs; entropy must be > 0, got {}",
        beta_svc.cochange_entropy
    );

    // All files have co-change entropy ≥ 0.0 — values are valid probabilities.
    for row in &rows {
        assert!(
            row.cochange_entropy >= 0.0,
            "co-change entropy must be non-negative for {:?}, got {}",
            row.path,
            row.cochange_entropy,
        );
    }

    // ── Hand-computed interleave for src/alpha/util.rs ───────────────────────
    // coupling_repo has a single author "Coupling" throughout.
    // Commits touching src/alpha/util.rs in chronological order:
    //   1. 2026-06-01 "seed all modules"  → author: Coupling
    //   2. 2026-06-07 "touch alpha/util"  → author: Coupling  (prev=Coupling, no switch)
    //   3. 2026-06-13 "alpha/util 2"      → author: Coupling  (prev=Coupling, no switch)
    //
    // switches = 0, n_commits = 3
    // interleave = switches / (n_commits - 1) = 0 / 2 = 0.0 exactly.
    let alpha_util = rows
        .iter()
        .find(|r| r.path == "src/alpha/util.rs")
        .expect("src/alpha/util.rs must be present");
    assert!(
        (alpha_util.interleave - 0.0_f64).abs() < 1e-9,
        "src/alpha/util.rs: 3 commits by same author → 0 switches → interleave=0.0, got {}",
        alpha_util.interleave
    );
    assert_eq!(
        alpha_util.total_commits, 3,
        "total_commits must disclose the interleave denominator's n, got {}",
        alpha_util.total_commits
    );
}

// ── 3. delivery_repo: multi-author fragmentation and non-single tiers ────────

#[test]
fn delivery_repo_fragmentation_and_nonsingle_tier() {
    // delivery_repo: Alice, Bob, Carol all touch src/core.rs (and other files).
    // After knowledge-share materialisation, the HHI complement for a file
    // touched by 3 authors must be > 0.
    let fixture = codelore_lib::test_support::delivery_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 365,
        ..Options::default()
    };

    let rows = run_coordination_needs(&db, &opts).expect("run coordination-needs");
    // delivery_repo has src/*.rs files (Rust, Tier-1) → complexity ingest fires
    // → knowledge_shares populated → at least one row.
    assert!(
        !rows.is_empty(),
        "delivery_repo must produce at least one coordination-needs row"
    );

    // At least one file must have authors > 1 and fragmentation > 0
    // (multiple authors touch src/core.rs).
    let multi_author = rows.iter().find(|r| r.authors > 1);
    assert!(
        multi_author.is_some(),
        "delivery_repo has 3 authors; at least one file must show authors > 1"
    );
    let multi = multi_author.unwrap();
    assert!(
        multi.fragmentation > 0.0,
        "multi-author file must have fragmentation > 0, got {}",
        multi.fragmentation
    );

    // At least one file must NOT be tier="single".
    let non_single = rows.iter().any(|r| r.tier != "single");
    assert!(
        non_single,
        "delivery_repo has multi-author files; at least one row must have tier != 'single'"
    );

    // ── Hand-computed interleave for src/rework.rs ────────────────────────────
    // delivery_repo commits touching src/rework.rs in chronological order:
    //   1. 2026-01-01 "seed: add initial files"   → author: Alice
    //   2. 2026-01-06 "feat: expand rework.rs"    → author: Alice  (prev=Alice, no switch)
    //   3. 2026-01-09 "refactor: trim rework.rs"  → author: Bob    (prev=Alice, switch!)
    //
    // switches = 1, n_commits = 3
    // interleave = switches / (n_commits - 1) = 1 / 2 = 0.5 exactly.
    //
    // src/rework.rs is a .rs file (Tier-1) with 3 commits and min_revs=1, so
    // complexity_metrics → knowledge_shares → coordination-needs chain fires.
    let rework = rows
        .iter()
        .find(|r| r.path == "src/rework.rs")
        .expect("src/rework.rs must appear: 3 commits, min_revs=1, Rust tier-1");
    assert!(
        (rework.interleave - 0.5_f64).abs() < 1e-9,
        "src/rework.rs: [Alice, Alice, Bob] → 1 switch / 2 intervals = 0.5, got {}",
        rework.interleave
    );
    assert_eq!(
        rework.total_commits, 3,
        "src/rework.rs: 3 commits total, got {}",
        rework.total_commits
    );
}

// ── 4-6. Hand-built FactsDb fixtures for SQL edge cases ─────────────────────
//
// The scenarios below (all-zero-loc_added history, a dormant multi-author
// file, exact fragmentation ties) are impractical to reproduce via a
// git-bundle fixture, so these tests build a minimal `FactsDb` directly via
// `execute_batch` (public API — see `facts::FactsDb`), bypassing git ingest
// entirely. `run_code_health_scoped`'s clone-detection walk tolerates a
// repo_path with no source files (empty tempdir): `health_band` is simply
// "unknown" for every row, which none of these tests assert on.

#[test]
fn binary_only_path_all_zero_loc_added_does_not_crash_and_has_zero_fragmentation() {
    // A path whose entire history is binary changes (loc_added=0 for every
    // commit) drives its only `knowledge_shares` row to k=0, hence
    // k_norm = 0 / NULLIF(0, 0) = NULL. Before the `frag` CTE's COALESCE
    // guard, `SUM(k_norm * k_norm)` over that all-NULL group was NULL, so
    // `fragmentation` was NULL — and `run_coordination_needs` errored out
    // reading that NULL column as a non-Option `f64`. Confirms (1) no
    // crash and (2) fragmentation is exactly 0.0 (degenerate-knowledge
    // default), mirroring `knowledge_islands`'s `HAVING SUM(loc) > 0`
    // defensive intent.
    let db = FactsDb::new_in_memory().expect("in-memory db");
    db.execute_batch(
        "INSERT INTO author_aliases (raw_name, raw_email, canonical, is_bot) \
         VALUES ('Alice', 'alice@x.com', 'Alice', false)",
    )
    .expect("insert author_aliases");
    db.execute_batch(
        "INSERT INTO commits (rev, author_email, author_name, \
         committer_email, canonical_author, date, committer_date, \
         message, is_merge, parent_count) VALUES \
         ('c1', 'alice@x.com', 'Alice', 'alice@x.com', 'Alice', \
          TIMESTAMP '2026-01-01', TIMESTAMP '2026-01-01', 'add binary', false, 1)",
    )
    .expect("insert commits");
    db.execute_batch(
        "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) \
         VALUES ('c1', 'assets/logo.png', 'binary', 0, 0)",
    )
    .expect("insert changes");

    let repo_dir = tempfile::tempdir().expect("tempdir");
    let opts = Options {
        repo_path: repo_dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 90,
        ..Options::default()
    };

    let rows = run_coordination_needs(&db, &opts)
        .expect("all-zero loc_added path must not crash run_coordination_needs");
    let row = rows
        .iter()
        .find(|r| r.path == "assets/logo.png")
        .expect("assets/logo.png must appear in results");
    assert!(
        row.fragmentation.abs() < f64::EPSILON,
        "all-zero-loc_added path must have fragmentation=0.0 (degenerate-knowledge \
         default), got {}",
        row.fragmentation
    );
}

#[test]
fn dormant_multi_author_file_is_not_classified_high() {
    // dormant.rs was touched by two authors (Alice, Bob) on the SAME
    // historical date — giving them identical decay factors, hence exactly
    // 0.5/0.5 knowledge shares (fragmentation == 0.50) — with one author
    // switch between them (interleave == 1.0). A later, unrelated commit
    // (c3, on other.rs) pushes the repo's anchor (MAX(date)) far enough
    // forward that BOTH of dormant.rs's commits fall outside the trailing
    // 90-day window: `authors` (which counts only active-window
    // contributors) is 0 for dormant.rs, even though its historical
    // fragmentation/interleave both clear the 0.50 "high" thresholds.
    // Before the authors<=1 guard, `authors == 0` fell through the
    // `single`-tier check straight into the `high` rule.
    let db = FactsDb::new_in_memory().expect("in-memory db");
    db.execute_batch(
        "INSERT INTO author_aliases (raw_name, raw_email, canonical, is_bot) VALUES \
         ('Alice', 'alice@x.com', 'Alice', false), \
         ('Bob', 'bob@x.com', 'Bob', false)",
    )
    .expect("insert author_aliases");
    db.execute_batch(
        "INSERT INTO commits (rev, author_email, author_name, \
         committer_email, canonical_author, date, committer_date, \
         message, is_merge, parent_count) VALUES \
         ('c1', 'alice@x.com', 'Alice', 'alice@x.com', 'Alice', \
          TIMESTAMP '2025-06-01', TIMESTAMP '2025-06-01', 'alice edit', false, 1), \
         ('c2', 'bob@x.com', 'Bob', 'bob@x.com', 'Bob', \
          TIMESTAMP '2025-06-01', TIMESTAMP '2025-06-01', 'bob edit', false, 1), \
         ('c3', 'alice@x.com', 'Alice', 'alice@x.com', 'Alice', \
          TIMESTAMP '2026-01-01', TIMESTAMP '2026-01-01', 'recent unrelated', false, 1)",
    )
    .expect("insert commits");
    db.execute_batch(
        "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) VALUES \
         ('c1', 'dormant.rs', 'added', 100, 0), \
         ('c2', 'dormant.rs', 'modified', 100, 0), \
         ('c3', 'other.rs', 'added', 10, 0)",
    )
    .expect("insert changes");

    let repo_dir = tempfile::tempdir().expect("tempdir");
    let opts = Options {
        repo_path: repo_dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 90,
        ..Options::default()
    };

    let rows = run_coordination_needs(&db, &opts).expect("run coordination-needs");
    let dormant = rows
        .iter()
        .find(|r| r.path == "dormant.rs")
        .expect("dormant.rs must appear in results");

    assert_eq!(
        dormant.authors, 0,
        "dormant.rs has no commits inside the trailing window; authors must be 0, got {dormant:?}"
    );
    assert!(
        dormant.fragmentation >= 0.50,
        "dormant.rs historical fragmentation must be >= 0.50 for this to be a \
         meaningful regression test, got {}",
        dormant.fragmentation
    );
    assert!(
        dormant.interleave >= 0.50,
        "dormant.rs historical interleave must be >= 0.50 for this to be a \
         meaningful regression test, got {}",
        dormant.interleave
    );
    assert_eq!(
        dormant.tier, "single",
        "a dormant file (authors=0, no CURRENT coordination activity) must fold into \
         the 'single' tier, not be misclassified 'high' from stale historical signal, \
         got {dormant:?}"
    );
    assert_eq!(
        dormant.total_commits, 2,
        "dormant.rs has exactly 2 historical commits (c1, c2) — total_commits must \
         disclose that thin sample, got {}",
        dormant.total_commits
    );
}

#[test]
fn equal_fragmentation_rows_are_ordered_deterministically_by_entity() {
    // Three single-author files (fragmentation == 0.0 for all three),
    // inserted in REVERSE alphabetical order on purpose. Before the
    // `entity ASC` tiebreak (both in the SQL `ORDER BY` and the Rust
    // `sort_by`), ties broke on whatever order the SQL engine happened to
    // produce — unspecified. After the fix, the final `Vec`'s tiebreak is
    // fully determined by `path ASC` regardless of insertion/incoming order.
    let db = FactsDb::new_in_memory().expect("in-memory db");
    db.execute_batch(
        "INSERT INTO author_aliases (raw_name, raw_email, canonical, is_bot) \
         VALUES ('Solo', 'solo@x.com', 'Solo', false)",
    )
    .expect("insert author_aliases");
    db.execute_batch(
        "INSERT INTO commits (rev, author_email, author_name, \
         committer_email, canonical_author, date, committer_date, \
         message, is_merge, parent_count) VALUES \
         ('c1', 'solo@x.com', 'Solo', 'solo@x.com', 'Solo', \
          TIMESTAMP '2026-01-01', TIMESTAMP '2026-01-01', 'z', false, 1), \
         ('c2', 'solo@x.com', 'Solo', 'solo@x.com', 'Solo', \
          TIMESTAMP '2026-01-02', TIMESTAMP '2026-01-02', 'm', false, 1), \
         ('c3', 'solo@x.com', 'Solo', 'solo@x.com', 'Solo', \
          TIMESTAMP '2026-01-03', TIMESTAMP '2026-01-03', 'a', false, 1)",
    )
    .expect("insert commits");
    db.execute_batch(
        "INSERT INTO changes (rev, path, change_type, loc_added, loc_deleted) VALUES \
         ('c1', 'z_file.rs', 'added', 10, 0), \
         ('c2', 'm_file.rs', 'added', 10, 0), \
         ('c3', 'a_file.rs', 'added', 10, 0)",
    )
    .expect("insert changes");

    let repo_dir = tempfile::tempdir().expect("tempdir");
    let opts = Options {
        repo_path: repo_dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 90,
        ..Options::default()
    };

    let rows = run_coordination_needs(&db, &opts).expect("run coordination-needs");
    assert_eq!(
        rows.len(),
        3,
        "expected exactly the 3 fixture paths, got {rows:?}"
    );
    for row in &rows {
        assert!(
            row.fragmentation.abs() < f64::EPSILON,
            "single-author file must have fragmentation=0, got {row:?}"
        );
    }
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["a_file.rs", "m_file.rs", "z_file.rs"],
        "tied fragmentation rows must be ordered deterministically by entity ASC"
    );
}

/// `cochange_entropy` is a floating-point SUM. `DuckDB` parallelises aggregation
/// and float addition is non-associative, so an unordered SUM can wobble by
/// ~1 ULP between runs and break byte-for-byte reproducibility. The ordered
/// aggregate in the entropy CTE pins one value, so re-running the analysis
/// must yield byte-identical rows every time. Uses `coupling_repo` because it
/// guarantees real co-change edges (non-trivial entropy terms).
#[test]
fn coordination_needs_output_is_run_to_run_identical() {
    let fixture = codelore_lib::test_support::coupling_repo::build();
    let db = ingest(fixture.dir.path());
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        window_days: 365,
        ..Options::default()
    };

    // Debug renders each f64 at shortest round-trip precision (Ryū), so a
    // 1-ULP drift in any entropy term produces a different string — the
    // comparison is exact at float granularity, no serde dependency needed.
    let baseline = format!(
        "{:?}",
        run_coordination_needs(&db, &opts).expect("run coordination-needs")
    );
    assert!(
        baseline.contains("cochange_entropy"),
        "fixture must exercise the entropy term"
    );
    for i in 0..8 {
        let again = format!(
            "{:?}",
            run_coordination_needs(&db, &opts).expect("re-run coordination-needs")
        );
        assert_eq!(
            again, baseline,
            "coordination-needs output must be identical across runs (iteration {i})"
        );
    }
}
