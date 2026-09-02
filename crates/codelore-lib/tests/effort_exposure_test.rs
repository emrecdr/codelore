use codelore_lib::Options;
use codelore_lib::analyses::effort_exposure::{
    EffortExposureRow, run_effort_exposure, run_effort_exposure_decomposed_scan,
};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// The seven pre-decomposition fields, for the additive byte-identical check.
fn existing_cols(r: &EffortExposureRow) -> (String, u32, String, String, String, String, String) {
    (
        r.band.clone(),
        r.files,
        format!("{:.6}", r.loc_share_pct),
        format!("{:.6}", r.commit_share_pct),
        format!("{:.6}", r.churn_share_pct),
        format!("{:.6}", r.commit_share_ci_low),
        format!("{:.6}", r.commit_share_ci_high),
    )
}

/// `biomarker_repo` has a deliberate complexity gradient with complex.rs,
/// big.rs, and duplicate files — enough to produce ≥1 health band row with
/// varied structural risk. Exact band composition (red/yellow/green) depends on
/// the scoring thresholds; we validate structural invariants, not band labels.
#[test]
fn effort_exposure_returns_bands_with_valid_metrics() {
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_effort_exposure(&db, &opts).expect("run effort-exposure");

    // At least one band must be present (repo is non-empty).
    assert!(!rows.is_empty(), "expected ≥1 band row");

    // Sanity-check numeric ranges for every band returned.
    for row in &rows {
        assert!(
            (0.0..=100.0).contains(&row.loc_share_pct),
            "loc_share_pct out of range for {}: {}",
            row.band,
            row.loc_share_pct
        );
        assert!(
            (0.0..=100.0).contains(&row.commit_share_pct),
            "commit_share_pct out of range for {}: {}",
            row.band,
            row.commit_share_pct
        );
        assert!(
            (0.0..=100.0).contains(&row.churn_share_pct),
            "churn_share_pct out of range for {}: {}",
            row.band,
            row.churn_share_pct
        );
        assert!(
            (0.0..=1.0).contains(&row.commit_share_ci_low),
            "ci_low out of [0,1] for {}: {}",
            row.band,
            row.commit_share_ci_low
        );
        assert!(
            (0.0..=1.0).contains(&row.commit_share_ci_high),
            "ci_high out of [0,1] for {}: {}",
            row.band,
            row.commit_share_ci_high
        );
        assert!(
            row.commit_share_ci_low <= row.commit_share_ci_high,
            "ci_low > ci_high for {}: {} > {}",
            row.band,
            row.commit_share_ci_low,
            row.commit_share_ci_high
        );
        // Wilson CI must contain the sample proportion.
        let share = row.commit_share_pct / 100.0;
        assert!(
            row.commit_share_ci_low <= share + 1e-9,
            "ci_low must be ≤ commit_share proportion for {}: {} > {}",
            row.band,
            row.commit_share_ci_low,
            share
        );
        assert!(
            row.commit_share_ci_high >= share - 1e-9,
            "ci_high must be ≥ commit_share proportion for {}: {} < {}",
            row.band,
            row.commit_share_ci_high,
            share
        );
    }
}

#[test]
fn effort_exposure_files_count_matches_code_health() {
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_effort_exposure(&db, &opts).expect("run");

    // Total files across all bands must be ≥ 1 (biomarker_repo has 6 files).
    let total_files: u32 = rows.iter().map(|r| r.files).sum();
    assert!(total_files >= 1, "expected ≥1 total files across all bands");
}

/// The gate path pre-computes code-health rows once and feeds them into
/// [`run_effort_exposure_with_health`]; the result must be identical to the
/// self-contained [`run_effort_exposure`], or the check command's dedup of
/// the health scan would silently change gate values.
#[test]
fn effort_exposure_with_precomputed_health_matches_self_contained() {
    use codelore_lib::analyses::code_health::{HealthScanCtx, run_code_health_scoped};
    use codelore_lib::analyses::effort_exposure::run_effort_exposure_with_health;

    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let self_contained = run_effort_exposure(&db, &opts).expect("self-contained run");
    let health = run_code_health_scoped(
        &db,
        &opts.with_no_row_limit(),
        &HealthScanCtx::head_default(),
    )
    .expect("health scan");
    let with_health =
        run_effort_exposure_with_health(&db, &opts, &health).expect("precomputed-health run");

    assert_eq!(
        format!("{self_contained:?}"),
        format!("{with_health:?}"),
        "precomputed-health variant must reproduce the self-contained result exactly"
    );
}

