use codelore_lib::Options;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn code_health_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce ≥1 row");

    for row in &rows {
        assert!(
            row.score >= 0.0 && row.score <= 100.0,
            "score should be in [0, 100], got {} for {}",
            row.score,
            row.path
        );
        assert!(
            row.cognitive >= 0.0,
            "cognitive should be >= 0, got {} for {}",
            row.cognitive,
            row.path
        );
    }
}

#[test]
fn code_health_ranks_least_healthy_first() {
    // Convention: ORDER BY score ASC — least healthy first (these are the
    // ones a developer should look at).
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    // Confirm ascending order
    for w in rows.windows(2) {
        assert!(
            w[0].score <= w[1].score,
            "expected ascending score order, got {} > {}",
            w[0].score,
            w[1].score
        );
    }
}

#[test]
fn code_health_penalizes_churn() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    // tiny_repo has src/main.rs (4 commits = high churn) and src/lib.rs (1 commit = low churn).
    // src/main.rs should rank LOWER (less healthy) than src/lib.rs in Code Health.
    // `.expect` (not `if let`) so a missing file fails loudly instead of
    // silently skipping the assertion.
    let m = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("src/main.rs should be scored");
    let l = rows
        .iter()
        .find(|r| r.path == "src/lib.rs")
        .expect("src/lib.rs should be scored");
    assert!(
        m.score <= l.score,
        "src/main.rs (4 commits) should rank <= src/lib.rs (1 commit) in code health, got main={} lib={}",
        m.score,
        l.score
    );
}

#[test]
fn code_health_reports_band_and_percentile() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(
            (0.0..=1.0).contains(&row.percentile),
            "percentile in [0,1]: {}",
            row.percentile
        );
        assert!(
            matches!(row.band.as_str(), "red" | "yellow" | "green"),
            "band must be red|yellow|green, got {}",
            row.band
        );
        assert!(
            (0.0..=1.0).contains(&row.structural_risk),
            "structural_risk in [0,1]: {}",
            row.structural_risk
        );
    }
}

#[test]
fn biomarkers_flag_complex_functions() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Running code-health materializes the biomarker table as a side effect.
    let _ = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");

    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM code_health_biomarkers_v1", [], |r| {
            r.get(0)
        })
        .expect("query biomarkers");
    assert!(count >= 1, "tiny_repo should produce >=1 biomarker row");

    // intensities are valid probabilities
    let bad: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM code_health_biomarkers_v1 WHERE intensity < 0.0 OR intensity > 1.0",
            [], |r| r.get(0),
        )
        .expect("query range");
    assert_eq!(bad, 0, "all intensities must be in [0,1]");
}

#[test]
fn coupling_becomes_shotgun_surgery_biomarker() {
    let repo_fx = codelore_lib::test_support::differential_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(repo_fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts =
        codelore_lib::test_support::permissive_coupling_opts(repo_fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let _ = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");

    let n: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM code_health_biomarkers_v1 WHERE smell = 'shotgun-surgery'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert!(
        n >= 1,
        "a coupling-heavy repo should yield shotgun-surgery biomarkers"
    );
}

#[test]
fn structural_risk_rewards_multiple_cooccurring_smells() {
    // Co-occurrence: a file flagged by MORE distinct biomarkers has higher
    // structural_risk than one flagged by fewer, because the weighted sum
    // accumulates terms. On the fixture, dup_a (complex-method + large-method +
    // dry + shotgun-surgery) vs trivial (no smells). Asserted on
    // structural_risk directly — the final score also mixes churn/ownership, so
    // it is not a clean single-variable invariant (the prior version asserted
    // that false invariant and passed only by tiny_repo coincidence).
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_code_health(&db, &opts).expect("run");
    let dup = rows
        .iter()
        .find(|r| r.path.ends_with("dup_a.rs"))
        .expect("dup_a scored");
    let trivial = rows
        .iter()
        .find(|r| r.path.ends_with("trivial.rs"))
        .expect("trivial scored");
    assert!(
        dup.structural_risk > trivial.structural_risk,
        "a file with several co-occurring smells (dup_a={}) must have higher structural_risk than a smell-free file (trivial={})",
        dup.structural_risk,
        trivial.structural_risk
    );
}

#[test]
fn code_health_v2_is_deterministic() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    let run = || {
        let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
        db.ingest(&repo, &opts).expect("ingest");
        codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run")
    };
    let a = run();
    let b = run();
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.path, y.path);
        assert!((x.score - y.score).abs() < 1e-9, "score must be stable");
        assert_eq!(x.band, y.band, "band must be stable");
    }
}

