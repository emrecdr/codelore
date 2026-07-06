# Rev-parameterizable code_health — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `code_health` computable at any revision — HEAD or historical — via a pluggable complexity/imports source, a history-date cutoff, and a clone toggle, so a future timeline (and any "health at commit X" feature) reuses one code-health definition. **HEAD output stays byte-identical.**

**Architecture:** Introduce `HealthScanCtx` (complexity source, imports source, optional history cutoff, include-clones flag). `run_code_health` becomes a thin wrapper calling `run_code_health_scoped(db, opts, &HealthScanCtx::head())`. The scoped form threads the ctx into: the already-`{cm_src}`/`{src}`-parameterized main SQL; a new `{cm_src}` placeholder in the biomarker INSERT + universe query; a scoped `god_classes`; a date-filtered coupling/history path; and a `{structural_scale}` divisor placeholder that re-normalizes `structural_risk` when DRY is excluded. Two new `at_rev` ingest helpers let a caller build the rev-scoped sources.

**Tech Stack:** Rust, DuckDB (`FactsDb`), existing `{cm_src}`/`{src}` source-table placeholder convention, tree-sitter complexity (`crate::complexity`).

**Spec:** `docs/superpowers/specs/2026-07-06-rev-parameterizable-code-health-design.md` — read first.

## Global Constraints

- `unsafe_code = "forbid"` — zero `unsafe`. No `unwrap()` outside `#[cfg(test)]`; library errors via `CodeLoreError`.
- No ticket IDs / plan refs / version numbers in code comments or docs.
- **HEAD byte-identical:** `run_code_health` output (CSV + JSON) MUST be unchanged. `HealthScanCtx::head()` must resolve every placeholder to today's literal (`{cm_src}`→`complexity_metrics`, `{structural_scale}`→empty, cutoff None, clones on).
- The `code_health_biomarkers_v1` cross-analysis contract (read by `refactoring_targets`) is preserved: same table name, columns, session scope.
- No `Repo` trait change; no facts-schema change; no `CACHE_EPOCH` bump.
- Conventional Commits, no co-author trailers. `cargo fmt --all` before every commit.
- Final gate: `cargo clippy --workspace --all-targets --all-features -- -D warnings` + `cargo fmt --all --check` + tests.

## Execution Guardrails (read before every task)

1. Run every command from repo root. Touch ONLY the files in the task's **Files** block.
2. Copy code blocks verbatim. If an insertion anchor isn't found verbatim, STOP and report.
3. If a command's output ≠ the step's **Expected**, STOP; re-read; if still off, report exact command + output + step number. Never loosen an assertion, add `#[allow]`/`#[ignore]`/`unwrap`, or sleep/retry.
4. The byte-identical proof (Task 2, Task 7) is load-bearing — do not skip or fudge it.

## File Structure

- Modify: `crates/codelore-lib/src/analyses/code_health.rs` — `HealthScanCtx`, `run_code_health_scoped`, placeholder + scale threading, clone toggle.
- Modify: `crates/codelore-lib/src/analyses/god_classes.rs` — `run_god_classes_scoped`.
- Create: `crates/codelore-lib/src/facts/ingest/at_rev.rs` — `ingest_complexity_at_rev`, `materialize_imports_at_rev`.
- Modify: `crates/codelore-lib/src/facts/ingest/mod.rs` — register `at_rev`.
- Modify: `crates/codelore-lib/tests/code_health_test.rs` — scoped/byte-identical/renorm tests.
- Create: `crates/codelore-lib/tests/health_scan_at_rev_test.rs` — rev-helper tests.

---

### Task 1: `HealthScanCtx` + renormalization constant

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs`

**Interfaces:**
- Produces (used by Tasks 2–7):
  - `pub struct HealthScanCtx { pub complexity_source: String, pub imports_source: String, pub history_cutoff: Option<String>, pub include_clones: bool }`
  - `HealthScanCtx::head() -> Self`
  - `const STRUCTURAL_SCALE_NO_DRY: &str = " / 0.85";` (the four non-DRY weights sum to 0.85; dividing renormalizes to 1.0)

- [ ] **Step 1: Add the struct, constructor, and a test.** Insert near the top of `code_health.rs`, after the existing `use` lines:

```rust
/// What revision / sources a code-health scan runs against. `head()` resolves
/// to today's HEAD tables so existing behaviour is byte-identical.
#[derive(Debug, Clone)]
pub struct HealthScanCtx {
    /// Complexity source table (HEAD: `"complexity_metrics"`).
    pub complexity_source: String,
    /// Imports source table for god-class fan-in/out (HEAD: `"imports"`).
    pub imports_source: String,
    /// When `Some(ts)`, history terms (churn, author, coupling) are limited to
    /// `commits.date <= ts`.
    pub history_cutoff: Option<String>,
    /// Include the clone/DRY biomarker (true at HEAD; false at a historical rev
    /// where clone detection is unavailable).
    pub include_clones: bool,
}

