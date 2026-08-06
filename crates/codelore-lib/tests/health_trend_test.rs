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

// ---------------------------------------------------------------------------
// arch_health must actually FALL as the import graph decays.
//
// Every other test here runs on `biomarker_repo`, which is six independent
// Rust files with no inter-file imports. Its import graph is empty, so
// `arch_health` is pinned at 100 in every sample and no assertion over that
// fixture can distinguish a working architecture score from a constant. This
// fixture introduces a dependency cycle partway through history — the same
// shape `architecture_trend`'s `trend_captures_cycle_introduction_over_time`
// uses — so the column is exercised where it is supposed to move.
// ---------------------------------------------------------------------------

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed");
}

fn write_file(root: &std::path::Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

fn commit_on(dir: &std::path::Path, day: u32, msg: &str) {
    git_in(dir, &["add", "."]);
    let stamp = format!("2026-02-{day:02}T12:00:00Z");
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit {msg} failed");
}

#[test]
fn arch_health_falls_when_a_cycle_enters_the_import_graph() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git_in(p, &["init", "-b", "main", "--quiet"]);
    git_in(p, &["config", "user.email", "t@example.com"]);
    git_in(p, &["config", "user.name", "T"]);

    write_file(
        p,
        "Cargo.toml",
        "[package]\nname=\"t\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    write_file(p, "src/lib.rs", "pub mod a;\npub mod b;\n");
    // Acyclic: a depends on b, b depends on nothing.
    write_file(p, "src/a.rs", "use crate::b;\npub fn a() { b::b(); }\n");
    write_file(p, "src/b.rs", "pub fn b() {}\n");
    commit_on(p, 1, "acyclic");

    // Pad the acyclic era so the even sampler lands at least one point
    // before the cycle exists.
    for (i, day) in [2u32, 3, 4, 5].iter().enumerate() {
        write_file(
            p,
            "src/a.rs",
            &format!("use crate::b;\npub fn a() {{ let _ = {i}; b::b(); }}\n"),
        );
        commit_on(p, *day, "edit a (still acyclic)");
    }

    // Close the loop: b now depends on a.
    write_file(p, "src/b.rs", "use crate::a;\npub fn b() { a::a(); }\n");
    commit_on(p, 6, "introduce a<->b cycle");
    for (i, day) in [7u32, 8, 9, 10].iter().enumerate() {
        write_file(
            p,
            "src/b.rs",
            &format!("use crate::a;\npub fn b() {{ let _ = {i}; a::a(); }}\n"),
        );
        commit_on(p, *day, "edit b (cyclic)");
    }

    let repo = codelore_lib::repo::GixRepo::open(p).expect("open repo");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(p.to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_health_trend(&db, &repo, &opts).expect("run health-trend");
    assert!(rows.len() >= 4, "expected several sample points: {rows:?}");
    all_scores_in_range(&rows);

    // The point of the fixture: the score must not be constant. On
    // `biomarker_repo` this assertion passes vacuously, which is why it needs
    // a graph-bearing repo to mean anything.
    let best = rows.iter().map(|r| r.arch_health).fold(f64::MIN, f64::max);
    let worst = rows.iter().map(|r| r.arch_health).fold(f64::MAX, f64::min);
    assert!(
        best > worst,
        "arch_health is constant across the trend, so a decaying import graph \
         would be invisible: {rows:?}"
    );

    // Direction: the cycle lands partway through and never leaves, so the
    // final sample must be no healthier than the healthiest earlier one.
    let last = rows.last().expect("non-empty");
    assert!(
        last.arch_health < best,
        "final arch_health {} should sit below the acyclic peak {best}: {rows:?}",
        last.arch_health
    );
}
