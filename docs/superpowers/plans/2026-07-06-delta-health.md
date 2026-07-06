# Delta Health Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-change health verdict for `codelore diff` / `codelore check`: a 0–100 `delta_health_ratio` + `improving`/`indeterminate`/`degrading` verdict computed by table-diffing per-function complexity metrics at base vs head, with context modulation for red-band files and clone-membership as a copy/paste penalty.

**Architecture:** New pure-logic analysis module `analyses/delta_health.rs` in `codelore-lib` (risk classification, outcome matrix, ratio/verdict math, one SQL extraction). The CLI's existing `codelore diff` flow (`diff.rs`) — which already builds full facts at base and head in temp worktrees — extracts function rows + red-file bands per rev into the serializable `RevAnalyses`, then calls the pure compute function. Gates extend the existing `[diff]` TOML section and `evaluate_diff_gate`.

**Tech Stack:** Rust, DuckDB (via existing `FactsDb`), serde, clap (untouched), existing `duckdb::params!` query pattern.

**Spec:** `docs/superpowers/specs/2026-07-06-delta-health-design.md` — read it first; every threshold and semantic below comes from it.

## Global Constraints

- `workspace.lints.rust: unsafe_code = "forbid"` — zero `unsafe`.
- No `unwrap()` outside `#[cfg(test)]`; library errors via `CodeLoreError` (thiserror), CLI errors via `anyhow`.
- No ticket IDs, plan references, or version numbers in code comments or docs.
- `#[serde(deny_unknown_fields)]` on threshold structs must be preserved.
- New `RevAnalyses` fields use `#[serde(default)]` (established base-cache back-compat convention).
- NO `Repo` trait changes; NO facts-schema changes; NO `CACHE_EPOCH` bump.
- Every commit message: Conventional Commits, no co-author trailers.
- Final gate must be the exact CI command: `cargo clippy --workspace --all-targets --all-features -- -D warnings` (use `just lint`) plus `cargo fmt --all --check` and full tests (`just ci`).

## Execution Guardrails (read before every task)

1. Run every command from the repository root.
2. Touch ONLY the files listed in the current task's **Files** block. Never
   reformat, "improve", or reorder code you are not instructed to change.
3. Every code block in a step is complete and final — copy it verbatim. Do
   not paraphrase, rename identifiers, or "simplify".
4. If a command's output does not match the step's **Expected** line, STOP.
   Re-read the step and the two files involved. If still mismatched after one
   careful re-read, report the exact command, full output, and step number —
   do not guess, do not loosen an assertion, do not add `#[allow(...)]`,
   `#[ignore]`, `unwrap()` outside tests, or sleep/retry loops.
5. Run `cargo fmt --all` before every commit (formatting-only changes to the
   files you touched are expected and fine to include).
6. Commit exactly the files listed in the step's `git add`, with the exact
   message given. Never add co-author trailers.
7. Insertion anchors are given as existing source lines. If an anchor cannot
   be found verbatim, STOP and report — do not pick a "similar" location.

## File Structure

- Create: `crates/codelore-lib/src/analyses/delta_health.rs` — risk model, compute, SQL extraction (one responsibility: change-level health).
- Create: `crates/codelore-lib/tests/delta_health_test.rs` — extraction integration test.
- Modify: `crates/codelore-lib/src/analyses/mod.rs` — register module.
- Modify: `crates/codelore-lib/src/quality_gates/mod.rs` — `DiffGates` + `is_empty` + `evaluate_diff_gate`.
- Modify: `crates/codelore-cli/src/diff.rs` — `RevAnalyses`, `analyze_at_rev`, `run_diff`, `DiffOutput`.
- Modify: `crates/codelore-cli/src/diff_output.rs` — text + markdown sections (JSON is free via serde).
- Modify: `crates/codelore-cli/tests/cli_test.rs` — end-to-end tests.
- Modify: `docs/advanced-usage.md`, `CHANGELOG.md`.

---

### Task 1: Risk model core (pure logic)

**Files:**
- Create: `crates/codelore-lib/src/analyses/delta_health.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs` (one line: `pub mod delta_health;` in alphabetical position)

**Interfaces:**
- Produces (used by Tasks 3/5):
  - `pub enum RiskClass { Low, Medium, High }` — `Copy, Ord`, serde lowercase
  - `pub enum Outcome { Good, Neutral, Bad }` — `Copy`, serde lowercase
  - `pub fn classify(loc: u32, cyclomatic: f64, clone_member: bool) -> RiskClass`
  - `pub fn outcome_for(before: Option<RiskClass>, after: Option<RiskClass>) -> Outcome`
  - `pub fn verdict_for(ratio: f64) -> &'static str`
  - Constants: `RATIO_DEGRADING_BELOW = 40.0`, `RATIO_IMPROVING_ABOVE = 70.0`, `RED_FILE_WEIGHT_MULTIPLIER = 1.5`

- [ ] **Step 1: Create the module with types, constants, and failing tests**

Create `crates/codelore-lib/src/analyses/delta_health.rs`:

```rust
//! `delta-health` — change-level health verdict for `codelore diff`.
//!
//! Judges the CHANGE, not the snapshot: each function added, removed, or
//! modified between base and head is classified low/medium/high risk from
//! absolute thresholds, given an outcome (good/neutral/bad) from its
//! before→after direction, and aggregated into a 0–100 ratio with an
//! explicit low-signal middle verdict. Snapshot scores are provably
//! insensitive to individual commits; this is the per-change complement.
//!
//! Thresholds are FIXED constants, not TOML-configurable: the gate cannot
//! be quietly loosened, and verdicts stay stable across PRs.

use serde::{Deserialize, Serialize};

/// Function LOC at or above this is medium risk (SIG unit-size bands).
pub const LOC_MEDIUM_FROM: u32 = 31;
/// Function LOC at or above this is high risk (SIG bands / Large Method > 70).
pub const LOC_HIGH_FROM: u32 = 71;
/// Cyclomatic complexity at or above this is medium risk (SIG bands).
pub const CYCLOMATIC_MEDIUM_FROM: f64 = 6.0;
/// Cyclomatic complexity at or above this is high risk (SIG bands / CC > 10).
pub const CYCLOMATIC_HIGH_FROM: f64 = 11.0;
/// Ratio strictly below this ⇒ `degrading` verdict.
pub const RATIO_DEGRADING_BELOW: f64 = 40.0;
/// Ratio strictly above this ⇒ `improving` verdict.
pub const RATIO_IMPROVING_ABOVE: f64 = 70.0;
/// Good/bad weight multiplier for functions in base-red-band files.
pub const RED_FILE_WEIGHT_MULTIPLIER: f64 = 1.5;

/// Risk class of a single function. Derive order matters: `Low < Medium
/// < High` powers the improved/degraded direction comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskClass {
    Low,
    Medium,
    High,
}

