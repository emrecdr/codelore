//! F69 bench-gate spike: compare totals-CTE pattern vs window-function
//! rewrite for the ownership / code-health HHI fractal-value computation.
//!
//! The current production SQL (see `analyses/ownership.rs::SQL`) computes
//! per-file Herfindahl-Hirschman concentration via:
//!
//! ```sql
//! WITH author_revs AS (...),
//!      totals AS (SELECT path, SUM(revs) FROM author_revs GROUP BY path),
//!      hhi AS (
//!          SELECT ar.path, 1.0 - SUM(POWER(ar.revs/t.total, 2))
//!          FROM author_revs ar
//!          INNER JOIN totals t ON ar.path = t.path
//!          GROUP BY ar.path
//!      )
//! ```
//!
//! The window-function alternative collapses `totals` + the join into one
//! pass:
//!
//! ```sql
//! WITH author_revs_with_total AS (
//!     SELECT path, author, revs,
//!            SUM(revs) OVER (PARTITION BY path) AS total
//!     FROM author_revs
//! )
//! ```
//!
//! The roadmap entry says `DuckDB`'s planner *may* already collapse these
//! into equivalent plans. This spike verifies:
//!
//! 1. **Semantic equivalence** — both queries return identical rows.
//! 2. **Plan shape** — `EXPLAIN ANALYZE` output for each variant is
//!    captured so a reader can see whether `DuckDB`'s optimiser is
//!    dedupe'ing the totals scan.
//!
//! Wall-clock comparison on the medium fixture (500 commits / 25 files)
//! is too small to dominate measurement noise, so the spike emits the
//! EXPLAIN ANALYZE text + per-query elapsed via `Instant`; a definitive
//! kernel-snapshot bench is left to `benches/end_to_end.rs` once this
//! spike shows whether the rewrite is worth the line-count change.

use std::time::Instant;

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// Inline fixture: 30 commits, 5 files, 3 authors. Small enough to be
/// rock-solid on every CI OS (the 500-commit `test_support::medium_repo`
/// fixture has a Linux-only fragility the broader test suite hasn't
/// exercised), but produces enough (path, author) groups and revs that
/// the CTE-vs-window plan difference is visible in `EXPLAIN ANALYZE`.
fn build_spike_fixture() -> tempfile::TempDir {
    fn git(path: &std::path::Path, args: &[&str]) {
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
    fn write(p: std::path::PathBuf, content: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-b", "main", "--quiet"]);
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "user.name", "T"]);

    let authors = [
        ("Alice", "alice@example.com"),
        ("Bob", "bob@example.com"),
        ("Carol", "carol@example.com"),
    ];
    let files = ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs", "src/e.rs"];

    for i in 0..30 {
        let f = files[i % files.len()];
        write(path.join(f), &format!("// v{i}\npub fn f_{i}() -> u32 {{ {i} }}\n"));
        let (name, email) = authors[i % authors.len()];
        let author = format!("{name} <{email}>");
        git(path, &["add", f]);
        git(
            path,
            &[
                "commit",
                "-m",
                &format!("c{i}"),
                "--author",
                &author,
                "--quiet",
            ],
        );
    }
    dir
}

const SQL_CTE: &str = "
    WITH author_revs AS (
        SELECT
            changes.path,
            commits.canonical_author AS author,
            COUNT(changes.rev) AS revs
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        GROUP BY changes.path, commits.canonical_author
    ),
    totals AS (
        SELECT path, SUM(revs) AS total
        FROM author_revs
        GROUP BY path
    ),
    hhi AS (
        SELECT
            ar.path,
            t.total,
            1.0 - SUM(POWER(CAST(ar.revs AS DOUBLE) / NULLIF(CAST(t.total AS DOUBLE), 0), 2)) AS fv
        FROM author_revs ar
        INNER JOIN totals t ON ar.path = t.path
        GROUP BY ar.path, t.total
    )
    SELECT path, total, fv FROM hhi ORDER BY path
";

