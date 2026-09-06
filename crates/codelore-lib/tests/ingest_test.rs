use std::path::Path;

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn run_git(path: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

#[test]
fn ingest_classifies_bot_commits_as_ai_authored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(
        path,
        &["config", "user.email", "dependabot[bot]@noreply.github.com"],
    );
    run_git(path, &["config", "user.name", "dependabot[bot]"]);

    write_file(
        path,
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    );
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "bump deps", "--quiet"]);

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    let ai_attr: String = db
        .query_one_value("SELECT ai_attribution FROM commits LIMIT 1")
        .expect("ai_attribution query");
    assert_eq!(
        ai_attr, "ai-authored",
        "dependabot commits should be ai-authored"
    );

    let is_bot: String = db
        .query_one_value(
            "SELECT CAST(is_bot AS TEXT) FROM author_aliases WHERE raw_email = 'dependabot[bot]@noreply.github.com'",
        )
        .expect("is_bot query");
    assert_eq!(
        is_bot, "true",
        "dependabot should be flagged as bot in author_aliases"
    );
}

#[test]
fn ingest_classifies_human_commits_correctly() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    // All tiny_repo commits are by a human author
    let ai_attr_values: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM commits WHERE ai_attribution = 'human'",
        )
        .expect("human count query");
    let count: u32 = ai_attr_values.parse().unwrap();
    assert_eq!(
        count, 5,
        "all 5 tiny repo commits should be classified as human"
    );
}

#[test]
fn ingest_populates_author_aliases() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    // tiny_repo uses "tiny@example.com" as the author
    let alias_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM author_aliases WHERE raw_email = 'tiny@example.com'",
        )
        .expect("alias count query");
    let n: u32 = alias_count.parse().unwrap();
    assert_eq!(n, 1, "expected one alias row for tiny@example.com");
}

#[test]
fn ingest_tiny_repo_writes_5_commits() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options::default();
    let n = db.ingest(&repo, &opts).expect("ingest");
    assert_eq!(n.commits_ingested, 5);

    let count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits")
        .expect("count");
    assert_eq!(count, "5");
}

#[test]
fn ingest_populates_complexity_for_tier1_files() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let entity_count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM entities WHERE path = 'src/main.rs'")
        .expect("entity count query");
    let n: u32 = entity_count.parse().unwrap();
    assert!(n >= 1, "expected ≥1 entity for src/main.rs, got {n}");

    let metric_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM complexity_metrics WHERE path = 'src/main.rs'",
        )
        .expect("metric count query");
    let m: u32 = metric_count.parse().unwrap();
    assert!(
        m >= 1,
        "expected ≥1 complexity row for src/main.rs, got {m}"
    );
}

/// Ingest the biomarker fixture and assert that the `complex` function row
/// carries real `nargs` and `max_nesting` values (both were wired as constant
/// zeros before schema v5). The `complex` function takes three named
/// parameters, and its body contains several layers of nested control flow.
#[cfg(feature = "test-support")]
#[test]
fn ingest_biomarker_repo_persists_nargs_and_nesting() {
    let biomarker = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(biomarker.dir.path()).expect("open biomarker repo");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: biomarker.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Entity names are stored with a `@{start}-{end}` suffix by dedup_entities.
    // Use LIKE to match the function by its base name.
    let nargs: String = db
        .query_one_value(
            "SELECT CAST(nargs AS TEXT) FROM complexity_metrics \
             WHERE path = 'src/complex.rs' AND name LIKE 'complex@%'",
        )
        .expect("nargs query");
    let nargs: u32 = nargs.parse().expect("nargs parse");
    assert!(
        nargs > 0,
        "complex() takes 3 args — nargs should be > 0, got {nargs}"
    );

    let max_nesting: String = db
        .query_one_value(
            "SELECT CAST(max_nesting AS TEXT) FROM complexity_metrics \
             WHERE path = 'src/complex.rs' AND name LIKE 'complex@%'",
        )
        .expect("max_nesting query");
    let max_nesting: u32 = max_nesting.parse().expect("max_nesting parse");
    assert!(
        max_nesting > 0,
        "complex() has nested loops and ifs — max_nesting should be > 0, got {max_nesting}"
    );
}

