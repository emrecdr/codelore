//! Engine tests for the change-set engine: the projected code-health half
//! (driven via `change_set::project_health` directly) and the full report
//! assembly (cycle splice, coupling absences, findings, sidecar memoisation)
//! via `change_set::build_change_set_report`. Each test clones its own copy
//! of a bundle fixture because it mutates the working tree.

use std::path::Path;

use codelore_lib::Options;
use codelore_lib::cache::repo_cache_dir;
use codelore_lib::change_set::{build_change_set_report, project_health};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::{GixRepo, Repo, WorktreeChange, WorktreeChangeKind};
use codelore_lib::test_support::coupling_repo;
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

/// Append `text` to the file at `path`.
fn append(path: &Path, text: &str) {
    let mut content = std::fs::read_to_string(path).expect("read file");
    content.push_str(text);
    std::fs::write(path, content).expect("write file");
}

/// Count the `.json` sidecar entries in `dir` (0 when the dir doesn't exist).
fn sidecar_count(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count()
    })
}

#[test]
fn newly_cyclic_detected_when_edit_introduces_cycle() {
    let (fx, repo, db, opts) = fresh();
    // A working-tree edit that makes src/main.rs and src/lib.rs import each
    // other via the `crate::` form the Rust resolver maps to `src/<name>.rs`.
    // Neither file imports the other at HEAD, so the pair is a NEW cycle.
    append(&fx.dir.path().join("src/main.rs"), "use crate::lib;\n");
    append(&fx.dir.path().join("src/lib.rs"), "use crate::main;\n");
    let cache_root = tempfile::tempdir().expect("cache root");

    let report = build_change_set_report(&db, &repo, &opts, cache_root.path()).expect("report");

    assert_eq!(
        report.newly_cyclic_paths,
        vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
        "the edit-introduced cycle members must be newly cyclic, sorted",
    );
    for path in ["src/lib.rs", "src/main.rs"] {
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == "newly-cyclic" && f.path == path),
            "a newly-cyclic finding must name {path}: {:?}",
            report.findings,
        );
    }
}

#[test]
fn absence_fires_for_historical_partner() {
    // The coupling fixture's src/alpha/svc.rs and src/beta/svc.rs co-change in
    // 7 of 19 commits (each has 9 revisions) — a Fisher-significant pair well
    // above the shared-revisions floor. Touching only one side must flag the
    // other as an absent historical partner.
    let fx = coupling_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open repo");
    let db = FactsDb::new_in_memory().expect("open fact store");
    let opts = Options {
        repo_path: fx.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    append(
        &fx.dir.path().join("src/alpha/svc.rs"),
        "\npub fn gate_probe() {}\n",
    );
    let cache_root = tempfile::tempdir().expect("cache root");

    let report = build_change_set_report(&db, &repo, &opts, cache_root.path()).expect("report");

    let absence = report
        .coupling_absences
        .iter()
        .find(|a| a.touched_file == "src/alpha/svc.rs" && a.expected_partner == "src/beta/svc.rs")
        .expect("the historically-coupled partner must be flagged absent");
    assert!(
        absence.historical_shared_revs >= 5,
        "the pair's shared revisions carry the signal strength: {absence:?}",
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.kind == "coupling-absence" && f.path == "src/alpha/svc.rs"),
        "a coupling-absence finding must name the touched file: {:?}",
        report.findings,
    );
}

#[test]
fn report_is_memoised_by_content() {
    let (fx, repo, db, opts) = fresh();
    let main_path = fx.dir.path().join("src/main.rs");
    append(&main_path, MONSTER_FN);
    let cache_root = tempfile::tempdir().expect("cache root");
    let sidecar_dir = repo_cache_dir(cache_root.path(), fx.dir.path()).join("change-set");

    let first = build_change_set_report(&db, &repo, &opts, cache_root.path()).expect("first");
    assert_eq!(
        sidecar_count(&sidecar_dir),
        1,
        "first build writes a sidecar"
    );
    let second = build_change_set_report(&db, &repo, &opts, cache_root.path()).expect("second");
    assert_eq!(first, second, "a warm rebuild must return the same report");
    assert_eq!(
        sidecar_count(&sidecar_dir),
        1,
        "unchanged content must hit the same sidecar entry",
    );

    append(&main_path, "\n// content flip\n");
    let _third = build_change_set_report(&db, &repo, &opts, cache_root.path()).expect("third");
    assert_eq!(
        sidecar_count(&sidecar_dir),
        2,
        "flipped content must produce a different cache key",
    );
}

