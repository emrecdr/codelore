# Repo Health Timeline (piece 2) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `health-trend` analysis that plots architectural health, code health, and a combined score (all 0–100) across ≤12 evenly-spaced historical revisions, surfaced as an overlaid 3-line SPA chart, reusing piece 1's rev-parameterizable `code_health` engine.

**Architecture:** A new `analyses/health_trend.rs` reuses `architecture_trend.rs`'s sampler (≤12 revs). Per sample it (a) builds the in-memory import graph → `arch_health` from `GraphMetrics`, and (b) materializes rev-scoped complexity + imports temp tables via piece 1's `ingest_complexity_at_rev` / `materialize_imports_at_rev`, then calls `run_code_health_scoped` (include_clones=false, history cutoff at the sample date) and averages the per-file scores → repo `code_health`. `combined = 0.5·arch + 0.5·code`. The analysis is registered like any other (CSV/JSON/markdown, explain topic) and the SPA gains a `renderHealthTrend` widget. On-demand, never cached — same cost profile as `architecture-trend` (~2×).

**Tech Stack:** Rust, DuckDB (`FactsDb`, session temp tables), gix (`GixRepo`), tree-sitter complexity, ECharts (vendored) for the SPA widget.

**Spec:** `docs/superpowers/specs/2026-07-06-repo-health-timeline-design.md` — read first.
**Internals reference:** `.superpowers/sdd/piece2-internals.md` — exact current signatures for every API this plan calls. Read it before Task 3.

## Global Constraints

- `workspace.lints.rust: unsafe_code = "forbid"` — zero `unsafe`. No `unwrap()`/`expect()` outside `#[cfg(test)]`; library errors via `CodeLoreError`, app errors via `anyhow`.
- Local gate MUST match CI exactly: `cargo clippy --workspace --all-targets --all-features -- -D warnings` + `cargo fmt --all --check` + tests. Run the full workspace clippy, not a narrower `-p` subset.
- No version numbers / ticket IDs / F-IDs / plan-section markers in code comments or non-CHANGELOG docs. Describe the current contract only.
- No hardcoded/static test counts in comments or README.
- Conventional Commits; **no `Co-Authored-By` trailer**. `cargo fmt --all` before every commit.
- On-demand, **never cached**: no `CACHE_EPOCH` bump, no schema change (temp tables are session-scoped), **no `Repo` trait change** (reuses `read_blob_at` via the at_rev helpers).
- Scores are 0–100, higher = healthier; one shared band: **green ≥ 70, yellow 40–69, red < 40**.
- **Weights are documented constants** (arch_risk 0.5/0.3/0.2; combined 0.5/0.5). Do not make them configurable in v1.
- **Build discipline:** never `cargo clean` / full release rebuild. For "byte-identical / unchanged" claims use the FIXTURE tests, never a self-repo `analyze --repo .` stash-diff (code-health/coupling analyze the worktree — a source edit confounds the diff). A background `graphify` rebuild hook may briefly hold the target lock; if a build stalls, wait, don't interrupt.

## Design decisions locked here (spec left these open)

- **Repo `code_health` per sample = arithmetic mean of per-file `CodeHealthRow.score`**, or `100.0` when no files scored. Computed against `opts.with_no_row_limit()` so `--rows N` truncation cannot bias the mean. (Mean = interpretable "average file health," symmetric with the single `arch_health`; documented + retunable. Median / LOC-weighting were considered; mean chosen for v1 simplicity.)
- **`arch_health` = `100·(1 − min(1.0, 0.5·propagation_cost + 0.3·cyclic_nodes/n + 0.2·largest_cycle/n))`**; `n == 0` ⇒ `100.0`.
- **SPA split toggle is a vanilla JS button** (overlay ⇄ split), matching `renderArchTrend`'s vanilla style — NOT Alpine. Same data, no recompute. (Deviation from the spec's "Alpine-backed" wording; simpler, no new coupling.)
- **Reuse, don't duplicate the sampler:** `evenly_spaced_indices` + `live_paths_at` become `pub(crate)`, and the commits-query+sampling is extracted into `pub(crate) sampled_commits(db)`, used by BOTH `run_architecture_trend` and `run_health_trend`.

## File Structure

- Create: `crates/codelore-lib/src/analyses/health_trend.rs` — `HealthTrendRow`, `health_band`, `arch_health`, `repo_code_health`, `run_health_trend`.
- Modify: `crates/codelore-lib/src/analyses/architecture_trend.rs` — expose `evenly_spaced_indices`/`live_paths_at` as `pub(crate)`; add `pub(crate) sampled_commits`; refactor `run_architecture_trend` to use it.
- Modify: `crates/codelore-lib/src/analyses/mod.rs` — `pub mod health_trend;`.
- Modify: `crates/codelore-lib/src/analysis.rs` — `HealthTrend` enum variant + `as_str` + `registry!`.
- Modify: `crates/codelore-cli/src/main.rs` — `dispatch_health_trend` + match arm + explain topic + `build_spa_dashboard` field wiring.
- Modify: `crates/codelore-lib/src/output/csv.rs` — `write_health_trend_csv`.
- Modify: `crates/codelore-lib/src/output/markdown.rs` (wherever `write_architecture_trend_markdown` lives — grep) — `write_health_trend_markdown`.
- Modify: `crates/codelore-lib/src/output/spa.rs` — `SpaDashboard.health_trend` field.
- Modify: `crates/codelore-lib/src/output/spa/widgets.js` — `renderHealthTrend` + WIDGETS entry.
- Tests: `crates/codelore-lib/tests/health_trend_test.rs` (unit+integration), extend `crates/codelore-cli/tests/cli_test.rs` (dispatch/explain), extend the SPA integration test.

