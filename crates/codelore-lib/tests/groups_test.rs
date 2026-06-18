//! Architectural grouping (`--group-file`) integration tests.
//!
//! Validates the end-to-end flow: parse group file → apply during ingest →
//! analyses see rewritten (grouped) paths.

use codelore_lib::Options;
use codelore_lib::analyses::god_classes::run_god_classes;
use codelore_lib::analyses::hotspots::run_hotspots;
use codelore_lib::analyses::revisions::run_revisions;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

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

fn write(p: std::path::PathBuf, content: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn grouping_rewrites_paths_to_logical_groups() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    // Fixture: 3 files in two logical groups.
    //   src/auth/login.rs  → "Auth"
    //   src/auth/session.rs → "Auth"
    //   src/db/migrate.rs  → "DB"
    // Two commits, each touching one auth file + the db file.
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Group file
    write(
        path.join("groups.txt"),
        "src/auth => Auth\nsrc/db   => DB\n",
    );

    // Commit 1
    write(path.join("src/auth/login.rs"), "v1\n");
    write(path.join("src/db/migrate.rs"), "v1\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c1", "--quiet"]);

    // Commit 2
    write(path.join("src/auth/session.rs"), "v1\n");
    write(path.join("src/db/migrate.rs"), "v2\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c2", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        group_file: Some(path.join("groups.txt")),
        // Non-strict default (CodeLore divergence from code-maat). Unmapped
        // entries — none expected here — would keep their raw path.
        strict_grouping: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let revs = run_revisions(&db, &opts).expect("revisions");
    let entities: Vec<&str> = revs.iter().map(|(e, _)| e.as_str()).collect();

    // After grouping: only "Auth" and "DB" should appear, not the raw paths.
    assert!(
        entities.contains(&"Auth"),
        "Auth group must appear: {entities:?}"
    );
    assert!(
        entities.contains(&"DB"),
        "DB group must appear: {entities:?}"
    );
    assert!(
        !entities
            .iter()
            .any(|e| e.contains("login.rs") || e.contains("session.rs")),
        "raw auth paths must NOT appear after grouping: {entities:?}"
    );

    // Auth saw 2 commits (commit 1 touched login.rs, commit 2 touched
    // session.rs). DB saw 2 commits (touched in both).
    let auth_revs = revs.iter().find(|(e, _)| e == "Auth").unwrap().1;
    let db_revs = revs.iter().find(|(e, _)| e == "DB").unwrap().1;
    assert_eq!(auth_revs, 2);
    assert_eq!(db_revs, 2);
}

#[test]
fn grouping_strict_mode_drops_unmapped_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Group file only covers src/auth — vendor/lib.rs is unmapped.
    write(path.join("groups.txt"), "src/auth => Auth\n");

    write(path.join("src/auth/login.rs"), "v1\n");
    write(path.join("vendor/lib.rs"), "v1\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c1", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        group_file: Some(path.join("groups.txt")),
        strict_grouping: true,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let revs = run_revisions(&db, &opts).expect("revisions");
    let entities: Vec<&str> = revs.iter().map(|(e, _)| e.as_str()).collect();
    assert!(entities.contains(&"Auth"));
    assert!(
        !entities.iter().any(|e| e.contains("vendor")),
        "vendor/lib.rs must be dropped in strict mode: {entities:?}"
    );
}

#[test]
fn grouping_non_strict_mode_keeps_unmapped_paths_raw() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    write(path.join("groups.txt"), "src/auth => Auth\n");

    write(path.join("src/auth/login.rs"), "v1\n");
    write(path.join("vendor/lib.rs"), "v1\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c1", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        group_file: Some(path.join("groups.txt")),
        strict_grouping: false, // CodeLore default
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let revs = run_revisions(&db, &opts).expect("revisions");
    let entities: Vec<&str> = revs.iter().map(|(e, _)| e.as_str()).collect();
    assert!(entities.contains(&"Auth"));
    assert!(
        entities.iter().any(|e| e.contains("vendor/lib.rs")),
        "vendor/lib.rs must be kept (raw path) in non-strict mode: {entities:?}"
    );
}

/// Group-level cognitive aggregation: after `--group-file` collapses
/// `src/core/*.rs` into the `Core` group, hotspots + `god_classes` must
/// report MAX(cognitive) ACROSS all entities of all files in the group,
/// not silently 0. Pre-fix, `complexity_metrics.path` stayed at raw
/// paths while `changes.path` was rewritten to the group name, so the
/// `LEFT JOIN file_complexity fc ON fc.path = fr.path` in hotspots and
/// the `INNER JOIN imports` in `god_classes` never matched — every
/// grouped row reported cognitive = 0 and was silently filtered by
/// the god-classes threshold. `apply_grouping` now also materialises
/// `complexity_metrics_grouped` with per-group MAX cognitive +
/// kind='unit' MI rolled up.
#[test]
fn grouping_rolls_up_cognitive_into_group_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);

    // Group file: collapse src/core/** into "Core".
    write(path.join("groups.txt"), "src/core => Core\n");

    // A Rust file with two functions, the second of which has a
    // deliberately gnarly cognitive complexity (deep nesting + chained
    // conditionals) so `MAX(cognitive)` ends up well above 0 after the
    // HEAD-time `codelore-rca` scan. The cognitive-complexity rule
    // gives ~1 per nesting level + branch, so this trips into double
    // digits.
    let gnarly = "
pub fn trivial() -> u32 { 1 }

pub fn nested(a: bool, b: bool, c: bool, d: bool, e: bool) -> u32 {
    if a {
        if b {
            if c {
                if d {
                    if e { 5 } else { 4 }
                } else if c && b { 3 } else { 2 }
            } else if b || a { 1 } else { 0 }
        } else if a && b && c { 9 } else { 8 }
    } else if a || b || c || d || e { 7 } else { 6 }
}
";
    write(path.join("src/core/nested.rs"), gnarly);
    // A second core file so the grouping really does collapse two paths
    // into one — exercises the rollup, not just a tagging operation.
    write(path.join("src/core/other.rs"), "pub fn x() -> u32 { 2 }\n");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "c1", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 0,
        group_file: Some(path.join("groups.txt")),
        strict_grouping: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let hotspots = run_hotspots(&db, &opts).expect("hotspots");
    let core = hotspots
        .iter()
        .find(|r| r.path == "Core")
        .expect("Core group must appear in hotspots output");
    assert!(
        core.cognitive > 0.0,
        "grouped Core row must report non-zero cognitive after rollup; got {core:?}"
    );

    // Cross-check god-classes: with a low `min_total_fan` threshold
    // satisfied (group surfaces both files' imports, fan-in/fan-out
    // collapsed), god-classes wouldn't fire on the trivial single-commit
    // fixture (no imports edges), so we instead assert the analysis
    // RUNS without panic against grouped paths — the pre-fix regression
    // wasn't about `god_classes`' ranking but the SQL bind on
    // `complexity_metrics`, which now resolves through the grouped
    // source.
    let _gods = run_god_classes(&db, &opts).expect("god-classes must run against grouped paths");
}
