# Refactoring-Targets Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a new first-class `--analysis refactoring-targets` that ranks files by ROI = (code-health deficit × hotspotness) ÷ inspection effort, annotates each with its dominant biomarker, and surfaces a ManualUp baseline for honesty.

**Architecture:** A Rust-orchestration analysis (like `architecture-trend`/`cycle-origins`, NOT a single SQL query). `run_refactoring_targets` calls the existing `run_code_health` (which also materializes the `code_health_biomarkers_v1` temp table as a side effect) and `run_hotspots`, both with the row limit removed, then queries per-file LOC and the dominant biomarker from the connection, joins everything by `path` in Rust, computes `priority`, sorts, and truncates to the user's `--rows`. This reuses Phase-1's metric wholesale.

**Tech Stack:** Rust (workspace), DuckDB, serde. No new crates. No JS (SPA is Plan 3).

## Global Constraints

- `workspace.lints.rust: unsafe_code = "forbid"` — zero `unsafe`; CI rejects additions.
- No `unwrap()`/`expect()` outside tests; library errors via `CodeLoreError::Analysis(...)`; application errors via `anyhow`.
- Local gate MUST match CI exactly: `cargo clippy --workspace --all-targets --all-features -- -D warnings` (`just lint`); full gate `just ci`.
- **macOS build quirk (this dev box):** prefix cargo/just with `MACOSX_DEPLOYMENT_TARGET=15.0`. Tests need `--features test-support`: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support <name>`.
- No version numbers, ticket IDs, task/plan markers, or static test counts in code/comments/non-CHANGELOG docs (present-state only — HARD RULE). Conventional Commits. **Never** add `Co-Authored-By: Claude`.
- **Row-limit discipline (the I-1 lesson from Plan 1):** pass `&opts.with_no_row_limit()` to the inner `run_code_health`/`run_hotspots` so their internal `LIMIT` cannot truncate the input set before ranking; apply the user's `opts.rows_limit` ONLY to the final sorted `refactoring-targets` output.
- **Determinism:** every SQL aggregate/pick must be deterministic — use `ROW_NUMBER() OVER (... ORDER BY <val> DESC, <tiebreak> ASC)` for "pick the top per group", never `arg_max` (arbitrary on ties). Final sort has an explicit `path ASC` tie-break.
- Constants introduced here (`EA_Z_FLOOR`, the priority formula) are **initial, tunable** — tests assert invariants (ordering, ranges, permutation, determinism), never exact magic values.
- **NOT a semantic change to any existing analysis** → no `CACHE_EPOCH` bump. This adds a brand-new analysis; existing outputs are untouched.

## Scope notes (deliberate Phase-1 trims — see design spec §4)

- **Deferred to Phase 2:** a formal effort-aware "beats ManualUp" score (PofB20/IFA/rank-correlation). Phase 1 ships the `manual_up_rank` COLUMN + an `explain` note so the reordering-vs-baseline is *visible in the data*, which is the honesty win; computing a summary "we beat it by X" metric is separate.
- **Deferred to Phase 2:** per-time-window thresholds (design §4 bullet 5). That bullet addresses defect-label verification latency; `refactoring-targets` ranks on health×hotspot, not on fix-inducing labels, so it does not apply here yet.
- **Out of scope (note, don't build):** wiring `refactoring-targets` into the `check` quality-gate. `check` currently evaluates `HotspotRow` against `Gates`; adding this analysis to the gate needs a new `Gates` field + evaluator and is a separable follow-up.

## File Structure

- **Create** `crates/codelore-lib/src/analyses/refactoring_targets.rs` — the analysis (Row + `run_refactoring_targets` + helpers). One responsibility.
- **Modify** `crates/codelore-lib/src/analyses/mod.rs` — `pub mod refactoring_targets;`.
- **Modify** `crates/codelore-lib/src/analysis.rs` — enum variant (`:8`+), `as_str` arm (`:147`+), `registry!(...)` list (`:225`+).
- **Modify** `crates/codelore-cli/src/main.rs` — `AnalysisName::RefactoringTargets => dispatch_refactoring_targets(...)` arm (near `:920`), the `dispatch_refactoring_targets` fn (mirror `dispatch_stale_code` at `:2095`), and the `explain` topics tuple (`:255`).
- **Modify** `crates/codelore-lib/src/output/csv.rs` + `output/markdown.rs` — `write_refactoring_targets_csv` / `_markdown` (manual column lists; json/ndjson/html are serde-auto).
- **Create** `crates/codelore-lib/tests/refactoring_targets_test.rs` — integration tests.
- **Modify** `CHANGELOG.md` — `[Unreleased]` entry.

## Reference: verified signatures this plan consumes

- `run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>>` — side-effect: materializes temp table `code_health_biomarkers_v1(path TEXT, smell TEXT, intensity DOUBLE)` on `db`'s connection. `CodeHealthRow { path: String, cognitive: f64, score: f64, structural_risk: f64, percentile: f64, band: String }`.
- `run_hotspots(db: &FactsDb, opts: &Options) -> Result<Vec<HotspotRow>>`. `HotspotRow { path: String, revisions: u32, cognitive: f64, code_health: f64, hotspot_score: f64, mi: Option<f64>, mi_rank: Option<f64>, ai_pct: Option<f64> }`.
- Per-file LOC: raw `complexity_metrics(path, loc)` — note `complexity_metrics_grouped` does NOT carry `loc`, so read the raw table: `SELECT path, MAX(loc) AS loc FROM complexity_metrics WHERE loc IS NOT NULL GROUP BY path`.
- `query_map_collect(db, sql, params, label, mapper) -> Result<Vec<T>>` (`analyses/query.rs:28`).
- `Options::with_no_row_limit(&self) -> Self` (`options.rs:247`); `opts.rows_limit: Option<u32>`.
- `db.query_row(sql, params, mapper)` is `pub`; `db.conn()` is `pub(crate)` (usable inside the lib).

---

### Task 1: Register the analysis + core priority ranking

**Files:**
- Create: `crates/codelore-lib/src/analyses/refactoring_targets.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs`, `crates/codelore-lib/src/analysis.rs`
- Test: `crates/codelore-lib/tests/refactoring_targets_test.rs`

**Interfaces:**
- Produces: `RefactoringTargetRow { path: String, priority: f64, combined_risk: f64, structural_risk: f64, hotspot_score: f64, revisions: u32, loc: u32, dominant_type: String, band: String, manual_up_rank: u32 }` (Task 1 populates `dominant_type = "none"` and `manual_up_rank = 0`; Task 2 fills them). `run_refactoring_targets(db: &FactsDb, opts: &Options) -> Result<Vec<RefactoringTargetRow>>`.
- Consumes: `run_code_health`, `run_hotspots`, raw `complexity_metrics`.

- [ ] **Step 1: Write the failing test**

Create `crates/codelore-lib/tests/refactoring_targets_test.rs`:

```rust
use codelore_lib::Options;
use codelore_lib::analyses::refactoring_targets::run_refactoring_targets;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

