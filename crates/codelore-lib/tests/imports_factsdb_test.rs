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

/// End-to-end coverage of `resolve_imports_at_head`'s UPDATE path.
///
/// The first test in this file only proves we *write* imports rows;
/// every row stays `resolved=false` because the fixture has no
/// targets that the per-language resolver can map to a tracked path.
/// That hid the entire UPDATE path from CI: a regression in
/// `resolve_imports_at_head` that left every import unresolved would
/// pass the existing test (which asserts unresolved-everything is the
/// only state). Architecturally that also broke `arch_violations`
/// silently — the rule engine returns empty for unresolved edges.
///
/// This fixture adds the file each import needs to land on (`src/lib.rs`
/// for the Rust `crate::lib::foo` case, `src/util.js` for the JS
/// `./util` case, and a Python package layout for the relative import)
/// and asserts the resolver flips them to `resolved=true` with the
/// correct `target_path`.
#[test]
#[allow(clippy::too_many_lines)]
fn ingest_resolves_imports_to_target_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::create_dir_all(path.join("src/pkg")).unwrap();

    // Rust resolvable: `crate::lib::foo` from src/main.rs → src/lib.rs.
    // (The terminal `foo` is an item INSIDE the lib module; the
    // resolver pops it and matches src/lib.rs.)
    std::fs::write(
        path.join("src/main.rs"),
        "use crate::lib::foo;\nfn main() {}\n",
    )
    .unwrap();
    std::fs::write(path.join("src/lib.rs"), "pub fn foo() {}\n").unwrap();

    // JS resolvable: `import x from './util'` from src/app.js → src/util.js.
    std::fs::write(
        path.join("src/app.js"),
        "import util from './util';\nutil();\n",
    )
    .unwrap();
    std::fs::write(
        path.join("src/util.js"),
        "export default function util() {}\n",
    )
    .unwrap();

    // Python resolvable: `from .core import run` from src/pkg/main.py
    // → src/pkg/core.py (relative import within the same package).
    std::fs::write(path.join("src/pkg/__init__.py"), "").unwrap();
    std::fs::write(
        path.join("src/pkg/main.py"),
        "from .core import run\nrun()\n",
    )
    .unwrap();
    std::fs::write(path.join("src/pkg/core.py"), "def run():\n    pass\n").unwrap();

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

    let conn = db.conn();

    // At least one row resolved overall — sanity check that the
    // UPDATE path fired.
    let resolved_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .expect("count resolved");
    assert!(
        resolved_count >= 3,
        "expected at least 3 resolved rows (Rust + JS + Python), got {resolved_count}",
    );

    // Rust: crate::lib::foo from src/main.rs → src/lib.rs.
    let rust_target: Option<String> = conn
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/main.rs' AND target LIKE '%lib::foo%' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        rust_target.as_deref(),
        Some("src/lib.rs"),
        "Rust import did not resolve to src/lib.rs",
    );

    // JS: ./util from src/app.js → src/util.js.
    let js_target: Option<String> = conn
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/app.js' AND target = './util' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        js_target.as_deref(),
        Some("src/util.js"),
        "JS import did not resolve to src/util.js",
    );

    // Python: .core from src/pkg/main.py → src/pkg/core.py.
    let py_target: Option<String> = conn
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/pkg/main.py' AND target LIKE '.core%' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        py_target.as_deref(),
        Some("src/pkg/core.py"),
        "Python import did not resolve to src/pkg/core.py",
    );

    // Sanity: total row count matches what the fixture produced — 3
    // imports (1 Rust, 1 JS, 1 Python), all resolvable to in-repo
    // targets. The `imports_pass_skips_non_tier1_files` neighbor test
    // covers the unresolved-external-imports inverse case, so this
    // test is allowed to fully resolve.
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .expect("count total");
    assert_eq!(
        total, 3,
        "expected exactly 3 imports in the fixture; got {total}",
    );
    assert_eq!(
        resolved_count, total,
        "every import in this fixture is resolvable to an in-repo target",
    );
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
