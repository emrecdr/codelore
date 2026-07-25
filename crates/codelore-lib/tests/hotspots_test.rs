use codelore_lib::Options;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use std::path::Path;
use std::process::Command;

#[test]
fn hotspots_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "should produce ≥1 hotspot row");

    // src/main.rs changed 4 times; src/lib.rs changed 1 time. Both Rust.
    // With similar complexity, main.rs should rank above lib.rs.
    let main_row = rows
        .iter()
        .find(|r| r.path == "src/main.rs")
        .expect("main.rs should be in hotspots");
    let lib_row = rows.iter().find(|r| r.path == "src/lib.rs");

    if let Some(lib) = lib_row {
        assert!(
            main_row.hotspot_score >= lib.hotspot_score,
            "main.rs (revs=4) should rank ≥ lib.rs (revs=1)"
        );
    }

    // Hotspot score should be in [0, 10] range (formula bounds:
    // percentile_rank ∈ [0,1], code_health ∈ [0,100], so score ∈ [0, 10])
    for row in &rows {
        assert!(
            row.hotspot_score >= 0.0,
            "hotspot score should be >= 0, got {} for {}",
            row.hotspot_score,
            row.path
        );
    }
}

/// The hotspots query joins `complexity_metrics` × `entities` on `kind='unit'`
/// to surface the file-level Maintainability Index (Coleman 1994 / SEI 1997
/// variant computed in `codelore-rca`). For the tiny Rust fixture, at least
/// one row should carry a non-null finite MI — the JOIN being broken would
/// silently return all-None.
#[test]
fn hotspots_surface_maintainability_index_for_rust_files() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    let any_finite_mi = rows
        .iter()
        .any(|r| matches!(r.mi, Some(v) if v.is_finite()));
    assert!(
        any_finite_mi,
        "expected ≥1 Rust hotspot row with a finite MI; got rows: {:?}",
        rows.iter()
            .map(|r| (r.path.as_str(), r.mi))
            .collect::<Vec<_>>()
    );
}

/// The hotspots SQL also surfaces an `ai_pct` column: share of commits
/// touching a file that carry the `ai-assisted` or `ai-authored`
/// attribution from `identity::bots`. Every hotspot row should have a
/// non-null `ai_pct` (in `[0, 100]`) — the LEFT JOIN with `file_ai` is
/// over the same node set as `file_revs`, so the only way to get None
/// here would be if the JOIN got silently broken (column collision,
/// rename drift, etc).
#[test]
fn hotspots_surface_ai_attribution_percentage() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "fixture should produce ≥1 hotspot row");
    for row in &rows {
        match row.ai_pct {
            Some(p) => assert!(
                (0.0..=100.0).contains(&p) && p.is_finite(),
                "ai_pct out of range for {}: {p}",
                row.path
            ),
            None => panic!(
                "ai_pct should be Some on every hotspot row (LEFT JOIN over the \
                 same node set as file_revs); got None on {}",
                row.path
            ),
        }
    }
}

/// `FactsDb::explain_sql` returns a non-empty `DuckDB` optimizer plan for
/// the hotspots SQL. The CLI's `--explain` flag routes through this
/// helper; missing or empty plan output would mean `--explain` silently
/// no-ops.
#[test]
fn explain_sql_returns_non_empty_plan() {
    use codelore_lib::analyses::hotspots::build_sql;
    use duckdb::params;
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Resolve the template placeholders via `build_sql` — the raw `SQL`
    // const carries `{cm_src}` / `{file_mi_cte}` markers that `DuckDB`
    // can't parse. Lineage off mirrors the ungrouped raw-`changes` runtime
    // path (default `Options` has canonical lineage ON, which would name a
    // `changes_lineage` temp this in-memory query never materialises).
    let sql = build_sql(
        &Options {
            use_canonical_lineage: false,
            ..Options::default()
        },
        "complexity_metrics",
    );
    let plan = db
        .explain_sql(&sql, params![1u32, i64::MAX])
        .expect("explain");
    assert!(
        !plan.is_empty(),
        "EXPLAIN plan should not be empty; got {plan:?}"
    );
    // DuckDB EXPLAIN output reliably contains either "PROJECTION" or
    // "HASH_JOIN" or "ORDER_BY" for any non-trivial query.
    let upper = plan.to_uppercase();
    assert!(
        upper.contains("PROJECTION")
            || upper.contains("ORDER")
            || upper.contains("JOIN")
            || upper.contains("AGGREGATE"),
        "EXPLAIN plan missing common operator names; got {plan:?}"
    );
}

