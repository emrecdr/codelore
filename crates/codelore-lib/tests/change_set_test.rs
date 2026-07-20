//! Engine tests for the projected code-health half of the change-set engine.
//! Each test clones its own copy of the 50-commit differential fixture (it
//! mutates the working tree) and drives `change_set::project_health` directly.

use codelore_lib::Options;
use codelore_lib::change_set::project_health;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::{GixRepo, Repo, WorktreeChange, WorktreeChangeKind};
use codelore_lib::test_support::differential_repo::{self, DifferentialRepo};

/// A monster function: nested loops + match + boolean conditionals so its
/// cyclomatic / cognitive / nesting / bool-op counts dominate the fixture,
/// pushing whatever file it lands in to the top of every complexity rank.
const MONSTER_FN: &str = r"
fn monster(x: i32) -> i32 {
    let mut acc = 0;
    for a in 0..x {
        if a % 2 == 0 && a % 3 == 0 || a % 5 == 0 {
            for b in 0..a {
                if b > 1 {
                    match b % 4 {
                        0 => { if b > 10 { acc += 1; } else { acc += 2; } }
                        1 => { while acc < 100 { acc += 1; if acc % 7 == 0 { break; } } }
                        2 => { for c in 0..b { if c > 3 && c < 9 || c == 5 { acc += c; } } }
                        _ => { if a > b { acc -= 1; } else { acc += 1; } }
                    }
                }
            }
        }
    }
    acc
}
";

/// Clone the fixture and ingest HEAD facts. Returns the fixture guard (keeps
/// the tempdir alive), an open repo handle, the fact store, and default opts.
fn fresh() -> (DifferentialRepo, GixRepo, FactsDb, Options) {
    let fx = differential_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("open fact store");
    let opts = Options {
        repo_path: fx.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    (fx, repo, db, opts)
}

fn modified(path: &str) -> WorktreeChange {
    WorktreeChange {
        path: path.to_string(),
        kind: WorktreeChangeKind::Modified,
        rename_from: None,
    }
}

#[test]
fn modified_file_gets_baseline_and_projected_scores() {
    let (fx, repo, db, opts) = fresh();
    let main_path = fx.dir.path().join("src/main.rs");
    let mut content = std::fs::read_to_string(&main_path).expect("read main.rs");
    content.push_str(MONSTER_FN);
    std::fs::write(&main_path, content).expect("write main.rs");

    let projection =
        project_health(&db, &repo, &opts, &[modified("src/main.rs")]).expect("project_health");
    let delta = projection
        .deltas
        .iter()
        .find(|d| d.path == "src/main.rs")
        .expect("src/main.rs has a delta row");

    let baseline = delta.baseline_score.expect("baseline scored");
    let projected = delta.projected_score.expect("projection scored");
    assert!(
        projected < baseline,
        "a deeply-nested high-complexity append must lower the score: \
         baseline {baseline}, projected {projected}",
    );
    assert!(
        delta.delta.expect("delta present") < 0.0,
        "delta must be negative: {:?}",
        delta.delta,
    );
    assert!(
        delta.reason.is_none(),
        "a fully-scored file needs no reason"
    );
}

#[test]
fn unchanged_repo_projects_zero_delta() {
    let (fx, repo, db, opts) = fresh();
    // Write the HEAD blob back so the working-tree bytes equal HEAD exactly.
    // Byte-identical bytes re-parse to byte-identical rows, which rank
    // identically, which yields a delta of exactly 0.0.
    let head_bytes = repo
        .read_blob_at_head("src/main.rs")
        .expect("read blob")
        .expect("main.rs tracked at HEAD");
    std::fs::write(fx.dir.path().join("src/main.rs"), &head_bytes).expect("restore main.rs");

    let projection =
        project_health(&db, &repo, &opts, &[modified("src/main.rs")]).expect("project_health");
    let delta = projection
        .deltas
        .iter()
        .find(|d| d.path == "src/main.rs")
        .expect("src/main.rs has a delta row");

    assert_eq!(
        delta.delta,
        Some(0.0),
        "an identical re-parse must project exactly zero delta",
    );
}

#[test]
fn added_file_reports_no_history_baseline() {
    let (fx, repo, db, opts) = fresh();
    std::fs::write(
        fx.dir.path().join("src/added_new.rs"),
        "pub fn helper(a: i32) -> i32 { a + 1 }\n",
    )
    .expect("write added file");

    let change = WorktreeChange {
        path: "src/added_new.rs".to_string(),
        kind: WorktreeChangeKind::Added,
        rename_from: None,
    };
    let projection = project_health(&db, &repo, &opts, &[change]).expect("project_health");
    let delta = projection
        .deltas
        .iter()
        .find(|d| d.path == "src/added_new.rs")
        .expect("added file has a delta row");

    assert_eq!(
        delta.reason.as_deref(),
        Some("new file (no history baseline)")
    );
    assert!(
        delta.baseline_score.is_none(),
        "an added file has no baseline"
    );
    assert!(delta.delta.is_none(), "an added file has no delta");
}

#[test]
fn non_tier1_file_reports_reason() {
    let (fx, repo, db, opts) = fresh();
    std::fs::write(fx.dir.path().join("README.md"), "# changed heading\n").expect("edit README");

    let projection =
        project_health(&db, &repo, &opts, &[modified("README.md")]).expect("project_health");
    let delta = projection
        .deltas
        .iter()
        .find(|d| d.path == "README.md")
        .expect("README.md has a delta row");

    assert_eq!(delta.reason.as_deref(), Some("not a Tier-1 source file"));
    assert!(delta.delta.is_none(), "a non-source file has no delta");
}

#[test]
fn project_health_leaves_the_fact_tables_untouched() {
    // Scoring isolation: the engine writes only session-scoped temp tables. The
    // persistent `complexity_metrics` row count and the set of permanent tables
    // must be identical before and after a projection.
    let (fx, repo, db, opts) = fresh();
    let main_path = fx.dir.path().join("src/main.rs");
    let mut content = std::fs::read_to_string(&main_path).expect("read main.rs");
    content.push_str(MONSTER_FN);
    std::fs::write(&main_path, content).expect("write main.rs");

    let cm_before = complexity_metrics_count(&db);
    let perm_before = permanent_table_count(&db);

    let _ = project_health(&db, &repo, &opts, &[modified("src/main.rs")]).expect("project_health");

    assert_eq!(
        complexity_metrics_count(&db),
        cm_before,
        "complexity_metrics row count must be unchanged",
    );
    assert_eq!(
        permanent_table_count(&db),
        perm_before,
        "no new permanent tables may be created",
    );
}

fn complexity_metrics_count(db: &FactsDb) -> i64 {
    db.query_row("SELECT COUNT(*) FROM complexity_metrics", [], |r| r.get(0))
        .expect("count complexity_metrics")
}

fn permanent_table_count(db: &FactsDb) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM duckdb_tables() WHERE temporary = false",
        [],
        |r| r.get(0),
    )
    .expect("count permanent tables")
}