fn opts_for(dir: &std::path::Path) -> Options {
    Options { repo_path: dir.to_path_buf(), min_revs: 1, ..Options::default() }
}

#[test]
fn refactoring_targets_ranks_by_priority_desc() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = opts_for(tiny.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_refactoring_targets(&db, &opts).expect("run");
    assert!(!rows.is_empty(), "tiny_repo should yield >=1 target");
    for r in &rows {
        assert!(r.priority >= 0.0, "priority non-negative: {}", r.priority);
        assert!((0.0..=1.0).contains(&r.structural_risk), "risk in [0,1]: {}", r.structural_risk);
        assert!(r.loc >= 1, "loc floored >=1: {}", r.loc);
    }
    // Sorted by priority DESC.
    for w in rows.windows(2) {
        assert!(w[0].priority >= w[1].priority - 1e-9, "must be sorted by priority DESC");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets_ranks_by_priority_desc`
Expected: FAIL to compile — module `refactoring_targets` does not exist.

- [ ] **Step 3: Create the analysis module**

Create `crates/codelore-lib/src/analyses/refactoring_targets.rs`:

```rust
//! `refactoring-targets` analysis.
//!
//! Ranks files by return-on-investment for refactoring: the intersection of
//! low code health and high development activity, divided by inspection
//! effort. `priority = (structural_risk × hotspot_score) / max(loc, floor)` —
//! an effort-aware ranking so a small, dense, churning, unhealthy file
//! outranks a large one with the same raw risk. Reuses the `code-health`
//! composite (which also materialises the per-file biomarker table) and the
//! `hotspots` activity signal; joins them per file.
//!
//! Research basis: effort-aware defect ranking (risk per unit inspection
//! effort; Popt / PofB20) with an EA-Z-style size floor to avoid tiny-file
//! ranking artifacts.

use std::collections::HashMap;

use crate::analyses::code_health::run_code_health;
use crate::analyses::hotspots::run_hotspots;
use crate::analyses::query::query_map_collect;
use crate::facts::FactsDb;
use crate::{Options, Result};

/// EA-Z-style effort floor: files smaller than this are treated as this many
/// lines when dividing risk by effort, so a 3-line file cannot dominate the
/// ranking on a near-zero denominator. Tunable.
const EA_Z_FLOOR: u32 = 25;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefactoringTargetRow {
    pub path: String,
    /// (structural_risk × hotspot_score) / max(loc, EA_Z_FLOOR). Higher = refactor sooner.
    pub priority: f64,
    /// structural_risk × hotspot_score (health deficit × hotspotness), pre-effort.
    pub combined_risk: f64,
    pub structural_risk: f64,
    pub hotspot_score: f64,
    pub revisions: u32,
    pub loc: u32,
    /// Dominant biomarker for this file (Task 2 fills this; "none" until then).
    pub dominant_type: String,
    pub band: String,
    /// ManualUp baseline rank (Task 2 fills this; 0 until then).
    pub manual_up_rank: u32,
}

/// Run the `refactoring-targets` analysis. Returns rows ranked by `priority`
/// DESC (worst-ROI-debt first), truncated to `opts.rows_limit`.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "refactoring-targets", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_refactoring_targets(db: &FactsDb, opts: &Options) -> Result<Vec<RefactoringTargetRow>> {
    // Row-limit discipline: the inner analyses must see the FULL file set, or
    // ranking would be computed over a truncated input. Truncate only the
    // final sorted output.
    let full = opts.with_no_row_limit();

    // run_code_health ALSO materialises `code_health_biomarkers_v1` on the
    // connection (used in Task 2 for the dominant biomarker).
    let health = run_code_health(db, &full)?;
    let hotspots = run_hotspots(db, &full)?;

    // Per-file LOC (effort). Raw complexity_metrics — the grouped table omits loc.
    let loc_by_path: HashMap<String, u32> = query_map_collect(
        db,
        "SELECT path, MAX(loc) AS loc FROM complexity_metrics WHERE loc IS NOT NULL GROUP BY path",
        [],
        "refactoring-targets:loc",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1).map(|v| u32::try_from(v).unwrap_or(0))?)),
    )?
    .into_iter()
    .collect();

    // Index hotspots by path for the join.
    let hs_by_path: HashMap<&str, &crate::analyses::hotspots::HotspotRow> =
        hotspots.iter().map(|h| (h.path.as_str(), h)).collect();

    let mut rows: Vec<RefactoringTargetRow> = health
        .iter()
        .filter_map(|h| {
            // Only files that are BOTH scored for health AND appear as hotspots.
            let hs = hs_by_path.get(h.path.as_str())?;
            let loc = loc_by_path.get(&h.path).copied().unwrap_or(0).max(1);
            let combined_risk = h.structural_risk * hs.hotspot_score;
            let priority = combined_risk / f64::from(loc.max(EA_Z_FLOOR));
            Some(RefactoringTargetRow {
                path: h.path.clone(),
                priority,
                combined_risk,
                structural_risk: h.structural_risk,
                hotspot_score: hs.hotspot_score,
                revisions: hs.revisions,
                loc,
                dominant_type: "none".to_owned(),
                band: h.band.clone(),
                manual_up_rank: 0,
            })
        })
        .collect();

    // Deterministic sort: priority DESC, then path ASC as a stable tie-break.
    rows.sort_by(|a, b| {
        b.priority
            .partial_cmp(&a.priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }
    Ok(rows)
}
```

- [ ] **Step 4: Declare the module**

In `crates/codelore-lib/src/analyses/mod.rs`, add alphabetically among the `pub mod` list:

```rust
pub mod refactoring_targets;
```

- [ ] **Step 5: Register the enum variant**

Three edits in `crates/codelore-lib/src/analysis.rs`:

Enum (after `CodeHealth,` at `:20`, keep grouping sensible):
```rust
    RefactoringTargets,
```
`as_str` arm (with the other slugs, ~`:158`):
```rust
            Self::RefactoringTargets => "refactoring-targets",
```
`registry!(...)` list (add near `CodeHealth,` ~`:234`):
```rust
            RefactoringTargets,
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets_ranks_by_priority_desc`
Expected: PASS.

- [ ] **Step 7: Lint**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/codelore-lib/src/analyses/refactoring_targets.rs crates/codelore-lib/src/analyses/mod.rs crates/codelore-lib/src/analysis.rs crates/codelore-lib/tests/refactoring_targets_test.rs
git commit -m "feat(refactoring-targets): rank files by health-deficit x hotspotness / effort"
```

---

### Task 2: Dominant-biomarker type + ManualUp baseline rank

**Files:**
- Modify: `crates/codelore-lib/src/analyses/refactoring_targets.rs`
- Test: `crates/codelore-lib/tests/refactoring_targets_test.rs`

**Interfaces:**
- Consumes: temp table `code_health_biomarkers_v1(path, smell, intensity)` (materialized by the `run_code_health` call in Task 1).
- Produces: `dominant_type` ∈ the biomarker smell set ∪ `{"none"}`; `manual_up_rank` a 1-based permutation over the returned rows.

- [ ] **Step 1: Write the failing test**

Append to `crates/codelore-lib/tests/refactoring_targets_test.rs`:

```rust
#[test]
fn refactoring_targets_annotate_type_and_manualup() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = opts_for(tiny.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_refactoring_targets(&db, &opts).expect("run");
    assert!(!rows.is_empty());

    let known = [
        "complex-method", "large-method", "god-class", "dry", "shotgun-surgery", "none",
    ];
    for r in &rows {
        assert!(known.contains(&r.dominant_type.as_str()), "unknown type: {}", r.dominant_type);
        assert!(r.manual_up_rank >= 1, "manual_up_rank is 1-based: {}", r.manual_up_rank);
    }
    // manual_up_rank is a permutation of 1..=n.
    let mut ranks: Vec<u32> = rows.iter().map(|r| r.manual_up_rank).collect();
    ranks.sort_unstable();
    let expected: Vec<u32> = (1..=rows.len() as u32).collect();
    assert_eq!(ranks, expected, "manual_up_rank must be a permutation of 1..=n");

    // ManualUp = ascending size. The smallest-loc row must have rank 1.
    let min_loc_row = rows.iter().min_by_key(|r| r.loc).unwrap();
    assert_eq!(min_loc_row.manual_up_rank, 1, "smallest file is ManualUp rank 1");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets_annotate_type_and_manualup`
Expected: FAIL — `dominant_type` is `"none"` for all, `manual_up_rank` is 0.

- [ ] **Step 3: Query dominant biomarker + fill both annotations**

In `refactoring_targets.rs`, after the `hs_by_path` map and before building `rows`, add the dominant-biomarker query (deterministic pick via `ROW_NUMBER`):

```rust
    // Dominant biomarker per file: highest-intensity smell, ties broken by
    // smell name so the pick is deterministic. Reads the temp table that
    // run_code_health materialised above.
    let dominant_by_path: HashMap<String, String> = query_map_collect(
        db,
        "SELECT path, smell FROM ( \
             SELECT path, smell, \
                    ROW_NUMBER() OVER (PARTITION BY path ORDER BY intensity DESC, smell ASC) AS rn \
             FROM code_health_biomarkers_v1 \
         ) WHERE rn = 1",
        [],
        "refactoring-targets:dominant",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?
    .into_iter()
    .collect();
```

In the `filter_map` closure, replace the `dominant_type: "none".to_owned(),` line with:
```rust
                dominant_type: dominant_by_path
                    .get(&h.path)
                    .cloned()
                    .unwrap_or_else(|| "none".to_owned()),
```

After the deterministic `rows.sort_by(...)` and BEFORE the `if let Some(limit)` truncation, assign the ManualUp baseline over the full ranked set:

```rust
    // ManualUp baseline: rank by ascending size (smallest first). Computed over
    // the full set so the rank is stable regardless of the priority truncation.
    let mut by_size: Vec<usize> = (0..rows.len()).collect();
    by_size.sort_by(|&i, &j| rows[i].loc.cmp(&rows[j].loc).then_with(|| rows[i].path.cmp(&rows[j].path)));
    for (rank, &idx) in by_size.iter().enumerate() {
        rows[idx].manual_up_rank = (rank + 1) as u32;
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets_annotate_type_and_manualup`
Expected: PASS.

- [ ] **Step 5: Determinism regression**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets`
Expected: both tests PASS.

- [ ] **Step 6: Lint + commit**

```bash
MACOSX_DEPLOYMENT_TARGET=15.0 cargo clippy --workspace --all-targets --all-features -- -D warnings
git add crates/codelore-lib/src/analyses/refactoring_targets.rs crates/codelore-lib/tests/refactoring_targets_test.rs
git commit -m "feat(refactoring-targets): annotate dominant biomarker type and ManualUp rank"
```

---

### Task 3: CLI dispatch + CSV/Markdown emitters

**Files:**
- Modify: `crates/codelore-cli/src/main.rs`, `crates/codelore-lib/src/output/csv.rs`, `crates/codelore-lib/src/output/markdown.rs`
- Test: `crates/codelore-lib/tests/refactoring_targets_test.rs`

**Interfaces:**
- Consumes: `run_refactoring_targets`, `RefactoringTargetRow`.
- Produces: `write_refactoring_targets_csv`/`_markdown`; `--analysis refactoring-targets --format csv|json|markdown|ndjson|html`.

- [ ] **Step 1: Write the failing test (CSV shape)**

Append to `crates/codelore-lib/tests/refactoring_targets_test.rs`:

```rust
#[test]
fn refactoring_targets_csv_has_header_and_rows() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = opts_for(tiny.dir.path());
    db.ingest(&repo, &opts).expect("ingest");
    let rows = run_refactoring_targets(&db, &opts).expect("run");

    let mut buf: Vec<u8> = Vec::new();
    codelore_lib::output::csv::write_refactoring_targets_csv(&rows, &mut buf).expect("csv");
    let out = String::from_utf8(buf).expect("utf8");
    let header = out.lines().next().unwrap();
    assert_eq!(header, "entity,priority,combined_risk,structural_risk,hotspot_score,revisions,loc,dominant_type,band,manual_up_rank");
    assert!(out.lines().count() >= 2, "header + >=1 data row");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets_csv_has_header_and_rows`
Expected: FAIL — `write_refactoring_targets_csv` does not exist.

- [ ] **Step 3: Add the CSV emitter**

In `crates/codelore-lib/src/output/csv.rs` (follow the existing `write_code_health_csv` / `quote_if_needed` pattern):

```rust
/// CSV for the `refactoring-targets` analysis.
///
/// # Errors
/// Propagates any write error from `w`.
pub fn write_refactoring_targets_csv<W: std::io::Write>(
    rows: &[crate::analyses::refactoring_targets::RefactoringTargetRow],
    w: &mut W,
) -> std::io::Result<()> {
    writeln!(w, "entity,priority,combined_risk,structural_risk,hotspot_score,revisions,loc,dominant_type,band,manual_up_rank")?;
    for row in rows {
        writeln!(
            w,
            "{},{:.6},{:.6},{:.4},{:.4},{},{},{},{},{}",
            quote_if_needed(&row.path),
            row.priority,
            row.combined_risk,
            row.structural_risk,
            row.hotspot_score,
            row.revisions,
            row.loc,
            quote_if_needed(&row.dominant_type),
            row.band,
            row.manual_up_rank,
        )?;
    }
    Ok(())
}
```

- [ ] **Step 4: Add the Markdown emitter**

In `crates/codelore-lib/src/output/markdown.rs` (follow `write_code_health_markdown` / `escape_md_cell`):

```rust
/// Markdown table for the `refactoring-targets` analysis.
///
/// # Errors
/// Propagates any write error from `w`.
pub fn write_refactoring_targets_markdown<W: std::io::Write>(
    rows: &[crate::analyses::refactoring_targets::RefactoringTargetRow],
    w: &mut W,
) -> std::io::Result<()> {
    writeln!(w, "| Entity | Priority | Combined risk | Structural risk | Hotspot | Revisions | LOC | Type | Band | ManualUp |")?;
    writeln!(w, "|---|---|---|---|---|---|---|---|---|---|")?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {:.6} | {:.6} | {:.4} | {:.4} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.path),
            row.priority,
            row.combined_risk,
            row.structural_risk,
            row.hotspot_score,
            row.revisions,
            row.loc,
            escape_md_cell(&row.dominant_type),
            row.band,
            row.manual_up_rank,
        )?;
    }
    Ok(())
}
```

- [ ] **Step 5: Widen the existing dispatcher to add csv/markdown**

NOTE: Task 1 already added the `AnalysisName::RefactoringTargets` match arm and a MINIMAL `dispatch_refactoring_targets` fn supporting only `json|ndjson|html` (serde-auto) — because adding the enum variant made the CLI's exhaustive `match` non-exhaustive, so the arm had to exist for the workspace to compile. This step WIDENS that existing fn: add the `"csv"` and `"markdown"` arms and update the fallback string. Do NOT create a second fn or a second match arm.

Edit the existing `dispatch_refactoring_targets` so its `match format` reads:
```rust
    match format {
        "csv" => codelore_lib::output::csv::write_refactoring_targets_csv(&rows, out)
            .context("write csv")?,
        "json" => codelore_lib::output::json::write_json(&rows, out).context("write json")?,
        "markdown" => codelore_lib::output::markdown::write_refactoring_targets_markdown(&rows, out)
            .context("write markdown")?,
        "ndjson" => codelore_lib::output::ndjson::write_ndjson(&rows, out).context("write ndjson")?,
        "html" => codelore_lib::output::html::write_html(
            &rows, out, &ctx.title, &ctx.repo_root, &ctx.generated_at,
        )
        .context("write html")?,
        fmt => {
            return Err(unsupported_format(
                "refactoring-targets",
                "csv|json|markdown|ndjson|html",
                fmt,
            ));
        }
    }
