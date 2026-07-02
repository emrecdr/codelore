use codelore_lib::Options;
use codelore_lib::analyses::refactoring_targets::run_refactoring_targets;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn opts_for(dir: &std::path::Path) -> Options {
    Options { repo_path: dir.to_path_buf(), min_revs: 1, ..Options::default() }
}

#[test]
fn refactoring_targets_ranks_by_priority_desc() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = opts_for(tiny.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_refactoring_targets(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "tiny_repo should yield >=1 target");
    for r in &rows {
        assert!(r.priority >= 0.0, "priority non-negative: {}", r.priority);
        assert!((0.0..=1.0).contains(&r.structural_risk), "risk in [0,1]: {}", r.structural_risk);
        assert!(r.loc >= 1, "loc floored >=1: {}", r.loc);
    }
    // Sorted by priority DESC.
    for w in rows.windows(2) {
        assert!(w[0].priority >= w[1].priority - 1e-9, "must be sorted by priority DESC");
    }
}
