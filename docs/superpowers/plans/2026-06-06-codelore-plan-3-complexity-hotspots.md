# CodeLore Plan 3: Complexity Integration + Hotspots + Code Health

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Plan 3 of 6 for v1 Spine release.** Builds on Plan 1 (`docs/superpowers/plans/2026-06-06-codelore-phase-0-walking-skeleton.md`) and Plan 2 (`docs/superpowers/plans/2026-06-06-codelore-plan-2-rca-vendor.md`).

**Goal:** Wire `codelore-rca`'s complexity metrics into `codelore-lib`, populate the `entities` and `complexity_metrics` tables during ingest at HEAD, ship two new analyses (`hotspots` with the spec §1.1 published formula, and `code-health` composite per spec §4.6), and surface them through the CLI.

**Architecture:** Plan 3 adds a `complexity` module to `codelore-lib` that wraps `codelore-rca`. Complexity is computed **only at HEAD** in Plan 3 (per spec §4.4 `--complexity-sample head` default); Plan 4 adds adaptive sampling. Entity granularity is **function-level** (spec §1.1: "Function-level entity baseline via tree-sitter; file-level rollups derive trivially"). Each function in a Tier-1 file becomes a row in `entities`; metrics live in `complexity_metrics`. `hotspots` joins `changes` × `entities` × `complexity_metrics`. `code-health` uses the §4.6 formula with weights configurable in `Options` (Plan 4 ingredients — fragmentation, coupling — set to weight zero in Plan 3 until those analyses exist).

**Tech Stack:**
- All Plan 1+2 stack
- Internal: `codelore_rca::{RustParser, PythonParser, TypescriptParser, JavascriptParser, JavaParser, metrics, FuncSpace}` (public API discovered in Plan 2 Task 6)
- Per-language dispatch via path extension matching

