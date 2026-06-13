//! `centrality` analysis integration tests.
//!
//! Validates the degree + weighted-degree formulation against a fixture
//! whose Fisher-significant coupling structure is small enough to verify
//! by hand.

use codelore_lib::Options;
use codelore_lib::analyses::centrality::run_centrality;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::facts::FactsDb;
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

/// Three files (`a`, `b`, `c`) co-change in 12 commits, then 15 unrelated
/// noise commits touch fresh single files. This shape gives every
/// `{a,b}`, `{a,c}`, `{b,c}` pair a 12-of-27 shared count over a 27-rev
/// universe, which Fisher's exact test clears at the default p=0.05.
/// Expected: every trio path has degree=2 (paired with each of the other
/// two), weighted_degree > 0.
#[test]
fn degree_equals_count_of_fisher_significant_partners() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // 12 trio commits — all three files together every time. Sample size
    // is what gives Fisher significance: 12/27 shared with only 12 revs
    // per file is a vast deviation from the 12*12/27 ≈ 5.3 expected
    // under independence.
    for i in 0..12 {
        write(path.join("a.txt"), &format!("v{i}\n"));
        write(path.join("b.txt"), &format!("v{i}\n"));
        write(path.join("c.txt"), &format!("v{i}\n"));
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("trio-{i}"), "--quiet"]);
    }
    // 15 noise commits — each touches a unique never-seen file so the
    // overall commit universe expands without contributing to any
    // shared-count tally. Keeps `total` large enough for Fisher to
    // distinguish real coupling from background co-occurrence.
    for i in 0..15 {
        let name = format!("noise_{i}.txt");
        write(path.join(&name), "x\n");
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("noise-{i}"), "--quiet"]);
    }

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        min_shared_revs: 0,
        min_coupling_pct: 0,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let pairs = run_coupling(&db, &opts).expect("coupling");
    assert!(
        pairs.len() >= 3,
        "expected ≥3 Fisher-significant pairs, got {}",
        pairs.len()
    );

    let rows = run_centrality(&db, &opts).expect("centrality");
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.entity == name)
            .unwrap_or_else(|| panic!("expected entity `{name}` in centrality output"))
    };

    assert_eq!(row("a.txt").degree, 2);
    assert_eq!(row("b.txt").degree, 2);
    assert_eq!(row("c.txt").degree, 2);

    assert!(row("a.txt").weighted_degree > 0.0);
    assert!(row("b.txt").weighted_degree > 0.0);
    assert!(row("c.txt").weighted_degree > 0.0);
}

/// Empty repo: no commits → no coupling pairs → empty centrality output.
#[test]
fn empty_repo_yields_empty_centrality() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Seed one empty commit so gix can walk; no file changes means no
    // coupling pairs.
    run_git(path, &["commit", "--allow-empty", "-m", "seed", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_centrality(&db, &opts).expect("centrality");
    assert!(
        rows.is_empty(),
        "expected empty centrality output, got {} rows",
        rows.len()
    );
}

/// `--rows N` truncates the OUTPUT, not the inner coupling computation:
/// a small `--rows 1` must not change degree values for the surviving
/// row vs an uncapped run.
#[test]
fn rows_limit_does_not_skew_inner_degree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    for i in 0..12 {
        write(path.join("a.txt"), &format!("v{i}\n"));
        write(path.join("b.txt"), &format!("v{i}\n"));
        write(path.join("c.txt"), &format!("v{i}\n"));
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("trio-{i}"), "--quiet"]);
    }
    for i in 0..15 {
        let name = format!("noise_{i}.txt");
        write(path.join(&name), "x\n");
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("noise-{i}"), "--quiet"]);
    }

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts_full = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        min_shared_revs: 0,
        min_coupling_pct: 0,
        ..Options::default()
    };
    let mut opts_capped = opts_full.clone();
    opts_capped.rows_limit = Some(1);

    db.ingest(&repo, &opts_full).expect("ingest");

    let full = run_centrality(&db, &opts_full).expect("centrality full");
    let capped = run_centrality(&db, &opts_capped).expect("centrality capped");

    assert_eq!(capped.len(), 1, "rows_limit should truncate to 1");
    let top_full = full.first().expect("full output must have ≥1 row");
    let top_capped = capped.first().expect("capped output has the survivor");
    assert_eq!(top_capped.entity, top_full.entity);
    assert_eq!(top_capped.degree, top_full.degree);
    assert!(
        (top_capped.weighted_degree - top_full.weighted_degree).abs() < 1e-9,
        "weighted_degree must be identical regardless of output truncation"
    );
}
