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
    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .expect("count imports");
    assert_eq!(total, 8, "expected 8 import edges, got {total}");

    // -- All rows have resolved=false / target_path NULL on Day 4 -----
    let unresolved: i64 = db
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
    let rust_abs: i64 = db
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

    let rust_rel: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/main.rs' AND kind = 'relative'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rust_rel, 1, "expected one relative (crate::) Rust import");

    let py_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE src_path = 'src/app.py'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(py_count, 2, "expected 2 Python imports");

    let js_react: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/ui.js' AND target = 'react' AND kind = 'absolute'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(js_react, 1, "expected react import as absolute");

    let js_rel: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/ui.js' AND kind = 'relative'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(js_rel, 1, "expected one relative ./util JS import");

    let java_wild: i64 = db
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

    // At least one row resolved overall — sanity check that the
    // UPDATE path fired.
    let resolved_count: i64 = db
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
    let rust_target: Option<String> = db
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
    let js_target: Option<String> = db
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
    let py_target: Option<String> = db
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
    let total: i64 = db
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
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "non-Tier-1 files must not produce import rows");
}

/// Grouped, `super`, and `#[cfg(test)]` Rust imports under a
/// non-`mod.rs` module layout (`src/foo.rs` owning `src/foo/`).
///
/// Proves three corrections end-to-end: `use crate::{a, b}` fans out to
/// both leaves, a production `use super::sibling` resolves to the sibling
/// module dir, and a `#[cfg(test)]` `use super::decoy` produces no row
/// (and thus no edge) even though `src/foo/decoy.rs` exists — the skip is
/// what stops a false edge.
#[test]
#[allow(clippy::too_many_lines)]
fn ingest_resolves_grouped_and_super_imports_in_non_mod_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src/foo")).unwrap();

    std::fs::write(path.join("src/a.rs"), "pub fn a() {}\n").unwrap();
    std::fs::write(path.join("src/b.rs"), "pub fn b() {}\n").unwrap();
    // Grouped import fans out to both crate-root leaves.
    std::fs::write(
        path.join("src/foo.rs"),
        "use crate::{a, b};\npub fn foo() {}\n",
    )
    .unwrap();
    // Production `super::sibling` → the sibling module. The
    // `#[cfg(test)]` `super::decoy` must not surface at all, even though
    // `src/foo/decoy.rs` exists.
    std::fs::write(
        path.join("src/foo/bar.rs"),
        "use super::sibling;\n#[cfg(test)]\nmod tests {\n    use super::decoy;\n}\n",
    )
    .unwrap();
    std::fs::write(path.join("src/foo/sibling.rs"), "pub fn sibling() {}\n").unwrap();
    std::fs::write(path.join("src/foo/decoy.rs"), "pub fn decoy() {}\n").unwrap();

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

    // Only the three real imports land — the cfg(test) `super::decoy`
    // never becomes a row.
    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .expect("count imports");
    assert_eq!(
        total, 3,
        "expected 3 import rows (decoy skipped), got {total}"
    );

    let decoy_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE target LIKE '%decoy%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(decoy_rows, 0, "cfg(test) import must not produce a row");

    // Grouped `use crate::{a, b}` → two resolved edges.
    let grouped: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/foo.rs' AND resolved = TRUE \
               AND target_path IN ('src/a.rs', 'src/b.rs')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(grouped, 2, "grouped import should resolve to both leaves");

    // Production `super::sibling` → `src/foo/sibling.rs`.
    let super_edge: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/foo/bar.rs' AND resolved = TRUE \
               AND target_path = 'src/foo/sibling.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        super_edge, 1,
        "super::sibling should resolve to the sibling module"
    );

    // No edge lands on the decoy module.
    let decoy_edge: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE target_path = 'src/foo/decoy.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(decoy_edge, 0, "no edge may resolve to the cfg(test) decoy");
}

