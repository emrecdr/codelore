//! Leiden `communities` analysis integration tests.
//!
//! Validates the partition shape against a fixture whose community
//! structure is small enough to verify by hand: three disjoint
//! triangles (`{a1,a2,a3}`, `{b1,b2,b3}`, `{c1,c2,c3}`) co-changing
//! within their own group plus 20 noise commits that touch fresh
//! singleton files so the Fisher significance threshold is clearly
//! discriminating intra-group from inter-group co-occurrence.

use codelore_lib::Options;
use codelore_lib::analyses::communities::run_communities;
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

fn commit_group(path: &std::path::Path, files: &[&str], label: &str, count: usize) {
    for i in 0..count {
        for f in files {
            write(path.join(f), &format!("{label}-{i}\n"));
        }
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", &format!("{label}-{i}"), "--quiet"]);
    }
}

#[test]
fn three_disjoint_cliques_yield_three_communities_with_positive_modularity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // 12 commits per triangle — large enough sample for Fisher exact to
    // gate intra-triangle pairs as significant while inter-triangle
    // pairs (none — different commits) remain absent from the coupling
    // pair list entirely.
    commit_group(path, &["A1.txt", "A2.txt", "A3.txt"], "A", 12);
    commit_group(path, &["B1.txt", "B2.txt", "B3.txt"], "B", 12);
    commit_group(path, &["C1.txt", "C2.txt", "C3.txt"], "C", 12);

    // Noise: 20 commits each touching a fresh singleton file. Expands
    // the commit universe so the Fisher null model has enough "non-A
    // commits" to estimate against.
    for i in 0..20 {
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

    let (rows, stats) = run_communities(&db, &opts).expect("communities");

    // Each triangle file appears in exactly one community.
    assert_eq!(stats.num_nodes, 9, "expected 9 trio files as nodes");
    assert_eq!(
        stats.num_communities, 3,
        "expected 3 communities, got {}",
        stats.num_communities
    );

    // Modularity Q on a perfectly-modular three-clique graph should be
    // very high (theoretical max for k disjoint cliques is `(k-1)/k`,
    // so ≈ 0.67 for k=3). Allowing 0.5 as a conservative lower bound
    // covers any minor weight-rounding effects.
    assert!(
        stats.modularity > 0.5,
        "expected modularity > 0.5 on a 3-clique graph, got {}",
        stats.modularity,
    );

    // Each community has exactly 3 members. The exact community_id
    // assignment depends on graph layout, so we verify by triangle
    // membership.
    let community_of = |entity: &str| {
        rows.iter()
            .find(|r| r.entity == entity)
            .unwrap_or_else(|| panic!("entity `{entity}` missing"))
            .community_id
    };
    assert_eq!(community_of("A1.txt"), community_of("A2.txt"));
    assert_eq!(community_of("A2.txt"), community_of("A3.txt"));
    assert_eq!(community_of("B1.txt"), community_of("B2.txt"));
    assert_eq!(community_of("B2.txt"), community_of("B3.txt"));
    assert_eq!(community_of("C1.txt"), community_of("C2.txt"));
    assert_eq!(community_of("C2.txt"), community_of("C3.txt"));

    // Different triangles → different communities.
    assert_ne!(community_of("A1.txt"), community_of("B1.txt"));
    assert_ne!(community_of("A1.txt"), community_of("C1.txt"));
    assert_ne!(community_of("B1.txt"), community_of("C1.txt"));

    // Pure intra-cluster co-change ⇒ all edge strength is intra,
    // inter is zero per node.
    for row in &rows {
        assert!(
            row.inter_strength.abs() < 1e-9,
            "expected zero inter_strength for disjoint triangles, got {} for `{}`",
            row.inter_strength,
            row.entity,
        );
        assert!(
            row.intra_strength > 0.0,
            "expected positive intra_strength, got {} for `{}`",
            row.intra_strength,
            row.entity,
        );
    }
}

#[test]
fn empty_repo_yields_empty_partition() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    run_git(path, &["commit", "--allow-empty", "-m", "seed", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let (rows, stats) = run_communities(&db, &opts).expect("communities");
    assert!(rows.is_empty());
    assert_eq!(stats.num_nodes, 0);
    assert_eq!(stats.num_communities, 0);
    assert_eq!(stats.num_edges, 0);
}

/// Determinism: two runs on the same fact store must produce the same
/// modularity score and the same per-node community partition (after
/// canonical renumbering). The seeded RNG is the load-bearing piece.
#[test]
fn deterministic_across_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    commit_group(path, &["A1.txt", "A2.txt", "A3.txt"], "A", 10);
    commit_group(path, &["B1.txt", "B2.txt", "B3.txt"], "B", 10);
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

    let (rows1, stats1) = run_communities(&db, &opts).expect("communities 1");
    let (rows2, stats2) = run_communities(&db, &opts).expect("communities 2");

    assert!((stats1.modularity - stats2.modularity).abs() < 1e-12);
    assert_eq!(stats1.num_communities, stats2.num_communities);
    assert_eq!(rows1.len(), rows2.len());
    for (a, b) in rows1.iter().zip(rows2.iter()) {
        assert_eq!(a.entity, b.entity);
        assert_eq!(a.community_id, b.community_id);
    }
}
