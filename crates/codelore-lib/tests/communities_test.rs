//! End-to-end coverage for `run_communities` over a real ingested repo.
//!
//! `analyses/communities.rs` only had inline tests that replicate the
//! partition body against hand-built `CouplingRow` slices; nothing drove
//! the public `run_communities(db, opts)` entry point. This binary ingests
//! the differential fixture, runs Leiden detection on its coupling graph,
//! and asserts the structural contract of the returned partition.

use codelore_lib::Options;
use codelore_lib::analyses::communities::run_communities;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use std::collections::{HashMap, HashSet};

/// Coupling-permissive options so the fixture's co-change pairs survive
/// into a non-empty graph for Leiden to partition.
fn permissive_opts(repo_path: std::path::PathBuf) -> Options {
    Options {
        repo_path,
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        fisher_significance: 1.0,
        ..Options::default()
    }
}

#[test]
fn communities_partition_is_a_valid_dense_cover() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = permissive_opts(fixture.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let pairs = run_coupling(&db, &opts).expect("coupling oracle");
    assert!(
        !pairs.is_empty(),
        "differential fixture should yield ≥1 coupling pair under permissive options"
    );
    let result = run_communities(&db, &opts).expect("run communities");
    assert!(
        !result.rows.is_empty(),
        "a non-empty coupling graph must produce ≥1 community row"
    );

    // Invariant 1: the partitioned node set equals the set of files in any
    // coupling pair — every coupled file lands in exactly one community,
    // and no extra nodes appear.
    let oracle_paths: HashSet<&str> = pairs
        .iter()
        .flat_map(|p| [p.entity_a.as_str(), p.entity_b.as_str()])
        .collect();
    let row_paths: HashSet<&str> = result.rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        oracle_paths, row_paths,
        "community node set must equal the set of files in coupling pairs"
    );
    assert_eq!(
        row_paths.len(),
        result.rows.len(),
        "each file must appear in exactly one community row (no duplicate paths)"
    );

    // Invariant 2: community IDs are a dense 0..k range and `community_count`
    // matches the number of distinct IDs.
    let distinct_ids: HashSet<u32> = result.rows.iter().map(|r| r.community_id).collect();
    assert_eq!(
        u32::try_from(distinct_ids.len()).expect("id count fits u32"),
        result.community_count,
        "community_count must equal the number of distinct community IDs"
    );
    let mut sorted_ids: Vec<u32> = distinct_ids.into_iter().collect();
    sorted_ids.sort_unstable();
    let expected_dense: Vec<u32> = (0..result.community_count).collect();
    assert_eq!(
        sorted_ids, expected_dense,
        "community IDs must form a dense 0..k range after renumbering"
    );

    // Invariant 3: `community_size` on each row equals the real membership
    // count of that community.
    let mut size_of: HashMap<u32, u32> = HashMap::new();
    for row in &result.rows {
        *size_of.entry(row.community_id).or_default() += 1;
    }
    for row in &result.rows {
        assert_eq!(
            row.community_size, size_of[&row.community_id],
            "community_size mismatch for {} (community {})",
            row.path, row.community_id
        );
    }

    // Invariant 4: modularity is a valid quality score in [-1, 1].
    assert!(
        (-1.0..=1.0).contains(&result.modularity) && result.modularity.is_finite(),
        "modularity out of range: {}",
        result.modularity
    );

    // Invariant 5: rows are sorted by (community_id ASC, path ASC) — the
    // stable-diff ordering contract.
    for w in result.rows.windows(2) {
        let ordered = w[0].community_id < w[1].community_id
            || (w[0].community_id == w[1].community_id && w[0].path <= w[1].path);
        assert!(
            ordered,
            "rows must be sorted by (community_id, path): ({}, {}) before ({}, {})",
            w[0].community_id, w[0].path, w[1].community_id, w[1].path
        );
    }
}

#[test]
fn communities_are_deterministic_across_runs() {
    // The seeded Leiden config promises identical partitions across runs;
    // two back-to-back invocations against the same fixture must agree on
    // the (path → community_id) assignment.
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = permissive_opts(fixture.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let first = run_communities(&db, &opts).expect("first run");
    let second = run_communities(&db, &opts).expect("second run");
    assert_eq!(
        first.community_count, second.community_count,
        "community_count drifted across runs"
    );
    assert!((first.modularity - second.modularity).abs() < 1e-12);

    let map = |r: &codelore_lib::analyses::communities::CommunitiesResult| {
        r.rows
            .iter()
            .map(|row| (row.path.clone(), row.community_id))
            .collect::<HashMap<_, _>>()
    };
    assert_eq!(
        map(&first),
        map(&second),
        "seeded Leiden must produce identical community assignments across runs"
    );
}
