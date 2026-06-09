//! `authors` analysis tests — per-entity author breakdown (Bird et al.
//! 2011 risk indicator). The previous per-author leaderboard behaviour
//! is exercised in `top_committers_test.rs`.

use codelore_lib::Options;
use codelore_lib::analyses::authors::run_authors;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn authors_per_entity_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    // tiny_repo has two files: src/main.rs (touched 4 times) and
    // src/lib.rs (touched once). Single author for both.
    assert!(!rows.is_empty(), "expected at least one entity");
    for row in &rows {
        assert_eq!(
            row.n_authors, 1,
            "tiny_repo has one author, got n_authors={} on {}",
            row.n_authors, row.entity
        );
        assert_eq!(row.n_humans, 1, "Tiny is human");
        assert_eq!(row.n_bots, 0);
        assert!(row.n_revs >= 1);
        assert!(!row.last_modified.is_empty());
        assert!(!row.last_author.is_empty());
    }
}

#[test]
fn authors_per_entity_separates_humans_and_bots() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    assert!(
        !rows.is_empty(),
        "differential repo must produce at least one entity row"
    );

    // The fixture includes a Dependabot commit. Somewhere across the
    // repo the bot column must register > 0 — that's the whole point
    // of the codelore identity-layer enrichment.
    let total_bot_rows = rows.iter().filter(|r| r.n_bots > 0).count();
    assert!(
        total_bot_rows > 0,
        "expected at least one entity to have a bot author, got rows={rows:?}",
    );

    // For every row: humans + bots == total distinct authors (closed
    // partition — no author classified as both).
    for row in &rows {
        assert_eq!(
            row.n_humans + row.n_bots,
            row.n_authors,
            "humans+bots must partition n_authors for {}",
            row.entity,
        );
    }
}

#[test]
fn authors_rows_sort_by_n_authors_desc_then_n_revs() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    for w in rows.windows(2) {
        let primary_ok = w[0].n_authors > w[1].n_authors;
        let tied_n_authors = w[0].n_authors == w[1].n_authors;
        let secondary_ok = tied_n_authors && w[0].n_revs >= w[1].n_revs;
        assert!(
            primary_ok || secondary_ok,
            "sort invariant violated: {} (n_authors={}, n_revs={}) then {} (n_authors={}, n_revs={})",
            w[0].entity,
            w[0].n_authors,
            w[0].n_revs,
            w[1].entity,
            w[1].n_authors,
            w[1].n_revs,
        );
    }
}

#[test]
fn authors_rows_limit_caps_output() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        rows_limit: Some(2),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_authors(&db, &opts).expect("run authors");
    assert!(rows.len() <= 2, "rows_limit should cap output");
}