impl HealthScanCtx {
    /// The HEAD scan — every source resolves to today's table, DRY included.
    #[must_use]
    pub fn head() -> Self {
        Self {
            complexity_source: "complexity_metrics".to_string(),
            imports_source: "imports".to_string(),
            history_cutoff: None,
            include_clones: true,
        }
    }
}

/// Divisor appended to the `structural_risk` SUM when the DRY biomarker is
/// excluded: the four remaining weights (0.30+0.25+0.15+0.15) sum to 0.85, so
/// dividing by 0.85 renormalizes the risk scale back to 1.0. Empty at HEAD.
const STRUCTURAL_SCALE_NO_DRY: &str = " / 0.85";
```

Add to the `#[cfg(test)] mod tests` (or create it) in `code_health.rs`:

```rust
    #[test]
    fn head_ctx_defaults_to_head_tables() {
        let c = super::HealthScanCtx::head();
        assert_eq!(c.complexity_source, "complexity_metrics");
        assert_eq!(c.imports_source, "imports");
        assert!(c.history_cutoff.is_none());
        assert!(c.include_clones);
    }

    #[test]
    fn no_dry_scale_renormalizes_to_one() {
        // 0.30 + 0.25 + 0.15 + 0.15 = 0.85; dividing by 0.85 restores a 1.0 ceiling.
        let sum = 0.30 + 0.25 + 0.15_f64 + 0.15;
        assert!((sum - 0.85).abs() < 1e-9);
        assert_eq!(super::STRUCTURAL_SCALE_NO_DRY, " / 0.85");
    }
```

- [ ] **Step 2: Run tests.** Run: `cargo test -p codelore-lib --features test-support --lib code_health::tests::head_ctx code_health::tests::no_dry`
  Expected: 2 passed. (If the module path differs, use `--lib code_health` and confirm both new tests pass.)

- [ ] **Step 3: Commit.**
```bash
git add crates/codelore-lib/src/analyses/code_health.rs
git commit -m "feat(code-health): add HealthScanCtx + no-DRY renormalization constant"
```

---

### Task 2: Scoped entry + biomarker `{cm_src}` + `{structural_scale}` — byte-identical at HEAD

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs`
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Consumes: `HealthScanCtx` (Task 1).
- Produces: `pub fn run_code_health_scoped(db, opts, cx: &HealthScanCtx) -> Result<Vec<CodeHealthRow>>`; `run_code_health` delegates to it with `HealthScanCtx::head()`.

- [ ] **Step 1: Capture the byte-identical baseline BEFORE any change.** Run:
```bash
cargo run -q -p codelore-cli -- analyze --analysis code-health --repo . --format csv > /tmp/ch_before.csv
cargo run -q -p codelore-cli -- analyze --analysis code-health --repo . --format json > /tmp/ch_before.json
wc -l /tmp/ch_before.csv
```
Expected: a non-empty CSV (header + rows). Keep these two files — Step 6 diffs against them.

- [ ] **Step 2: Add `{cm_src}` to `BIOMARKERS_INSERT` and the universe query.** In `code_health.rs`, in the `BIOMARKERS_INSERT` const, change the `lang_fn` CTE's source line:

```rust
        FROM {cm_src}
        WHERE cyclomatic IS NOT NULL
            AND loc IS NOT NULL
```
(was `FROM complexity_metrics`).

In `materialize_biomarkers`, change the universe query string:
```rust
        "SELECT DISTINCT path FROM {cm_src} \
         WHERE cyclomatic IS NOT NULL AND loc IS NOT NULL",