**Out of scope for this plan (deferred to Plan 4):**
- Adaptive complexity sampling (Plan 1's `--complexity-sample adaptive` flag)
- Per-revision complexity tracking (history of metrics, not just HEAD)
- Function-level coupling / X-Ray (Plan 4)
- Fisher exact significance on coupling (Plan 4 — coupling analysis itself)
- Identity resolution / `.mailmap` (Plan 4)
- 9 other code-maat analyses (Plan 4: refactoring-main-dev, entity-effort, soc, messages, etc.)
- SARIF output (Plan 5)

**Definition of Done for Plan 3:**
- `codelore-lib::complexity` module exists, wraps `codelore-rca` for the 5 Tier-1 languages
- `FactsDb::ingest()` populates `entities` and `complexity_metrics` for HEAD files
- `codelore_lib::analyses::hotspots::run_hotspots(&db, &opts) -> Result<Vec<HotspotRow>>` computes per-entity hotspot score per the §1.1 formula
- `codelore_lib::analyses::code_health::run_code_health(&db, &opts) -> Result<Vec<CodeHealthRow>>` computes the composite per §4.6 (with v1.5 inputs zeroed)
- `codelore analyze --analysis hotspots --format csv` and `codelore analyze --analysis code-health --format csv` work end-to-end
- New integration tests verify hotspot ranking and Code Health composite on the `tiny_repo` fixture
- All previous tests still pass; new tests pass; clippy/fmt/deny clean

---

## §1 — Plan 2 carry-over (Phase 3.A)

The Plan 2 final reviewer flagged nothing blocking. One small follow-up worth doing first:

### Task 1: Workspace `cargo clean` audit + rename `tier1_languages_smoke` test count message

**Files:**
- None modified — this is an environment-hygiene task

- [ ] **Step 1: Document current disk state**

```bash
df -h /Users/emrec/Projects/playground/codescene/ | tail -1
du -sh /Users/emrec/Projects/playground/codescene/target/
```

Record both numbers. If `target/` is over 10 GiB AND disk free < 5 GiB, run `cargo clean -p codelore-lib -p codelore-cli` (NOT `codelore-rca` — its incremental cache is small and the C++ DuckDB is what's bloating us, which is in codelore-lib's build deps).

If disk is fine, skip the cargo clean — no point burning 5-10 min on a recompile we don't need.

- [ ] **Step 2: Note this is an audit-only step**

Nothing to commit unless `cargo clean` ran and made a difference. Move to Task 2.

---

## §2 — Wire `codelore-rca` into `codelore-lib` (Phase 3.B)

### Task 2: Add `codelore-rca` dependency + language detection

**Files:**
- Modify: `crates/codelore-lib/Cargo.toml`
- Create: `crates/codelore-lib/src/complexity/mod.rs`
- Create: `crates/codelore-lib/src/complexity/language.rs`
- Modify: `crates/codelore-lib/src/lib.rs`

- [ ] **Step 1: Add codelore-rca dep to codelore-lib**

In `crates/codelore-lib/Cargo.toml`, append to `[dependencies]`:

```toml
codelore-rca = { path = "../codelore-rca", version = "0.1.0-alpha.1" }
```

- [ ] **Step 2: Create `crates/codelore-lib/src/complexity/language.rs`**

```rust
//! Path → codelore-rca Language dispatch for Tier-1 languages.
//!
//! Returns None for unsupported file types. Plan 3 only handles
//! files with the listed extensions; Plan 4 may add custom mapping
//! via TOML config.

/// Tier-1 language identifier wrapping codelore-rca's per-language parser types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier1Language {
    Rust,
    Python,
    Java,
    JavaScript,
    TypeScript,
}

impl Tier1Language {
    /// Returns the language for a path, if it's a Tier-1 file extension.
    pub fn from_path(path: &str) -> Option<Self> {
        let ext = path.rsplit('.').next()?;
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "tsx" => Some(Self::TypeScript),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Java => "java",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
        }
    }
}
```

- [ ] **Step 3: Create `crates/codelore-lib/src/complexity/mod.rs`**

Plan 3 ships a minimal skeleton; the real `compute_for_file` lands in Task 3.

```rust
//! Per-language complexity metric computation via vendored codelore-rca.
//!
//! Plan 3 scope: HEAD-only file-level + function-level entity extraction
//! for Tier-1 languages (Rust, TS/JS, Python, Java).
//! See spec §4 and Plan 3 §3.

pub mod language;

pub use language::Tier1Language;

use crate::Result;
use std::path::Path;

/// One function (or class, or file-level unit) with its complexity metrics.
#[derive(Debug, Clone)]
pub struct ComplexityEntity {
    pub path: String,
    pub name: String,
    pub kind: String,  // "function", "method", "class", "file"
    pub start_line: u32,
    pub end_line: u32,
    pub cyclomatic: f64,
    pub cognitive: f64,
    pub halstead_volume: Option<f64>,
    pub halstead_difficulty: Option<f64>,
    pub halstead_effort: Option<f64>,
    pub mi: Option<f64>,
    pub nom: u32,
    pub nexits: u32,
    pub loc: u32,
    pub sloc: u32,
    pub max_nesting: u32,
    pub mean_nesting: f64,
    pub sd_nesting: f64,
    pub total_nesting: u32,
}

/// Compute complexity entities for a Tier-1 source file.
/// Plan 3 stub; real impl in Task 3.
pub fn compute_for_file(
    _path: &Path,
    _source: &[u8],
    _lang: Tier1Language,
) -> Result<Vec<ComplexityEntity>> {
    Ok(vec![])
}
```

- [ ] **Step 4: Add module to `lib.rs`**

In `crates/codelore-lib/src/lib.rs`, add `pub mod complexity;` in alphabetical position (between `pub mod arrow_facade;` and `pub mod error;`). No re-export at crate root.

- [ ] **Step 5: Verify build**

```bash
cargo build -p codelore-lib
cargo test -p codelore-lib --all-features
```

Expected: clean build + 18 tests pass (no new tests yet).

- [ ] **Step 6: Run clippy + fmt**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 7: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): add complexity module skeleton + Tier-1 language dispatch"
```

---

### Task 3: Implement `compute_for_file` via codelore-rca

**Files:**
- Modify: `crates/codelore-lib/src/complexity/mod.rs`
- Create: `crates/codelore-lib/tests/complexity_test.rs`

- [ ] **Step 1: Write failing test**

Create `crates/codelore-lib/tests/complexity_test.rs`:

```rust
use codelore_lib::complexity::{compute_for_file, Tier1Language};
use std::path::Path;

