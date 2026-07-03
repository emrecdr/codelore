use codelore_lib::Options;
use codelore_lib::analyses::refactoring_targets::run_refactoring_targets;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn opts_for(dir: &std::path::Path) -> Options {
    Options {
        repo_path: dir.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    }
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
        assert!(
            (0.0..=1.0).contains(&r.structural_risk),
            "risk in [0,1]: {}",
            r.structural_risk
        );
        // `loc` is the true file size (0 when no LOC data); the EA-Z effort
        // floor lives only in the priority denominator, so `priority` stays
        // finite regardless — verified by the non-negativity assert above.
        assert!(r.priority.is_finite(), "priority finite: {}", r.priority);
    }
    // Sorted by priority DESC.
    for w in rows.windows(2) {
        assert!(
            w[0].priority >= w[1].priority - 1e-9,
            "must be sorted by priority DESC"
        );
    }
}

#[test]
fn refactoring_targets_annotate_type_and_manualup() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = opts_for(tiny.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_refactoring_targets(&db, &opts).expect("run");
    assert!(!rows.is_empty());

    let known = [
        "complex-method",
        "large-method",
        "god-class",
        "dry",
        "shotgun-surgery",
        "none",
    ];
    for r in &rows {
        assert!(
            known.contains(&r.dominant_type.as_str()),
            "unknown type: {}",
            r.dominant_type
        );
        assert!(
            r.manual_up_rank >= 1,
            "manual_up_rank is 1-based: {}",
            r.manual_up_rank
        );
    }
    // manual_up_rank is a permutation of 1..=n.
    let mut ranks: Vec<u32> = rows.iter().map(|r| r.manual_up_rank).collect();
    ranks.sort_unstable();
    let expected: Vec<u32> = (1..=u32::try_from(rows.len()).unwrap()).collect();
    assert_eq!(
        ranks, expected,
        "manual_up_rank must be a permutation of 1..=n"
    );

    // ManualUp = ascending size, ties broken by path (the exact rank-1
    // selection criterion). `min_by_key(loc)` alone returns the first minimum
    // in priority order, which can differ from the path-tie-broken rank-1 file.
    let min_loc_row = rows
        .iter()
        .min_by(|a, b| a.loc.cmp(&b.loc).then_with(|| a.path.cmp(&b.path)))
        .unwrap();
    assert_eq!(
        min_loc_row.manual_up_rank, 1,
        "smallest file (path-tie-broken) is ManualUp rank 1"
    );
}

#[test]
fn refactoring_targets_csv_has_header_and_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = opts_for(tiny.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_refactoring_targets(&db, &opts).expect("run");

    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::csv::write_refactoring_targets_csv(&rows, &mut buf).expect("csv");
    let out = String::from_utf8(buf).expect("utf8");
    let header = out.lines().next().unwrap();
    assert_eq!(
        header,
        "entity,priority,combined_risk,structural_risk,hotspot_score,revisions,loc,dominant_type,band,manual_up_rank"
    );
    assert!(out.lines().count() >= 2, "header + >=1 data row");
}

#[test]
fn refactoring_targets_dominant_type_varies_on_biomarker_repo() {
    // On a fixture that fires several distinct smells, `dominant_type` must
    // actually resolve to real biomarkers (not stuck at "none") and vary by
    // file — the tiny_repo fixtures can't exercise this.
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: fx.dir.path().to_path_buf(),
        min_revs: 1,
        fisher_significance: 1.0,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_refactoring_targets(&db, &opts).expect("run");
    assert!(rows.len() >= 5, "fixture should yield several targets");

    let types: std::collections::HashSet<&str> =
        rows.iter().map(|r| r.dominant_type.as_str()).collect();
    assert!(
        types.iter().any(|t| *t != "none"),
        "at least one target must have a real dominant biomarker, got {types:?}"
    );
    assert!(
        types.len() >= 2,
        "dominant_type must vary across files, got {types:?}"
    );
}
