//! End-to-end coverage for the six dashboard SQL query functions in
//! `analyses::dashboard`. These feed the SPA payload but had no direct
//! test — the browser smoke test only builds synthetic row structs, and
//! the `--format spa` CLI test only asserts the HTML file exists (and can
//! skip the SQL paths entirely on a tiny fixture). This drives each real
//! query over the ingested differential fixture and asserts it executes
//! and returns well-formed rows.

use codelore_lib::analyses::dashboard::{
    run_clone_summary, run_daily_commits, run_imports_for_arch_graph, run_kamei_risk, run_trends,
    run_xray,
};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::test_support::permissive_coupling_opts;

#[test]
fn dashboard_queries_run_over_ingested_fixture() {
    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = permissive_coupling_opts(fixture.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    // daily-commits: the fixture has commit history, so this must be
    // non-empty with well-formed (date, positive-count) rows.
    let daily = run_daily_commits(&db).expect("run_daily_commits");
    assert!(!daily.is_empty(), "fixture has commits → daily rows");
    for d in &daily {
        assert!(!d.date.is_empty(), "daily row has a date");
        assert!(d.count >= 1, "an active day has ≥1 commit, got {}", d.count);
    }

    // x-ray: any returned function has a sane line range + a name.
    let xray = run_xray(&db, 100).expect("run_xray");
    for x in &xray {
        assert!(!x.function.is_empty(), "xray entry has a function name");
        assert!(
            x.start_line <= x.end_line,
            "xray line range {}..{} inverted for {}",
            x.start_line,
            x.end_line,
            x.path
        );
    }

    // clone-summary: each row counts ≥1 clone group for its path.
    let clones = run_clone_summary(&db).expect("run_clone_summary");
    for c in &clones {
        assert!(c.groups >= 1, "clone-summary groups ≥1, got {}", c.groups);
    }

    // trends: exercise the real SQL with actual fixture paths.
    let mut paths: Vec<String> = xray.iter().map(|x| x.path.clone()).collect();
    paths.sort();
    paths.dedup();
    let trends = run_trends(&db, &paths).expect("run_trends");
    for t in &trends {
        assert!(!t.month.is_empty(), "trend point has a month");
    }

    // kamei-risk + arch-graph imports: must execute without error and
    // return well-formed (possibly empty) results over the fixture.
    let _kamei = run_kamei_risk(&db, 100).expect("run_kamei_risk");
    let _imports = run_imports_for_arch_graph(&db).expect("run_imports_for_arch_graph");
}
