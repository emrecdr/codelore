use codelore_lib::analyses::health_trend::{HealthTrendRow, health_band, run_health_trend};

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