#[test]
fn report_key_folds_in_defect_calibration() {
    // `--defect-calibration` substitutes smell weights inside the scoring
    // engine, so it changes the MEASURED scores in the report — not just the
    // verdict. A calibrated and an uncalibrated run on the same worktree and
    // HEAD must not share a cache entry, and the key must track the artifact's
    // CONTENT so editing it in place is visible.
    let fx = differential_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open repo");
    let head_sha = repo.head_sha().expect("head sha");
    let root = fx.dir.path();
    let changes = [modified("src/main.rs")];

    let calib_a = root.join("a.calib.json");
    std::fs::write(&calib_a, br#"{"weights":"a"}"#).expect("write calib a");
    let calib_b = root.join("b.calib.json");
    std::fs::write(&calib_b, br#"{"weights":"b"}"#).expect("write calib b");

    let key = |cal: Option<&std::path::Path>| {
        codelore_lib::change_set::cache::report_key(&head_sha, root, &changes, cal)
            .expect("report_key")
    };

    let uncalibrated = key(None);
    let calibrated_a = key(Some(&calib_a));
    let calibrated_a_again = key(Some(&calib_a));
    let calibrated_b = key(Some(&calib_b));

    assert_ne!(
        uncalibrated, calibrated_a,
        "calibrated and uncalibrated runs must not share a cache key",
    );
    assert_eq!(
        calibrated_a, calibrated_a_again,
        "identical artifact content must yield a stable key",
    );
    assert_ne!(
        calibrated_a, calibrated_b,
        "different artifact content must yield different keys",
    );
}

#[test]
fn finding_ids_stable_across_runs() {
    let (fx, repo, db, opts) = fresh();
    append(&fx.dir.path().join("src/main.rs"), MONSTER_FN);
    // Distinct cache roots so the second build recomputes rather than being
    // served from the first build's sidecar.
    let cache_a = tempfile::tempdir().expect("cache a");
    let cache_b = tempfile::tempdir().expect("cache b");

    let first = build_change_set_report(&db, &repo, &opts, cache_a.path()).expect("first");
    let second = build_change_set_report(&db, &repo, &opts, cache_b.path()).expect("second");

    assert!(
        !first.findings.is_empty(),
        "the monster append must produce at least a health-drop finding",
    );
    let ids_a: Vec<&str> = first.findings.iter().map(|f| f.id.as_str()).collect();
    let ids_b: Vec<&str> = second.findings.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids_a, ids_b, "finding ids must be stable across runs");
    for f in &first.findings {
        assert_eq!(
            f.id.len(),
            12,
            "finding id is a 12-hex digest prefix: {f:?}"
        );
        assert!(
            f.id.chars().all(|c| c.is_ascii_hexdigit()),
            "finding id must be hex: {f:?}",
        );
    }
}

#[test]
fn report_renders_byte_identical_across_two_builds() {
    // Full determinism: two independent ingests of the same mutated clone,
    // each with its own cold cache, must serialize to byte-identical JSON.
    let fx = differential_repo::build();
    let repo = GixRepo::open(fx.dir.path()).expect("open repo");
    let opts = Options {
        repo_path: fx.dir.path().to_path_buf(),
        ..Options::default()
    };
    append(&fx.dir.path().join("src/main.rs"), MONSTER_FN);

    let mut jsons = Vec::new();
    for label in ["a", "b"] {
        let db = FactsDb::new_in_memory().expect("open fact store");
        db.ingest(&repo, &opts).expect("ingest");
        let cache_root = tempfile::tempdir().expect("cache root");
        let report = build_change_set_report(&db, &repo, &opts, cache_root.path()).expect("report");
        jsons.push(serde_json::to_string(&report).unwrap_or_else(|e| panic!("json {label}: {e}")));
    }
    assert_eq!(jsons[0], jsons[1], "rendered report must be byte-identical");
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