/// End-to-end coverage of the AST-based JS/TS extraction plus the
/// `NodeNext` `.js`→`.ts` strip-retry.
///
/// Exercises three forms the old string-parse resolver missed: a barrel
/// re-export (`export … from`), a `CommonJS` `require`, and an ESM
/// `import … from "./widget.js"` whose emit-extension specifier must
/// resolve onto the authored `widget.ts`. All three land as resolved
/// edges pointing at in-repo targets.
#[test]
#[allow(clippy::too_many_lines)]
fn ingest_resolves_reexport_require_and_nodenext_specifier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src")).unwrap();

    // Shared TS target for the barrel re-export and the NodeNext import.
    std::fs::write(path.join("src/widget.ts"), "export const thing = 1;\n").unwrap();
    // Barrel re-export: `export { thing } from './widget'` → src/widget.ts.
    std::fs::write(
        path.join("src/barrel.ts"),
        "export { thing } from './widget';\n",
    )
    .unwrap();
    // NodeNext ESM specifier names the `.js` emit; resolves to widget.ts.
    std::fs::write(
        path.join("src/consumer.ts"),
        "import { thing } from './widget.js';\nthing;\n",
    )
    .unwrap();
    // CommonJS require → src/helper.js. `module.exports` is not a call,
    // so helper.js contributes no edge of its own.
    std::fs::write(
        path.join("src/legacy.js"),
        "const dep = require('./helper');\ndep;\n",
    )
    .unwrap();
    std::fs::write(path.join("src/helper.js"), "module.exports = {};\n").unwrap();

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

    // Exactly three edges: the barrel, the NodeNext import, the require.
    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
        .expect("count imports");
    assert_eq!(total, 3, "expected 3 import edges, got {total}");

    // Barrel re-export → src/widget.ts.
    let barrel: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/barrel.ts' AND target = './widget' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        barrel.as_deref(),
        Some("src/widget.ts"),
        "barrel re-export did not resolve to src/widget.ts",
    );

    // NodeNext `.js` specifier strips to the authored `.ts` source.
    let consumer: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/consumer.ts' AND target = './widget.js' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        consumer.as_deref(),
        Some("src/widget.ts"),
        "NodeNext .js specifier did not resolve to src/widget.ts",
    );

    // CommonJS require → src/helper.js.
    let require_edge: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/legacy.js' AND target = './helper' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        require_edge.as_deref(),
        Some("src/helper.js"),
        "require('./helper') did not resolve to src/helper.js",
    );
}

/// End-to-end coverage of the AST-based Python extractor plus the
/// bare-dot relative and absolute first-party resolvers.
///
/// A single-package fixture exercises every Python import shape the old
/// string+relative-only path dropped: `from . import helper` (bare-dot
/// sibling), `from .util import calc` (dotted relative), `from
/// mypkg.core import boot` and `import mypkg.util` (absolute first-party,
/// resolved by dotted-path suffix), and `import os` (stdlib, correctly
/// left unresolved).
#[test]
#[allow(clippy::too_many_lines)]
fn ingest_resolves_python_relative_and_absolute_imports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src/mypkg")).unwrap();

    std::fs::write(path.join("src/mypkg/__init__.py"), "").unwrap();
    std::fs::write(
        path.join("src/mypkg/helper.py"),
        "def helper():\n    pass\n",
    )
    .unwrap();
    std::fs::write(path.join("src/mypkg/util.py"), "def calc():\n    pass\n").unwrap();
    std::fs::write(path.join("src/mypkg/core.py"), "def boot():\n    pass\n").unwrap();
    std::fs::write(
        path.join("src/mypkg/app.py"),
        "from . import helper\n\
         from .util import calc\n\
         from mypkg.core import boot\n\
         import mypkg.util\n\
         import os\n",
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

    // Five edges, all from app.py — one per import statement.
    let app_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE src_path = 'src/mypkg/app.py'",
            [],
            |r| r.get(0),
        )
        .expect("count app rows");
    assert_eq!(app_rows, 5, "expected 5 Python import edges from app.py");

    // `from . import helper` → src/mypkg/helper.py (bare-dot relative).
    let helper: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/mypkg/app.py' AND target = '.helper' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        helper.as_deref(),
        Some("src/mypkg/helper.py"),
        "bare-dot `from . import helper` did not resolve",
    );

    // `from mypkg.core import boot` → src/mypkg/core.py (absolute).
    let core: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/mypkg/app.py' AND target = 'mypkg.core' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        core.as_deref(),
        Some("src/mypkg/core.py"),
        "absolute `from mypkg.core` did not resolve",
    );

    // `import mypkg.util` → src/mypkg/util.py (absolute).
    let util: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/mypkg/app.py' AND target = 'mypkg.util' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        util.as_deref(),
        Some("src/mypkg/util.py"),
        "absolute `import mypkg.util` did not resolve",
    );

    // `import os` stays unresolved — stdlib has no tracked file.
    let os_unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/mypkg/app.py' AND target = 'os' \
               AND resolved = FALSE AND target_path IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count os row");
    assert_eq!(os_unresolved, 1, "stdlib `import os` must stay unresolved");
}