#[test]
fn complexity_for_rust_function() {
    let src = b"fn complex(x: i32) -> i32 {
    if x > 0 {
        for i in 0..x { println!(\"{i}\"); }
    } else if x < 0 {
        match x { -1 => return -1, _ => return -2 }
    }
    0
}
";
    let entities = compute_for_file(
        Path::new("src/test.rs"),
        src,
        Tier1Language::Rust,
    ).expect("compute");

    assert!(!entities.is_empty(), "should extract at least the file unit");

    // At least one entity should have meaningful complexity
    let max_cyclomatic = entities.iter().map(|e| e.cyclomatic as i64).max().unwrap();
    assert!(max_cyclomatic > 1, "branching code should produce cyclomatic > 1");
}

#[test]
fn complexity_for_python_function() {
    let src = b"def complex(x):
    if x > 0:
        for i in range(x):
            print(i)
    elif x < 0:
        return -1
    return 0
";
    let entities = compute_for_file(
        Path::new("test.py"),
        src,
        Tier1Language::Python,
    ).expect("compute");
    assert!(!entities.is_empty());
}
```

Add `[[test]] name = "complexity_test"` stanza to `codelore-lib/Cargo.toml` (no required-features needed).

- [ ] **Step 2: Run and confirm fail**

```bash
cargo test -p codelore-lib --test complexity_test
```

Expected: tests fail because `compute_for_file` returns empty Vec.

- [ ] **Step 3: Implement `compute_for_file`**

Replace the stub in `crates/codelore-lib/src/complexity/mod.rs` with:

```rust
pub fn compute_for_file(
    path: &Path,
    source: &[u8],
    lang: Tier1Language,
) -> Result<Vec<ComplexityEntity>> {
    use codelore_rca::{
        JavaParser, JavascriptParser, PythonParser, RustParser, TypescriptParser,
        metrics,
    };

    // codelore-rca's parser types are concrete; dispatch on language.
    let path_buf = path.to_path_buf();
    let source_vec = source.to_vec();
    let func_space = match lang {
        Tier1Language::Rust => {
            let parser = RustParser::new(source_vec, &path_buf, None);
            metrics(&parser, &path_buf)
        }
        Tier1Language::Python => {
            let parser = PythonParser::new(source_vec, &path_buf, None);
            metrics(&parser, &path_buf)
        }
        Tier1Language::Java => {
            let parser = JavaParser::new(source_vec, &path_buf, None);
            metrics(&parser, &path_buf)
        }
        Tier1Language::JavaScript => {
            let parser = JavascriptParser::new(source_vec, &path_buf, None);
            metrics(&parser, &path_buf)
        }
        Tier1Language::TypeScript => {
            let parser = TypescriptParser::new(source_vec, &path_buf, None);
            metrics(&parser, &path_buf)
        }
    };

    let Some(root) = func_space else {
        return Ok(vec![]); // parse failed or no metrics
    };

    let mut entities = Vec::new();
    flatten_func_space(&root, path.to_string_lossy().as_ref(), &mut entities);
    Ok(entities)
}

fn flatten_func_space(
    space: &codelore_rca::FuncSpace,
    path: &str,
    out: &mut Vec<ComplexityEntity>,
) {
    let kind = match space.kind {
        codelore_rca::SpaceKind::Unit => "file",
        codelore_rca::SpaceKind::Function => "function",
        codelore_rca::SpaceKind::Class => "class",
        codelore_rca::SpaceKind::Struct => "struct",
        codelore_rca::SpaceKind::Trait => "trait",
        codelore_rca::SpaceKind::Impl => "impl",
        codelore_rca::SpaceKind::Namespace => "namespace",
        codelore_rca::SpaceKind::Interface => "interface",
        codelore_rca::SpaceKind::Unknown => "unknown",
    };

    out.push(ComplexityEntity {
        path: path.to_string(),
        name: space.name.clone().unwrap_or_else(|| "<unit>".to_string()),
        kind: kind.to_string(),
        start_line: space.start_line as u32,
        end_line: space.end_line as u32,
        cyclomatic: space.metrics.cyclomatic.cyclomatic_sum(),
        cognitive: space.metrics.cognitive.cognitive_sum(),
        // Halstead may not be available for all languages; wrap in Option
        halstead_volume: Some(space.metrics.halstead.volume()),
        halstead_difficulty: Some(space.metrics.halstead.difficulty()),
        halstead_effort: Some(space.metrics.halstead.effort()),
        mi: Some(space.metrics.mi.mi_sei()),
        nom: space.metrics.nom.functions() as u32,
        nexits: space.metrics.nexits.exit_sum() as u32,
        loc: space.metrics.loc.ploc() as u32,
        sloc: space.metrics.loc.sloc() as u32,
        max_nesting: space.metrics.nesting.max() as u32,
        mean_nesting: space.metrics.nesting.average(),
        sd_nesting: 0.0,   // codelore-rca doesn't expose SD; Plan 4 can compute
        total_nesting: space.metrics.nesting.total() as u32,
    });

    for child in &space.spaces {
        flatten_func_space(child, path, out);
    }
}
```

**Adapt as needed**: the exact field/method names on `codelore_rca::FuncSpace.metrics.*` may differ (e.g. `.cyclomatic_sum()` vs `.value()`). Read `codelore-rca`'s source if any field is wrong. Plan 2 Task 6's implementer confirmed:
- `func_space.metrics.cyclomatic.cyclomatic_sum()` exists
- `func_space.metrics.cognitive.cognitive_sum()` exists

For Halstead/MI/NOM/NEXITS/LOC/Nesting, check the `codelore-rca` source or upstream RCA docs. If a metric isn't available on your codelore-rca version, set the field to `None` or `0` and document.

- [ ] **Step 4: Run tests, confirm pass**

```bash
cargo test -p codelore-lib --test complexity_test --all-features
```

Expected: 2 tests pass.

```bash
cargo test --workspace --all-features
```

Expected: 22 lib + 4 cli + 199 rca + 6 rca-integration + 2 new complexity = 233.

- [ ] **Step 5: Clippy + fmt**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): complexity::compute_for_file via codelore-rca for Tier-1 languages"
```

---

## §3 — Populate the schema (Phase 3.C)

### Task 4: Extend `ingest` to populate `entities` and `complexity_metrics` at HEAD

**Files:**
- Modify: `crates/codelore-lib/src/facts/ingest.rs`
- Modify: `crates/codelore-lib/src/facts/mod.rs` (re-export new types if needed)
- Modify: `crates/codelore-lib/tests/ingest_test.rs`

- [ ] **Step 1: Add HEAD-file complexity walk to `ingest()`**

After the existing commit/changes ingest loop in `crates/codelore-lib/src/facts/ingest.rs`, add a post-pass that:

1. Walks the gix HEAD tree (`repo.walk_tree_at_head()`)
2. For each Tier-1 file, reads the blob via `gix::Repository::find_blob(oid)`
3. Calls `compute_for_file(path, blob_data, lang)` from the complexity module
4. Appends entities to the `entities` table and metrics to `complexity_metrics`

```rust
// In FactsDb::ingest, after the channel-based commit/changes ingest:
//
// Plan 3 addition: walk HEAD, compute complexity for Tier-1 files,
// populate `entities` and `complexity_metrics` tables.
self.ingest_complexity_at_head(repo, opts)?;
```

Then implement `ingest_complexity_at_head`:

```rust
impl FactsDb {
    fn ingest_complexity_at_head<R: Repo>(
        &self,
        repo: &R,
        _opts: &Options,
    ) -> Result<()> {
        // The Repo trait doesn't currently expose a "walk HEAD tree" method.
        // For Plan 3, take a HEAD shortcut: query the changes table for the
        // most-recent commit of each path, fetch the blob for that (path, rev),
        // compute complexity.
        //
        // This is correct for HEAD-only Plan 3 scope. Plan 4 will add per-revision
        // complexity tracking and a richer Repo::list_head_files() method.

        let rev_lookup_sql = "
            SELECT changes.path, MAX(changes.rev) AS head_rev
            FROM changes
            INNER JOIN commits ON changes.rev = commits.rev
            WHERE changes.change_type != 'deleted'
            GROUP BY changes.path
            ORDER BY commits.date DESC
        ";
        // ... iterate and compute complexity
        Ok(())
    }
}
```

**Note**: the actual implementation needs careful blob-reading via gix. The simplest path is to extend the `Repo` trait with a `read_blob_at_rev(rev: &str, path: &str) -> Result<Vec<u8>>` method, implement it in `GixRepo`, and use it here.

Given this is a substantial addition, BREAK Task 4 into two sub-tasks if needed: 4a (extend Repo trait + GixRepo impl), 4b (the ingest pass).

For Plan 3 walking skeleton, an acceptable simplification: read files from the working tree (assume HEAD checked out), via `std::fs::read(repo_path.join(&rel_path))`. This requires:
- The repo to be a working copy (not bare)
- HEAD to be checked out
- Files exist on disk

Document the simplification in the code. Plan 4 will fix it.

- [ ] **Step 2: Update `ingest_test.rs` to verify complexity rows**

Append to `crates/codelore-lib/tests/ingest_test.rs`:

```rust
#[test]
fn ingest_populates_complexity_for_tier1_files() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options { min_revs: 1, ..Options::default() };
    db.ingest(&repo, &opts).expect("ingest");

    // tiny_repo has src/main.rs (Rust) — should produce at least 1 entity
    let count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM entities WHERE path = 'src/main.rs'"
        )
        .expect("query");
    let n: u32 = count.parse().unwrap();
    assert!(n >= 1, "expected at least 1 entity for src/main.rs, got {n}");

    let metric_count: String = db
        .query_one_value(
            "SELECT CAST(COUNT(*) AS TEXT) FROM complexity_metrics WHERE path = 'src/main.rs'"
        )
        .expect("query");
    let m: u32 = metric_count.parse().unwrap();
    assert!(m >= 1, "expected at least 1 complexity row for src/main.rs, got {m}");
}
```

- [ ] **Step 3: Run + iterate**

```bash
cargo test -p codelore-lib --test ingest_test --all-features
```

If it fails, iterate. The blob-reading path is the most likely failure point.

- [ ] **Step 4: Workspace check**

```bash
cargo test --workspace --all-features 2>&1 | tail -3
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): populate entities + complexity_metrics at HEAD during ingest"
```

---

## §4 — New analyses (Phase 3.D)

### Task 5: Hotspots analysis

**Files:**
- Create: `crates/codelore-lib/src/analyses/hotspots.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs`
- Create: `crates/codelore-lib/tests/hotspots_test.rs`

- [ ] **Step 1: Write failing test**

Create `crates/codelore-lib/tests/hotspots_test.rs`:

```rust
use codelore_lib::analyses::hotspots::{run_hotspots, HotspotRow};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::Options;

#[test]
fn hotspots_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options { min_revs: 1, ..Options::default() };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_hotspots(&db, &opts).expect("run");
    // src/main.rs changed 4 times; src/lib.rs changed 1 time. Both Rust.
    // Highest revs = main.rs should rank first (assuming similar complexity).
    assert!(!rows.is_empty(), "should produce at least 1 row");
    let top = &rows[0];
    assert_eq!(top.path, "src/main.rs", "main.rs should be top hotspot");
}
```

Add `[[test]] name = "hotspots_test" required-features = ["test-support"]` to `codelore-lib/Cargo.toml`.

- [ ] **Step 2: Create the analysis with the spec §1.1 formula**

Create `crates/codelore-lib/src/analyses/hotspots.rs`:

```rust
//! Hotspot ranking analysis per spec §1.1 published formula:
//!
//!   hotspot_score(entity) = percentile_rank(revisions)
//!                         × percentile_rank(cognitive_complexity)
//!                         × (10 − code_health) / 10
//!
//! In Plan 3, `code_health` is computed inline from cognitive complexity only
//! (other inputs land in Plan 4). The formula simplifies but the shape is preserved.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone)]
pub struct HotspotRow {
    pub path: String,
    pub name: String,
    pub revisions: u32,
    pub cognitive: f64,
    pub code_health: f64,
    pub hotspot_score: f64,
}

pub fn run_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>> {
    let limit = opts.rows_limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();

    // Aggregate revisions per path, join with complexity at the file level,
    // compute percentile ranks via window functions, then the formula.
    let sql = format!(
        "WITH file_revs AS (
             SELECT path, COUNT(DISTINCT rev) AS revs
             FROM changes
             GROUP BY path
             HAVING revs >= {min}
         ),
         file_complexity AS (
             SELECT path, name, MAX(cognitive) AS cognitive
             FROM complexity_metrics
             GROUP BY path, name
         ),
         joined AS (
             SELECT
                 fc.path, fc.name, fr.revs, fc.cognitive,
                 PERCENT_RANK() OVER (ORDER BY fr.revs) AS pr_rev,
                 PERCENT_RANK() OVER (ORDER BY fc.cognitive) AS pr_cx
             FROM file_complexity fc
             INNER JOIN file_revs fr ON fc.path = fr.path
         )
         SELECT
             path, name, revs, cognitive,
             100.0 * (1.0 - 0.40 * (cognitive / NULLIF(MAX(cognitive) OVER (), 0))) AS code_health,
             pr_rev * pr_cx * (10.0 - 100.0 * (1.0 - 0.40 * (cognitive / NULLIF(MAX(cognitive) OVER (), 0)))) / 10.0 AS score
         FROM joined
         ORDER BY score DESC, path ASC{limit}",
        min = opts.min_revs,
        limit = limit,
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare hotspots: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HotspotRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                revisions: u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX),
                cognitive: r.get::<_, f64>(3)?,
                code_health: r.get::<_, f64>(4)?,
                hotspot_score: r.get::<_, f64>(5)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query hotspots: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect hotspots: {e}")))
}
```

- [ ] **Step 3: Add to `analyses/mod.rs`**

```rust
pub mod hotspots;
pub mod revisions;
```

- [ ] **Step 4: Run test, iterate**

```bash
cargo test -p codelore-lib --test hotspots_test --all-features
```

Expected: 1 test passes.

- [ ] **Step 5: Workspace check**

```bash
cargo test --workspace --all-features 2>&1 | tail -3
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): hotspots analysis per spec §1.1 published formula"
```

---

### Task 6: Code Health composite analysis

**Files:**
- Create: `crates/codelore-lib/src/analyses/code_health.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs`
- Modify: `crates/codelore-lib/src/options.rs` (add Code Health weight overrides — though optional for Plan 3)
- Create: `crates/codelore-lib/tests/code_health_test.rs`

- [ ] **Step 1: Write failing test**

Create `crates/codelore-lib/tests/code_health_test.rs`:

```rust
use codelore_lib::analyses::code_health::{run_code_health, CodeHealthRow};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
use codelore_lib::Options;

#[test]
fn code_health_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options { min_revs: 1, ..Options::default() };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(row.score >= 0.0 && row.score <= 100.0,
            "score should be in [0, 100], got {} for {}",
            row.score, row.path);
    }
}
```

Add `[[test]] name = "code_health_test" required-features = ["test-support"]` to Cargo.toml.

- [ ] **Step 2: Create the analysis per spec §4.6**

Create `crates/codelore-lib/src/analyses/code_health.rs`:

```rust
//! Code Health composite analysis per spec §4.6.
//!
//! Formula:
//!   codehealth(entity) = 100 × (1
//!     - w_cx · normalize(cognitive_complexity)
//!     - w_cn · normalize(churn_rate)
//!     - w_au · normalize(author_fragmentation_FV)
//!     - w_cp · normalize(coupling_centrality_SoC)
//!   )
//!
//! Default weights (spec §4.6):
//!   w_cx = 0.40, w_cn = 0.25, w_au = 0.15, w_cp = 0.20
//!
//! Plan 3 ships with churn / fragmentation / coupling effectively zero
//! (their analyses don't exist yet); the formula reduces to:
//!   codehealth(entity) ≈ 100 × (1 - 0.40 · normalize(cognitive))
//!
//! Plan 4 will populate the missing inputs and the full formula will activate.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone)]
pub struct CodeHealthRow {
    pub path: String,
    pub name: String,
    pub cognitive: f64,
    pub score: f64,  // 0..=100; higher = healthier
}

pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    let limit = opts.rows_limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();

    // For Plan 3: only the cognitive input is active. normalize() is
    // value / repo-95th-percentile per spec §4.6; we approximate using max.
    let sql = format!(
        "WITH file_complexity AS (
             SELECT path, name, MAX(cognitive) AS cognitive
             FROM complexity_metrics
             GROUP BY path, name
         ),
         normalized AS (
             SELECT path, name, cognitive,
                 CASE WHEN MAX(cognitive) OVER () > 0
                      THEN cognitive / MAX(cognitive) OVER ()
                      ELSE 0
                 END AS norm_cx
             FROM file_complexity
         )
         SELECT path, name, cognitive,
                GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * norm_cx))) AS score
         FROM normalized
         ORDER BY score ASC, path ASC{limit}",
        limit = limit,
    );

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-health: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CodeHealthRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                cognitive: r.get::<_, f64>(2)?,
                score: r.get::<_, f64>(3)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query code-health: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-health: {e}")))
}
```

- [ ] **Step 3: Add to `analyses/mod.rs`**

```rust
pub mod code_health;
pub mod hotspots;
pub mod revisions;
```

- [ ] **Step 4: Run, verify pass**

```bash
cargo test -p codelore-lib --test code_health_test --all-features
```

Expected: 1 test passes.

- [ ] **Step 5: Workspace check, clippy, fmt**

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): code-health composite analysis per spec §4.6"
```

---

## §5 — CLI integration (Phase 3.E)

### Task 7: Wire `hotspots` and `code-health` into the CLI

**Files:**
- Modify: `crates/codelore-cli/src/main.rs`
- Create: `crates/codelore-lib/src/output/csv.rs` — add `write_hotspots_csv` and `write_code_health_csv` helpers
- Modify: `crates/codelore-cli/tests/cli_test.rs`

- [ ] **Step 1: Add new CSV emitters to codelore-lib output module**

In `crates/codelore-lib/src/output/csv.rs`, append:

```rust
use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::hotspots::HotspotRow;