```

- [ ] **Step 6: Run the CSV test + build the CLI**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test -p codelore-lib --features test-support refactoring_targets_csv_has_header_and_rows`
Expected: PASS.
Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p codelore-cli`
Expected: builds (dispatch arm wired, no non-exhaustive-match error).

- [ ] **Step 7: Lint + commit**

```bash
MACOSX_DEPLOYMENT_TARGET=15.0 cargo clippy --workspace --all-targets --all-features -- -D warnings
git add crates/codelore-cli/src/main.rs crates/codelore-lib/src/output/csv.rs crates/codelore-lib/src/output/markdown.rs crates/codelore-lib/tests/refactoring_targets_test.rs
git commit -m "feat(refactoring-targets): wire CLI dispatch and csv/markdown emitters"
```

---

### Task 4: explain entry + CHANGELOG + full gate

**Files:**
- Modify: `crates/codelore-cli/src/main.rs` (explain topics `:255`), `CHANGELOG.md`

**Interfaces:** none new.

- [ ] **Step 1: Add the explain topic**

In `crates/codelore-cli/src/main.rs`, add to the `topics` tuple table (`:255`) a new entry (read the surrounding `("code-health", ...)` entry first to match tuple shape exactly):

```rust
        (
            "refactoring-targets",
            "effort-aware refactoring priority: (code-health structural_risk × hotspot_score) ÷ inspection effort, with a ManualUp baseline (Popt / PofB20 framing)",
            "priority = (structural_risk × hotspot_score) / max(loc, 25). Ranked DESC. `manual_up_rank` ranks the same files by ascending LOC (the \"inspect the small dense files first\" baseline the composite is meant to beat); `dominant_type` is the file's highest-intensity biomarker.",
            "See analyses/refactoring_targets.rs.",
        ),