impl RiskClass {
    /// Lowercase display form, matching the serde encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Outcome of one changed function within the scored change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Good,
    Neutral,
    Bad,
}

impl Outcome {
    /// Lowercase display form, matching the serde encoding.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Neutral => "neutral",
            Self::Bad => "bad",
        }
    }
}

/// Classify one function from its persisted metrics. Worst triggered
/// property wins; clone membership forces High (the copy/paste penalty —
/// AI-pasted duplicates cannot score low-risk).
#[must_use]
pub fn classify(loc: u32, cyclomatic: f64, clone_member: bool) -> RiskClass {
    if clone_member || loc >= LOC_HIGH_FROM || cyclomatic >= CYCLOMATIC_HIGH_FROM {
        return RiskClass::High;
    }
    if loc >= LOC_MEDIUM_FROM || cyclomatic >= CYCLOMATIC_MEDIUM_FROM {
        return RiskClass::Medium;
    }
    RiskClass::Low
}

/// Outcome matrix per the design: added = ∅→class, removed = class→∅.
/// Good — ends Low, strictly improves, or removes a High-risk function.
/// Bad — ends High or strictly degrades. Neutral — everything else.
#[must_use]
pub fn outcome_for(before: Option<RiskClass>, after: Option<RiskClass>) -> Outcome {
    match (before, after) {
        (None, Some(a)) => match a {
            RiskClass::Low => Outcome::Good,
            RiskClass::Medium => Outcome::Neutral,
            RiskClass::High => Outcome::Bad,
        },
        (Some(b), None) => {
            if b == RiskClass::High {
                Outcome::Good
            } else {
                Outcome::Neutral
            }
        }
        (Some(b), Some(a)) => {
            if a == RiskClass::Low || a < b {
                Outcome::Good
            } else if a == RiskClass::High || a > b {
                Outcome::Bad
            } else {
                Outcome::Neutral
            }
        }
        // A function neither present at base nor head is never a change
        // candidate; keep the match total without panicking.
        (None, None) => Outcome::Neutral,
    }
}

/// Verdict from the ratio. The middle band is deliberately labeled
/// `indeterminate` — the design's honest replacement for a binary cut.
#[must_use]
pub fn verdict_for(ratio: f64) -> &'static str {
    if ratio < RATIO_DEGRADING_BELOW {
        "degrading"
    } else if ratio > RATIO_IMPROVING_ABOVE {
        "improving"
    } else {
        "indeterminate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_loc_boundaries() {
        assert_eq!(classify(30, 1.0, false), RiskClass::Low);
        assert_eq!(classify(31, 1.0, false), RiskClass::Medium);
        assert_eq!(classify(70, 1.0, false), RiskClass::Medium);
        assert_eq!(classify(71, 1.0, false), RiskClass::High);
    }

    #[test]
    fn classify_cyclomatic_boundaries() {
        assert_eq!(classify(10, 5.0, false), RiskClass::Low);
        assert_eq!(classify(10, 6.0, false), RiskClass::Medium);
        assert_eq!(classify(10, 10.0, false), RiskClass::Medium);
        assert_eq!(classify(10, 11.0, false), RiskClass::High);
    }

    #[test]
    fn classify_clone_membership_forces_high() {
        assert_eq!(classify(5, 1.0, true), RiskClass::High);
    }

    #[test]
    fn classify_worst_property_wins() {
        // Low LOC but high cyclomatic ⇒ High.
        assert_eq!(classify(10, 20.0, false), RiskClass::High);
        // High LOC but trivial cyclomatic ⇒ High.
        assert_eq!(classify(100, 1.0, false), RiskClass::High);
    }

    #[test]
    fn outcome_added() {
        assert_eq!(outcome_for(None, Some(RiskClass::Low)), Outcome::Good);
        assert_eq!(outcome_for(None, Some(RiskClass::Medium)), Outcome::Neutral);
        assert_eq!(outcome_for(None, Some(RiskClass::High)), Outcome::Bad);
    }

    #[test]
    fn outcome_removed() {
        assert_eq!(outcome_for(Some(RiskClass::High), None), Outcome::Good);
        assert_eq!(outcome_for(Some(RiskClass::Medium), None), Outcome::Neutral);
        assert_eq!(outcome_for(Some(RiskClass::Low), None), Outcome::Neutral);
    }

    #[test]
    fn outcome_modified_matrix() {
        use RiskClass::{High, Low, Medium};
        // Stayed low ⇒ good; improved ⇒ good (even High→Medium).
        assert_eq!(outcome_for(Some(Low), Some(Low)), Outcome::Good);
        assert_eq!(outcome_for(Some(High), Some(Medium)), Outcome::Good);
        assert_eq!(outcome_for(Some(Medium), Some(Low)), Outcome::Good);
        // Ends high or degrades ⇒ bad.
        assert_eq!(outcome_for(Some(High), Some(High)), Outcome::Bad);
        assert_eq!(outcome_for(Some(Low), Some(Medium)), Outcome::Bad);
        assert_eq!(outcome_for(Some(Medium), Some(High)), Outcome::Bad);
        // Stayed medium ⇒ neutral.
        assert_eq!(outcome_for(Some(Medium), Some(Medium)), Outcome::Neutral);
    }

    #[test]
    fn verdict_cut_points() {
        assert_eq!(verdict_for(39.9), "degrading");
        assert_eq!(verdict_for(40.0), "indeterminate");
        assert_eq!(verdict_for(70.0), "indeterminate");
        assert_eq!(verdict_for(70.1), "improving");
    }
}
```

The only import this task needs is the `serde` line shown at the top of the
module; Tasks 2 and 3 each add their own imports explicitly.

- [ ] **Step 2: Register the module**

In `crates/codelore-lib/src/analyses/mod.rs`, add in alphabetical order among the existing `pub mod` lines:

```rust
pub mod delta_health;
```

- [ ] **Step 3: Run the tests — expect PASS**

Run: `cargo test -p codelore-lib --features test-support delta_health`
Expected: 8 tests pass (`classify_*`, `outcome_*`, `verdict_cut_points`).

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/src/analyses/delta_health.rs crates/codelore-lib/src/analyses/mod.rs
git commit -m "feat(delta-health): risk classes, outcome matrix, verdict cut-points"
```