pub fn write_hotspots_csv<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,name,revisions,cognitive,code-health,hotspot-score")
        .map_err(CodeLoreError::Io)?;
    for row in rows {
        let escaped_path = quote_if_needed(&row.path);
        let escaped_name = quote_if_needed(&row.name);
        writeln!(
            w, "{},{},{},{:.2},{:.2},{:.4}",
            escaped_path, escaped_name,
            row.revisions, row.cognitive, row.code_health, row.hotspot_score
        ).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_health_csv<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,name,cognitive,score").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w, "{},{},{:.2},{:.2}",
            quote_if_needed(&row.path),
            quote_if_needed(&row.name),
            row.cognitive,
            row.score
        ).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

fn quote_if_needed(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}
```

(Refactor the existing `write_revisions_csv` to use the same `quote_if_needed` helper.)

- [ ] **Step 2: Wire hotspots + code-health into main.rs**

In `crates/codelore-cli/src/main.rs`, update the `analyze()` function. Currently it bails for any analysis other than `Revisions`. Update to dispatch:

```rust
fn analyze(args: AnalyzeArgs) -> Result<()> {
    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("parsing --analysis {:?}", args.analysis))?;
    if args.format != "csv" {
        anyhow::bail!(
            "Plan 3 walking skeleton only supports --format csv. \
             JSON, SARIF, Markdown, Parquet, SQLite land in Plan 5."
        );
    }

    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: args.min_revs,
        rows_limit: args.rows,
        ..Options::default()
    };

    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db = FactsDb::new_in_memory().context("open fact store")?;
    db.ingest(&repo, &opts).context("ingest commits")?;

    let mut out: Box<dyn Write> = match args.output {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };

    match analysis {
        AnalysisName::Revisions => {
            let rows = run_revisions(&db, &opts)?;
            codelore_lib::output::csv::write_revisions_csv(&rows, &mut out)?;
        }
        AnalysisName::Hotspots => {
            let rows = codelore_lib::analyses::hotspots::run_hotspots(&db, &opts)?;
            codelore_lib::output::csv::write_hotspots_csv(&rows, &mut out)?;
        }
        AnalysisName::CodeHealth => {
            let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts)?;
            codelore_lib::output::csv::write_code_health_csv(&rows, &mut out)?;
        }
        _ => anyhow::bail!(
            "Plan 3 supports --analysis revisions | hotspots | code-health. \
             Other analyses land in Plan 4."
        ),
    }
    Ok(())
}
```

- [ ] **Step 3: Add CLI tests for hotspots and code-health**

Append to `crates/codelore-cli/tests/cli_test.rs`:

```rust
#[test]
fn analyze_hotspots_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze", "--analysis", "hotspots",
            "--repo", tiny.dir.path().to_str().unwrap(),
            "--format", "csv", "--min-revs", "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,name,revisions"));
}