## Execution Guardrails (read before every task)

1. Run commands from repo root. Touch ONLY the files in the task's **Files** block.
2. Copy code blocks verbatim. If an insertion anchor isn't found verbatim, STOP and report (the file may have drifted).
3. If a command's output ≠ the step's **Expected**, STOP; re-read; if still off, report the exact command + output + step number. Never loosen an assertion, add `#[allow]`/`#[ignore]`, or mask a failure.
4. Read `.superpowers/sdd/piece2-internals.md` for any signature you're unsure of — do not invent APIs.

---

### Task 1: Scoring core — `HealthTrendRow`, `health_band`, `arch_health`, `repo_code_health`

**Files:**
- Create: `crates/codelore-lib/src/analyses/health_trend.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs`
- Test: inline `#[cfg(test)]` in `health_trend.rs`

**Interfaces:**
- Consumes: `crate::analyses::import_graph::GraphMetrics` (fields: `n: usize`, `propagation_cost: f64`, `cyclic_nodes: u32`, `largest_cycle: u32`); `crate::analyses::code_health::CodeHealthRow` (field `score: f64`).
- Produces (used by Task 3 + emitters):
  - `pub struct HealthTrendRow { pub date: String, pub rev: String, pub files: u32, pub arch_health: f64, pub code_health: f64, pub combined_health: f64, pub arch_band: String, pub code_band: String, pub combined_band: String }` (derives `Debug, Clone, serde::Serialize, serde::Deserialize`)
  - `pub fn health_band(score: f64) -> &'static str`
  - `pub fn arch_health(m: &GraphMetrics) -> f64`
  - `pub(crate) fn repo_code_health(rows: &[CodeHealthRow]) -> f64`
  - `pub(crate) fn combined_health(arch: f64, code: f64) -> f64`

- [ ] **Step 1: Create the module with the struct + pure scoring fns.** Write `crates/codelore-lib/src/analyses/health_trend.rs`:

