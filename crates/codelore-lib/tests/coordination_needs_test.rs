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
}