/// Guard: turning canonical lineage OFF must make `build_sql` a pure
/// pass-through — the assembled SQL still reads raw `changes` in BOTH the
/// `file_revs` (`FROM changes`) and the aliased `file_ai` (`FROM changes ch`)
/// CTEs, with no `changes_lineage` anywhere. Turning lineage ON must be
/// exactly the source-table swap applied to that same reference in BOTH
/// CTEs — the `file_ai` rewrite being the whole point of routing through
/// `lineage::rewrite` instead of the old literal `FROM changes\n` replace
/// that only matched `file_revs`.
#[test]
fn build_sql_lineage_off_is_noop_and_on_swaps_both_ctes() {
    use codelore_lib::analyses::hotspots::build_sql;

    let sql_off = build_sql(
        &Options {
            use_canonical_lineage: false,
            ..Options::default()
        },
        "complexity_metrics",
    );
    // Lineage off is a no-op: raw `changes` in both CTEs, no lineage table.
    assert!(
        !sql_off.contains("changes_lineage"),
        "lineage-off must not route through changes_lineage:\n{sql_off}"
    );
    assert!(
        sql_off.contains("FROM changes\n"),
        "file_revs must read raw `changes` when lineage is off:\n{sql_off}"
    );
    assert!(
        sql_off.contains("FROM changes ch"),
        "file_ai must read raw `changes` when lineage is off:\n{sql_off}"
    );

    let sql_on = build_sql(
        &Options {
            use_canonical_lineage: true,
            ..Options::default()
        },
        "complexity_metrics",
    );
    // Lineage on is exactly the source-table swap over the off reference,
    // in BOTH CTEs (the aliased `file_ai` included) and nothing else.
    let expected_on = sql_off
        .replace("FROM changes\n", "FROM changes_lineage AS changes\n")
        .replace("FROM changes ch", "FROM changes_lineage ch");
    assert_eq!(
        sql_on, expected_on,
        "lineage-on must rewrite the source table in both file_revs and \
         file_ai and change nothing else"
    );
}

// ─── file_ai honors renames under canonical lineage ──────────────────────────

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("spawn git")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Stage everything and commit; each `msg` becomes its own message paragraph
/// (so an AI trailer lands in the commit body).
fn commit(dir: &Path, msgs: &[&str]) {
    git(dir, &["add", "."]);
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).arg("commit").arg("--quiet");
    for m in msgs {
        cmd.arg("-m").arg(m);
    }
    let ok = cmd.status().expect("spawn git commit").success();
    assert!(ok, "git commit failed for {msgs:?}");
}

/// A file's AI-attribution percentage must aggregate over its full rename
/// lineage, not just the commits that touched its current name. The pre-fix
/// `build_sql` rewrote only `file_revs` (`FROM changes`) to the lineage table
/// and left `file_ai` (`FROM changes ch`) reading raw `changes`, so a renamed
/// file's `ai_pct` counted a different — post-rename-only — population than
/// its `revs`.
#[test]
fn ai_pct_covers_pre_rename_commits_under_canonical_lineage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "ai@example.com"]);
    git(p, &["config", "user.name", "Dev"]);

    std::fs::create_dir_all(p.join("src")).expect("mkdir");
    // src/old.rs: a human seed, then two AI-assisted edits — all pre-rename.
    std::fs::write(p.join("src/old.rs"), "pub fn f() -> u32 {\n    1\n}\n").expect("write");
    commit(p, &["seed old"]);
    std::fs::write(p.join("src/old.rs"), "pub fn f() -> u32 {\n    2\n}\n").expect("write");
    commit(p, &["edit old", "Co-Authored-By: Claude"]);
    std::fs::write(p.join("src/old.rs"), "pub fn f() -> u32 {\n    3\n}\n").expect("write");
    commit(p, &["edit old again", "Co-Authored-By: Claude"]);

    // Pure rename to src/new.rs (human), then one human post-rename edit.
    git(p, &["mv", "src/old.rs", "src/new.rs"]);
    commit(p, &["rename old to new"]);
    std::fs::write(p.join("src/new.rs"), "pub fn f() -> u32 {\n    4\n}\n").expect("write");
    commit(p, &["edit new"]);

    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        min_revs: 1,
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");

    // The old path folds into the canonical (latest) name.
    assert!(
        !rows.iter().any(|r| r.path == "src/old.rs"),
        "old.rs should merge into src/new.rs under canonical lineage; got {rows:?}"
    );
    let row = rows
        .iter()
        .find(|r| r.path == "src/new.rs")
        .expect("src/new.rs must appear in hotspots");

    // Exactly two AI-assisted commits (both pre-rename) fold into the
    // canonical population. `ai_pct`'s denominator is that same population,
    // so it must equal `2 / revs * 100`. On the pre-fix path `file_ai` saw
    // only the post-rename human commits and reported 0.
    let ai = row.ai_pct.expect("ai_pct present on the canonical row");
    let expected = 2.0 / f64::from(row.revisions) * 100.0;
    assert!(
        (ai - expected).abs() < 1e-9,
        "ai_pct must cover the same population as revs: ai_pct={ai} revs={} expected={expected}",
        row.revisions
    );
    assert!(
        ai > 0.0,
        "pre-rename AI commits must fold into the canonical ai_pct"
    );
}