```rust
//! Repo Health Timeline: architectural, code, and combined health (each 0–100,
//! higher = healthier) across evenly-spaced historical revisions. Reuses the
//! `architecture_trend` sampler for the rev set and piece-1's rev-parameterizable
//! `code_health` engine for the per-rev code score. On-demand, never cached.

use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::import_graph::GraphMetrics;

/// One sampled revision's three health scores + bands. Emitted oldest-first.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthTrendRow {
    /// `YYYY-MM-DD` prefix of the commit timestamp.
    pub date: String,
    /// First 12 chars of the commit SHA.
    pub rev: String,
    /// Nodes in the resolved import graph at this rev.
    pub files: u32,
    /// Architectural health 0..=100 (structural only — no complexity).
    pub arch_health: f64,
    /// Code health 0..=100 (mean of per-file code-health scores, DRY excluded).
    pub code_health: f64,
    /// Combined health 0..=100 = mean of arch + code.
    pub combined_health: f64,
    pub arch_band: String,
    pub code_band: String,
    pub combined_band: String,
}

/// Shared band for all three scores: green ≥ 70, yellow ≥ 40, else red.
#[must_use]
pub fn health_band(score: f64) -> &'static str {
    if score >= 70.0 {
        "green"
    } else if score >= 40.0 {
        "yellow"
    } else {
        "red"
    }
}

/// Architectural health from the per-rev import-graph metrics. Purely
/// structural: propagation cost (dominant) plus the fraction of the codebase
/// tangled in cycles and the span of the single largest tangle. An empty graph
/// (`n == 0`) is trivially healthy (nothing to be unhealthy about).
#[must_use]
pub fn arch_health(m: &GraphMetrics) -> f64 {
    if m.n == 0 {
        return 100.0;
    }
    let n = m.n as f64;
    let arch_risk =
        0.5 * m.propagation_cost + 0.3 * (f64::from(m.cyclic_nodes) / n) + 0.2 * (f64::from(m.largest_cycle) / n);
    100.0 * (1.0 - arch_risk.min(1.0))
}

/// Repo-level code health for one rev: the arithmetic mean of the per-file
/// code-health scores (all files, un-truncated). No files scored ⇒ 100.
#[must_use]
pub(crate) fn repo_code_health(rows: &[CodeHealthRow]) -> f64 {
    if rows.is_empty() {
        return 100.0;
    }
    let sum: f64 = rows.iter().map(|r| r.score).sum();
    sum / rows.len() as f64
}

/// Combined health: equal blend of systemic (architecture) and local (code).
#[must_use]
pub(crate) fn combined_health(arch: f64, code: f64) -> f64 {
    0.5 * arch + 0.5 * code
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyses::import_graph::GraphMetrics;

    fn metrics(n: usize, pc: f64, cyclic: u32, largest: u32) -> GraphMetrics {
        GraphMetrics {
            n,
            ccd: 0.0,
            propagation_cost: pc,
            cycle_count: 0,
            largest_cycle: largest,
            cyclic_nodes: cyclic,
        }
    }

    #[test]
    fn band_boundaries() {
        assert_eq!(health_band(69.9), "yellow");
        assert_eq!(health_band(70.0), "green");
        assert_eq!(health_band(40.0), "yellow");
        assert_eq!(health_band(39.9), "red");
        assert_eq!(health_band(100.0), "green");
        assert_eq!(health_band(0.0), "red");
    }

    #[test]
    fn arch_health_empty_graph_is_perfect() {
        assert!((arch_health(&metrics(0, 0.0, 0, 0)) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn arch_health_acyclic_is_100_minus_half_pc() {
        // n=10, pc=0.2, no cycles → risk = 0.5*0.2 = 0.10 → health = 90.
        let h = arch_health(&metrics(10, 0.2, 0, 0));
        assert!((h - 90.0).abs() < 1e-9, "got {h}");
    }

    #[test]
    fn arch_health_fully_tangled_is_low() {
        // n=10, pc=1.0, all 10 cyclic, largest 10 → risk = 0.5 + 0.3 + 0.2 = 1.0 → health 0.
        let h = arch_health(&metrics(10, 1.0, 10, 10));
        assert!(h.abs() < 1e-9, "got {h}");
    }

    #[test]
    fn arch_risk_caps_at_one() {
        // Over-unity raw risk must clamp so health never goes negative.
        let h = arch_health(&metrics(2, 1.0, 2, 2));
        assert!(h >= 0.0, "health must not be negative, got {h}");
    }

    #[test]
    fn combined_is_mean_of_arch_and_code() {
        assert!((combined_health(80.0, 60.0) - 70.0).abs() < 1e-9);
    }

    #[test]
    fn repo_code_health_empty_is_100() {
        assert!((repo_code_health(&[]) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn repo_code_health_averages_scores() {
        let rows = vec![
            CodeHealthRow { path: "a".into(), cognitive: 0.0, score: 90.0, structural_risk: 0.0, percentile: 0.0, band: "green".into() },
            CodeHealthRow { path: "b".into(), cognitive: 0.0, score: 50.0, structural_risk: 0.0, percentile: 0.0, band: "yellow".into() },
        ];
        assert!((repo_code_health(&rows) - 70.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Register the module.** In `crates/codelore-lib/src/analyses/mod.rs`, add `pub mod health_trend;` in alphabetical order (after `god_classes` / before `hotspots` — match the existing ordering).

- [ ] **Step 3: Run the unit tests.** Run: `cargo test -p codelore-lib --features test-support --lib health_trend::tests`
  Expected: 8 passed. (If `GraphMetrics` construction fails to compile, re-check its exact fields in `.superpowers/sdd/piece2-internals.md` §2 and the live `import_graph.rs` — do NOT guess field names.)

- [ ] **Step 4: Commit.**
```bash
git add crates/codelore-lib/src/analyses/health_trend.rs crates/codelore-lib/src/analyses/mod.rs
git commit -m "feat(health-trend): scoring core — row struct, bands, arch_health, aggregation"
```

---

### Task 2: Expose the shared sampler (reuse, not duplicate)

**Files:**
- Modify: `crates/codelore-lib/src/analyses/architecture_trend.rs`
- Test: `crates/codelore-lib/tests/architecture_trend_test.rs` (or wherever arch-trend is tested — grep `run_architecture_trend` in `crates/codelore-lib/tests/`)

**Interfaces:**
- Produces (used by Task 3):
  - `pub(crate) fn sampled_commits(db: &FactsDb) -> Result<Vec<(String, String)>>` — up to `SAMPLE_POINTS` evenly-spaced `(rev, timestamp)` pairs, oldest→newest, newest always included; empty when no commits.
  - `pub(crate) fn live_paths_at(db: &FactsDb, ts: &str) -> Result<Vec<String>>` (visibility widened from private).
  - `pub(crate) fn evenly_spaced_indices(len: usize, k: usize) -> Vec<usize>` (visibility widened; already used internally).
- `run_architecture_trend` is refactored to call `sampled_commits` — its output MUST be unchanged (verified by the existing arch-trend test).

- [ ] **Step 1: Widen visibility + add `sampled_commits`.** In `architecture_trend.rs`:
  - Change `fn evenly_spaced_indices(` → `pub(crate) fn evenly_spaced_indices(`.
  - Change `fn live_paths_at(` → `pub(crate) fn live_paths_at(`.
  - Add this new helper (place it just above `run_architecture_trend`):

```rust
/// The ≤`SAMPLE_POINTS` evenly-spaced `(rev, timestamp)` commit samples,
/// oldest→newest (newest always included). Shared by `architecture-trend` and
/// `health-trend` so the rev set is identical between the two views.
pub(crate) fn sampled_commits(db: &FactsDb) -> Result<Vec<(String, String)>> {
    let commits: Vec<(String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT rev, CAST(date AS TEXT) FROM commits ORDER BY date ASC, rowid ASC",
        [],
        "sampled-commits",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let picks = evenly_spaced_indices(commits.len(), SAMPLE_POINTS);
    Ok(picks.into_iter().map(|i| commits[i].clone()).collect())
}
```

- [ ] **Step 2: Refactor `run_architecture_trend` to use it.** Replace its steps 1–2 (the inline commits query + `evenly_spaced_indices` call + the `if commits.is_empty()` guard) so the loop iterates `sampled_commits`:

```rust
#[tracing::instrument(name = "architecture-trend", skip_all)]
pub fn run_architecture_trend<R: Repo>(
    db: &FactsDb,
    repo: &R,
    _opts: &Options,
) -> Result<Vec<ArchitectureTrendRow>> {
    let samples = sampled_commits(db)?;
    let mut rows = Vec::with_capacity(samples.len());
    for (rev, ts) in &samples {
        let graph = import_graph_at_rev(db, repo, rev, ts)?;
        let m = graph_metrics(&graph);
        rows.push(ArchitectureTrendRow {
            date: ts.get(..10).unwrap_or(ts).to_string(),
            rev: rev.chars().take(12).collect(),
            files: u32::try_from(m.n).unwrap_or(u32::MAX),
            propagation_cost: m.propagation_cost,
            cycle_count: m.cycle_count,
            largest_cycle: m.largest_cycle,
        });
    }
    Ok(rows)
}
```
(This preserves identical behavior — same commits, same order, same rows. If the current body differs from the internals-reference quote, keep its exact row-construction; only swap the commits-query+sampling for `sampled_commits`.)

- [ ] **Step 3: Confirm arch-trend behavior unchanged.** Run the existing arch-trend test: `cargo test -p codelore-lib --features test-support --test architecture_trend_test` (adjust the test name to the actual file from your grep).
  Expected: all pass — the refactor is behavior-preserving. If no dedicated test file exists, run `cargo test -p codelore-lib --features test-support architecture_trend`.

- [ ] **Step 4: Commit.**
```bash
git add crates/codelore-lib/src/analyses/architecture_trend.rs
git commit -m "refactor(architecture-trend): extract sampled_commits + expose sampler for reuse"
```

---

### Task 3: `run_health_trend` — the per-sample engine

**Files:**
- Modify: `crates/codelore-lib/src/analyses/health_trend.rs`
- Test: `crates/codelore-lib/tests/health_trend_test.rs`

**Interfaces:**
- Consumes: `sampled_commits`, `live_paths_at`, `import_graph_at_rev` (Task 2 + existing); `graph_metrics` (`import_graph`); `run_code_health_scoped`, `HealthScanCtx` (code_health); `ingest_complexity_at_rev`, `materialize_imports_at_rev` (facts::ingest::at_rev); `Options::with_no_row_limit`.
- Produces: `pub fn run_health_trend<R: Repo>(db: &FactsDb, repo: &R, opts: &Options) -> Result<Vec<HealthTrendRow>>`.

- [ ] **Step 1: Read the internals reference** `.superpowers/sdd/piece2-internals.md` §1, §5, and Gotchas — confirm every signature below matches the live source before writing.

- [ ] **Step 2: Add `run_health_trend`.** Append to `health_trend.rs` (add the imports to the top `use` block):

```rust
use crate::analyses::architecture_trend::{import_graph_at_rev, live_paths_at, sampled_commits};
use crate::analyses::code_health::{run_code_health_scoped, HealthScanCtx};
use crate::analyses::import_graph::graph_metrics;
use crate::facts::ingest::at_rev::{ingest_complexity_at_rev, materialize_imports_at_rev};
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::{Options, Result};

/// Session-scoped temp-table names the rev-scoped `HealthScanCtx` points at.
/// `CREATE OR REPLACE` inside the helpers means reusing them across samples is
/// safe — each iteration replaces the prior rev's contents.
const CM_AT_REV: &str = "cm_at_rev";
const IMPORTS_AT_REV: &str = "imports_at_rev";

/// Compute the three health scores across ≤12 evenly-spaced historical revs.
///
/// Per sample: build the in-memory import graph → `arch_health`; materialize
/// rev-scoped complexity + imports temp tables and run the (DRY-excluded,
/// date-cut) code-health engine → mean per-file score = `code_health`;
/// `combined = mean(arch, code)`. Every sample — including the newest — is
/// computed the same reduced way, so the series is internally consistent (it
/// may sit slightly above the standalone HEAD `code-health` number, which
/// includes DRY + full external fan-out).
///
/// # Errors
/// Returns [`CodeLoreError::Analysis`] on any query / ingest failure.
#[tracing::instrument(name = "health-trend", skip_all)]
pub fn run_health_trend<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
) -> Result<Vec<HealthTrendRow>> {
    let samples = sampled_commits(db)?;
    // ALL files must feed the code-health mean — never the user's `--rows` cut.
    let scan_opts = opts.with_no_row_limit();
    let mut rows = Vec::with_capacity(samples.len());
    for (rev, ts) in &samples {
        // Architectural half — purely structural, from the in-memory graph.
        let graph = import_graph_at_rev(db, repo, rev, ts)?;
        let m = graph_metrics(&graph);
        let files = u32::try_from(m.n).unwrap_or(u32::MAX);
        let arch = arch_health(&m);

        // Code half — rev-scoped sources into piece-1's scoped engine.
        let live = live_paths_at(db, ts)?;
        ingest_complexity_at_rev(db, repo, rev, &live, CM_AT_REV)?;
        materialize_imports_at_rev(db, &graph, IMPORTS_AT_REV)?;
        let cx = HealthScanCtx {
            complexity_source: CM_AT_REV.to_string(),
            imports_source: IMPORTS_AT_REV.to_string(),
            history_cutoff: Some(ts.clone()),
            include_clones: false,
        };
        let code_rows = run_code_health_scoped(db, &scan_opts, &cx)?;
        let code = repo_code_health(&code_rows);

        let combined = combined_health(arch, code);
        rows.push(HealthTrendRow {
            date: ts.get(..10).unwrap_or(ts).to_string(),
            rev: rev.chars().take(12).collect(),
            files,
            arch_health: arch,
            code_health: code,
            combined_health: combined,
            arch_band: health_band(arch).to_string(),
            code_band: health_band(code).to_string(),
            combined_band: health_band(combined).to_string(),
        });
    }
    Ok(rows)
}
```
(If `Options::with_no_row_limit` is spelled differently, grep `fn with_no_row_limit` in `code_health.rs`/`options.rs` and use the exact name. If `run_code_health_scoped` errors because a rev has zero complexity rows, that surfaces as a `CodeLoreError` — acceptable; the integration test uses a fixture where every sample has Rust files.)

- [ ] **Step 3: Write the integration test.** Create `crates/codelore-lib/tests/health_trend_test.rs`:

```rust
use codelore_lib::analyses::health_trend::{health_band, run_health_trend, HealthTrendRow};

fn all_scores_in_range(rows: &[HealthTrendRow]) {
    for r in rows {
        for (name, v) in [
            ("arch", r.arch_health),
            ("code", r.code_health),
            ("combined", r.combined_health),
        ] {
            assert!(
                (0.0..=100.0).contains(&v),
                "{name} health out of range for {}: {v}",
                r.rev
            );
        }
        assert_eq!(r.arch_band, health_band(r.arch_health));
        assert_eq!(r.code_band, health_band(r.code_health));
        assert_eq!(r.combined_band, health_band(r.combined_health));
        // combined is exactly the mean of the two.
        assert!((r.combined_health - 0.5 * (r.arch_health + r.code_health)).abs() < 1e-9);
    }
}

#[test]
fn health_trend_produces_a_row_per_sample_oldest_first() {
    let fx = codelore_lib::test_support::biomarker_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(fx.dir.path().to_path_buf());
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_health_trend(&db, &repo, &opts).expect("health-trend");
    assert!(!rows.is_empty(), "fixture with >=2 commits must yield samples");
    all_scores_in_range(&rows);

    // Oldest-first by date (non-decreasing).
    for w in rows.windows(2) {
        assert!(w[0].date <= w[1].date, "rows must be oldest-first");
    }
}
```

- [ ] **Step 4: Run it.** Run: `cargo test -p codelore-lib --features test-support --test health_trend_test`
  Expected: PASS. (First compile pulls in the at_rev + code_health scoped path; may take a minute — wait for it.)

- [ ] **Step 5: Commit.**
```bash
git add crates/codelore-lib/src/analyses/health_trend.rs crates/codelore-lib/tests/health_trend_test.rs
git commit -m "feat(health-trend): per-sample arch+code+combined engine over sampled history"
```

---

### Task 4: Register the `health-trend` analysis (enum, dispatch, emitters, explain)

**Files:**
- Modify: `crates/codelore-lib/src/analysis.rs`
- Modify: `crates/codelore-cli/src/main.rs`
- Modify: `crates/codelore-lib/src/output/csv.rs`
- Modify: `crates/codelore-lib/src/output/markdown.rs` (or the file holding `write_architecture_trend_markdown` — grep)
- Test: `crates/codelore-cli/tests/cli_test.rs`

**Interfaces:**
- Consumes: `run_health_trend`, `HealthTrendRow` (Task 3).
- Produces: `AnalysisName::HealthTrend`; `write_health_trend_csv`; `write_health_trend_markdown`; a `dispatch_health_trend` in `main.rs`.

- [ ] **Step 1: Enum + registry.** In `crates/codelore-lib/src/analysis.rs`:
  - Add the variant next to `ArchitectureTrend`: `HealthTrend,`
  - Add the `as_str` arm: `Self::HealthTrend => "health-trend",`
  - Add `HealthTrend,` to the `registry!(...)` list (alphabetical-ish, near `ArchitectureTrend`). Omitting it is a compile error (the macro's exhaustiveness guard).

- [ ] **Step 2: CSV emitter.** In `crates/codelore-lib/src/output/csv.rs`, beside `write_architecture_trend_csv`, add:

```rust
pub fn write_health_trend_csv<W: Write>(
    rows: &[crate::analyses::health_trend::HealthTrendRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "date,rev,files,arch-health,code-health,combined-health,arch-band,code-band,combined-band"
    )?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.2},{:.2},{:.2},{},{},{}",
            quote_if_needed(&row.date),
            quote_if_needed(&row.rev),
            row.files,
            row.arch_health,
            row.code_health,
            row.combined_health,
            row.arch_band,
            row.code_band,
            row.combined_band,
        )?;
    }
    Ok(())
}
```
(Match the `use`/`quote_if_needed`/`Result` in scope at `write_architecture_trend_csv` — do not add new imports if they're already module-level.)

- [ ] **Step 3: Markdown emitter.** Grep `fn write_architecture_trend_markdown` to find the file; beside it add `write_health_trend_markdown` mirroring its structure:

```rust
pub fn write_health_trend_markdown<W: Write>(
    rows: &[crate::analyses::health_trend::HealthTrendRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "| Date | Rev | Files | Arch | Code | Combined |")?;
    writeln!(w, "|---|---|---:|---:|---:|---:|")?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {:.1} ({}) | {:.1} ({}) | {:.1} ({}) |",
            row.date,
            row.rev,
            row.files,
            row.arch_health,
            row.arch_band,
            row.code_health,
            row.code_band,
            row.combined_health,
            row.combined_band,
        )?;
    }
    Ok(())
}
```
(If `write_architecture_trend_markdown` uses a different table style/helper, match ITS style instead — the point is one consistent markdown pattern, not this exact layout.)

- [ ] **Step 4: CLI dispatch.** In `crates/codelore-cli/src/main.rs`:
  - Add the match arm beside `ArchitectureTrend`:
```rust
AnalysisName::HealthTrend => {
    dispatch_health_trend(&db, &repo, &opts, format, &ctx, &mut out)?;
}
```
  - Add `dispatch_health_trend` beside `dispatch_architecture_trend`, mirroring it (note: `repo: &GixRepo`, the same special-case):
```rust
fn dispatch_health_trend(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)?;
            codelore_lib::cli_api::output::csv::write_health_trend_csv(&rows, out)?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)?;
            write_json(&rows, out)?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)?;
            codelore_lib::cli_api::output::markdown::write_health_trend_markdown(&rows, out)?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("health-trend", "csv|json|markdown", fmt)),
    }
    Ok(())
}
```
(Match `dispatch_architecture_trend`'s EXACT call conventions — the `cli_api` path prefix, the `write_json` helper name, `html_not_wired`/`unsupported_format` — copy them from that fn. If arch-trend qualifies paths differently, mirror that.)

- [ ] **Step 5: Explain topic.** In `run_explain_cmd` (main.rs), beside the `architecture-trend` tuple, add:
```rust
(
    "health-trend",
    "Repo health (architectural + code + combined) over the commit sequence",
    "Computes three 0-100 scores at up to 12 historical revs (evenly spaced): \
     architectural health (structural — propagation cost + dependency tangle), \
     code health (the rev-parameterized code-health engine with duplication \
     excluded, averaged over files), and their equal blend. Bands: green >= 70, \
     yellow 40-69, red < 40. Rebuilds the import graph + re-scans complexity per \
     sample, so it is heavier than SQL-only analyses; computed on demand, never \
     cached.",
    "See analyses/health_trend.rs.",
),
```

- [ ] **Step 6: End-to-end CLI test.** In `crates/codelore-cli/tests/cli_test.rs`, add a test that runs the analysis (mirror an existing `--analysis architecture-trend` CLI test — grep `architecture-trend` in that file for the harness):
```rust
#[test]
fn health_trend_csv_has_header_and_rows() {
    // Mirror the existing architecture-trend CLI test harness: build a fixture
    // repo, run `analyze --analysis health-trend --format csv`, assert the
    // header line + at least one data row.
    // (Copy the exact command-runner + fixture setup from the arch-trend test.)
}
```
Fill the body by copying the arch-trend CLI test verbatim and swapping `architecture-trend`→`health-trend` and the expected header to `date,rev,files,arch-health,code-health,combined-health,arch-band,code-band,combined-band`.

- [ ] **Step 7: Run the anti-drift + new tests.** Run:
  `cargo test -p codelore-cli --test cli_test explain_covers_every_registered_analysis` then
  `cargo test -p codelore-cli --test cli_test health_trend`
  Expected: both PASS. The explain anti-drift test passing proves `health-trend` has its explain entry (if it fails, either the explain tuple is missing or you must add `"health-trend"` to `EXPLAIN_UNCOVERED` — prefer the explain entry).

- [ ] **Step 8: Commit.**
```bash
git add crates/codelore-lib/src/analysis.rs crates/codelore-cli/src/main.rs crates/codelore-lib/src/output/csv.rs crates/codelore-lib/src/output/markdown.rs crates/codelore-cli/tests/cli_test.rs
git commit -m "feat(health-trend): register analysis — dispatch, csv/json/markdown, explain"
```

---

### Task 5: SPA dashboard field + wiring

**Files:**
- Modify: `crates/codelore-lib/src/output/spa.rs`
- Modify: `crates/codelore-cli/src/main.rs` (`build_spa_dashboard`)
- Test: the SPA integration test (grep `SpaDashboard` / `spa_integration` in `crates/codelore-cli/tests/`)

**Interfaces:**
- Consumes: `run_health_trend`, `HealthTrendRow`.
- Produces: `SpaDashboard.health_trend: Vec<HealthTrendRow>`.

- [ ] **Step 1: Add the field.** In `crates/codelore-lib/src/output/spa.rs`, beside `architecture_trend`:
```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub health_trend: Vec<crate::analyses::health_trend::HealthTrendRow>,
```

- [ ] **Step 2: Populate it in `build_spa_dashboard`.** In `main.rs`, beside the `architecture_trend` block, add the analogous block (open its own repo, degrade to empty on failure):
```rust
let health_trend = codelore_lib::cli_api::repo::GixRepo::open(repo_path)
    .map_err(anyhow::Error::from)
    .and_then(|repo| {
        codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, &repo, opts)
            .map_err(anyhow::Error::from)
    })
    .unwrap_or_else(|e| {
        tracing::warn!("dashboard: health-trend failed; skipping: {e}");
        Vec::new()
    });