/// Regression test: previously the `hunks` table existed in
/// schema but `append_change` never wrote to it — `Repo::diff_hunks`
/// parsed the headers, attached them to `FileChange.hunks`, and the
/// payload was dropped on the floor. The new ingest path writes one
/// row per hunk; this test makes two edits to the same file in
/// separate, non-adjacent regions (the gix diff splits them into two
/// hunks) and asserts both land in the table with the schema-required
/// NOT NULL fields and unique composite key.
#[test]
fn ingest_writes_hunk_rows_to_hunks_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "alice@example.com"]);
    run_git(path, &["config", "user.name", "Alice"]);

    // Initial file with a 30-line body — two later edits will land in
    // separate hunks (gix's diff coalesces only within ~3-line
    // proximity, so an edit at line 2 and an edit at line 25 are
    // guaranteed to surface as two distinct `@@ … @@` headers).
    let initial: String = (1..=30)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    write_file(path, "src/code.rs", &format!("{initial}\n"));
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial", "--quiet"]);

    // Edit lines 2 and 25 — far enough apart that gix emits two hunks.
    let edited: String = (1..=30)
        .map(|n| match n {
            2 => "line 2 EDITED".to_string(),
            25 => "line 25 EDITED".to_string(),
            other => format!("line {other}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_file(path, "src/code.rs", &format!("{edited}\n"));
    run_git(
        path,
        &["commit", "-am", "two non-adjacent edits", "--quiet"],
    );

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options::default();
    db.ingest(&repo, &opts).expect("ingest");

    // Pre-fix this returned 0 — `append_change` never wrote a hunks
    // row, even though `FileChange.hunks` carried the parsed payload.
    let hunk_count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM hunks WHERE path = 'src/code.rs'")
        .expect("hunk count query");
    let n: u32 = hunk_count.parse().unwrap();
    assert!(
        n >= 2,
        "expected ≥2 hunk rows for the two-edit commit on src/code.rs, got {n}"
    );

    // Verify schema v4 NOT NULL invariant: every persisted hunk row
    // has all four offsets populated (the parser drops malformed
    // hunks, and `Hunk` fields are u32 in Rust, so NULLs would only
    // happen if a future code path bypassed the type system).
    let nulls: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM hunks \
             WHERE old_start IS NULL OR old_lines IS NULL \
                OR new_start IS NULL OR new_lines IS NULL",
        )
        .expect("null offsets query");
    assert_eq!(nulls, "0", "no hunk row may have NULL offset columns");
}

/// Run `git commit` with author and committer dates pinned to a fixed
/// instant, so the fixture this test builds is byte-deterministic across
/// runs and machines (no reliance on wall-clock time). Gated with its only
/// caller, the volume regression test below.
#[cfg(not(target_os = "windows"))]
fn commit_at(path: &Path, message: &str, date: &str) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["commit", "-q", "-m", message, "--date", date])
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git commit");
    assert!(status.success(), "git commit '{message}' failed");
}