/// `--rows N` MUST NOT change the score computed for a path that survives
/// the truncation. The bug it regression-protects: `materialize_centrality`
/// used to pass the parent `opts` (with `rows_limit = N`) straight into
/// `run_coupling`, so the centrality term was computed over a sliver of
/// the coupling graph and the final score drifted by `rows_limit`.
#[test]
fn code_health_score_invariant_under_rows_limit() {
    let diff_repo = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff_repo.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts_unlimited = Options {
        repo_path: diff_repo.dir.path().to_path_buf(),
        min_revs: 1,
        fisher_significance: 1.0,
        rows_limit: None,
        ..Options::default()
    };
    db.ingest(&repo, &opts_unlimited).expect("ingest");
    let baseline = run_code_health(&db, &opts_unlimited).expect("baseline");
    assert!(baseline.len() >= 2, "need ≥2 rows to test truncation");

    let opts_capped = Options {
        rows_limit: Some(2),
        ..opts_unlimited.clone()
    };
    let capped = run_code_health(&db, &opts_capped).expect("capped");
    assert!(capped.len() <= 2, "rows_limit=2 should truncate output");

    // Each capped row's score MUST match the baseline score for the same path.
    // If the centrality term were computed over a truncated coupling graph,
    // these would drift.
    for row in &capped {
        let baseline_row = baseline
            .iter()
            .find(|b| b.path == row.path)
            .expect("capped path must be in baseline");
        assert!(
            (row.score - baseline_row.score).abs() < 1e-9,
            "score drift for {}: capped={} baseline={} — rows_limit leaked into centrality?",
            row.path,
            row.score,
            baseline_row.score
        );
    }
}

fn biomarker_opts(dir: &std::path::Path) -> Options {
    Options {
        repo_path: dir.to_path_buf(),
        min_revs: 1,
        fisher_significance: 1.0,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        ..Options::default()
    }
}

/// Distribution guard on a purpose-built fixture with a real complexity
/// gradient, a duplicated pair, and co-changed files. `structural_risk` must
/// DISCRIMINATE — spread across files, not collapse to the ceiling. This is the
/// regression guard the per-row invariant tests lacked: the prior formula
/// ranked functions then MAX-ed and OR-ed the intensities, pinning ~every file
/// at 1.0 on real repos while every range/monotonicity test still passed.
#[test]
fn code_health_structural_risk_discriminates() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_code_health(&db, &opts).expect("run");
    assert!(rows.len() >= 5, "fixture should score several files");

    let distinct: std::collections::HashSet<String> = rows
        .iter()
        .map(|r| format!("{:.3}", r.structural_risk))
        .collect();
    assert!(
        distinct.len() >= 3,
        "structural_risk must spread across files, got {} distinct value(s)",
        distinct.len()
    );
    let max = rows
        .iter()
        .map(|r| r.structural_risk)
        .fold(0.0_f64, f64::max);
    let min = rows
        .iter()
        .map(|r| r.structural_risk)
        .fold(1.0_f64, f64::min);
    assert!(
        max < 1.0,
        "no file should saturate at the ceiling, got max={max}"
    );
    assert!(max - min > 0.2, "expected a real spread, got {min}..{max}");

    // Ordering sanity: the trivial file is healthiest; the deeply-nested file
    // is among the worst.
    let trivial = rows
        .iter()
        .find(|r| r.path.ends_with("trivial.rs"))
        .expect("trivial file scored");
    let complex = rows
        .iter()
        .find(|r| r.path.ends_with("complex.rs"))
        .expect("complex file scored");
    assert!(
        trivial.structural_risk < complex.structural_risk,
        "trivial ({}) must be less risky than complex ({})",
        trivial.structural_risk,
        complex.structural_risk
    );
}

/// The biomarker layer fires the expected DISTINCT smells on the fixture.
/// Closes the earlier gap where a test could pass while a `UNION` arm was
/// silently dropped (the vocabulary was only asserted as `>= 1` distinct).
/// god-class needs fan-in a tiny fixture can't manufacture, so it is not
/// required here.
#[test]
fn code_health_biomarkers_fire_distinct_smells() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let _ = run_code_health(&db, &opts).expect("run");

    let smells: std::collections::HashSet<String> =
        codelore_lib::analyses::query::query_map_collect(
            &db,
            "SELECT DISTINCT smell FROM code_health_biomarkers_v1",
            [],
            "smells",
            |r| r.get::<_, String>(0),
        )
        .expect("smells")
        .into_iter()
        .collect();
    for expected in ["complex-method", "large-method", "dry", "shotgun-surgery"] {
        assert!(
            smells.contains(expected),
            "expected smell {expected} to fire on the fixture, got {smells:?}"
        );
    }
}