```
  And add `health_trend,` to the `SpaDashboard { ... }` struct literal beside `architecture_trend,`.

- [ ] **Step 3: Extend the SPA integration test.** In the SPA integration test, add `health_trend` to the shape assertions (mirror how `architecture_trend` is asserted present in the embedded JSON). If the test asserts a specific set of keys, add `"health_trend"` (note: `skip_serializing_if = "Vec::is_empty"` means the key is ABSENT when empty — assert on a fixture that produces rows, matching how arch-trend is asserted).

- [ ] **Step 4: Run.** Run: `cargo test -p codelore-cli --features test-support spa` (adjust to the actual SPA test name).
  Expected: PASS.

- [ ] **Step 5: Commit.**
```bash
git add crates/codelore-lib/src/output/spa.rs crates/codelore-cli/src/main.rs crates/codelore-cli/tests/
git commit -m "feat(health-trend): wire health_trend into the SPA dashboard payload"
```

---

### Task 6: `renderHealthTrend` SPA widget (overlaid 3-line + split toggle)

**Files:**
- Modify: `crates/codelore-lib/src/output/spa/widgets.js`
- (Possibly) Modify: the SPA HTML shell for the widget body element — grep `widget-arch-trend-body` to find where widget containers are declared and mirror it for `widget-health-trend-body`.
- Test: the SPA browser/shape test (grep `renderArchTrend` usage in the browser test, if any) or a manual render check.

**Interfaces:**
- Consumes: `data.health_trend` (array of `HealthTrendRow` — fields `date, rev, files, arch_health, code_health, combined_health, arch_band, code_band, combined_band`).

- [ ] **Step 1: Find the widget-container + WIDGETS conventions.** Grep in `widgets.js` (and the HTML shell it's embedded in): `widget-arch-trend-body`, the `WIDGETS` array, and `mountEcharts`. The new widget mirrors these: a container `id="widget-health-trend-body"` and a WIDGETS entry. Confirm how a widget body element gets into the DOM (static HTML in the shell vs. generated) and replicate for `health-trend`.

- [ ] **Step 2: Add the WIDGETS entry.** Beside the `arch-trend` entry:
```js
{ name: 'health-trend', render: () => renderHealthTrend(data.health_trend || []) },
```

- [ ] **Step 3: Add `renderHealthTrend`.** Beside `renderArchTrend` in `widgets.js`. Default overlaid 3-line chart (Combined bold; Architectural + Code lighter) on a 0–100 axis with faint red/yellow/green band background + a vanilla "Overlay / Split" toggle button that re-renders as three stacked small-multiples. Use the same `mountEcharts` helper and theme color accessors the file already uses (grep `infoColor`/`errColor`/`okColor` or the theme palette used in `renderArchTrend` — reuse those exact accessors; do not invent color variables):

```js
function healthTrendBands() {
  // Faint green/yellow/red horizontal bands (0–40 red, 40–70 yellow, 70–100 green).
  return {
    silent: true,
    data: [
      [{ yAxis: 0 }, { yAxis: 40 }],
      [{ yAxis: 40 }, { yAxis: 70 }],
      [{ yAxis: 70 }, { yAxis: 100 }],
    ],
    itemStyle: { opacity: 0.06 },
  };
}