```
(was `FROM complexity_metrics`). These are now format templates — they must be `.replace("{cm_src}", …)` before use (Step 4).

- [ ] **Step 3: Add the `{structural_scale}` placeholder to the main `SQL`.** In the `file_structural` CTE, change the closing of the weighted sum:
```rust
            LEAST(1.0, SUM(intensity * CASE smell
                WHEN 'complex-method'  THEN 0.30
                WHEN 'god-class'       THEN 0.25
                WHEN 'large-method'    THEN 0.15
                WHEN 'dry'             THEN 0.15
                WHEN 'shotgun-surgery' THEN 0.15
                ELSE 0.0
            END){structural_scale})) AS structural_risk
```
(inserted `{structural_scale}` immediately after the `END`, before the two closing parens). The `dry` arm stays in the CASE — when DRY rows aren't inserted, that arm simply never matches; the divisor handles renormalization.

- [ ] **Step 4: Thread the ctx through `materialize_biomarkers` and add `run_code_health_scoped`.** Change `materialize_biomarkers` signature and its two SQL uses:

```rust
fn materialize_biomarkers(db: &FactsDb, opts: &Options, cx: &HealthScanCtx) -> Result<()> {
    db.conn()
        .execute(BIOMARKERS_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create biomarker temp table: {e}")))?;
    let biomarkers_insert = BIOMARKERS_INSERT.replace("{cm_src}", &cx.complexity_source);
    db.conn()
        .execute(&biomarkers_insert, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert complexity biomarkers: {e}")))?;
    db.conn()
        .execute(SHOTGUN_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert shotgun-surgery biomarkers: {e}")))?;
```
Change the universe `query_map_collect` to substitute and (for now) keep god-class + dry as-is (they're re-scoped in Tasks 3/5). Replace the universe SQL literal with:
```rust
    let universe_sql = "SELECT DISTINCT path FROM {cm_src} \
         WHERE cyclomatic IS NOT NULL AND loc IS NOT NULL"
        .replace("{cm_src}", &cx.complexity_source);
    let universe = crate::analyses::query::query_map_collect(
        db, &universe_sql, [], "biomarker-universe", |r| r.get::<_, String>(0),
    )?;
```

Now split the entry point. Replace the current `run_code_health` body:
```rust
#[tracing::instrument(name = "code-health", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    run_code_health_scoped(db, opts, &HealthScanCtx::head())
}

/// Code health against the sources named by `cx`. `cx = HealthScanCtx::head()`
/// reproduces the HEAD analysis byte-for-byte.
pub fn run_code_health_scoped(
    db: &FactsDb,
    opts: &Options,
    cx: &HealthScanCtx,
) -> Result<Vec<CodeHealthRow>> {
    materialize_centrality(db, opts)?;
    materialize_biomarkers(db, opts, cx)?;

    crate::analyses::lineage::materialize_source(db, opts)?;
    let src = crate::analyses::lineage::source_table(opts);
    let cm_src = &cx.complexity_source;
    let structural_scale = if cx.include_clones { "" } else { STRUCTURAL_SCALE_NO_DRY };
    let sql = SQL
        .replace("{src}", src)
        .replace("{cm_src}", cm_src)
        .replace("{structural_scale}", structural_scale);
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(db, &sql, params![opts.min_revs, row_limit], "code-health", opts)?;
    let mut stmt = db.conn().prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-health: {e}")))?;
    let rows = stmt.query_map(params![opts.min_revs, row_limit], |r| {
        Ok(CodeHealthRow {
            path: r.get::<_, String>(0)?,
            cognitive: r.get::<_, f64>(1)?,
            score: r.get::<_, f64>(2)?,
            structural_risk: r.get::<_, f64>(3)?,
            percentile: r.get::<_, f64>(4)?,
            band: r.get::<_, String>(5)?,
        })
    }).map_err(|e| CodeLoreError::Analysis(format!("query code-health: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-health: {e}")))
}
```
Note: `run_code_health_scoped` no longer references `crate::analyses::grouped_complexity::source_table` for `cm_src` — it uses `cx.complexity_source`. At HEAD that is `"complexity_metrics"`, matching the old resolver's default. (If `--group-file` was a supported code-health mode via `grouped_complexity`, confirm with the reviewer whether `HealthScanCtx::head()` must instead call `grouped_complexity::source_table(opts)`; the byte-identical diff in Step 6 will catch any divergence.)

- [ ] **Step 5: Build + run the existing code-health tests.** Run: `cargo test -p codelore-lib --features test-support --test code_health_test`
  Expected: all existing tests pass (unchanged behavior).

- [ ] **Step 6: Prove byte-identical HEAD output.** Run:
```bash
cargo run -q -p codelore-cli -- analyze --analysis code-health --repo . --format csv > /tmp/ch_after.csv
cargo run -q -p codelore-cli -- analyze --analysis code-health --repo . --format json > /tmp/ch_after.json
diff /tmp/ch_before.csv /tmp/ch_after.csv && echo "CSV IDENTICAL"
diff /tmp/ch_before.json /tmp/ch_after.json && echo "JSON IDENTICAL"
```
Expected: both print `IDENTICAL` with no diff output. **If either differs, STOP** — the placeholder threading changed behavior; do not proceed.

- [ ] **Step 7: Commit** (paste the diff-clean confirmation into the message body).
```bash
git add crates/codelore-lib/src/analyses/code_health.rs
git commit -m "refactor(code-health): scoped entry + {cm_src}/{structural_scale} placeholders

HEAD output byte-identical: code-health CSV+JSON diff-clean before/after."
```

---

### Task 3: Scoped `god_classes`

**Files:**
- Modify: `crates/codelore-lib/src/analyses/god_classes.rs`
- Modify: `crates/codelore-lib/src/analyses/code_health.rs`

**Interfaces:**
- Produces: `pub fn run_god_classes_scoped(db, opts, complexity_source: &str, imports_source: &str) -> Result<Vec<GodClassRow>>`; `run_god_classes` delegates with `("complexity_metrics", "imports")`.
- Consumed by: `materialize_biomarkers` (passes `cx.complexity_source`, `cx.imports_source`).

- [ ] **Step 1: Read the two hardwired table names in `god_classes.rs`.** Run: `grep -n "FROM imports\|FROM complexity_metrics\|complexity_metrics\|imports" crates/codelore-lib/src/analyses/god_classes.rs`
  Expected: the fan-in/fan-out CTEs referencing `imports` and the cognitive read referencing `complexity_metrics`.

- [ ] **Step 2: Parameterize.** Turn the god-class SQL into a template with `{cm_src}` and `{imports_src}` placeholders wherever `complexity_metrics` / `imports` appear, and split the entry:
```rust
pub fn run_god_classes(db: &FactsDb, opts: &Options) -> Result<Vec<GodClassRow>> {
    run_god_classes_scoped(db, opts, "complexity_metrics", "imports")
}

pub fn run_god_classes_scoped(
    db: &FactsDb,
    opts: &Options,
    complexity_source: &str,
    imports_source: &str,
) -> Result<Vec<GodClassRow>> {
    // ... existing body, but the SQL const is `.replace("{cm_src}", complexity_source)
    //     .replace("{imports_src}", imports_source)` before prepare ...
}
```
(Repeat the exact existing body; only the SQL string(s) gain the two `.replace(...)` calls and the `FROM complexity_metrics` / `FROM imports` become `FROM {cm_src}` / `FROM {imports_src}`.)

- [ ] **Step 3: Call the scoped variant from `materialize_biomarkers`.** In `code_health.rs`, change:
```rust
    let gods = crate::analyses::god_classes::run_god_classes_scoped(
        db, &opts.with_no_row_limit(), &cx.complexity_source, &cx.imports_source,
    )?;
```

- [ ] **Step 4: Byte-identical + tests.** Run the existing god-class + code-health tests:
  `cargo test -p codelore-lib --features test-support --test god_classes_test --test code_health_test`
  Expected: pass. Then repeat Task 2 Step 6's HEAD byte-identical diff (`complexity_metrics`/`imports` defaults ⇒ identical). Expected: `IDENTICAL`.

- [ ] **Step 5: Commit.**
```bash
git add crates/codelore-lib/src/analyses/god_classes.rs crates/codelore-lib/src/analyses/code_health.rs
git commit -m "refactor(code-health): scoped god_classes source tables (HEAD byte-identical)"
```

---

### Task 4: History cutoff (churn / author / coupling)

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs`

**Interfaces:**
- Consumes: `cx.history_cutoff: Option<String>`.
- Produces: `materialize_centrality(db, opts, cx)` honors the cutoff; churn/author `{src}` is date-scoped when set.

- [ ] **Step 1: Decide the cutoff mechanism (documented).** The history terms read `{src}` (from `lineage::source_table`) and coupling reads full history via `run_coupling`. When `cx.history_cutoff` is `Some(ts)`, materialize a session-local date-filtered changes view and point both at it. Add near the other DDL consts:
```rust
/// A changes view limited to commits at/-before a cutoff timestamp, so
/// history-derived terms (churn, author fragmentation, coupling) are rev-scoped.
const CHANGES_AT_TS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY VIEW changes_at_ts AS
    SELECT c.* FROM changes c
    INNER JOIN commits ON commits.rev = c.rev
    WHERE commits.date <= CAST(? AS TIMESTAMP)
";
```

- [ ] **Step 2: Build the view + choose sources in `run_code_health_scoped`.** After `materialize_source`, when a cutoff is set, create the view and override `src`:
```rust
    let src_owned;
    let src: &str = if let Some(ts) = &cx.history_cutoff {
        db.conn().execute(CHANGES_AT_TS_DDL, params![ts])
            .map_err(|e| CodeLoreError::Analysis(format!("create changes_at_ts view: {e}")))?;
        src_owned = "changes_at_ts".to_string();
        &src_owned
    } else {
        crate::analyses::lineage::source_table(opts)
    };
```
(Replace the prior `let src = …` line with this. At HEAD `history_cutoff` is None ⇒ `src` is the exact old value ⇒ byte-identical.)

- [ ] **Step 3: Date-scope coupling.** Change `materialize_centrality` to accept `cx` and, when a cutoff is set, restrict coupling to the cutoff. The minimal, non-forking approach: `run_coupling` reads from the changes history; pass a date-filtered `Options` OR gate on the `changes_at_ts` view if `run_coupling` honors the lineage source. Inspect `run_coupling`'s source resolution first:
  Run: `grep -n "source_table\|FROM changes\|fn run_coupling" crates/codelore-lib/src/analyses/coupling.rs`
  - If `run_coupling` reads `lineage::source_table(opts)` / a `{src}` placeholder, set that to `changes_at_ts` for the scoped call (same mechanism as Step 2).
  - If `run_coupling` hardwires `FROM changes`, add a `run_coupling_scoped(db, opts, changes_source: &str)` mirroring Task 3's pattern and call it here.
  Implement whichever the grep shows; keep `materialize_centrality(db, opts, cx)` and at HEAD (cutoff None) call the unscoped/`"changes"` path ⇒ byte-identical.

- [ ] **Step 4: Update the `materialize_centrality` call** in `run_code_health_scoped` to pass `cx`.

- [ ] **Step 5: Tests + byte-identical.** `cargo test -p codelore-lib --features test-support --test code_health_test`; then Task 2 Step 6 diff ⇒ `IDENTICAL`. Expected: pass + identical.

- [ ] **Step 6: Commit.**
```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/src/analyses/coupling.rs
git commit -m "refactor(code-health): optional history cutoff for churn/author/coupling (HEAD byte-identical)"
```

---

### Task 5: Clone toggle (skip DRY when disabled)

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs`
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Consumes: `cx.include_clones`.

- [ ] **Step 1: Gate DRY in `materialize_biomarkers`.** Wrap the `run_clones` + dry accounting and the `"dry"` arm of the per-language loop so they only run when `cx.include_clones`:
```rust
    let dry_counts: HashMap<String, u32> = if cx.include_clones {
        let clones = crate::analyses::clones::run_clones(opts)?;
        let mut m: HashMap<String, u32> = HashMap::new();
        for c in &clones {
            *m.entry(c.entity.clone()).or_insert(0) += 1;
        }
        m
    } else {
        HashMap::new()
    };
```
And in the smell loop, choose the smell set by the flag:
```rust
        let smells: &[&str] = if cx.include_clones { &["god-class", "dry"] } else { &["god-class"] };
        for smell in smells {
            // ... unchanged body, using *smell ...
        }
```
(The `dry_counts` map is empty when clones are off, but skipping the `"dry"` smell entirely avoids inserting any zero-intensity dry rows.)

- [ ] **Step 2: Confirm the scale wiring.** `run_code_health_scoped` already sets `structural_scale = STRUCTURAL_SCALE_NO_DRY` when `!cx.include_clones` (Task 2 Step 4). No change needed here — verify by re-reading that line.

- [ ] **Step 3: Write a scoped renormalization test.** Add to `code_health_test.rs`:
```rust
#[test]
fn scoped_no_clones_excludes_dry_and_renormalizes() {
    use codelore_lib::analyses::code_health::{run_code_health, run_code_health_scoped, HealthScanCtx};
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let gix = codelore_lib::repo::GixRepo::open(&repo.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo.dir.path().to_path_buf());
    db.ingest(&gix, &opts).expect("ingest");

    let head = run_code_health(&db, &opts).expect("head");
    let mut cx = HealthScanCtx::head();
    cx.include_clones = false;
    let no_dry = run_code_health_scoped(&db, &opts, &cx).expect("no-dry");

    // Same file set, same order key; the no-DRY scores are >= HEAD scores for
    // files whose only-or-partial risk came from duplication (DRY removed can
    // only lower structural_risk, hence raise or hold the score).
    assert_eq!(head.len(), no_dry.len(), "same file universe");
    let head_by: std::collections::HashMap<_, _> =
        head.iter().map(|r| (r.path.clone(), r.score)).collect();
    for r in &no_dry {
        let h = head_by.get(&r.path).copied().expect("path present in both");
        assert!(r.score + 1e-9 >= h, "no-DRY score must not be below HEAD for {}", r.path);
    }
    // And at least one file differs (biomarker_repo has a DRY clone pair).
    assert!(no_dry.iter().any(|r| (r.score - head_by[&r.path]).abs() > 1e-9),
        "expected DRY removal to move at least one score");
}
```

- [ ] **Step 4: Run it.** `cargo test -p codelore-lib --features test-support --test code_health_test scoped_no_clones`
  Expected: PASS. (If the "at least one file differs" assertion fails, the `biomarker_repo` DRY pair may not register at the default thresholds — inspect and adjust the fixture reference with the reviewer; do NOT delete the assertion.)

- [ ] **Step 5: Commit.**
```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): include_clones toggle drops DRY + renormalizes structural risk"
```

---

### Task 6: `at_rev` ingest helpers

**Files:**
- Create: `crates/codelore-lib/src/facts/ingest/at_rev.rs`
- Modify: `crates/codelore-lib/src/facts/ingest/mod.rs`
- Test: `crates/codelore-lib/tests/health_scan_at_rev_test.rs`

**Interfaces:**
- Produces (used by piece 2):
  - `pub fn ingest_complexity_at_rev<R: Repo>(db: &FactsDb, repo: &R, rev: &str, live_paths: &[String], dest_table: &str) -> Result<()>`
  - `pub fn materialize_imports_at_rev(db: &FactsDb, graph: &ImportGraph, dest_table: &str) -> Result<()>`

- [ ] **Step 1: Read the HEAD complexity ingest to mirror it.** Run: `sed -n '1,135p' crates/codelore-lib/src/facts/ingest/complexity_head.rs` — note `append_metric_row`/`append_entity_row`, the size guard, `read_blob_at_head`, the appender pattern.

- [ ] **Step 2: Create `at_rev.rs`.** Write `ingest_complexity_at_rev` mirroring `ingest_complexity_at_head` but (a) reading `repo.read_blob_at(rev, &path)` instead of `read_blob_at_head`, and (b) creating + appending to a caller-named temp table with the `complexity_metrics` column shape (reuse the same `append_metric_row` writer against a `CREATE TEMPORARY TABLE <dest_table> (LIKE complexity_metrics)` — or an explicit column DDL matching `complexity_metrics`). And `materialize_imports_at_rev` that writes `graph.id_to_path` + `graph.adj` edges into a temp table shaped like `imports` (columns the god-class fan-in/out CTEs read — confirm exact `imports` columns via `grep -n "CREATE TABLE.*imports\|imports (" crates/codelore-lib/src/facts/schema_v1.sql`). Full code depends on those exact column names — the implementer writes it against the schema, mirroring `complexity_head.rs`'s appender pattern verbatim for the complexity half.

  (This task's code is not fully pre-written because the temp-table DDL must match the live `complexity_metrics` / `imports` column lists exactly; the implementer reads the schema in Step 1/2 and reproduces those columns. Every other pattern — blob read, size guard, appender drain — is copied verbatim from `complexity_head.rs`.)

- [ ] **Step 3: Register the module.** In `crates/codelore-lib/src/facts/ingest/mod.rs`, add `pub mod at_rev;` (alphabetical).

- [ ] **Step 4: Test.** Create `health_scan_at_rev_test.rs`: build a 2-commit fixture, `ingest_complexity_at_rev(db, repo, head_rev, &live_paths, "cm_at_rev")`, and assert the temp table's `(path, name, cyclomatic, loc)` rows match a HEAD complexity scan of the same tree (same function set + metrics). Assert `materialize_imports_at_rev` produces the expected edge rows for a known import.

- [ ] **Step 5: Run + commit.** `cargo test -p codelore-lib --features test-support --test health_scan_at_rev_test` → PASS.
```bash
git add crates/codelore-lib/src/facts/ingest/at_rev.rs crates/codelore-lib/src/facts/ingest/mod.rs crates/codelore-lib/tests/health_scan_at_rev_test.rs
git commit -m "feat(facts): ingest_complexity_at_rev + materialize_imports_at_rev helpers"
```

---

### Task 7: End-to-end scoped test + byte-identical regression guard

**Files:**
- Test: `crates/codelore-lib/tests/code_health_test.rs`

- [ ] **Step 1: Add a byte-identical regression test** so future edits can't silently break HEAD parity:
```rust
#[test]
fn head_wrapper_equals_scoped_head_ctx() {
    use codelore_lib::analyses::code_health::{run_code_health, run_code_health_scoped, HealthScanCtx};
    let repo = codelore_lib::test_support::biomarker_repo::build();
    let gix = codelore_lib::repo::GixRepo::open(&repo.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo.dir.path().to_path_buf());
    db.ingest(&gix, &opts).expect("ingest");

    let a = run_code_health(&db, &opts).expect("wrapper");
    let b = run_code_health_scoped(&db, &opts, &HealthScanCtx::head()).expect("scoped-head");
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.path, y.path);
        assert!((x.score - y.score).abs() < 1e-12, "score parity for {}", x.path);
        assert!((x.structural_risk - y.structural_risk).abs() < 1e-12);
        assert_eq!(x.band, y.band);
    }
}
```

- [ ] **Step 2: Run it.** `cargo test -p codelore-lib --features test-support --test code_health_test head_wrapper_equals_scoped` → PASS.

- [ ] **Step 3: Full local gate.** Run:
  `cargo fmt --all --check` (fix with `cargo fmt --all` if needed) then
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` then
  `cargo test -p codelore-lib --features test-support`
  Expected: all clean/pass.

