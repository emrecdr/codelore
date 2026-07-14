use std::io::Write as _;

use codelore_lib::Options;
use codelore_lib::analyses::code_health::run_code_health;
use codelore_lib::calibration::{
    CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MetricQuantiles,
    QUANTILE_POINTS, Stratum,
};
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
    for expected in [
        "complex-method",
        "large-method",
        "dry",
        "shotgun-surgery",
        "deep-nesting",
        "many-args",
        "complex-conditional",
    ] {
        assert!(
            smells.contains(expected),
            "expected smell {expected} to fire on the fixture, got {smells:?}"
        );
    }
}

/// The intensity a given smell carries for a file in the biomarker table.
/// Returns `None` when the file has no row for that smell. Used by the
/// per-smell firing tests below.
fn smell_intensity(db: &FactsDb, path: &str, smell: &str) -> Option<f64> {
    codelore_lib::analyses::query::query_map_collect(
        db,
        "SELECT intensity FROM code_health_biomarkers_v1 WHERE path = ? AND smell = ?",
        duckdb::params![path, smell],
        "smell-intensity",
        |r| r.get::<_, f64>(0),
    )
    .expect("query smell intensity")
    .into_iter()
    .next()
}

/// `deep-nesting` fires on `src/nested.rs` — its `deeply_nested` function
/// reaches `max_nesting == 5`, the top of the Rust file distribution, so the
/// per-language `PERCENT_RANK` of its per-file MAX nesting is a positive
/// intensity.
#[test]
fn deep_nesting_biomarker_fires_on_nested_file() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let _ = run_code_health(&db, &opts).expect("run");

    let intensity = smell_intensity(&db, "src/nested.rs", "deep-nesting")
        .expect("nested.rs should carry a deep-nesting biomarker row");
    assert!(
        intensity > 0.0,
        "deep-nesting intensity for src/nested.rs must be > 0, got {intensity}"
    );
}

/// `many-args` fires on `src/many_args.rs` — its `many_args` function takes
/// `nargs == 7`, the maximum in the Rust file distribution, so its per-file MAX
/// nargs ranks at the top of the per-language `PERCENT_RANK`.
#[test]
fn many_args_biomarker_fires_on_many_args_file() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let _ = run_code_health(&db, &opts).expect("run");

    let intensity = smell_intensity(&db, "src/many_args.rs", "many-args")
        .expect("many_args.rs should carry a many-args biomarker row");
    assert!(
        intensity > 0.0,
        "many-args intensity for src/many_args.rs must be > 0, got {intensity}"
    );
}

/// `complex-conditional` fires on `src/conditional.rs` — its `gate` function's
/// single `if` chains four boolean operators (`bool_ops == 3`), the only file
/// with a non-zero boolean-operator count, so it ranks at the top of the
/// per-language `PERCENT_RANK` over per-file MAX `bool_ops`. This also exercises
/// the `bool_ops` metric flowing end to end through the composite.
#[test]
fn complex_conditional_biomarker_fires_on_conditional_file() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = biomarker_opts(fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let _ = run_code_health(&db, &opts).expect("run");

    let intensity = smell_intensity(&db, "src/conditional.rs", "complex-conditional")
        .expect("conditional.rs should carry a complex-conditional biomarker row");
    assert!(
        intensity > 0.0,
        "complex-conditional intensity for src/conditional.rs must be > 0, got {intensity}"
    );
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
        corpus_percentile: None,
        beyond_corpus: false,
    }];
    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::csv::write_code_health_csv(&rows, &mut buf).expect("csv");
    let out = String::from_utf8(buf).expect("utf8");
    assert_eq!(
        out.lines().next().unwrap(),
        "entity,cognitive,score,structural_risk,percentile,band,corpus-pct"
    );
    assert!(
        out.lines().nth(1).unwrap().starts_with("src/x.rs,"),
        "data row should carry the path"
    );
}

