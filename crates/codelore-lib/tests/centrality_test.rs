//! End-to-end coverage for `run_centrality` over a real ingested repo.
//!
//! The inline unit tests in `analyses/centrality.rs` exercise
//! `compute_centrality` on hand-built `CouplingRow` slices; this binary
//! drives the full `ingest → run_coupling → run_centrality` path and
//! cross-checks the network metrics against the coupling oracle they are
//! derived from.

use codelore_lib::Options;
use codelore_lib::analyses::centrality::run_centrality;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;
use std::collections::{HashMap, HashSet};

#[test]
fn centrality_metrics_agree_with_coupling_oracle() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let pairs = run_coupling(&db, &opts).expect("coupling oracle");
    assert!(
        !pairs.is_empty(),
        "differential fixture should yield ≥1 coupling pair under permissive options"
    );
    let rows = run_centrality(&db, &opts).expect("run centrality");
    assert!(
        !rows.is_empty(),
        "a non-empty coupling graph must produce ≥1 centrality row"
    );

    // Invariant 1: the node set is exactly the set of files appearing in
    // any coupling pair. A row for a path with no edges (or a missing row
    // for a coupled path) would mean the graph build diverged from the
    // oracle.
    let oracle_paths: HashSet<&str> = pairs
        .iter()
        .flat_map(|p| [p.entity_a.as_str(), p.entity_b.as_str()])
        .collect();
    let row_paths: HashSet<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        oracle_paths, row_paths,
        "centrality node set must equal the set of files in coupling pairs"
    );

    // Invariant 2: each file's unweighted `degree` equals its number of
    // distinct coupling partners (each undirected pair contributes one to
    // both endpoints).
    let mut expected_degree: HashMap<&str, u32> = HashMap::new();
    for p in &pairs {
        *expected_degree.entry(p.entity_a.as_str()).or_default() += 1;
        *expected_degree.entry(p.entity_b.as_str()).or_default() += 1;
    }
    for row in &rows {
        assert_eq!(
            row.degree,
            expected_degree[row.path.as_str()],
            "degree mismatch for {} vs coupling-pair count",
            row.path
        );
    }

    // Invariant 3: PageRank conserves L1 mass (teleport + dangling
    // redistribution keep Σ rank = 1).
    let pr_sum: f64 = rows.iter().map(|r| r.pagerank).sum();
    assert!(
        (pr_sum - 1.0).abs() < 1e-6,
        "PageRank should sum to 1, got {pr_sum}"
    );

    // Invariant 4: the eigenvector vector is L2-normalised by construction.
    let ev_norm_sq: f64 = rows.iter().map(|r| r.eigenvector * r.eigenvector).sum();
    assert!(
        (ev_norm_sq - 1.0).abs() < 1e-6,
        "eigenvector L2 norm² should be 1, got {ev_norm_sq}"
    );

    // Invariant 5: rows are emitted PageRank-descending (deterministic
    // ordering contract).
    for w in rows.windows(2) {
        assert!(
            w[0].pagerank >= w[1].pagerank,
            "rows must be sorted by PageRank descending: {} ({}) < {} ({})",
            w[0].path,
            w[0].pagerank,
            w[1].path,
            w[1].pagerank,
        );
    }

    // Invariant 6: every score is finite (no NaN/Inf leaking from the
    // power-iteration kernels) and non-negative (Perron-Frobenius on the
    // undirected, shifted graph).
    for row in &rows {
        assert!(
            row.weighted_degree.is_finite() && row.weighted_degree >= 0.0,
            "weighted_degree out of range for {}: {}",
            row.path,
            row.weighted_degree
        );
        assert!(
            row.pagerank.is_finite() && row.pagerank >= 0.0,
            "pagerank out of range for {}: {}",
            row.path,
            row.pagerank
        );
        assert!(
            row.eigenvector.is_finite() && row.eigenvector >= 0.0,
            "eigenvector out of range for {}: {}",
            row.path,
            row.eigenvector
        );
    }
}

#[test]
fn centrality_empty_when_no_significant_coupling() {
    // `fisher_significance = 0.0` rejects every pair (p-values are strictly
    // positive), so the coupling graph is empty and centrality yields no
    // rows — the empty-graph branch, not an error.
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        fisher_significance: 0.0,
        ..permissive_coupling_opts(fixture.dir.path().to_path_buf())
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_centrality(&db, &opts).expect("run centrality");
    assert!(
        rows.is_empty(),
        "no Fisher-significant pairs should yield an empty centrality result, got {} rows",
        rows.len()
    );
}