/// Regression test for the FK-flush ordering hazard on large-repo ingest.
///
/// The `commits` → `changes` → `hunks` tables are written through three
/// independent `DuckDB` Appenders. An Appender buffers rows and only performs
/// its foreign-key-checked physical write when the buffer reaches a large
/// internal threshold (observed reproducibly at `204_800` rows on a file-backed
/// connection — 100 standard vectors). At that write `DuckDB` validates every
/// buffered row's FK. The three buffers reach the threshold at uncorrelated
/// points, so before the fix the `hunks` (or `changes`) buffer performed its
/// checked write while the referenced `changes` (or `commits`) rows were still
/// buffered unwritten — the referent was absent and ingest aborted with
/// `append hunk: Failed to append: Violates foreign key constraint`.
///
/// Reproducing it requires two things the existing ingest tests lack:
///   1. A **file-backed** `FactsDb` (`FactsDb::open`). Every other ingest test
///      uses `FactsDb::new_in_memory()`, whose Appenders never trip this
///      check — which is exactly why the bug shipped invisibly.
///   2. Enough hunk volume to cross the `204_800`-row threshold. This builds a
///      repo whose modify commits churn 50 files with 20 non-adjacent hunks
///      each, so `hunks` passes `204_800` rows (`~215_000` total) well inside
///      the drain, while early commits/changes are still buffered.
///
/// On unfixed main this panics at `db.ingest(...)` with the FK violation; with
/// the flush-ordering guard it completes and every child row's parent exists.
///
/// Fixture trade-off: unlike the shared bundle-backed fixtures in `test_support`
/// (checked-in bundles, chosen because multi-process git builders flaked on
/// loaded CI runners), this is a single test-local builder — no shared fixture
/// has the high-volume shape the threshold needs. It shells out to `git` per
/// commit within one test process and uses fixed author/committer dates so the
/// built repo is deterministic.
///
/// Not run on windows: this test is volume coverage, not platform coverage,
/// and its per-commit git spawns are exactly the workload whose process-spawn
/// overhead priced the full suite off hosted windows runners. The flush-order
/// mechanism it guards is platform-independent.
#[cfg(not(target_os = "windows"))]
#[test]
fn ingest_large_repo_crosses_fk_flush_threshold() {
    // 50 files * 20 hunks * 215 modify commits ≈ 215_000 hunk rows, past the
    // 204_800-row FK-check threshold. Files, edits, and commit count are the
    // three knobs on total hunk volume; keep their product above the threshold.
    const FILE_COUNT: usize = 50;
    const EDITS_PER_FILE: usize = 20;
    const MODIFY_COMMITS: usize = 215;
    // Lines between successive edit anchors. Git coalesces edits within
    // 2 * context (2 * 3 = 6) lines into one hunk, so an 8-line gap guarantees
    // each of the EDITS_PER_FILE anchors surfaces as its own `@@ … @@` hunk.
    const ANCHOR_GAP: usize = 8;
    const BODY_LINES: usize = EDITS_PER_FILE * ANCHOR_GAP + 4;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "flush@example.com"]);
    run_git(path, &["config", "user.name", "Flush Tester"]);

    // A file body where line `1 + e*ANCHOR_GAP` (for e in 0..EDITS_PER_FILE)
    // carries a per-commit marker; the gaps between anchors keep the edits in
    // separate hunks.
    let body = |marker: usize| -> String {
        let mut lines: Vec<String> = (0..BODY_LINES).map(|n| format!("line {n}")).collect();
        for e in 0..EDITS_PER_FILE {
            lines[1 + e * ANCHOR_GAP] = format!("edit {e} v{marker}");
        }
        lines.join("\n") + "\n"
    };

    // Fixed epoch base for deterministic, strictly-increasing commit dates.
    // git accepts `@<unix-seconds> <tz>`; a fixed stride per commit keeps
    // chronology monotonic without calendar-overflow arithmetic.
    // 2026-01-01T00:00:00Z = 1_767_225_600; one hour (3600s) per commit.
    let commit_date = |seq: u64| -> String { format!("@{} +0000", 1_767_225_600 + seq * 3600) };

    // Seed: add all files. These Added change rows carry no hunks (adds have
    // empty hunk lists) but establish the files the modify commits churn.
    for f in 0..FILE_COUNT {
        write_file(path, &format!("src/f{f}.rs"), &body(0));
    }
    run_git(path, &["add", "."]);
    commit_at(path, "seed", &commit_date(0));

    // Each modify commit rewrites every file's anchors, producing
    // EDITS_PER_FILE non-adjacent hunks per file. FILE_COUNT change rows and
    // FILE_COUNT * EDITS_PER_FILE hunk rows accrue per commit, pushing `hunks`
    // past the 204_800-row FK-check threshold before the drain ends.
    for c in 0..MODIFY_COMMITS {
        for f in 0..FILE_COUNT {
            write_file(path, &format!("src/f{f}.rs"), &body(c + 1));
        }
        run_git(path, &["add", "."]);
        commit_at(path, &format!("modify {c}"), &commit_date(c as u64 + 1));
    }

    let repo = GixRepo::open(path).expect("open");
    // File-backed store — the in-memory store does not exhibit the FK-check
    // behaviour, so this test must not use `FactsDb::new_in_memory()`.
    let db = FactsDb::open(dir.path().join("facts.duckdb")).expect("db");
    let opts = Options::default();
    // On unfixed main this aborts with a foreign-key violation once the `hunks`
    // buffer performs its FK-checked write ahead of the parent buffers.
    db.ingest(&repo, &opts)
        .expect("ingest must not abort on FK-flush ordering");

    // Confirm we actually crossed the FK-check threshold — otherwise the test
    // would pass on unfixed main and prove nothing.
    let hunk_rows: u64 = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM hunks")
        .expect("hunk count query")
        .parse()
        .expect("parse hunk count");
    assert!(
        hunk_rows > 204_800,
        "test must push `hunks` past the 204_800-row FK-check threshold; got {hunk_rows}"
    );

    // Every child row's FK referent must be present (the constraint DuckDB
    // enforces at flush). A clean ingest already guarantees this, but assert it
    // directly so a future regression that silently drops rows is caught.
    let orphan_changes: u64 = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM changes c \
             WHERE NOT EXISTS (SELECT 1 FROM commits m WHERE m.rev = c.rev)",
        )
        .expect("orphan changes query")
        .parse()
        .expect("parse orphan changes");
    assert_eq!(
        orphan_changes, 0,
        "no change row may reference a missing commit"
    );
    let orphan_hunks: u64 = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM hunks h \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM changes c WHERE c.rev = h.rev AND c.path = h.path \
             )",
        )
        .expect("orphan hunks query")
        .parse()
        .expect("parse orphan hunks");
    assert_eq!(
        orphan_hunks, 0,
        "no hunk row may reference a missing change"
    );
}

