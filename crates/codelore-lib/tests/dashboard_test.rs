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
    // `biomarker_repo`, not `differential_repo`: the latter's functions are
    // all empty bodies (`fn later_15() {}`), so their cognitive complexity is
    // zero and `run_xray`'s `WHERE cm.cognitive > 0` filtered every row —
    // three of the six shape loops below never executed a single assertion,
    // and this file is the only coverage those queries have. The biomarker
    // fixture carries branchy functions, a deliberate clone pair
    // (`dup_a.rs` / `dup_b.rs`) and dated commits, so every query returns
    // rows to check.
    let fixture = codelore_lib::test_support::biomarker_repo::build();
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
    assert!(
        !xray.is_empty(),
        "the fixture has functions; an empty x-ray passes the shape loop vacuously"
    );
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
    assert!(
        !clones.is_empty(),
        "the fixture carries clone pairs; an empty summary passes the loop vacuously"
    );
    for c in &clones {
        assert!(c.groups >= 1, "clone-summary groups ≥1, got {}", c.groups);
    }

    // trends: exercise the real SQL with actual fixture paths.
    let mut paths: Vec<String> = xray.iter().map(|x| x.path.clone()).collect();
    paths.sort();
    paths.dedup();
    let trends = run_trends(&db, &opts, &paths).expect("run_trends");
    assert!(
        !trends.is_empty(),
        "committed fixture paths must produce monthly trend points"
    );
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

/// Same-second ties must resolve to the NEWER commit in newest-first
/// queries. Convention (documented at the ingest): gix walks
/// reverse-chronologically, so smaller `rowid` = newer commit. The revs
/// here are chosen so SHA-lex order INVERTS chronology — the older commit
/// sorts lexicographically last — which is exactly the case where the
/// previous `rowid DESC` / `rev DESC` tiebreaks picked the wrong commit.
#[test]
fn same_second_ties_resolve_to_the_newer_commit_in_newest_first_queries() {
    let db = FactsDb::new_in_memory().expect("db");
    let commit = |rev: &str| {
        format!(
            "INSERT INTO commits (rev, author_email, author_name, committer_email, \
             canonical_author, date, committer_date, message, is_merge, parent_count, \
             la, ld, nf, nd, ndev, nuc, exp, entropy, fix) \
             VALUES ('{rev}', 'a@x', 'A', 'a@x', 'a@x', \
             '2026-03-05 12:00:00', '2026-03-05 12:00:00', 'm', false, 1, \
             1, 0, 1, 1, 1, 1, 1, 0.0, false)"
        )
    };
    // Newest inserted FIRST (smaller rowid = newer, per the walk order).
    for stmt in [
        commit("aaa_newer"),
        commit("zzz_older"),
        "INSERT INTO changes VALUES ('aaa_newer', 'f.rs', 'modified', NULL, 2, 1)".to_string(),
        "INSERT INTO changes VALUES ('zzz_older', 'f.rs', 'added', NULL, 5, 0)".to_string(),
    ] {
        db.execute_batch(&stmt).expect("seed");
    }

    // Newest-first LIMIT 1: must be the newer commit, not the one whose
    // rowid (or SHA) sorts higher.
    let kamei = run_kamei_risk(&db, 1).expect("kamei");
    assert_eq!(
        kamei[0].rev, "aaa_newer",
        "kamei sparkline's last-N must start at the NEWER of a same-second pair"
    );

    let opts = codelore_lib::Options {
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    let evidence = codelore_lib::quality_gates::evidence::evidence_for_path(&db, &opts, "f.rs", 5)
        .expect("evidence");
    let revs: Vec<&str> = evidence.iter().map(|e| e.rev.as_str()).collect();
    assert_eq!(
        revs,
        vec!["aaa_newer", "zzz_older"],
        "evidence chain must list the newer commit first at a same-second tie"
    );
}