/// End-to-end coverage of the Java FQN resolver plus the ingest allow-list
/// that now feeds `.java` rows to it.
///
/// A conventional `src/main/java` layout exercises every Java import shape:
/// a same-package class (`com.example.Service`), a sub-package class
/// (`com.example.util.Helper`), a static-member import
/// (`static com.example.Service.CONSTANT`, which strips the member and
/// lands on the class file), and a JDK import (`java.util.List`, which has
/// no tracked file and stays unresolved). This also guards the ingest SQL
/// fix — without the `.java` clause in the resolver-pass allow-list, none
/// of these would resolve.
#[test]
#[allow(clippy::too_many_lines)]
fn ingest_resolves_java_imports_to_target_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    std::fs::create_dir_all(path.join("src/main/java/com/example/util")).unwrap();

    std::fs::write(
        path.join("src/main/java/com/example/Service.java"),
        "package com.example;\nclass Service {}\n",
    )
    .unwrap();
    std::fs::write(
        path.join("src/main/java/com/example/util/Helper.java"),
        "package com.example.util;\nclass Helper {}\n",
    )
    .unwrap();
    std::fs::write(
        path.join("src/main/java/com/example/App.java"),
        "package com.example;\n\
         import com.example.Service;\n\
         import com.example.util.Helper;\n\
         import static com.example.Service.CONSTANT;\n\
         import java.util.List;\n\
         class App {}\n",
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

    // Four edges, all from App.java — one per import statement.
    let app_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports WHERE src_path = 'src/main/java/com/example/App.java'",
            [],
            |r| r.get(0),
        )
        .expect("count app rows");
    assert_eq!(app_rows, 4, "expected 4 Java import edges from App.java");

    // `import com.example.Service` → the same-package class file.
    let service: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/main/java/com/example/App.java' \
               AND target = 'com.example.Service' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        service.as_deref(),
        Some("src/main/java/com/example/Service.java"),
        "`import com.example.Service` did not resolve",
    );

    // `import com.example.util.Helper` → the sub-package class file.
    let helper: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/main/java/com/example/App.java' \
               AND target = 'com.example.util.Helper' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        helper.as_deref(),
        Some("src/main/java/com/example/util/Helper.java"),
        "`import com.example.util.Helper` did not resolve",
    );

    // `import static com.example.Service.CONSTANT` strips the member and
    // lands on the enclosing class file.
    let static_member: Option<String> = db
        .query_row(
            "SELECT target_path FROM imports \
             WHERE src_path = 'src/main/java/com/example/App.java' \
               AND target = 'com.example.Service.CONSTANT' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(
        static_member.as_deref(),
        Some("src/main/java/com/example/Service.java"),
        "static-member import did not strip to the class file",
    );

    // `import java.util.List` stays unresolved — the JDK has no tracked file.
    let jdk_unresolved: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/main/java/com/example/App.java' AND target = 'java.util.List' \
               AND resolved = FALSE AND target_path IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count jdk row");
    assert_eq!(
        jdk_unresolved, 1,
        "JDK `import java.util.List` must stay unresolved"
    );

    // Exactly three of the four Java edges resolve to in-repo targets.
    let resolved_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM imports \
             WHERE src_path = 'src/main/java/com/example/App.java' AND resolved = TRUE",
            [],
            |r| r.get(0),
        )
        .expect("count resolved");
    assert_eq!(resolved_count, 3, "expected 3 resolved Java edges");
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git invocation failed");
    assert!(status.success(), "git {args:?} failed");
}