#[test]
fn analyze_code_health_emits_csv() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze", "--analysis", "code-health",
            "--repo", tiny.dir.path().to_str().unwrap(),
            "--format", "csv", "--min-revs", "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,name,cognitive,score"));
}
```

- [ ] **Step 4: Verify**

```bash
cargo test -p codelore-cli --all-features
cargo test --workspace --all-features 2>&1 | tail -3
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 5: Smoke test**

```bash
cargo build --release -p codelore-cli
./target/release/codelore analyze --analysis hotspots --repo . --rows 5 --min-revs 1
./target/release/codelore analyze --analysis code-health --repo . --rows 5 --min-revs 1
```

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat(cli): expose hotspots and code-health analyses via CLI"
```

---

## §6 — Docs + Plan 3 Done (Phase 3.F)

### Task 8: Update CHANGELOG and README

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `README.md`

- [ ] **Step 1: Insert Plan 3 section in CHANGELOG**

Above the existing Plan 2 / Plan 1 sections:

```markdown
## [Unreleased]

### Added (Plan 3: Complexity Integration + Hotspots + Code Health)
- `codelore-lib::complexity` module wraps `codelore-rca` for Tier-1 languages (Rust, TS/JS, Python, Java)
- Path-based language dispatch (`Tier1Language::from_path`)
- Function-level entity extraction via `codelore-rca::FuncSpace` traversal
- `FactsDb::ingest()` now populates `entities` and `complexity_metrics` at HEAD
- `hotspots` analysis per spec §1.1 published formula: `percentile_rank(revisions) × percentile_rank(cognitive) × (10 − code_health) / 10`
- `code-health` composite analysis per spec §4.6 (Plan 3 wires cognitive input only; churn/fragmentation/coupling land in Plan 4)
- CLI: `codelore analyze --analysis hotspots | code-health --format csv`

