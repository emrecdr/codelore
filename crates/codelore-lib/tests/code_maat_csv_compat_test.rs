//! CSV column-name compatibility under `--code-maat-compat`.
//!
//! These tests lock the EXACT header line each parity-affected writer
//! emits in each of the two modes. Any future change to a writer that
//! flips a header without updating these tests will fail loudly.
//!
//! The header strings here are copy-pasteable into a code-maat-targeted
//! dashboard's CSV parser; treat them as a contract.

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn first_line(buf: &[u8]) -> String {
    let s = std::str::from_utf8(buf).expect("utf-8");
    s.lines().next().unwrap_or("").to_string()
}

fn run(repo_path: &std::path::Path, opts: &Options) -> FactsDb {
    let repo = GixRepo::open(repo_path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    db.ingest(&repo, opts).expect("ingest");
    db
}

#[test]
fn summary_csv_emits_metric_value_by_default() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let db = run(tiny.dir.path(), &opts);
    let rows = codelore_lib::analyses::summary::run_summary(&db, &opts).expect("summary");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_summary_csv(&rows, &mut buf, false).expect("csv");
    assert_eq!(first_line(&buf), "metric,value");
}

#[test]
fn summary_csv_emits_statistic_value_under_compat() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        ..Options::default()
    };
    let db = run(tiny.dir.path(), &opts);
    let rows = codelore_lib::analyses::summary::run_summary(&db, &opts).expect("summary");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_summary_csv(&rows, &mut buf, true).expect("csv");
    assert_eq!(first_line(&buf), "statistic,value");
}

#[test]
fn code_age_csv_emits_modern_columns_by_default() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    let db = run(tiny.dir.path(), &opts);
    let rows = codelore_lib::analyses::code_age::run_code_age(&db, &opts).expect("code-age");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_code_age_csv(&rows, &mut buf, false).expect("csv");
    assert_eq!(first_line(&buf), "entity,age_months,age_days,last_modified");
}

#[test]
fn code_age_csv_emits_code_maat_columns_under_compat() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    let db = run(tiny.dir.path(), &opts);
    let rows = codelore_lib::analyses::code_age::run_code_age(&db, &opts).expect("code-age");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_code_age_csv(&rows, &mut buf, true).expect("csv");
    assert_eq!(first_line(&buf), "entity,age-months");
    // Each data row must be `path,age_months` only — drop the extra cols.
    let s = std::str::from_utf8(&buf).expect("utf-8");
    for line in s.lines().skip(1) {
        assert_eq!(
            line.matches(',').count(),
            1,
            "compat code-age row must have exactly one comma: {line:?}",
        );
    }
}

#[test]
fn communication_csv_emits_author_a_b_by_default() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        // differential_repo's pairs co-edit fewer files than the default
        // min_shared_revs; lower to let SOME pairs through.
        min_shared_revs: 1,
        ..Options::default()
    };
    let db = run(fixture.dir.path(), &opts);
    let rows = codelore_lib::analyses::communication::run_communication(&db, &opts)
        .expect("communication");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_communication_csv(&rows, &mut buf, false).expect("csv");
    assert_eq!(
        first_line(&buf),
        "author-a,author-b,shared,average,strength"
    );
}

#[test]
fn communication_csv_emits_author_peer_under_compat() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_shared_revs: 1,
        ..Options::default()
    };
    let db = run(fixture.dir.path(), &opts);
    let rows = codelore_lib::analyses::communication::run_communication(&db, &opts)
        .expect("communication");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_communication_csv(&rows, &mut buf, true).expect("csv");
    assert_eq!(first_line(&buf), "author,peer,shared,average,strength");
}

#[test]
fn ownership_csv_emits_modern_columns_by_default() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    let db = run(fixture.dir.path(), &opts);
    let rows = codelore_lib::analyses::ownership::run_ownership(&db, &opts).expect("ownership");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_ownership_csv(&rows, &mut buf, false).expect("csv");
    assert_eq!(
        first_line(&buf),
        "entity,main-author,total-revs,fractal-value"
    );
}

#[test]
fn ownership_csv_emits_legacy_columns_under_compat() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let opts = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    let db = run(fixture.dir.path(), &opts);
    let rows = codelore_lib::analyses::ownership::run_ownership(&db, &opts).expect("ownership");
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_ownership_csv(&rows, &mut buf, true).expect("csv");
    assert_eq!(first_line(&buf), "entity,fractal-value,total-revs");
    // Each data row must have exactly two commas (3 columns).
    let s = std::str::from_utf8(&buf).expect("utf-8");
    for line in s.lines().skip(1) {
        assert_eq!(
            line.matches(',').count(),
            2,
            "compat ownership row must have exactly two commas: {line:?}",
        );
    }
}
