use codelore_lib::analyses::health_trend::{
    HealthTrendRow, health_band, run_health_trend, run_health_trend_detail, run_sample_trends,
};

fn all_scores_in_range(rows: &[HealthTrendRow]) {
    for r in rows {
        for (name, v) in [
            ("arch", r.arch_health),
            ("code", r.code_health),
            ("combined", r.combined_health),
        ] {
            assert!(
                (0.0..=100.0).contains(&v),
                "{name} health out of range for {}: {v}",
                r.rev
            );
        }
        assert_eq!(r.arch_band, health_band(r.arch_health));
        assert_eq!(r.code_band, health_band(r.code_health));
        assert_eq!(r.combined_band, health_band(r.combined_health));
        // combined is exactly the mean of the two.
        assert!((r.combined_health - 0.5 * (r.arch_health + r.code_health)).abs() < 1e-9);
    }
}

#[test]
fn health_trend_produces_a_row_per_sample_oldest_first() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_health_trend(&db, &repo, &opts).expect("health-trend");
    assert!(
        !rows.is_empty(),
        "fixture with >=2 commits must yield samples"
    );
    all_scores_in_range(&rows);

    // Oldest-first by date (non-decreasing).
    for w in rows.windows(2) {
        assert!(w[0].date <= w[1].date, "rows must be oldest-first");
    }
}

#[test]
fn detail_trend_matches_wrapper_and_transitions_are_valid() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    // run_health_trend is now a thin wrapper — trend field must be byte-equal.
    let wrapper = run_health_trend(&db, &repo, &opts).expect("wrapper");
    // Re-open a fresh db for the detail call (single connection is
    // consumed by the wrapper's inner detail run).
    let repo2 = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open2");
    let db2 = codelore_lib::facts::FactsDb::new_in_memory().expect("db2");
    db2.ingest(&repo2, &opts).expect("ingest2");
    let detail = run_health_trend_detail(&db2, &repo2, &opts).expect("detail");

    assert_eq!(wrapper.len(), detail.trend.len(), "trend lengths match");
    for (w, d) in wrapper.iter().zip(detail.trend.iter()) {
        assert_eq!(w.date, d.date);
        assert_eq!(w.rev, d.rev);
        assert!((w.arch_health - d.arch_health).abs() < 1e-9);
        assert!((w.code_health - d.code_health).abs() < 1e-9);
        assert!((w.combined_health - d.combined_health).abs() < 1e-9);
    }

    // file_series: every entry has a valid band and YYYY-MM-DD date.
    // Note: CodeHealthRow::band is derived from structural_risk thresholds, not
    // from the 0–100 score, so health_band(score) != band is expected.
    let valid_bands = ["red", "yellow", "green"];
    for pt in &detail.file_series {
        assert!(
            valid_bands.contains(&pt.band.as_str()),
            "unexpected band {:?} for path {} score={}",
            pt.band,
            pt.path,
            pt.score,
        );
        assert!(
            (0.0..=100.0).contains(&pt.score),
            "score out of range for {}: {}",
            pt.path,
            pt.score
        );
        assert!(
            pt.date.len() >= 10 && pt.date.chars().nth(4) == Some('-'),
            "date not YYYY-MM-DD: {}",
            pt.date
        );
    }

    // transitions: every entry has valid direction and bands.
    let valid_bands = ["red", "yellow", "green"];
    let valid_dirs = ["improved", "regressed"];
    for tr in &detail.transitions {
        assert!(
            valid_bands.contains(&tr.from_band.as_str()),
            "bad from_band: {}",
            tr.from_band
        );
        assert!(
            valid_bands.contains(&tr.to_band.as_str()),
            "bad to_band: {}",
            tr.to_band
        );
        assert!(
            valid_dirs.contains(&tr.direction.as_str()),
            "bad direction: {}",
            tr.direction
        );
        assert_ne!(
            tr.from_band, tr.to_band,
            "transitions must be actual changes"
        );
    }
}

#[test]
fn shared_driver_matches_standalone_trends() {
    use codelore_lib::analyses::architecture_trend::run_architecture_trend;

    // One fixture; each analysis runs on its own freshly-ingested db so temp
    // tables from one call never bleed into another (matches the wrapper test).
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let opts = codelore_lib::test_support::permissive_coupling_opts(fx.dir.path().to_path_buf());
    let fresh = || {
        let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open");
        let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
        db.ingest(&repo, &opts).expect("ingest");
        (repo, db)
    };

    let (repo_s, db_s) = fresh();
    let shared = run_sample_trends(&db_s, &repo_s, &opts).expect("sample-trends");

    let (repo_a, db_a) = fresh();
    let arch = run_architecture_trend(&db_a, &repo_a, &opts).expect("arch-trend");

    let (repo_h, db_h) = fresh();
    let health = run_health_trend_detail(&db_h, &repo_h, &opts).expect("health-detail");

    // Architecture-decay rows from the shared driver equal the standalone run.
    assert_eq!(shared.architecture.len(), arch.len(), "arch row count");
    for (s, a) in shared.architecture.iter().zip(arch.iter()) {
        assert_eq!(s.date, a.date);
        assert_eq!(s.rev, a.rev);
        assert_eq!(s.files, a.files);
        assert!(
            (s.propagation_cost - a.propagation_cost).abs() < 1e-12,
            "propagation_cost {} vs {} at {}",
            s.propagation_cost,
            a.propagation_cost,
            s.rev
        );
        assert_eq!(s.cycle_count, a.cycle_count);
        assert_eq!(s.largest_cycle, a.largest_cycle);
    }

    // Health view from the shared driver equals the standalone detail run.
    assert_eq!(shared.health.trend.len(), health.trend.len(), "trend rows");
    for (s, h) in shared.health.trend.iter().zip(health.trend.iter()) {
        assert_eq!(s.date, h.date);
        assert_eq!(s.rev, h.rev);
        assert_eq!(s.files, h.files);
        assert!((s.arch_health - h.arch_health).abs() < 1e-12);
        assert!((s.code_health - h.code_health).abs() < 1e-12);
        assert!((s.combined_health - h.combined_health).abs() < 1e-12);
    }
    assert_eq!(
        shared.health.file_series.len(),
        health.file_series.len(),
        "file_series length"
    );
    assert_eq!(
        shared.health.transitions.len(),
        health.transitions.len(),
        "transitions length"
    );
}