/// Collect a canonical (multiset-comparable) sequence of per-function
/// complexity facts. ORDER BY over every compared column turns the row set
/// into a stable sequence, so `Vec` equality is multiset equality.
fn complexity_facts(db: &FactsDb) -> Vec<String> {
    let mut stmt = db
        .prepare(
            "SELECT path, name, cyclomatic, cognitive, sloc, nargs, max_nesting, bool_ops \
             FROM complexity_metrics \
             ORDER BY path, name, cyclomatic, cognitive, sloc, nargs, max_nesting, bool_ops",
        )
        .expect("prepare complexity rows");
    let mapped = stmt
        .query_map([], |r| {
            Ok(format!(
                "{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<i64>>(7)?,
            ))
        })
        .expect("query complexity rows");
    mapped
        .collect::<Result<Vec<_>, _>>()
        .expect("collect complexity rows")
}

/// Head-only ingest must (a) leave every history table empty with
/// truthfully-zero stats, and (b) extract the IDENTICAL per-function
/// complexity fact set a full ingest of the same fixture produces — the
/// HEAD tree is ground truth for both modes, so any divergence means one
/// of them scans the wrong file set.
#[test]
fn head_only_ingest_matches_full_ingest_complexity_and_leaves_history_empty() {
    let bio = codelore_lib::test_support::biomarker_repo::build();
    let repo = GixRepo::open(bio.dir.path()).expect("open");

    let full_opts = Options {
        repo_path: bio.dir.path().to_path_buf(),
        ..Options::default()
    };
    let head_only_opts = Options {
        head_only_ingest: true,
        ..full_opts.clone()
    };

    let full_db = FactsDb::new_in_memory().expect("full db");
    full_db.ingest(&repo, &full_opts).expect("full ingest");

    let head_db = FactsDb::new_in_memory().expect("head-only db");
    let stats = head_db
        .ingest(&repo, &head_only_opts)
        .expect("head-only ingest");

    // Truthful stats: nothing was walked.
    assert_eq!(stats.commits_ingested, 0, "no commits walked in head-only");
    assert_eq!(stats.changes_ingested, 0, "no changes walked in head-only");
    assert_eq!(stats.clones_ingested, 0, "clones pass skipped in head-only");

    let count = |db: &FactsDb, table: &str| -> u64 {
        db.query_one_value(&format!("SELECT CAST(COUNT(*) AS TEXT) FROM {table}"))
            .expect("count query")
            .parse()
            .expect("parse count")
    };
    assert_eq!(count(&head_db, "commits"), 0, "commits must stay empty");
    assert_eq!(count(&head_db, "changes"), 0, "changes must stay empty");
    assert!(
        count(&head_db, "complexity_metrics") > 0,
        "complexity_metrics must be populated by head-only ingest"
    );

    // Head-only rows are stamped with the real HEAD SHA (there is no
    // commits table to derive a rev from).
    let head_rev = head_db
        .query_one_value("SELECT DISTINCT rev FROM complexity_metrics")
        .expect("distinct rev");
    assert_eq!(
        head_rev, bio.head_sha,
        "head-only rows must carry the fixture's HEAD SHA"
    );

    // Identical multiset of per-function complexity facts.
    let head_rows = complexity_facts(&head_db);
    let full_rows = complexity_facts(&full_db);
    assert!(!head_rows.is_empty(), "fixture must yield complexity rows");
    assert_eq!(
        head_rows, full_rows,
        "head-only and full ingest must extract the same complexity facts from the same tree"
    );

    // Identical multiset of resolved import edges. `src/importer.rs` in the
    // fixture carries a resolvable `use crate::trivial::trivial;` edge, so
    // both modes must agree on at least one non-NULL `target_path` row —
    // the HEAD tree is the same ground truth for imports as it is for
    // complexity.
    let import_rows = |db: &FactsDb| -> Vec<String> {
        let mut stmt = db
            .prepare(
                "SELECT src_path, target_path FROM imports \
                 WHERE target_path IS NOT NULL \
                 ORDER BY 1, 2",
            )
            .expect("prepare imports rows");
        let mapped = stmt
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                ))
            })
            .expect("query imports rows");
        mapped
            .collect::<Result<Vec<_>, _>>()
            .expect("collect imports rows")
    };
    let head_import_rows = import_rows(&head_db);
    let full_import_rows = import_rows(&full_db);
    assert!(
        !head_import_rows.is_empty(),
        "fixture must yield at least one resolved import edge"
    );
    assert_eq!(
        head_import_rows, full_import_rows,
        "head-only and full ingest must resolve the same import edges from the same tree"
    );

    // Commits stay empty for head-only regardless of how much else the
    // head-only path now populates.
    assert_eq!(
        count(&head_db, "commits"),
        0,
        "commits must stay empty for head-only ingest"
    );
}

