//! Integration test that proves `FactsDb::ingest` populates the
//! `imports` table from a multi-language fixture.
//!
//! Each language gets one file with two distinctive imports — one
//! absolute, one with a kind quirk (relative / wildcard / `from`-
//! style) — so the test verifies both extraction and classification
//! end-to-end through the rayon → `DuckDB` Appender path.

use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
#[allow(clippy::too_many_lines)] // exhaustive multi-language assertion block
fn ingest_populates_imports_table_from_multilang_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src")).unwrap();

    // Rust: one absolute (std), one crate-relative.
    std::fs::write(
        path.join("src/main.rs"),
        "use std::fs::read_to_string;\nuse crate::lib::foo;\nfn main() {}\n",
    )
    .unwrap();
    // Python: one absolute `import`, one `from`-style.
    std::fs::write(
        path.join("src/app.py"),
        "import os\nfrom collections import deque\n\nprint('ok')\n",
    )
    .unwrap();
    // JavaScript: one absolute (npm), one relative.
    std::fs::write(
        path.join("src/ui.js"),
        "import React from 'react';\nimport util from './util';\n",
    )
    .unwrap();
    // Java: absolute + wildcard.
    std::fs::write(
        path.join("src/A.java"),
        "package com.example;\nimport java.util.List;\nimport java.io.*;\nclass A {}\n",
    )
    .unwrap();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // -- Row count -----------------------------------------------------
    // Rust: 2 · Python: 2 · JS: 2 · Java: 2 = 8 total.
    let conn = db.conn();
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .expect("count imports");
    assert_eq!(total, 8, "expected 8 import edges, got {total}");

    // -- All rows have resolved=false / target_path NULL on Day 4 -----
    let unresolved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE resolved = FALSE AND target_path IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count unresolved");
    assert_eq!(
        unresolved, 8,
        "Day 4 should write resolved=false for every row"
    );

    // -- Per-language correctness checks ------------------------------
    let rust_abs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/main.rs' AND kind = 'absolute' AND target = 'std::fs::read_to_string'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        rust_abs, 1,
        "expected std::fs::read_to_string absolute import"
    );

    let rust_rel: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/main.rs' AND kind = 'relative'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rust_rel, 1, "expected one relative (crate::) Rust import");

    let py_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE src_path = 'src/app.py'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(py_count, 2, "expected 2 Python imports");

    let js_react: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/ui.js' AND target = 'react' AND kind = 'absolute'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(js_react, 1, "expected react import as absolute");

    let js_rel: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/ui.js' AND kind = 'relative'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(js_rel, 1, "expected one relative ./util JS import");

    let java_wild: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/A.java' AND kind = 'wildcard'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(java_wild, 1, "expected one java.io.* wildcard import");
}

#[test]
fn imports_pass_skips_non_tier1_files() {
    // Markdown / text files mustn't surface in the imports table even
    // when they contain `import`-like words.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("docs")).unwrap();
    std::fs::write(
        path.join("docs/README.md"),
        "# import\n\nThis file mentions `import foo;` but is markdown.\n",
    )
    .unwrap();

    run_git(path, &["init", "-b", "main", "--quiet"]);
    run_git(path, &["config", "user.email", "t@e.com"]);
    run_git(path, &["config", "user.name", "T"]);
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "init", "--quiet"]);

    let repo = GixRepo::open(path).expect("gix open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "non-Tier-1 files must not produce import rows");
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git invocation failed");
    assert!(status.success(), "git {args:?} failed");
}
