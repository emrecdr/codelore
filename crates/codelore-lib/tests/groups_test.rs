//! Architectural grouping (`--group-file`) integration tests.
//!
//! Validates the end-to-end flow: parse group file → apply during ingest →
//! analyses see rewritten (grouped) paths.

use codelore_lib::Options;
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