### Added (Plan 2: RCA Vendor)
...
```

- [ ] **Step 2: Update README "What works today"**

Replace/extend bullets in the "What works today" section to include:

```markdown
- `codelore analyze --analysis hotspots --format csv` — file-level hotspot ranking with the published formula
- `codelore analyze --analysis code-health --format csv` — Code Health composite (cognitive-only in Plan 3; full formula in Plan 4)
- Function-level entity extraction at HEAD for Tier-1 languages
```

Update the Roadmap line for Plan 3 to ✅ status.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md README.md
git commit -m "docs: CHANGELOG + README for Plan 3 complexity integration + new analyses"
```

---

## Plan 3 Definition of Done

- [ ] `codelore-lib::complexity::compute_for_file` works for all 5 Tier-1 languages
- [ ] `FactsDb::ingest` populates `entities` and `complexity_metrics` at HEAD
- [ ] `run_hotspots` and `run_code_health` analyses exist and pass tests
- [ ] `codelore analyze --analysis hotspots --format csv` and `--analysis code-health --format csv` work
- [ ] All previous tests pass (22 lib + 4 cli + 205 codelore-rca + new Plan 3 tests = ~240)
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- [ ] `cargo fmt --all --check` clean
- [ ] `cargo deny check` clean
- [ ] CHANGELOG and README updated