/// `corpus-pct` column: populated when `corpus_percentile` is `Some`,
/// empty when `None`, and `beyond_corpus` does not affect the rendered value.
#[test]
fn code_health_csv_corpus_pct_populated_and_none() {
    use codelore_lib::analyses::code_health::CodeHealthRow;
    let make = |cp: Option<f64>, bc: bool| CodeHealthRow {
        path: "f.rs".into(),
        cognitive: 1.0,
        score: 50.0,
        structural_risk: 0.1,
        percentile: 0.2,
        band: "green".into(),
        corpus_percentile: cp,
        beyond_corpus: bc,
    };
    let rows = vec![
        make(Some(0.75), false),
        make(Some(1.0), true),
        make(None, false),
    ];
    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::csv::write_code_health_csv(&rows, &mut buf).expect("csv");
    let csv = String::from_utf8(buf).expect("utf8");
    let lines: Vec<&str> = csv.lines().collect();
    // header
    assert!(
        lines[0].ends_with(",corpus-pct"),
        "header must end with corpus-pct"
    );
    // row 0: populated, beyond_corpus false → raw value
    assert!(
        lines[1].ends_with(",0.75"),
        "populated row must carry the corpus-pct value: {}",
        lines[1]
    );
    // row 1: populated, beyond_corpus true → raw value (beyond_corpus doesn't change cell)
    assert!(
        lines[2].ends_with(",1.00"),
        "beyond-corpus row must carry 1.00: {}",
        lines[2]
    );
    // row 2: None → empty cell (trailing comma, no value)
    assert!(
        lines[3].ends_with(','),
        "None row must emit empty corpus-pct cell: {}",
        lines[3]
    );
}

/// `Corpus percentile` column in the markdown emitter: populated when
/// `corpus_percentile` is `Some`, em-dash when `None`.
#[test]
fn code_health_markdown_corpus_pct_column() {
    use codelore_lib::analyses::code_health::CodeHealthRow;
    let make = |cp: Option<f64>, bc: bool| CodeHealthRow {
        path: "f.rs".into(),
        cognitive: 1.0,
        score: 50.0,
        structural_risk: 0.1,
        percentile: 0.2,
        band: "green".into(),
        corpus_percentile: cp,
        beyond_corpus: bc,
    };
    let rows = vec![
        make(Some(0.74), false),
        make(Some(1.0), true),
        make(None, false),
    ];
    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::markdown::write_code_health_markdown(&rows, &mut buf).expect("md");
    let md = String::from_utf8(buf).expect("utf8");
    // Header must include the new column
    assert!(
        md.contains("Corpus percentile"),
        "markdown header must contain 'Corpus percentile'"
    );
    // Populated row: rendered as integer percent
    assert!(
        md.contains("74%"),
        "populated corpus_percentile 0.74 must render as 74%: {md}"
    );
    // beyond_corpus row: rendered with '+' suffix
    assert!(
        md.contains("100%+"),
        "beyond_corpus row must render as 100%+: {md}"
    );
    // None row: em-dash
    assert!(
        md.contains("—"),
        "None corpus_percentile must render as em-dash"
    );
}

#[test]
fn scoped_no_clones_excludes_dry_and_renormalizes() {
    use codelore_lib::analyses::code_health::{
        CodeHealthRow, HealthScanCtx, run_code_health, run_code_health_scoped,
    };
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let gix = codelore_lib::repo::GixRepo::open(repo.dir.path()).expect("open");
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
    // `/0.88` renormalization applies. Its no-clones risk is therefore exactly
    // the HEAD risk divided by 0.88, proving the renormalization divisor is
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
        (big_nodry - big_head / 0.88).abs() < 1e-6,
        "renorm: big.rs no-clones risk {big_nodry} must equal HEAD {big_head} / 0.88"
    );

    // `dup_a.rs` is a clone of `dup_b.rs`, so at HEAD it carries a DRY term.
    // Excluding DRY removes that term; even after the `/0.88` bump the net risk
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
    let gix = codelore_lib::repo::GixRepo::open(repo.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo.dir.path().to_path_buf());
    db.ingest(&gix, &opts).expect("ingest");

    let a = run_code_health(&db, &opts).expect("wrapper");
    let b = run_code_health_scoped(&db, &opts, &HealthScanCtx::head()).expect("scoped-head");
    // Non-vacuity: a regression guard that passed on an empty result would be
    // worthless — the fixture must yield scored rows for the parity loop to bite.
    assert!(!a.is_empty(), "biomarker_repo must yield scored rows");
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