- [ ] **Step 4: Commit.**
```bash
git add crates/codelore-lib/tests/code_health_test.rs
git commit -m "test(code-health): head-wrapper equals scoped-head-ctx parity guard"
```

---

## Self-Review Notes (applied)

- **Spec coverage:** HealthScanCtx (T1); scoped entry + `{cm_src}`/`{structural_scale}` (T2); god_classes scoped (T3); history cutoff (T4); clone toggle + renorm (T5); at_rev helpers (T6); byte-identical guarantees (T2/T3/T4 diffs + T7 parity test). Renormalization = `/0.85` divisor (spec §4). No `refactoring_targets` change (it still consumes `code_health_biomarkers_v1` post-`run_code_health` — unchanged at HEAD).
- **Known implementer judgement points (flagged, not placeholders):** T2 Step 4 `grouped_complexity` `--group-file` interaction (byte-identical diff catches it); T4 Step 3 coupling source shape (grep decides scoped-vs-view); T6 Step 2 temp-table DDL must match live `complexity_metrics`/`imports` columns (read from schema). Each is a "read this exact thing then mirror it" instruction, resolved by the byte-identical/parity tests.
- **Type consistency:** `run_code_health_scoped(db, opts, &HealthScanCtx)` and `run_god_classes_scoped(db, opts, &str, &str)` signatures consistent T2↔T3; `materialize_biomarkers(db, opts, cx)` / `materialize_centrality(db, opts, cx)` consistent T2↔T4↔T5.