// ─── ai_pct survives --time-bucket ────────────────────────────────────────────

/// `ai_pct` is a COMMIT-LEVEL percentage: the share of the file's real
/// commits that carry an AI-attribution signal. Under `--time-bucket` the
/// change source feeding `file_revs` is `changes_bucketed`, whose `rev` is a
/// synthetic `date_trunc` string, not a real SHA. `file_ai` therefore must
/// keep reading a real-rev source so its `INNER JOIN commits co ON co.rev =
/// ch.rev` still matches; if `file_ai` followed the bucket rewrite the join
/// would never match and every file's `ai_pct` would come back NULL.
#[test]
fn ai_pct_is_populated_under_time_bucket() {
    use codelore_lib::options::TimeBucket;

    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "dev@example.com"]);
    git(p, &["config", "user.name", "Dev"]);

    std::fs::create_dir_all(p.join("src")).expect("mkdir");
    // One human seed, then one AI-assisted edit — same file, so its commit
    // population is one human + one AI commit (ai_pct expected = 50%).
    std::fs::write(p.join("src/app.rs"), "pub fn f() -> u32 {\n    1\n}\n").expect("write");
    commit(p, &["seed app"]);
    std::fs::write(p.join("src/app.rs"), "pub fn f() -> u32 {\n    2\n}\n").expect("write");
    commit(p, &["edit app", "Co-Authored-By: Claude"]);

    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        min_revs: 1,
        time_bucket: Some(TimeBucket::Month),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    let row = rows
        .iter()
        .find(|r| r.path == "src/app.rs")
        .expect("src/app.rs must appear in bucketed hotspots");

    // The regression routed file_ai through the bucket rewrite, making its
    // `rev` a date string that never joined `commits`, so this came back None
    // for every file under --time-bucket.
    let ai = row.ai_pct.expect(
        "ai_pct must be populated under --time-bucket (real-rev source, not changes_bucketed)",
    );
    assert!(
        ai > 0.0,
        "the AI-assisted commit must lift ai_pct above zero; got {ai}"
    );
    // One AI of two real commits → 50%, regardless of how the two collapse
    // into month-buckets (the bucket count only drives `revs`, never ai_pct).
    assert!(
        (ai - 50.0).abs() < 1e-9,
        "ai_pct is commit-level (1 AI / 2 commits = 50%), independent of bucketing; got {ai}"
    );
}

/// `--time-bucket` composed with `--use-canonical-lineage`: `file_ai` must
/// read `changes_lineage` (a real-rev source) so pre-rename AI commits still
/// fold into the canonical file's `ai_pct`. This also proves `changes_lineage`
/// is materialised on the bucketed-lineage path — `changes_bucketed` is itself
/// built `FROM changes_lineage`, so the lineage view is guaranteed present.
#[test]
fn ai_pct_is_populated_under_time_bucket_with_canonical_lineage() {
    use codelore_lib::options::TimeBucket;

    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-b", "main", "--quiet"]);
    git(p, &["config", "user.email", "dev@example.com"]);
    git(p, &["config", "user.name", "Dev"]);

    std::fs::create_dir_all(p.join("src")).expect("mkdir");
    // AI-assisted edits pre-rename, then a rename + human edit post-rename.
    std::fs::write(p.join("src/old.rs"), "pub fn f() -> u32 {\n    1\n}\n").expect("write");
    commit(p, &["seed old"]);
    std::fs::write(p.join("src/old.rs"), "pub fn f() -> u32 {\n    2\n}\n").expect("write");
    commit(p, &["edit old", "Co-Authored-By: Claude"]);
    git(p, &["mv", "src/old.rs", "src/new.rs"]);
    commit(p, &["rename old to new"]);
    std::fs::write(p.join("src/new.rs"), "pub fn f() -> u32 {\n    3\n}\n").expect("write");
    commit(p, &["edit new"]);

    let repo = GixRepo::open(p).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: p.to_path_buf(),
        min_revs: 1,
        time_bucket: Some(TimeBucket::Month),
        use_canonical_lineage: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    // Pre-rename history folds into the canonical (latest) name.
    let row = rows
        .iter()
        .find(|r| r.path == "src/new.rs")
        .expect("src/new.rs must appear under bucketed canonical lineage");

    let ai = row
        .ai_pct
        .expect("ai_pct must be populated under --time-bucket + canonical lineage");
    assert!(
        ai > 0.0,
        "the pre-rename AI commit must fold into the canonical ai_pct; got {ai}"
    );
}