/// Exercises the `history_cutoff` scoped path end to end: with a cutoff mid-way
/// through the fixture's history, the churn / author / coupling terms must see
/// only commits at-or-before the cutoff date, so at least one file's score
/// moves relative to a full-history scan. This is the only coverage of the
/// `changes_at_ts` view + `run_coupling_scoped` SQL — without it a broken
/// cutoff would ship silently and only surface in the timeline consumer.
#[test]
fn scoped_history_cutoff_limits_churn_and_coupling() {
    use codelore_lib::analyses::code_health::{HealthScanCtx, run_code_health_scoped};
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let gix = codelore_lib::repo::GixRepo::open(repo.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo.dir.path().to_path_buf());
    db.ingest(&gix, &opts).expect("ingest");

    // Full history (cutoff None) vs a cutoff after the 3rd of six commits
    // (fixture dates run 2026-06-01 .. 2026-06-06; the dup co-changes and the
    // final complex touch land on 06-04..06-06 and are excluded here).
    let full = run_code_health_scoped(&db, &opts, &HealthScanCtx::head()).expect("full");
    let mut cx = HealthScanCtx::head();
    cx.history_cutoff = Some("2026-06-03T23:59:59Z".to_string());
    let cut = run_code_health_scoped(&db, &opts, &cx).expect("cutoff");

    // The cutoff path must execute (no SQL error above) and yield valid scores.
    assert!(!cut.is_empty(), "cutoff scan must yield rows");
    for r in &cut {
        assert!(
            (0.0..=100.0).contains(&r.score),
            "score in [0,100] for {}: {}",
            r.path,
            r.score
        );
    }

    // Excluding the later commits changes the churn/coupling inputs, so at
    // least one file's score must differ from the full-history scan.
    let full_by: std::collections::HashMap<_, _> =
        full.iter().map(|r| (r.path.clone(), r.score)).collect();
    let moved = cut.iter().any(|r| {
        full_by
            .get(&r.path)
            .is_none_or(|f| (r.score - f).abs() > 1e-9)
    });
    assert!(
        moved,
        "history cutoff must change at least one file's score vs full history"
    );
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

// ─── corpus-percentile lens (additive) ───────────────────────────────────────

/// A `QUANTILE_POINTS`-long breakpoint vector rising linearly from `min` to
/// `max`, so `q[i] == min + (max - min) * i / (QUANTILE_POINTS - 1)`. With
/// `min = 0`, `max = 1000` the breakpoint index equals the value, so a metric
/// value of `v` resolves to corpus percentile `v / 1000`.
#[allow(clippy::cast_precision_loss)]
fn linear_quantiles(min: f64, max: f64) -> Vec<f64> {
    let last = (QUANTILE_POINTS - 1) as f64;
    (0..QUANTILE_POINTS)
        .map(|i| min + (max - min) * (i as f64) / last)
        .collect()
}

/// Every corpus metric for `language` carries the same `0..=1000` linear ramp
/// and a sample count above the floor, so any file's per-metric percentile is
/// its raw value / 1000. Covers only the named language(s) — a file in any
/// other language falls outside the artifact and gets `corpus_percentile: None`.
fn ramp_artifact(languages: &[&str]) -> CalibrationArtifact {
    let metrics = ["cyclomatic", "cognitive", "sloc", "nargs", "max_nesting"];
    CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-ramp".to_string(),
        generated_at: "2026-07-12T00:00:00Z".to_string(),
        repos_included: 2,
        repos_attempted: 2,
        languages: languages
            .iter()
            .map(|lang| LanguageTable {
                language: (*lang).to_string(),
                sample_functions: 4_000,
                strata: vec![Stratum {
                    sloc_min: 0,
                    sloc_max: u64::MAX,
                    metrics: metrics
                        .iter()
                        .map(|m| MetricQuantiles {
                            metric: (*m).to_string(),
                            quantiles: linear_quantiles(0.0, 1000.0),
                        })
                        .collect(),
                }],
            })
            .collect(),
        repo_metrics: None,
    }
}

fn write_calibration(art: &CalibrationArtifact) -> tempfile::TempPath {
    let mut f = tempfile::Builder::new()
        .prefix("code-health-calib")
        .suffix(".calib.json")
        .tempfile()
        .expect("create temp artifact");
    f.write_all(&serde_json::to_vec(art).expect("serialize"))
        .expect("write artifact");
    f.into_temp_path()
}