```

- [ ] **Step 2: Verify explain output**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 cargo run -p codelore-cli -- explain refactoring-targets`
Expected: prints the formula/description without error (confirms the topic is registered).

- [ ] **Step 3: CHANGELOG entry**

In `CHANGELOG.md`, under `[Unreleased] > ### Added`, add:

```markdown
- **`refactoring-targets` analysis.** A new `--analysis refactoring-targets` ranks files by return-on-investment for refactoring: `priority = (code-health structural_risk × hotspot_score) / max(loc, 25)` — the intersection of low health and high development activity, divided by inspection effort. Each target is annotated with its `dominant_type` (highest-intensity biomarker) and a `manual_up_rank` (the ascending-size "inspect small dense files first" baseline the composite is designed to beat). Supported formats: csv, json, markdown, ndjson, html.
```

- [ ] **Step 4: Full CI gate**

Run: `MACOSX_DEPLOYMENT_TARGET=15.0 just ci`
Expected: fmt-check, clippy `-D warnings`, deny, and the full test suite (incl. `differential_repo_test`) pass. This plan touches no `Repo`-trait method — differential gate must stay green.
If the local `spa`-feature link step fails on the known macOS-26 deployment-target/linker issue (unrelated to this change), confirm the non-spa gate is green and `MACOSX_DEPLOYMENT_TARGET=15.0 cargo check -p codelore-cli --features spa` exits 0; note it and proceed (GitHub Actions on macOS-15 is unaffected).

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-cli/src/main.rs
git commit -m "feat(refactoring-targets): register explain topic"
git add CHANGELOG.md
git commit -m "docs(changelog): note refactoring-targets analysis"
```

---

## Self-Review

**Spec coverage** (design spec §4):
- "Ranking = risk ÷ effort (risk ÷ LOC)" → Task 1 `priority = combined_risk / max(loc, EA_Z_FLOOR)`. ✓
- "ManualUp baseline surfaced in explain" → Task 2 `manual_up_rank` column + Task 4 explain text. ✓ (formal PofB20 "beats" score deferred — noted in Scope.)
- "EA-Z probability floor" → `EA_Z_FLOOR = 25` via `max(loc, EA_Z_FLOOR)`. ✓
- "dominant biomarker as type" → Task 2 `dominant_type` from `code_health_biomarkers_v1` via deterministic ROW_NUMBER pick. ✓
- "per-time-window thresholds" → deferred to Phase 2 (Scope note). ✓ (explicit, not silent.)
- "flows to all emitters + check" → emitters (csv/json/markdown/ndjson/html) in Task 3; `check`-gate wiring explicitly out of scope (Scope note). ✓/deferred.

**Placeholder scan:** no TBD/"handle edge cases"/"similar to Task N". Every code step shows real code; every command is exact. Constants (`EA_Z_FLOOR=25`, priority formula) are explicit and flagged tunable; tests assert invariants (sorted-DESC, [0,1] risk, permutation, known type set), not magic values. ✓

**Type consistency:** `RefactoringTargetRow` fields defined in Task 1 are used identically in Tasks 2/3 (csv/markdown column order matches the struct). `run_refactoring_targets(db, opts)` signature stable across tasks. `dominant_type`/`manual_up_rank` introduced as placeholder values in Task 1, filled in Task 2 — no signature churn. The `with_no_row_limit` + final-truncate discipline (I-1 lesson) is applied in Task 1 and untouched after. ✓

**Open risk for the executor:** the priority/combined-risk formula and the `EA_Z_FLOOR` are best-effort Phase-1 constants; the SQL sub-queries (LOC, dominant biomarker) are best-effort-not-executed. The *tests are the contract* — if a query errors under DuckDB, fix the query to satisfy the invariant test, never weaken the test. `hotspot_score`'s absolute range is not asserted (it is a positive activity score); `combined_risk`/`priority` are ranked relatively, so tests check ordering + non-negativity, not absolute magnitude.
