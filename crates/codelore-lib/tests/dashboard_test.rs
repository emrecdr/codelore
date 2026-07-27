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
    let trends = run_trends(&db, &opts, &paths).expect("run_trends");
    for t in &trends {
        assert!(!t.month.is_empty(), "trend point has a month");
    }

    // kamei-risk + arch-graph imports: must execute without error and
    // return well-formed (possibly empty) results over the fixture.
    let _kamei = run_kamei_risk(&db, 100).expect("run_kamei_risk");
    let _imports = run_imports_for_arch_graph(&db).expect("run_imports_for_arch_graph");
}

/// The drawer sparkline (`run_trends`) is keyed to the lineage-canonical
/// hotspot paths, so it must aggregate a renamed file's pre-rename history
/// onto its head path. The `differential_repo` fixture renames
/// `src/old_name.rs` → `src/new_name.rs`; with lineage on, the head-path
/// series must cover strictly more revisions than with lineage off (which
/// only sees the post-rename commits).
#[test]
fn trends_sparkline_covers_pre_rename_history_under_lineage() {
    use codelore_lib::Options;
    use codelore_lib::analyses::revisions::run_revisions;

    let fixture = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(fixture.dir.path()).expect("open");

    // Discover the canonical (post-rename) head path via lineage-on revisions,
    // so the test doesn't hard-code the fixture's exact path string.
    let opts_on = Options {
        repo_path: fixture.dir.path().to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    let db_on = FactsDb::new_in_memory().expect("db");
    db_on.ingest(&repo, &opts_on).expect("ingest");
    let head_path = run_revisions(&db_on, &opts_on)
        .expect("revisions")
        .into_iter()
        .find(|(p, _)| p.contains("new_name"))
        .map(|(p, _)| p)
        .expect("fixture has a renamed new_name path");
    let paths = vec![head_path];

    let on_sum: f64 = run_trends(&db_on, &opts_on, &paths)
        .expect("trends lineage-on")
        .iter()
        .map(|t| t.hotspot_score)
        .sum();

    // Same query, lineage off: the head path only sees its post-rename
    // commits — pre-rename history stays under the old path and is missed.
    let opts_off = Options {
        use_canonical_lineage: false,
        ..opts_on.clone()
    };
    let db_off = FactsDb::new_in_memory().expect("db");
    db_off.ingest(&repo, &opts_off).expect("ingest");
    let off_sum: f64 = run_trends(&db_off, &opts_off, &paths)
        .expect("trends lineage-off")
        .iter()
        .map(|t| t.hotspot_score)
        .sum();

    assert!(
        on_sum > off_sum,
        "lineage must fold pre-rename revisions into the head-path sparkline: on={on_sum} off={off_sum}"
    );
    assert!(
        on_sum >= 2.0,
        "head path should aggregate ≥2 revisions (pre-rename + post-rename); got {on_sum}"
    );
}