/// Locks the code-health CSV column contract (order + names). refactoring-targets
/// had this; code-health did not — a column rename/reorder would have gone
/// undetected.
#[test]
fn code_health_csv_column_contract() {
    let rows = vec![codelore_lib::analyses::code_health::CodeHealthRow {
        path: "src/x.rs".to_string(),
        cognitive: 12.0,
        score: 88.5,
        structural_risk: 0.3,
        percentile: 0.5,
        band: "yellow".to_string(),
    }];
    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::csv::write_code_health_csv(&rows, &mut buf).expect("csv");
    let out = String::from_utf8(buf).expect("utf8");
    assert_eq!(
        out.lines().next().unwrap(),
        "entity,cognitive,score,structural_risk,percentile,band"
    );
    assert!(
        out.lines().nth(1).unwrap().starts_with("src/x.rs,"),
        "data row should carry the path"
    );
}

#[test]
fn scoped_no_clones_excludes_dry_and_renormalizes() {
    use codelore_lib::analyses::code_health::{
        CodeHealthRow, HealthScanCtx, run_code_health, run_code_health_scoped,
    };
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let gix = codelore_lib::repo::GixRepo::open(&repo.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo.dir.path().to_path_buf());
    db.ingest(&gix, &opts).expect("ingest");

    let head = run_code_health(&db, &opts).expect("head");
    let mut cx = HealthScanCtx::head();
    cx.include_clones = false;
    let no_dry = run_code_health_scoped(&db, &opts, &cx).expect("no-dry");

    assert_eq!(head.len(), no_dry.len(), "same file universe");

    let risk = |rows: &[CodeHealthRow], suffix: &str| -> f64 {
        rows.iter()
            .find(|r| r.path.ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} should be scored"))
            .structural_risk
    };

    // `big.rs` (large-method, unique — no clone) carries no DRY term, so
    // dropping DRY leaves its weighted biomarker sum untouched: only the
    // `/0.85` renormalization applies. Its no-clones risk is therefore exactly
    // the HEAD risk divided by 0.85, proving the renormalization divisor is
    // wired. (Not a `>=` score relation — renormalization deliberately RAISES a
    // no-duplication file's risk; the no-clones series is internally consistent
    // with itself, not comparable to the with-DRY HEAD score.)
    let big_head = risk(&head, "big.rs");
    let big_nodry = risk(&no_dry, "big.rs");
    assert!(
        big_head > 0.0,
        "big.rs must carry a non-DRY smell for this check"
    );
    assert!(
        (big_nodry - big_head / 0.85).abs() < 1e-6,
        "renorm: big.rs no-clones risk {big_nodry} must equal HEAD {big_head} / 0.85"
    );

    // `dup_a.rs` is a clone of `dup_b.rs`, so at HEAD it carries a DRY term.
    // Excluding DRY removes that term; even after the `/0.85` bump the net risk
    // DROPS below HEAD, proving the DRY biomarker was present and is now gone.
    let dup_head = risk(&head, "dup_a.rs");
    let dup_nodry = risk(&no_dry, "dup_a.rs");
    assert!(
        dup_nodry < dup_head - 1e-6,
        "DRY excluded: dup_a.rs no-clones risk {dup_nodry} must drop below HEAD {dup_head}"
    );

    // Renormalization keeps every risk in range.
    for r in &no_dry {
        assert!(
            (0.0..=1.0).contains(&r.structural_risk),
            "structural_risk out of range for {}: {}",
            r.path,
            r.structural_risk
        );
    }
}

#[test]
fn head_wrapper_equals_scoped_head_ctx() {
    use codelore_lib::analyses::code_health::{
        HealthScanCtx, run_code_health, run_code_health_scoped,
    };
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let gix = codelore_lib::repo::GixRepo::open(&repo.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo.dir.path().to_path_buf());
    db.ingest(&gix, &opts).expect("ingest");

    let a = run_code_health(&db, &opts).expect("wrapper");
    let b = run_code_health_scoped(&db, &opts, &HealthScanCtx::head()).expect("scoped-head");
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.path, y.path);
        assert!(
            (x.score - y.score).abs() < 1e-12,
            "score parity for {}",
            x.path
        );
        assert!((x.structural_risk - y.structural_risk).abs() < 1e-12);
        assert_eq!(x.band, y.band);
    }
}

/// Locks the 0.55 / 0.28 band cut points: every row's band must equal the
/// threshold function applied to its `structural_risk`. Catches a silent
/// threshold change in the SQL that the range/membership tests would miss.
#[test]
fn code_health_band_matches_thresholds() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty());
    for r in &rows {
        let expected = if r.structural_risk >= 0.55 {
            "red"
        } else if r.structural_risk >= 0.28 {
            "yellow"
        } else {
            "green"
        };
        assert_eq!(
            r.band, expected,
            "band {} != expected {} for structural_risk {}",
            r.band, expected, r.structural_risk
        );
    }
}