/// A real ingest must leave the HEAD scan's coverage behind it.
///
/// The unit tests around `head_scan_coverage_verdict` seed `provenance`
/// themselves, so they verify the predicate but never that anything writes it.
/// Deleting both `set_provenance` calls from the scan leaves every one of them
/// green while the gate reads `Unknown` forever and the enforcement is inert.
/// This test closes that gap by running the scan for real and asserting the
/// counts arrive, so the write and the read are pinned by different tests.
#[test]
fn a_real_ingest_detects_an_oversize_majority_the_loss_ratio_cannot_see() {
    // The first end-to-end test of the coverage feature, and it exists because
    // this is the ONE defect reachable from file content. The loss reasons
    // (`blob read failed`, `parse error`) need object-store failure — nine `.rs`
    // files of invalid UTF-8 beside one valid file still ingest as
    // `Met { scored: 10, eligible: 10 }`, because tree-sitter tolerates the
    // garbage and scores every file. The size cap does not: it is applied
    // before the parser, from `source.len()` alone.
    //
    // The shape being caught: this repository loses NOTHING, so its loss ratio
    // is a perfect 1/1 and reads as complete coverage, while the table it
    // produced describes one file out of three.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "dev@example.com"]);
    run_git(path, &["config", "user.name", "Dev"]);

    std::fs::create_dir_all(path.join("src")).expect("mkdir");
    std::fs::write(path.join("src/real.rs"), "pub fn a() -> u32 { 1 }\n").expect("write real");
    // Two files past the AST cap, so the skipped set outnumbers the scanned one.
    let bulk = "// ".to_string() + &"x".repeat(2 * 1024 * 1024) + "\n";
    for name in ["src/bundle_a.rs", "src/bundle_b.rs"] {
        std::fs::write(path.join(name), &bulk).expect("write bundle");
    }
    run_git(path, &["add", "."]);
    run_git(
        path,
        &[
            "commit",
            "-m",
            "one real file beside two bundles",
            "--quiet",
        ],
    );

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let verdict = db.head_scan_coverage_verdict().expect("verdict");
    assert_eq!(
        verdict,
        codelore_lib::facts::ScanCoverageVerdict::OversizeMajority {
            scored: 1,
            oversize: 2
        },
        "a scan that skipped more than it scored must say so; \
         `Met` here would report complete coverage of a table describing one file in three"
    );
}

#[test]
fn a_real_ingest_records_the_head_scan_coverage() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Read through the verdict rather than the raw counts, so the write is
    // pinned by the same surface the gate consumes: an absent write reads as
    // `Unknown`, which this destructure rejects.
    let codelore_lib::facts::ScanCoverageVerdict::Met { scored, eligible } =
        db.head_scan_coverage_verdict().expect("verdict")
    else {
        panic!(
            "the HEAD complexity scan must record its coverage as `Met` on a \
             clean fixture; anything else means nothing persists the counts and \
             the gate can never enforce the floor"
        );
    };

    assert!(
        eligible >= 2,
        "expected the fixture's Tier-1 sources eligible, got eligible={eligible}"
    );
    assert_eq!(
        scored, eligible,
        "a clean fixture loses nothing, so every eligible file should be scored"
    );
}