/// THE ADDITIVITY CONTRACT (the plan's non-negotiable). Running code-health with
/// a calibration artifact must not perturb ANY pre-existing field: the corpus
/// lens is a pure additive post-pass join. We run twice on `biomarker_repo` —
/// once with the default artifact resolution (the embedded world corpus),
/// once with an explicit covering rust ramp artifact — and
/// assert every shipped field (`path`, `cognitive`, `score`, `structural_risk`,
/// `percentile`, `band`) is byte-identical between the two runs. Since the new
/// fields carry `skip_serializing_if`, stripping them is equivalent to matching
/// the shipped serialized form.
#[test]
fn corpus_lens_is_additive_over_shipped_fields() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");

    let run = |calibration: Option<std::path::PathBuf>| {
        let db = FactsDb::new_in_memory().expect("db");
        db.ingest(&repo, &biomarker_opts(fx.dir.path()))
            .expect("ingest");
        let opts = Options {
            calibration,
            ..biomarker_opts(fx.dir.path())
        };
        run_code_health(&db, &opts).expect("run")
    };

    let calib = write_calibration(&ramp_artifact(&["rust"]));
    let without = run(None);
    let with = run(Some(calib.to_path_buf()));

    assert!(!without.is_empty(), "fixture must yield scored rows");
    assert_eq!(
        without.len(),
        with.len(),
        "the corpus pass must not add or drop rows"
    );

    // The shipped fields, serialized. Reuse the row's own serde but drop the two
    // additive keys — that is exactly the "strip the new fields" the plan asks.
    let shipped = |row: &codelore_lib::analyses::code_health::CodeHealthRow| -> serde_json::Value {
        let mut v = serde_json::to_value(row).expect("serialize row");
        let obj = v.as_object_mut().expect("row is an object");
        obj.remove("corpus_percentile");
        obj.remove("beyond_corpus");
        v
    };

    for (a, b) in without.iter().zip(with.iter()) {
        assert_eq!(
            shipped(a),
            shipped(b),
            "corpus calibration perturbed a shipped field for {}",
            a.path
        );
    }

    // Non-vacuity: at least one row must actually carry a corpus percentile in
    // the calibrated run, or the additivity check would pass trivially.
    assert!(
        with.iter().any(|r| r.corpus_percentile.is_some()),
        "the covering rust artifact must populate corpus_percentile on ≥1 rust file"
    );
}

/// With a covering artifact, every rust file's `corpus_percentile` is populated
/// and in range; when the artifact covers only a language the repo lacks, every
/// row stays `None` (unknown-language contract) and no shipped field moves.
#[test]
fn corpus_lens_populates_covered_language_and_skips_others() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    db.ingest(&repo, &biomarker_opts(fx.dir.path()))
        .expect("ingest");

    // Covering artifact (rust): rust files get a percentile in [0,1].
    let rust_calib = write_calibration(&ramp_artifact(&["rust"]));
    let covered = {
        let opts = Options {
            calibration: Some(rust_calib.to_path_buf()),
            ..biomarker_opts(fx.dir.path())
        };
        run_code_health(&db, &opts).expect("run covered")
    };
    let rust_rows: Vec<_> = covered
        .iter()
        .filter(|r| {
            codelore_lib::complexity::Tier1Language::from_path(&r.path)
                == Some(codelore_lib::complexity::Tier1Language::Rust)
        })
        .collect();
    assert!(!rust_rows.is_empty(), "fixture is rust");
    for r in &rust_rows {
        let p = r
            .corpus_percentile
            .unwrap_or_else(|| panic!("rust file {} must carry a corpus percentile", r.path));
        assert!(
            (0.0..=1.0).contains(&p),
            "corpus percentile in [0,1] for {}: {p}",
            r.path
        );
    }

    // Non-covering artifact (python only): rust files fall outside → all None.
    let py_calib = write_calibration(&ramp_artifact(&["python"]));
    let uncovered = {
        let opts = Options {
            calibration: Some(py_calib.to_path_buf()),
            ..biomarker_opts(fx.dir.path())
        };
        run_code_health(&db, &opts).expect("run uncovered")
    };
    for r in &uncovered {
        assert!(
            r.corpus_percentile.is_none() && !r.beyond_corpus,
            "a rust file must get None from a python-only artifact, got {:?} for {}",
            r.corpus_percentile,
            r.path
        );
    }
}