function renderHealthTrend(rows, mode) {
  const container = document.getElementById('widget-health-trend-body');
  if (!container) return;
  if (!rows.length) {
    container.innerHTML =
      '<div class="empty">Not enough history for a health timeline — need at least 2 commits.</div>';
    return;
  }
  const view = mode || 'overlay';
  const dates = rows.map(function (r) { return r.date; });
  const arch = rows.map(function (r) { return Number((r.arch_health || 0).toFixed(2)); });
  const code = rows.map(function (r) { return Number((r.code_health || 0).toFixed(2)); });
  const combined = rows.map(function (r) { return Number((r.combined_health || 0).toFixed(2)); });

  // Toggle button + chart host.
  container.innerHTML =
    '<div class="ht-toolbar"><button id="ht-toggle" class="ht-btn">' +
    (view === 'overlay' ? 'Split view' : 'Overlay view') +
    '</button></div><div id="ht-charts"></div>';
  const toggle = document.getElementById('ht-toggle');
  if (toggle) {
    toggle.onclick = function () {
      renderHealthTrend(rows, view === 'overlay' ? 'split' : 'overlay');
    };
  }
  const host = document.getElementById('ht-charts');

  const okColor = cssVar('--ok') || '#3fb950';
  const warnColor = cssVar('--warn') || '#d29922';
  const errColor = cssVar('--err') || '#f85149';
  const combinedColor = cssVar('--fg') || '#e6edf3';

  const baseAxis = {
    tooltip: { trigger: 'axis' },
    grid: { left: 44, right: 16, top: 28, bottom: 28 },
    xAxis: { type: 'category', data: dates, boundaryGap: false },
    yAxis: { type: 'value', min: 0, max: 100 },
  };

  if (view === 'overlay') {
    const chart = mountEcharts(host);
    chart.setOption(Object.assign({}, baseAxis, {
      legend: { data: ['Combined', 'Architectural', 'Code'] },
      series: [
        { name: 'Architectural', type: 'line', smooth: true, symbol: 'circle', data: arch,
          lineStyle: { color: okColor, width: 1.5, opacity: 0.7 } },
        { name: 'Code', type: 'line', smooth: true, symbol: 'circle', data: code,
          lineStyle: { color: warnColor, width: 1.5, opacity: 0.7 } },
        { name: 'Combined', type: 'line', smooth: true, symbol: 'circle', data: combined,
          lineStyle: { color: combinedColor, width: 3 },
          markArea: healthTrendBands() },
      ],
    }));
    return;
  }

  // Split: three stacked small-multiples, same data.
  const panels = [
    { label: 'Combined', series: combined, color: combinedColor },
    { label: 'Architectural', series: arch, color: okColor },
    { label: 'Code', series: code, color: warnColor },
  ];
  host.innerHTML = panels
    .map(function (p, i) { return '<div id="ht-sm-' + i + '" class="ht-sm"></div>'; })
    .join('');
  panels.forEach(function (p, i) {
    const el = document.getElementById('ht-sm-' + i);
    if (!el) return;
    const c = mountEcharts(el);
    c.setOption(Object.assign({}, baseAxis, {
      title: { text: p.label, left: 8, top: 4, textStyle: { fontSize: 12 } },
      series: [
        { name: p.label, type: 'line', smooth: true, symbol: 'circle', data: p.series,
          lineStyle: { color: p.color, width: 2 }, markArea: healthTrendBands() },
      ],
    }));
  });
}
```
Notes for the implementer:
- `cssVar(...)` is the file's existing theme-var accessor — grep it (it may be named differently, e.g. `themeVar` / `getColor`). Use the ACTUAL accessor `renderArchTrend` uses for `infoColor`/`errColor`; the `cssVar(...) || '#fallback'` shape above is a placeholder for that accessor.
- `.ht-sm` / `.ht-btn` need minimal CSS (height for the small-multiple divs, e.g. `120px` each; basic button styling). Add them where the SPA's widget CSS lives (grep the `<style>` block or the css string in `spa.rs`) — mirror an existing small class. If the small-multiple divs have no height, ECharts renders 0px; give `.ht-sm { height: 120px; }`.

- [ ] **Step 4: Verify it renders.** If the repo has an SPA browser test (grep `renderArchTrend`/`gif`/`spa_browser` in tests or `just` recipes), run it. Otherwise, build a dashboard on a fixture and confirm no console errors:
  `just codelore -- analyze --analysis code-health --repo . --format spa --output /tmp/ht.html` (or the actual SPA output invocation), then open `/tmp/ht.html` and confirm the health-trend widget shows a 3-line chart and the toggle switches to 3 small-multiples. (If there is an automated SPA browser test harness, prefer it and assert the widget mounts.)

- [ ] **Step 5: Full gate + commit.** Run: `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test -p codelore-lib --features test-support --test health_trend_test`
  Expected: clean + pass.
```bash
git add crates/codelore-lib/src/output/spa/widgets.js crates/codelore-lib/src/output/spa.rs
git commit -m "feat(health-trend): SPA renderHealthTrend widget — overlaid 3-line + split toggle"
```

---

## Self-Review Notes (applied)

- **Spec coverage:** scoring model (T1: arch_health/code_health/combined/bands) ✓; §2 architecture & reuse (T2 shared sampler + T3 per-rev walk reusing `import_graph_at_rev` + adding the complexity/imports scan) ✓; HealthTrendRow shape (T1, matches spec §Output row) ✓; §3 SPA overlaid-default + split-toggle (T6) ✓; §4 CLI on-demand never-cached CSV/JSON (T4) ✓; §5 testing — band boundaries, arch_health on known graph, code proxy vs formula, combined=mean, integration per-sample + degrading-arch (T1 units + T3 integration; the "degrading architecture ⇒ arch_health decreasing" case can be strengthened in T3 with a purpose-built fixture if `biomarker_repo` doesn't degrade — noted) ✓; no CACHE_EPOCH / Repo-trait change (Global Constraints) ✓.
- **Open judgement points flagged (not placeholders):** T2 arch-trend test file name (grep); T3 `with_no_row_limit` exact spelling; T4 markdown emitter file + `dispatch_*` call conventions (mirror arch-trend); T5 SPA test shape; T6 the theme-color accessor + widget-body DOM insertion + small-multiple CSS (grep + mirror `renderArchTrend`). Each is "find this exact existing thing and mirror it," resolved by the tests + the internals reference.
- **Type consistency:** `HealthTrendRow` fields identical across T1 (def), T3 (construction), T4 (emitters), T6 (JS consumption). `run_health_trend(db, repo, opts)` signature consistent T3↔T4↔T5. `arch_health(&GraphMetrics)`/`repo_code_health(&[CodeHealthRow])`/`combined_health(f64,f64)` consistent T1↔T3.
- **Reuse:** no duplicated sampler (T2), no duplicated code-health definition (calls piece-1 `run_code_health_scoped`), no new vendored JS lib (reuses ECharts + `mountEcharts`).
- **Degrading-arch integration case:** if `biomarker_repo` does not exhibit a monotone arch decline, T3 should add a small fixture whose later commits introduce an import cycle and assert `arch_health` at the newest sample < an earlier sample. Left to the implementer to add if the existing fixture is flat (the spec asks for it; the primary integration test above already covers shape + ranges + oldest-first).