/// The decomposed scan must leave every existing column byte-identical to the
/// base analysis — the additive-only contract — on a multi-band fixture.
#[test]
fn decomposed_scan_keeps_existing_columns_byte_identical() {
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let base = run_effort_exposure(&db, &opts).expect("base run");
    let decomposed =
        run_effort_exposure_decomposed_scan(&db, &repo, &opts).expect("decomposed run");

    let base_cols: Vec<_> = base.iter().map(existing_cols).collect();
    let dec_cols: Vec<_> = decomposed.iter().map(existing_cols).collect();
    assert_eq!(
        base_cols, dec_cols,
        "the seven existing columns must be identical before and after decomposition"
    );
}

// ───────── temporal improving/degrading end-to-end ─────────

fn run_git(path: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit_at(path: &std::path::Path, msg: &str, date: &str) {
    run_git(path, &["add", "."]);
    let out = std::process::Command::new("git")
        .args(["commit", "-m", msg, "--quiet"])
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(path)
        .output()
        .expect("commit");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A High-risk `worker` (cyclomatic well above 11 via nested `&&` conditions
/// plus a run of flat branches) — absolute `delta-health` class High.
const EXTREME_WORKER: &str = "\
pub fn worker(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 {
    let mut x = a;
    if a > 0 && b > 0 && c > 0 {
        x += 1;
        if d > 0 && e > 0 {
            x += 2;
            if f > 0 && g > 0 {
                x += 3;
                if h > 0 { x += 4; }
            }
        }
    }
    if a > 1 { x += 1; }
    if a > 2 { x += 1; }
    if a > 3 { x += 1; }
    if b > 1 { x += 1; }
    if b > 2 { x += 1; }
    if b > 3 { x += 1; }
    if c > 1 { x += 1; }
    if c > 2 { x += 1; }
    if c > 3 { x += 1; }
    if d > 1 { x += 1; }
    if d > 2 { x += 1; }
    if d > 3 { x += 1; }
    x + b + c + d + e + f + g + h
}
";

/// A Medium-risk `worker` (cyclomatic ~8, one nested block and one `&&`) — the
/// same eight-arg signature so it pairs with `EXTREME_WORKER` by bare name, and
/// still the population complexity max against trivial filler, so its file stays
/// red even though its own absolute class dropped to Medium.
const MODEST_WORKER: &str = "\
pub fn worker(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32, h: i32) -> i32 {
    let mut x = a;
    if a > 0 && b > 0 {
        x += 1;
        if c > 0 { x += 2; }
    }
    if d > 0 { x += 1; }
    if e > 0 { x += 1; }
    if f > 0 { x += 1; }
    if g > 0 { x += 1; }
    x + b + c + d + e + f + g + h
}
";

/// Build a repo with nine trivial filler files plus one `target.rs` whose sole
/// function is `baseline` at an out-of-window commit, then `head` at a recent
/// in-window commit. Ten Rust files put the language cohort at the code-health
/// per-language structural-biomarker floor, so the percent-rank is meaningful; a
/// thinner cohort produces no structural biomarkers at all. The target is the
/// unique complexity maximum, so it is the only red file; the recent commit is
/// the only window churn.
fn build_temporal_repo(baseline: &str, head: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    for i in 0..9 {
        std::fs::write(
            path.join(format!("filler{i}.rs")),
            format!("pub fn a{i}() -> i32 {{ {i} }}\n"),
        )
        .expect("write filler");
    }
    std::fs::write(path.join("target.rs"), baseline).expect("write target baseline");
    // Baseline commit sits well outside the 90-day window.
    commit_at(path, "baseline", "2020-01-01T00:00:00");

    std::fs::write(path.join("target.rs"), head).expect("write target head");
    // Recent commit is the only in-window churn (window anchors to MAX date).
    commit_at(path, "evolve target", "2020-08-01T00:00:00");
    dir
}

fn red_row(rows: &[EffortExposureRow]) -> &EffortExposureRow {
    rows.iter()
        .find(|r| r.band == "red")
        .expect("engineered target must produce a red band row")
}

#[test]
fn decomposition_books_a_refactored_red_file_as_improving() {
    // target.rs goes from an absolute-High worker to an absolute-Medium one
    // (a genuine refactor) while staying the population complexity max, so it is
    // red at HEAD yet net-improved over the window → its churn is improving.
    let repo_dir = build_temporal_repo(EXTREME_WORKER, MODEST_WORKER);
    let repo = GixRepo::open(repo_dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: repo_dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_effort_exposure_decomposed_scan(&db, &repo, &opts).expect("decomposed run");
    let red = red_row(&rows);
    let improving = red
        .churn_share_improving_pct
        .expect("red row must carry the improving split");
    let degrading = red
        .churn_share_degrading_pct
        .expect("red row must carry the degrading split");

    assert!(
        improving > 0.0,
        "the refactored red file's churn must be booked improving: {improving}"
    );
    assert!(
        degrading.abs() < 1e-6,
        "no red file degraded, so degrading share must be 0: {degrading}"
    );
    // The two shares reconcile to the band's total churn share.
    assert!(
        (improving + degrading - red.churn_share_pct).abs() < 0.5,
        "improving ({improving}) + degrading ({degrading}) must reconcile to churn_share_pct ({})",
        red.churn_share_pct
    );
}

#[test]
fn decomposition_books_a_worsened_red_file_as_degrading() {
    // The mirror image: target.rs goes from Medium to High → it is red at HEAD
    // and net-degraded, so all of its churn is degrading and none is exempt.
    let repo_dir = build_temporal_repo(MODEST_WORKER, EXTREME_WORKER);
    let repo = GixRepo::open(repo_dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: repo_dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_effort_exposure_decomposed_scan(&db, &repo, &opts).expect("decomposed run");
    let red = red_row(&rows);
    let improving = red
        .churn_share_improving_pct
        .expect("red row must carry the improving split");
    let degrading = red
        .churn_share_degrading_pct
        .expect("red row must carry the degrading split");

    assert!(
        improving.abs() < 1e-6,
        "the worsened red file must expose no improving churn: {improving}"
    );
    assert!(
        degrading > 0.0,
        "the worsened red file's churn must be booked degrading: {degrading}"
    );
    assert!(
        (improving + degrading - red.churn_share_pct).abs() < 0.5,
        "improving + degrading must reconcile to churn_share_pct"
    );
}

/// `complexity_metrics` holds the whole-file unit space PLUS one row per
/// nested impl/class/function, whose spans overlap. A file's SLOC is the
/// unit row's span; summing the rows counts function bodies twice. This
/// seeds both row shapes and pins the per-file value `eh_bands_v1` carries.
#[test]
fn file_sloc_is_the_unit_row_not_the_sum_of_overlapping_entity_rows() {
    use codelore_lib::analyses::code_health::CodeHealthRow;
    use codelore_lib::analyses::effort_exposure::run_effort_exposure_with_health;

    let db = FactsDb::new_in_memory().expect("db");
    for stmt in [
        "INSERT INTO commits (rev, author_email, author_name, committer_email, \
         canonical_author, date, committer_date, message, is_merge, parent_count) \
         VALUES ('c1','a@x','A','a@x','a@x','2026-03-01 12:00:00', \
         '2026-03-01 12:00:00','m',false,1)",
        "INSERT INTO changes VALUES ('c1','dense.rs','added',NULL,10,0)",
        "INSERT INTO complexity_metrics (path, name, rev, sloc) \
         VALUES ('dense.rs','<unit>','c1',100)",
        "INSERT INTO complexity_metrics (path, name, rev, sloc) \
         VALUES ('dense.rs','busy_fn','c1',40)",
    ] {
        db.execute_batch(stmt).expect("seed");
    }
    let health = vec![CodeHealthRow {
        path: "dense.rs".into(),
        cognitive: 1.0,
        score: 50.0,
        structural_risk: 0.5,
        percentile: 0.5,
        band: "yellow".into(),
        corpus_percentile: None,
        beyond_corpus: false,
        corpus_percentile_ci_low: None,
        corpus_percentile_ci_high: None,
    }];
    let opts = Options {
        min_revs: 1,
        ..Options::default()
    };
    run_effort_exposure_with_health(&db, &opts, &health).expect("run");

    let sloc: i64 = db
        .query_row(
            "SELECT sloc FROM eh_bands_v1 WHERE path = 'dense.rs'",
            [],
            |r| r.get(0),
        )
        .expect("eh_bands_v1 row");
    assert_eq!(
        sloc, 100,
        "file SLOC must be the unit row's span, not the 140 an overlapping-row sum produces"
    );
}