---

### Task 2: Per-rev function-metric extraction

**Files:**
- Modify: `crates/codelore-lib/src/analyses/delta_health.rs`
- Test: `crates/codelore-lib/tests/delta_health_test.rs`

**Interfaces:**
- Produces (used by Task 5):
  - `pub struct FunctionMetricRow { pub path: String, pub name: String, pub loc: u32, pub cyclomatic: f64 }` — `Clone, Serialize, Deserialize` (it rides inside the CLI's base-cache JSON). `name` is the **bare** function name: persisted entity names embed the line span (`fn@start-end`), which is unstable across revisions, so `run_function_metrics` strips the suffix (`regexp_replace(name, '@[0-9]+-[0-9]+$', '')`) and aggregates same-named functions per file to worst-case metrics (`MAX(loc)`, `MAX(cyclomatic)`). Task 3 therefore pairs base/head on this stable bare `(path, name)` key.
  - `pub fn run_function_metrics(db: &FactsDb) -> Result<Vec<FunctionMetricRow>>`
- Consumes: `FactsDb` (existing), `complexity_metrics` + `entities` tables (existing schema).

- [ ] **Step 1: Write the failing integration test**

Create `crates/codelore-lib/tests/delta_health_test.rs`:

```rust
//! Integration test for the delta-health function-metric extraction:
//! function/method rows come back; class- and file-level complexity rows
//! do not leak in as functions.

use std::path::Path;
use std::process::Command;

use codelore_lib::cli_api::Options;
use codelore_lib::cli_api::analyses::delta_health::run_function_metrics;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::GixRepo;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

#[test]
fn extracts_function_rows_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn tiny() -> i32 {\n    1\n}\n\npub fn also_tiny() -> i32 {\n    2\n}\n",
    )
    .unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "init"]);

    let opts = Options {
        repo_path: repo.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    let gix = GixRepo::open(repo).expect("open repo");
    let db = FactsDb::new_in_memory().expect("in-memory facts");
    db.ingest(&gix, &opts).expect("ingest");

    let rows = run_function_metrics(&db).expect("extract");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"tiny"), "missing fn tiny in {names:?}");
    assert!(names.contains(&"also_tiny"), "missing fn also_tiny in {names:?}");
    // No file-level unit row masquerading as a function.
    assert!(
        rows.iter().all(|r| r.name != "src/lib.rs" && !r.name.is_empty()),
        "file/unit rows leaked: {names:?}"
    );
    for r in &rows {
        assert_eq!(r.path, "src/lib.rs");
        assert!(r.loc >= 1, "loc should be populated, got {}", r.loc);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p codelore-lib --features test-support --test delta_health_test`
Expected: FAIL to compile with "cannot find function `run_function_metrics`".

- [ ] **Step 3: Implement the extraction**

First replace the module's import block (currently just the `serde` line) with:

```rust
use serde::{Deserialize, Serialize};

use crate::facts::FactsDb;
use crate::{CodeLoreError, Result};
```

Then append to `crates/codelore-lib/src/analyses/delta_health.rs` (above the `#[cfg(test)]` module):

```rust
/// One function's persisted metrics at a single rev. Serialized into the
/// CLI's `--base-cache` JSON, so field changes need the same
/// back-compat care as the cache struct itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionMetricRow {
    pub path: String,
    pub name: String,
    pub loc: u32,
    pub cyclomatic: f64,
}

/// Extract per-function metric rows from an ingested fact store.
///
/// `complexity_metrics` also stores class- and file-level rows; the join
/// on `entities.kind` keeps only real functions/methods so nothing
/// file-shaped is ever classified as a changed function.
///
/// # Errors
///
/// [`CodeLoreError::Analysis`] on SQL failures.
pub fn run_function_metrics(db: &FactsDb) -> Result<Vec<FunctionMetricRow>> {
    const SQL: &str = "
        SELECT DISTINCT cm.path, cm.name, cm.loc, CAST(cm.cyclomatic AS DOUBLE)
        FROM complexity_metrics cm
        JOIN entities e ON e.path = cm.path AND e.name = cm.name
        WHERE e.kind IN ('function', 'method')
        ORDER BY cm.path, cm.name";
    let mut stmt = db
        .conn()
        .prepare(SQL)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare delta-health metrics: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FunctionMetricRow {
                path: r.get::<_, String>(0)?,
                name: r.get::<_, String>(1)?,
                loc: r.get::<_, u32>(2)?,
                cyclomatic: r.get::<_, f64>(3)?,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query delta-health metrics: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("read delta-health metrics: {e}")))?;
    Ok(rows)
}
```

`db.conn()` is crate-visible and is exactly how the sibling
`analyses/code_health.rs::run_code_health` prepares its statement — that
function is the canonical template if anything about the query pattern is
unclear.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p codelore-lib --features test-support --test delta_health_test`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/analyses/delta_health.rs crates/codelore-lib/tests/delta_health_test.rs
git commit -m "feat(delta-health): per-function metric extraction with entity-kind filter"
```

---

### Task 3: `compute_delta_health` (pairing, weights, ratio)

**Files:**
- Modify: `crates/codelore-lib/src/analyses/delta_health.rs`

**Interfaces:**
- Produces (used by Task 5):
  - `pub struct DeltaFunctionRow { pub path: String, pub function: String, pub before: Option<RiskClass>, pub after: Option<RiskClass>, pub outcome: Outcome, pub weight: f64, pub in_red_file: bool, pub reasons: Vec<String> }` — `Clone, Serialize`
  - `pub struct DeltaHealthCounts { pub added: u32, pub modified: u32, pub removed: u32, pub skipped: u32 }` — `Clone, Default, Serialize`
  - `pub struct DeltaHealthSection { pub ratio: Option<f64>, pub verdict: String, pub counts: DeltaHealthCounts, pub functions: Vec<DeltaFunctionRow> }` — `Clone, Serialize`
  - `pub fn compute_delta_health(base: &[FunctionMetricRow], head: &[FunctionMetricRow], pr_files: &HashSet<String>, head_clone_members: &HashSet<(String, String)>, base_red_files: &HashSet<String>) -> DeltaHealthSection`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` in `delta_health.rs`:

```rust
    fn row(path: &str, name: &str, loc: u32, cyclo: f64) -> FunctionMetricRow {
        FunctionMetricRow {
            path: path.into(),
            name: name.into(),
            loc,
            cyclomatic: cyclo,
        }
    }

    fn files(paths: &[&str]) -> std::collections::HashSet<String> {
        paths.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn no_changed_functions_is_no_code_change() {
        let base = vec![row("a.rs", "f", 10, 1.0)];
        let head = vec![row("a.rs", "f", 10, 1.0)]; // identical ⇒ untouched
        let s = compute_delta_health(
            &base,
            &head,
            &files(&["a.rs", "README.md"]),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(s.verdict, "no-code-change");
        assert_eq!(s.ratio, None);
        assert!(s.functions.is_empty());
        // README.md changed but has no functions at either rev ⇒ skipped.
        assert_eq!(s.counts.skipped, 1);
    }

    #[test]
    fn added_high_risk_function_degrades() {
        let base: Vec<FunctionMetricRow> = vec![];
        let head = vec![row("a.rs", "monster", 120, 15.0)];
        let s = compute_delta_health(
            &base,
            &head,
            &files(&["a.rs"]),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(s.counts.added, 1);
        assert_eq!(s.ratio, Some(0.0));
        assert_eq!(s.verdict, "degrading");
        assert_eq!(s.functions[0].outcome, Outcome::Bad);
        assert_eq!(s.functions[0].before, None);
        assert_eq!(s.functions[0].after, Some(RiskClass::High));
        assert!(!s.functions[0].reasons.is_empty());
    }

    #[test]
    fn functions_outside_pr_files_are_ignored() {
        let base = vec![row("other.rs", "f", 10, 1.0)];
        let head = vec![row("other.rs", "f", 200, 30.0)]; // differs, but not a PR file
        let s = compute_delta_health(
            &base,
            &head,
            &files(&["a.rs"]),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(s.verdict, "no-code-change");
    }

    #[test]
    fn clone_member_added_function_is_bad_even_if_tiny() {
        let head = vec![row("a.rs", "pasted", 8, 1.0)];
        let clones: std::collections::HashSet<(String, String)> =
            [("a.rs".to_string(), "pasted".to_string())].into();
        let s = compute_delta_health(&[], &head, &files(&["a.rs"]), &clones, &Default::default());
        assert_eq!(s.functions[0].after, Some(RiskClass::High));
        assert_eq!(s.functions[0].outcome, Outcome::Bad);
        assert!(
            s.functions[0].reasons.iter().any(|r| r.contains("clone")),
            "reasons: {:?}",
            s.functions[0].reasons
        );
    }

    #[test]
    fn red_file_multiplier_amplifies_good_and_bad_not_neutral() {
        // One good (10 LOC) + one bad (10 LOC) + one neutral (40 LOC) change.
        let base = vec![
            row("red.rs", "improved", 80, 1.0),  // High → Low = good
            row("red.rs", "worsened", 10, 1.0),  // Low → High = bad
            row("red.rs", "meh", 40, 1.0),       // Medium → Medium = neutral
        ];
        let head = vec![
            row("red.rs", "improved", 10, 1.0),
            row("red.rs", "worsened", 100, 1.0),
            row("red.rs", "meh", 41, 1.0),
        ];
        let red = files(&["red.rs"]);
        let s = compute_delta_health(&base, &head, &files(&["red.rs"]), &Default::default(), &red);
        // good_w = 10*1.5, bad_w = 100*1.5, neutral_w = 41 (unmodulated).
        // ratio = 100 * 15 / (15 + 150 + 41)
        let expected = 100.0 * 15.0 / (15.0 + 150.0 + 41.0);
        let got = s.ratio.expect("ratio");
        assert!((got - expected).abs() < 1e-9, "got {got}, expected {expected}");
        assert!(s.functions.iter().all(|f| f.in_red_file));
    }

    #[test]
    fn removed_high_risk_function_counts_as_good_with_base_weight() {
        let base = vec![row("a.rs", "monster", 120, 15.0)];
        let s = compute_delta_health(
            &base,
            &[],
            &files(&["a.rs"]),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(s.counts.removed, 1);
        assert_eq!(s.ratio, Some(100.0));
        assert_eq!(s.verdict, "improving");
        assert!((s.functions[0].weight - 120.0).abs() < f64::EPSILON);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codelore-lib --features test-support delta_health`
Expected: FAIL to compile with "cannot find function `compute_delta_health`".

- [ ] **Step 3: Implement compute**

First extend the module's import block by adding one line after the `serde` import:

```rust
use std::collections::{HashMap, HashSet};
```

(std imports go first in this codebase's import ordering — place the line
above the `serde` line, matching `code_health.rs`.)

Then append to `delta_health.rs` (above the test module):

```rust
/// One changed function in the scored set. `weight` is the RAW LOC
/// weight; the red-file multiplier applies only inside the ratio so the
/// reported numbers stay physical (`in_red_file` tells the story).
#[derive(Debug, Clone, Serialize)]
pub struct DeltaFunctionRow {
    pub path: String,
    pub function: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<RiskClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<RiskClass>,
    pub outcome: Outcome,
    pub weight: f64,
    pub in_red_file: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DeltaHealthCounts {
    pub added: u32,
    pub modified: u32,
    pub removed: u32,
    /// Changed files with no analyzable functions at either rev
    /// (unsupported languages, config/docs). Surfaced so coverage gaps
    /// are visible instead of silently omitted.
    pub skipped: u32,
}

/// The `delta_health` section of a diff run. `ratio == None` ⟺
/// `verdict == "no-code-change"`.
#[derive(Debug, Clone, Serialize)]
pub struct DeltaHealthSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    pub verdict: String,
    pub counts: DeltaHealthCounts,
    pub functions: Vec<DeltaFunctionRow>,
}

fn reasons_for(loc: u32, cyclomatic: f64, clone_member: bool) -> Vec<String> {
    let mut out = Vec::new();
    if clone_member {
        out.push("member of a clone group".to_string());
    }
    if loc >= LOC_HIGH_FROM {
        out.push(format!("loc {loc} \u{2265} {LOC_HIGH_FROM}"));
    } else if loc >= LOC_MEDIUM_FROM {
        out.push(format!("loc {loc} \u{2265} {LOC_MEDIUM_FROM}"));
    }
    if cyclomatic >= CYCLOMATIC_HIGH_FROM {
        out.push(format!("cyclomatic {cyclomatic:.0} \u{2265} {CYCLOMATIC_HIGH_FROM:.0}"));
    } else if cyclomatic >= CYCLOMATIC_MEDIUM_FROM {
        out.push(format!("cyclomatic {cyclomatic:.0} \u{2265} {CYCLOMATIC_MEDIUM_FROM:.0}"));
    }
    out
}

/// Pair base/head function rows for the PR's changed files, classify,
/// score, and produce the section. Pure — all inputs are plain rows/sets
/// so this is directly reusable by future MCP/feed consumers.
#[must_use]
pub fn compute_delta_health(
    base: &[FunctionMetricRow],
    head: &[FunctionMetricRow],
    pr_files: &HashSet<String>,
    head_clone_members: &HashSet<(String, String)>,
    base_red_files: &HashSet<String>,
) -> DeltaHealthSection {
    let index = |rows: &[FunctionMetricRow]| -> HashMap<(String, String), FunctionMetricRow> {
        rows.iter()
            .filter(|r| pr_files.contains(&r.path))
            .map(|r| ((r.path.clone(), r.name.clone()), r.clone()))
            .collect()
    };
    let base_idx = index(base);
    let head_idx = index(head);

    let mut keys: Vec<(String, String)> = base_idx.keys().chain(head_idx.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    let mut counts = DeltaHealthCounts::default();
    let mut functions = Vec::new();
    let (mut good_w, mut neutral_w, mut bad_w) = (0.0_f64, 0.0_f64, 0.0_f64);

    for key in keys {
        let b = base_idx.get(&key);
        let h = head_idx.get(&key);
        // Identical rows are untouched functions — excluded entirely.
        if let (Some(b), Some(h)) = (b, h)
            && b == h
        {
            continue;
        }
        let clone_member = head_clone_members.contains(&key);
        let before = b.map(|r| classify(r.loc, r.cyclomatic, false));
        let after = h.map(|r| classify(r.loc, r.cyclomatic, clone_member));
        match (b.is_some(), h.is_some()) {
            (false, true) => counts.added += 1,
            (true, false) => counts.removed += 1,
            _ => counts.modified += 1,
        }
        let outcome = outcome_for(before, after);
        let weight = f64::from(h.or(b).map_or(0, |r| r.loc));
        let in_red_file = base_red_files.contains(&key.0);
        let mult = if in_red_file && outcome != Outcome::Neutral {
            RED_FILE_WEIGHT_MULTIPLIER
        } else {
            1.0
        };
        match outcome {
            Outcome::Good => good_w += weight * mult,
            Outcome::Neutral => neutral_w += weight,
            Outcome::Bad => bad_w += weight * mult,
        }
        let reasons = h.map_or_else(Vec::new, |r| reasons_for(r.loc, r.cyclomatic, clone_member));
        functions.push(DeltaFunctionRow {
            path: key.0,
            function: key.1,
            before,
            after,
            outcome,
            weight,
            in_red_file,
            reasons,
        });
    }

    // Changed files with no function rows at either rev.
    let covered: HashSet<&String> = base_idx.keys().chain(head_idx.keys()).map(|k| &k.0).collect();
    counts.skipped = u32::try_from(pr_files.iter().filter(|p| !covered.contains(p)).count())
        .unwrap_or(u32::MAX);

    let total = good_w + neutral_w + bad_w;
    if functions.is_empty() || total <= 0.0 {
        return DeltaHealthSection {
            ratio: None,
            verdict: "no-code-change".to_string(),
            counts,
            functions,
        };
    }
    let ratio = 100.0 * good_w / total;
    DeltaHealthSection {
        ratio: Some(ratio),
        verdict: verdict_for(ratio).to_string(),
        counts,
        functions,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p codelore-lib --features test-support delta_health`
Expected: PASS (14 tests: 8 from Task 1 + 6 new).

- [ ] **Step 5: Clippy the lib crate, then commit**

Run: `cargo clippy -p codelore-lib --all-targets --all-features -- -D warnings`
Expected: clean (fix anything it flags before committing).

```bash
git add crates/codelore-lib/src/analyses/delta_health.rs
git commit -m "feat(delta-health): change-set pairing, context-weighted ratio, verdict"
```

---

### Task 4: `[diff]` gate extension

**Files:**
- Modify: `crates/codelore-lib/src/quality_gates/mod.rs`

**Interfaces:**
- Produces (used by Task 5):
  - `DiffGates.delta_health_min: Option<f64>`, `DiffGates.deny_degrading_verdict: bool`
  - `evaluate_diff_gate(thresholds, new_hotspot_count, delta_code_health, base_cycles, head_cycles, delta_health_ratio: Option<f64>, delta_health_verdict: Option<&str>) -> Vec<GateViolation>` (two appended parameters)

- [ ] **Step 1: Write the failing tests**

In the existing `#[cfg(test)]` module of `quality_gates/mod.rs`, add (mirroring the style of the existing `evaluate_diff_gate` tests):

```rust
    #[test]
    fn delta_health_min_gate_fires_below_floor() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        let v = evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(42.0), Some("indeterminate"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "delta_health_min");
    }

    #[test]
    fn delta_health_min_gate_passes_at_floor_and_skips_no_code_change() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        assert!(evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(50.0), Some("indeterminate")).is_empty());
        // no-code-change ⇒ ratio None ⇒ vacuous pass.
        assert!(evaluate_diff_gate(&t, 0, 0.0, 0, 0, None, Some("no-code-change")).is_empty());
    }

    #[test]
    fn deny_degrading_verdict_gate() {
        let t = Thresholds::from_text("[diff]\ndeny_degrading_verdict = true\n").unwrap();
        let v = evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(10.0), Some("degrading"));
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].gate, "deny_degrading_verdict");
        assert!(evaluate_diff_gate(&t, 0, 0.0, 0, 0, Some(60.0), Some("indeterminate")).is_empty());
    }

    #[test]
    fn is_empty_accounts_for_delta_health_keys() {
        let t = Thresholds::from_text("[diff]\ndelta_health_min = 50.0\n").unwrap();
        assert!(!t.is_empty());
        let t = Thresholds::from_text("[diff]\ndeny_degrading_verdict = true\n").unwrap();
        assert!(!t.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p codelore-lib --features test-support quality_gates`
Expected: FAIL to compile — unknown fields / wrong arity on `evaluate_diff_gate`.

- [ ] **Step 3: Implement**

In `DiffGates`, append after `no_new_cycles`:

```rust
    /// Minimum delta-health ratio (0–100): the share of changed-function
    /// weight ending low-risk or improved. A ratio below this fails.
    /// Skipped entirely on `no-code-change` diffs (no ratio, no signal).
    pub delta_health_min: Option<f64>,
    /// When true, a `degrading` delta-health verdict fails the gate.
    #[serde(default)]
    pub deny_degrading_verdict: bool,
```

In `Thresholds::is_empty`, extend the conjunction:

```rust
            && self.diff.delta_health_min.is_none()
            && !self.diff.deny_degrading_verdict
```

Change `evaluate_diff_gate`'s signature to append the two parameters, and add before the final `out`:

```rust
pub fn evaluate_diff_gate(
    thresholds: &Thresholds,
    new_hotspot_count: u32,
    delta_code_health: f64,
    base_cycles: u32,
    head_cycles: u32,
    delta_health_ratio: Option<f64>,
    delta_health_verdict: Option<&str>,
) -> Vec<GateViolation> {
```

```rust
    if let Some(min) = d.delta_health_min
        && let Some(ratio) = delta_health_ratio
        && ratio < min
    {
        out.push(GateViolation {
            gate: "delta_health_min".into(),
            path: "(diff-summary)".into(),
            actual: format!("{ratio:.1}"),
            threshold: format!("\u{2265} {min:.1}"),
        });
    }
    if d.deny_degrading_verdict && delta_health_verdict == Some("degrading") {
        out.push(GateViolation {
            gate: "deny_degrading_verdict".into(),
            path: "(diff-summary)".into(),
            actual: "degrading".into(),
            threshold: "verdict != degrading".into(),
        });
    }
```

Then fix the existing test call sites mechanically: search this file for every
call of the form `evaluate_diff_gate(&t, ` (7 occurrences in the `#[cfg(test)]`
module) and append `, None, None` inside the closing parenthesis of each —
e.g. `evaluate_diff_gate(&t, 999, -100.0, 0, 5)` becomes
`evaluate_diff_gate(&t, 999, -100.0, 0, 5, None, None)`.

Arity note: the new signature has exactly 7 parameters, which is within
clippy's default `too_many_arguments` limit — no lint fires and no `#[allow]`
is needed (adding one would violate the repo's rules anyway).

(The CLI call site is updated in Task 5 — `cargo test -p codelore-cli` will
not compile until then, which is why Steps 4–5 test the lib crate only.)

- [ ] **Step 4: Run lib tests to verify they pass**

Run: `cargo test -p codelore-lib --features test-support quality_gates`
Expected: PASS (all existing + 4 new).

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/quality_gates/mod.rs
git commit -m "feat(delta-health): delta_health_min and deny_degrading_verdict diff gates"
```

---

### Task 5: CLI wiring — RevAnalyses, run_diff, emitters

**Files:**
- Modify: `crates/codelore-cli/src/diff.rs`
- Modify: `crates/codelore-cli/src/diff_output.rs`

**Interfaces:**
- Consumes: `run_function_metrics`, `compute_delta_health`, `DeltaHealthSection`, `FunctionMetricRow` (Tasks 2–3); `run_code_health` + `CodeHealthRow.band` (existing); new `evaluate_diff_gate` arity (Task 4).
- Produces: `DiffOutput.delta_health: Option<DeltaHealthSection>` (consumed by Task 6 tests and all emitters).

- [ ] **Step 1: Extend imports and `RevAnalyses` in `diff.rs`**

Add imports:

```rust
use codelore_lib::cli_api::analyses::code_health::run_code_health;
use codelore_lib::cli_api::analyses::delta_health::{
    DeltaHealthSection, FunctionMetricRow, compute_delta_health, run_function_metrics,
};
```

Extend `RevAnalyses` (after `dependency_cycles`, same `#[serde(default)]` convention):

```rust
    /// Per-function metric rows for delta-health. `#[serde(default)]` so a
    /// base-cache written before this field deserialises to empty; the
    /// consumer treats empty-with-nonempty-hotspots as "stale cache" and
    /// skips delta-health rather than misreading every head function as
    /// added.
    #[serde(default)]
    pub functions: Vec<FunctionMetricRow>,
    /// Paths whose file-level code-health band is red at this rev. Powers
    /// the delta-health context multiplier.
    #[serde(default)]
    pub red_files: Vec<String>,
```

- [ ] **Step 2: Populate the new fields in `analyze_at_rev`**

After the `dependency_cycles` computation, before the `Ok(RevAnalyses { ... })`:

```rust
    let functions = run_function_metrics(&db).context("function metrics at rev")?;
    let red_files: Vec<String> = run_code_health(&db, &opts)
        .context("code health at rev")?
        .into_iter()
        .filter(|r| r.band == "red")
        .map(|r| r.path)
        .collect();
```

and add `functions, red_files,` to the returned struct literal.

- [ ] **Step 3: Compute the section in `run_diff` and extend `DiffOutput`**

Add to `DiffOutput` (after `gate_violations`):

```rust
    /// Change-level health verdict. `None` when the base analysis lacks
    /// function metrics (stale `--base-cache` written by an older binary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_health: Option<DeltaHealthSection>,
```

In `run_diff`, after `let pr_files = list_pr_files(...)`:

```rust
    // Delta health: always computed (not gated behind thresholds) — the
    // section is standalone review signal. Guard against a base-cache
    // written before function metrics existed: empty base functions with
    // a non-empty base analysis would misread every head function as
    // "added" and poison the verdict.
    let delta_health = if base_analyses.functions.is_empty()
        && !base_analyses.hotspots.is_empty()
        && !head_analyses.functions.is_empty()
    {
        tracing::warn!(
            "base analysis has no function metrics (stale --base-cache?); \
             skipping delta-health — delete the cache file to recompute"
        );
        None
    } else {
        let clone_members: std::collections::HashSet<(String, String)> = head_analyses
            .clones
            .iter()
            .map(|c| (c.entity.clone(), c.function.clone()))
            .collect();
        let red: std::collections::HashSet<String> =
            base_analyses.red_files.iter().cloned().collect();
        Some(compute_delta_health(
            &base_analyses.functions,
            &head_analyses.functions,
            &pr_files,
            &clone_members,
            &red,
        ))
    };
```

Extend the gate-trigger condition to include the new keys:

```rust
        && (t.diff.delta_code_health_min.is_some()
            || t.diff.new_hotspot_max.is_some()
            || t.diff.no_new_cycles
            || t.diff.delta_health_min.is_some()
            || t.diff.deny_degrading_verdict)
```

Update the `evaluate_diff_gate` call to pass the two new arguments:

```rust
                delta_health.as_ref().and_then(|d| d.ratio),
                delta_health.as_ref().map(|d| d.verdict.as_str()),
```

Add `delta_health,` to the final `Ok(DiffOutput { ... })` literal.

- [ ] **Step 4: Text + markdown emitters in `diff_output.rs`**

Both insertions rely on the `as_str()` helpers defined on `RiskClass` and
`Outcome` in Task 1 — no extra imports are needed because the structs arrive
through `output.delta_health` and the helpers are inherent methods.

**Anchor (text):** in `emit_text`, find the block that begins with the
existing line

```rust
    if !output.gate_violations.is_empty() {
```

and insert the following immediately AFTER that block's closing `}` (the one
followed by a blank `writeln!(out)?;` inside the block — insert after the
whole `if` block ends):

```rust
    if let Some(dh) = &output.delta_health {
        match dh.ratio {
            Some(ratio) => writeln!(
                out,
                "Delta health: {ratio:.1}/100 — {} ({} added, {} modified, {} removed, {} files skipped)",
                dh.verdict, dh.counts.added, dh.counts.modified, dh.counts.removed, dh.counts.skipped
            )?,
            None => writeln!(out, "Delta health: {} (no analyzable code changed)", dh.verdict)?,
        }
        const MAX_ROWS: usize = 20;
        for f in dh.functions.iter().take(MAX_ROWS) {
            writeln!(
                out,
                "    [{}] {}::{} {} \u{2192} {}{}",
                f.outcome.as_str(),
                f.path,
                f.function,
                f.before.map_or("\u{2205}", |c| c.as_str()),
                f.after.map_or("\u{2205}", |c| c.as_str()),
                if f.in_red_file { " (red file)" } else { "" },
            )?;
        }
        if dh.functions.len() > MAX_ROWS {
            writeln!(out, "    \u{2026} and {} more", dh.functions.len() - MAX_ROWS)?;
        }
        writeln!(out)?;
    }
```

**Anchor (markdown):** in `emit_markdown`, find the existing line

```rust
    if !output.hotspots.rank_entrants.is_empty() {
```

and insert the following immediately BEFORE it (this places the section right
after the gate-violations section, mirroring the text emitter's order):

```rust
    if let Some(dh) = &output.delta_health {
        writeln!(out, "## Delta health")?;
        writeln!(out)?;
        match dh.ratio {
            Some(ratio) => writeln!(out, "**{ratio:.1}/100 — {}**", dh.verdict)?,
            None => writeln!(out, "**{}** — no analyzable code changed", dh.verdict)?,
        }
        writeln!(out)?;
        if !dh.functions.is_empty() {
            writeln!(out, "| Function | Before | After | Outcome | Reasons |")?;
            writeln!(out, "|---|---|---|---|---|")?;
            const MAX_ROWS: usize = 20;
            for f in dh.functions.iter().take(MAX_ROWS) {
                writeln!(
                    out,
                    "| `{}::{}`{} | {} | {} | {} | {} |",
                    f.path,
                    f.function,
                    if f.in_red_file { " \u{1F534}" } else { "" },
                    f.before.map_or("\u{2205}", |c| c.as_str()),
                    f.after.map_or("\u{2205}", |c| c.as_str()),
                    f.outcome.as_str(),
                    codelore_lib::cli_api::output::markdown::escape_md_cell(&f.reasons.join("; ")),
                )?;
            }
            if dh.functions.len() > MAX_ROWS {
                writeln!(out)?;
                writeln!(out, "\u{2026} and {} more", dh.functions.len() - MAX_ROWS)?;
            }
        }
        writeln!(out)?;
    }
```

(`escape_md_cell` is the same helper the gate-violations table above it
already uses — copy its fully-qualified path from that call site.)

- [ ] **Step 5: Build the workspace and run existing diff tests**

Run: `cargo test -p codelore-cli`
Expected: compiles; all existing tests pass (notably `diff_rejects_base_equals_head`).

- [ ] **Step 6: Full-workspace clippy**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: clean. (`evaluate_diff_gate` at 7 parameters is within clippy's
default limit — see the arity note in Task 4.) If clippy reports anything
else, apply Guardrail 4: report rather than suppress.

- [ ] **Step 7: Commit**

```bash
git add crates/codelore-cli/src/diff.rs crates/codelore-cli/src/diff_output.rs crates/codelore-lib/src/quality_gates/mod.rs crates/codelore-lib/src/analyses/delta_health.rs
git commit -m "feat(delta-health): wire section into codelore diff output and [diff] gates"
```

---

### Task 6: End-to-end tests + docs

**Files:**
- Modify: `crates/codelore-cli/tests/cli_test.rs`
- Modify: `docs/advanced-usage.md` (the `codelore diff` + thresholds sections)
- Modify: `CHANGELOG.md` (`[Unreleased]` → `Added`)

**Interfaces:**
- Consumes: the `codelore` binary's `diff` subcommand with `--format json`; `delta_health` JSON shape from Task 3.

- [ ] **Step 1: Write the failing end-to-end tests**

Add to `crates/codelore-cli/tests/cli_test.rs` (reusing the file's `Command::cargo_bin` + git-fixture idioms):

```rust
/// Build a two-commit repo: commit 1 has a trivial function, commit 2
/// adds a large, branchy function. Returns (dir, base_sha, head_sha).
fn delta_health_fixture() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn tiny() -> i32 {\n    1\n}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);

    // A >70-line, CC>10 function: 12 sequential if-blocks + filler lets
    // both the LOC and cyclomatic High thresholds trigger.
    let mut monster = String::from("pub fn monster(x: i32) -> i32 {\n    let mut acc = 0;\n");
    for i in 0..12 {
        monster.push_str(&format!(
            "    if x > {i} {{\n        acc += {i};\n    }}\n"
        ));
    }
    for i in 0..40 {
        monster.push_str(&format!("    acc += {i};\n"));
    }
    monster.push_str("    acc\n}\n");
    std::fs::write(
        repo.join("src/lib.rs"),
        format!("pub fn tiny() -> i32 {{\n    1\n}}\n\n{monster}"),
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "add monster"]);
    let head = git(&["rev-parse", "HEAD"]);
    (dir, base, head)
}

#[test]
fn diff_emits_degrading_delta_health_for_added_monster() {
    let (dir, base, head) = delta_health_fixture();
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let dh = &json["delta_health"];
    assert_eq!(dh["verdict"], "degrading", "delta_health: {dh}");
    assert_eq!(dh["counts"]["added"].as_u64(), Some(1));
    let f = &dh["functions"][0];
    assert_eq!(f["function"], "monster");
    assert_eq!(f["after"], "high");
    assert_eq!(f["outcome"], "bad");
}

#[test]
fn diff_delta_health_gate_fails_the_run() {
    let (dir, base, head) = delta_health_fixture();
    let thresholds = dir.path().join("gates.toml");
    std::fs::write(&thresholds, "[diff]\ndeny_degrading_verdict = true\n").unwrap();
    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            dir.path().to_str().unwrap(),
            "--min-revs",
            "1",
            "--thresholds-file",
            thresholds.to_str().unwrap(),
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "deny_degrading_verdict should fail the run"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["gate_violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["gate"] == "deny_degrading_verdict"),
        "violations: {}",
        json["gate_violations"]
    );
}

#[test]
fn diff_docs_only_change_is_no_code_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn tiny() -> i32 {\n    1\n}\n").unwrap();
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.join("README.md"), "hello world\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "docs"]);
    let head = git(&["rev-parse", "HEAD"]);

    let output = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "diff",
            "--repo",
            repo.to_str().unwrap(),
            "--min-revs",
            "1",
            "--format",
            "json",
            &format!("{base}..{head}"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["delta_health"]["verdict"], "no-code-change");
    assert!(json["delta_health"]["ratio"].is_null());
}
```

`serde_json = "1"` is already a regular dependency of `codelore-cli` (it
serializes the diff base-cache), so the tests can use it without touching
`Cargo.toml`.

- [ ] **Step 2: Run to verify current behavior**

Run: `cargo test -p codelore-cli --test cli_test delta_health`
Expected: all three new tests PASS (the production wiring landed in Task 5;
these tests verify it end-to-end). If any test fails, apply Guardrail 4:
find the root cause in Task 5's wiring — NEVER adjust an assertion to match
observed output.

- [ ] **Step 3: Docs**

In `docs/advanced-usage.md`:
- `codelore diff` section: document the `delta_health` output section — ratio semantics (share of changed-function weight ending low-risk or improved, LOC-weighted, red-file context multiplier), the four verdicts, the fixed thresholds table (LOC 31/71, cyclomatic 6/11, clone ⇒ high), and the stale-base-cache skip behavior.
- Thresholds section: document `[diff] delta_health_min` and `[diff] deny_degrading_verdict` with a TOML example.
- Present-tense contract only — no version numbers, no plan/spec references.

In `CHANGELOG.md` under `[Unreleased]` / `### Added`:

```markdown
- `codelore diff` now emits a `delta_health` section: a change-level health
  verdict (`improving`/`indeterminate`/`degrading`) from a 0–100 ratio of
  changed-function weight ending low-risk, with clone-membership as a
  copy/paste penalty and heavier weighting inside red-band files. Two new
  `[diff]` gates: `delta_health_min` and `deny_degrading_verdict`.
```

- [ ] **Step 4: Full local gate**

Run: `just ci`
Expected: fmt-check, clippy (exact CI invocation), deny, and the full test suite all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-cli/tests/cli_test.rs crates/codelore-cli/Cargo.toml docs/advanced-usage.md CHANGELOG.md
git commit -m "feat(delta-health): end-to-end diff tests, gate docs, changelog"
```

---

## Self-Review Notes (already applied)

- **Spec coverage:** §1 table-diff + kind filter → Task 2; §2 risk model (nesting dropped per amended spec) → Task 1; §3 scoring/context/verdict/no-code-change → Task 3; §4 gates + output + skipped semantics → Tasks 4–5; §5 tests/invariants → Tasks 1–3 units, Task 2 integration, Task 6 end-to-end (docs-only regression included); out-of-scope list honored (no rename tracking, no line weighting, no TOML risk thresholds).
- **Type consistency:** `FunctionMetricRow` needs `PartialEq` (used by the `b == h` untouched check in Task 3) — it is derived in Task 2. `RiskClass` serializes lowercase, matching Task 6's `"high"` assertions. `evaluate_diff_gate` arity matches between Task 4 and Task 5.
- **Known judgment call:** identical-row detection compares `loc` + `cyclomatic` only (the persisted signal); body edits that move neither metric are invisible to delta health by design (spec §1).