After Plan 3: author **Plan 4** (9 other code-maat analyses + Fisher significance + identity resolution + complete the Code Health composite).

---

## Self-Review

### Spec coverage check

| Spec section | Plan 3 coverage |
|---|---|
| §1.1 hotspot ranking formula | ✓ Task 5 |
| §1.1 function-level entity baseline | ✓ Task 3 (via FuncSpace traversal) |
| §4 complexity strategy | ✓ Tasks 2-4 (HEAD-only computation) |
| §4.4 computation modes | Partial — `head` (default) only; `adaptive` and `full` are Plan 4 |
| §4.6 Code Health composite | ✓ Task 6 (cognitive input only in Plan 3) |
| Schema `entities` + `complexity_metrics` tables | ✓ Task 4 |

### Placeholder scan

No `TBD`/`TODO`/"similar to" placeholders. Forward references ("Plan 4 will fix the simplification") are deliberate cross-plan dependencies.

### Known soft spots

- **HEAD blob reading via filesystem**: Task 4 uses `std::fs::read(repo_path/path)` instead of `gix::Repository::find_blob(oid)`. Works for working-copy repos; fails for bare. Plan 4 fix.
- **Code Health is incomplete**: only cognitive input. churn/fragmentation/coupling weights are 0 until their analyses ship in Plan 4.
- **Per-revision complexity history not tracked**: only HEAD is stored. Plan 4 will add the sampling modes.
- **codelore-rca API exact field names**: Task 3's `space.metrics.cyclomatic.cyclomatic_sum()` etc. were confirmed working in Plan 2 Task 6; Halstead/MI/etc. field names need verification when wiring up. Implementer should adapt.

---

*End of Plan 3.*