const SQL_WINDOW: &str = "
    WITH author_revs AS (
        SELECT
            changes.path,
            commits.canonical_author AS author,
            COUNT(changes.rev) AS revs
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        GROUP BY changes.path, commits.canonical_author
    ),
    author_revs_with_total AS (
        SELECT
            path,
            author,
            revs,
            SUM(revs) OVER (PARTITION BY path) AS total
        FROM author_revs
    ),
    hhi AS (
        SELECT
            path,
            ANY_VALUE(total) AS total,
            1.0 - SUM(POWER(CAST(revs AS DOUBLE) / NULLIF(CAST(total AS DOUBLE), 0), 2)) AS fv
        FROM author_revs_with_total
        GROUP BY path
    )
    SELECT path, total, fv FROM hhi ORDER BY path
";

#[derive(Debug, PartialEq)]
struct HhiRow {
    path: String,
    total: i64,
    fv: f64,
}

fn collect_hhi(db: &FactsDb, sql: &str) -> Vec<HhiRow> {
    let mut stmt = db.conn().prepare(sql).expect("prepare");
    let rows = stmt
        .query_map([], |r| {
            Ok(HhiRow {
                path: r.get::<_, String>(0)?,
                total: r.get::<_, i64>(1)?,
                fv: r.get::<_, f64>(2)?,
            })
        })
        .expect("query_map");
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect")
}

#[test]
fn cte_and_window_return_byte_identical_rows() {
    let fixture = build_spike_fixture();
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        ..Options::default()
    };
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    db.ingest(&repo, &opts).expect("ingest");

    let t_cte = Instant::now();
    let rows_cte = collect_hhi(&db, SQL_CTE);
    let dur_cte = t_cte.elapsed();

    let t_win = Instant::now();
    let rows_win = collect_hhi(&db, SQL_WINDOW);
    let dur_win = t_win.elapsed();

    // Semantic equivalence: identical row sets (same length + every row
    // matches under field-by-field equality with a tight float epsilon).
    assert_eq!(
        rows_cte.len(),
        rows_win.len(),
        "row counts differ: cte={} window={}",
        rows_cte.len(),
        rows_win.len(),
    );
    for (a, b) in rows_cte.iter().zip(rows_win.iter()) {
        assert_eq!(a.path, b.path, "path mismatch");
        assert_eq!(a.total, b.total, "total mismatch for `{}`", a.path);
        assert!(
            (a.fv - b.fv).abs() < 1e-12,
            "fv mismatch for `{}`: cte={} window={}",
            a.path,
            a.fv,
            b.fv,
        );
    }

    // Surface the wall-clock times. The 30-commit fixture is small
    // enough that either variant completes in single-digit milliseconds,
    // so these numbers are largely indicative; the real bench-gate runs
    // on the kernel snapshot in CI. Tests pipe stdout to nextest's
    // per-test buffer, visible with `cargo test -- --nocapture`.
    eprintln!("[F69 spike] inline-fixture timings:");
    eprintln!("  CTE-totals:       {dur_cte:?}");
    eprintln!("  window function:  {dur_win:?}");
    eprintln!(
        "  delta:            {}",
        if dur_cte > dur_win {
            format!("window faster by {:?}", dur_cte.saturating_sub(dur_win))
        } else {
            format!("CTE faster by {:?}", dur_win.saturating_sub(dur_cte))
        }
    );
}

/// Emits both EXPLAIN ANALYZE plans so a reader can compare scan / hash
/// / aggregate counts at a glance. Not an assertion test — purely
/// observational; runs with `cargo test -- --nocapture` to see output.
#[test]
fn capture_explain_analyze_plans() {
    let fixture = build_spike_fixture();
    let opts = Options {
        repo_path: fixture.path().to_path_buf(),
        ..Options::default()
    };
    let repo = GixRepo::open(fixture.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    db.ingest(&repo, &opts).expect("ingest");

    let dump = |label: &str, sql: &str| {
        // DuckDB's `EXPLAIN ANALYZE` returns two columns: `explain_key`
        // (always `analyzed_plan` for the ANALYZE variant) and
        // `explain_value` (the actual plan text). Reading only column 0
        // surfaces the key, not the plan; we want column 1.
        let explain_sql = format!("EXPLAIN ANALYZE {sql}");
        let mut stmt = db.conn().prepare(&explain_sql).expect("prepare");
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .expect("query_map");
        let plan = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("collect")
            .join("\n");
        eprintln!("\n=== EXPLAIN ANALYZE: {label} ===\n{plan}\n");
    };
    dump("CTE-totals", SQL_CTE);
    dump("window function", SQL_WINDOW);
}
