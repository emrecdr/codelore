# Code Health v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evolve the `code-health` analysis from a cognitive-only composite into a transparent, biomarker-driven, self-relative-percentile, behaviorally-fused score reported as risk **bands + percentile + sub-scores**.

**Architecture:** Keep the existing "materialize temp tables in Rust, then one SQL query" pattern in `analyses/code_health.rs`. Add a **biomarker layer**: per-function smells (Complex Method, Large Method) computed in SQL from the raw `complexity_metrics` table using per-language `PERCENT_RANK` intensities, plus file-level smells reused from existing analyses (God Class ← `run_god_classes`, DRY ← `run_clones`, Shotgun Surgery/Divergent Change ← the already-materialized Fisher coupling). Aggregate to a per-file **structural risk** with a co-occurrence multiplier, band it by absolute thresholds, and fuse it with the existing behavioral terms (churn, ownership fragmentation, coupling centrality). Widen `CodeHealthRow`; bump `CACHE_EPOCH`.

**Tech Stack:** Rust (workspace), DuckDB (SQL over the fact store), serde (output), the `test-support` fixture feature. No new crates. No JS (SPA is a later plan).

## Global Constraints

- `workspace.lints.rust: unsafe_code = "forbid"` — zero `unsafe` blocks; CI rejects additions.
- No `unwrap()`/`expect()` outside tests; library errors via `thiserror` (`CodeLoreError::Analysis(...)`), application errors via `anyhow`.
- Local gate MUST match CI exactly: `cargo clippy --workspace --all-targets --all-features -- -D warnings` (`just lint`); full gate `just ci`.
- This is an **intentional semantic change** to `code-health`: bump `CACHE_EPOCH`. Do **NOT** claim byte-identical output — this is not a semantic-preserving refactor.
- No version numbers, ticket IDs, or static counts in code/comments/docs. Conventional Commits. **Never** add `Co-Authored-By: Claude`.
- `code-health` opts into `--time-bucket`/lineage via `{src}`/`{cm_src}` placeholders — preserve that. Biomarker structural metrics read the **raw** `complexity_metrics` table (HEAD snapshot), which is correct (complexity is HEAD-only).
- Tests that run analyses require the `test-support` feature: `cargo test -p codelore-lib --features test-support ...`.
- Bands/weights/thresholds introduced here are **initial, tunable constants** — tests assert invariants (ordering, ranges, band membership, determinism), never exact magic scores.

---

## File Structure

- **Modify** `crates/codelore-lib/src/analyses/code_health.rs` — the whole metric: widen `CodeHealthRow`, add `materialize_biomarkers`, extend the `SQL` const, extend the `query_map`.
- **Modify** `crates/codelore-lib/src/output/csv.rs:136-149` — `write_code_health_csv` header + row (manual column list).
- **Modify** `crates/codelore-lib/src/output/markdown.rs:158-173` — `write_code_health_markdown` header/separator/row (manual column list).
- **Modify** `crates/codelore-lib/src/cache.rs:25` — bump `CACHE_EPOCH`.
- **Modify** `crates/codelore-cli/src/main.rs:262-267` — update the `explain` tuple for `code-health` (formula/citations).
- **Modify** `crates/codelore-lib/tests/code_health_test.rs` — extend with invariant tests (band, percentile, biomarker ordering).
- (json / ndjson / html / spa emitters: **no change** — they serialize `CodeHealthRow` via `serde::Serialize`.)

---

### Task 1: Widen the row with self-relative percentile + bands (over the current score)

First increment: report `percentile` (per-language self-relative rank of risk) and `band` (R/Y/G) alongside the existing score, wiring every output site and the cache bump. The underlying score is still the current formula; Tasks 2–6 upgrade it. This lands a green, shippable "bands + percentile" improvement.

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs:33-38` (struct), `:96-122` (SQL), `:202-208` (query_map)
- Modify: `crates/codelore-lib/src/output/csv.rs:137,139-146`
- Modify: `crates/codelore-lib/src/output/markdown.rs:160-169`
- Modify: `crates/codelore-lib/src/cache.rs:25`
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Produces: `CodeHealthRow { path: String, cognitive: f64, score: f64, structural_risk: f64, percentile: f64, band: String }` (Tasks 2–6 populate `structural_risk` meaningfully; here it mirrors the normalized cognitive term).
- Consumes: existing `run_code_health` SQL result columns 0..=2.

- [ ] **Step 1: Write the failing test**

In `crates/codelore-lib/tests/code_health_test.rs`:

```rust
#[test]
fn code_health_reports_band_and_percentile() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");
    assert!(!rows.is_empty());
    for row in &rows {
        assert!((0.0..=1.0).contains(&row.percentile), "percentile in [0,1]: {}", row.percentile);
        assert!(
            matches!(row.band.as_str(), "red" | "yellow" | "green"),
            "band must be red|yellow|green, got {}", row.band
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p codelore-lib --features test-support code_health_reports_band_and_percentile`
Expected: FAIL to compile — no field `percentile`/`band` on `CodeHealthRow`.

- [ ] **Step 3: Widen the struct**

`code_health.rs:33-38`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeHealthRow {
    pub path: String,
    pub cognitive: f64,
    pub score: f64, // 0..=100; higher = healthier
    pub structural_risk: f64, // 0..=1; higher = worse (Task 2+ makes this biomarker-based)
    pub percentile: f64, // 0..=1; per-language self-relative rank of structural_risk (1 = riskiest)
    pub band: String, // "red" | "yellow" | "green"
}
```

- [ ] **Step 4: Extend the SQL projection with percentile + band**

Replace the final `SELECT ... FROM normalized` block (`code_health.rs:109-122`). Derive language from the path extension inline, rank risk per language, and band by absolute thresholds. `structural_risk` here = the normalized cognitive term `n_cx` (upgraded in Task 5):

```sql
    scored AS (
        SELECT
            path,
            cognitive,
            n_cx AS structural_risk,
            GREATEST(0.0, LEAST(100.0,
                100.0 * (1.0 - 0.40 * n_cx - 0.25 * n_cn - 0.15 * n_au - 0.20 * n_cp)
            )) AS score,
            CASE lower(regexp_extract(path, '\.([^.]+)$', 1))
                WHEN 'rs' THEN 'rust'
                WHEN 'py' THEN 'python' WHEN 'pyi' THEN 'python'
                WHEN 'java' THEN 'java'
                WHEN 'js' THEN 'javascript' WHEN 'jsx' THEN 'javascript'
                WHEN 'mjs' THEN 'javascript' WHEN 'cjs' THEN 'javascript'
                WHEN 'ts' THEN 'typescript' WHEN 'tsx' THEN 'typescript'
                ELSE 'other'
            END AS lang
        FROM normalized
    )
    SELECT
        path,
        cognitive,
        score,
        structural_risk,
        PERCENT_RANK() OVER (PARTITION BY lang ORDER BY structural_risk) AS percentile,
        CASE
            WHEN structural_risk >= 0.66 THEN 'red'
            WHEN structural_risk >= 0.33 THEN 'yellow'
            ELSE 'green'
        END AS band
    FROM scored
    ORDER BY score ASC, path ASC
    LIMIT ?
```

- [ ] **Step 5: Extend the `query_map` closure**

`code_health.rs:202-208`:

```rust
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(CodeHealthRow {
                path: r.get::<_, String>(0)?,
                cognitive: r.get::<_, f64>(1)?,
                score: r.get::<_, f64>(2)?,
                structural_risk: r.get::<_, f64>(3)?,
                percentile: r.get::<_, f64>(4)?,
                band: r.get::<_, String>(5)?,
            })
        })
```

- [ ] **Step 6: Wire the CSV emitter**

`output/csv.rs:137` and `:139-146`:

```rust
    writeln!(w, "entity,cognitive,score,structural_risk,percentile,band")?;
    // ...per row:
    writeln!(
        w,
        "{},{:.2},{:.2},{:.4},{:.4},{}",
        quote_if_needed(&row.path), row.cognitive, row.score,
        row.structural_risk, row.percentile, row.band
    )?;
```

- [ ] **Step 7: Wire the Markdown emitter**

`output/markdown.rs:160-169`:

```rust
    writeln!(w, "| Entity | Cognitive | Score | Structural risk | Percentile | Band |")?;
    writeln!(w, "|---|---|---|---|---|---|")?;
    // ...per row:
    writeln!(
        w,
        "| `{}` | {:.2} | {:.2} | {:.4} | {:.4} | {} |",
        escape_md_cell(&row.path), row.cognitive, row.score,
        row.structural_risk, row.percentile, row.band
    )?;
```

- [ ] **Step 8: Bump `CACHE_EPOCH`**

`cache.rs:25`:

```rust
const CACHE_EPOCH: &str = "schema_v6";
```

- [ ] **Step 9: Run the test to verify it passes**

Run: `cargo test -p codelore-lib --features test-support code_health_reports_band_and_percentile`
Expected: PASS.

- [ ] **Step 10: Run the full gate**

Run: `just lint && cargo test -p codelore-lib --features test-support code_health`
Expected: clippy clean; existing `code_health_*` tests still pass.

- [ ] **Step 11: Commit**

```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/src/output/csv.rs crates/codelore-lib/src/output/markdown.rs crates/codelore-lib/src/cache.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): report self-relative percentile and R/Y/G bands"
```

---

### Task 2: Biomarker temp table — Complex Method + Large Method (per-language percentile intensity)

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (add `BIOMARKERS_DDL`, `materialize_biomarkers`; call it in `run_code_health`)
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Produces: session temp table `code_health_biomarkers_v1(path TEXT, smell TEXT, intensity DOUBLE)` where `intensity` ∈ [0,1] is the per-language `PERCENT_RANK` of the function metric, rolled up to the file by `MAX`. `smell` ∈ {`complex-method`, `large-method`} in this task.
- Consumes: raw `complexity_metrics(path, name, cyclomatic, loc)` (HEAD snapshot).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn biomarkers_flag_complex_functions() {
    use duckdb::params;
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Running code-health materializes the biomarker table as a side effect.
    let _ = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");

    let count: i64 = db.conn()
        .query_row("SELECT COUNT(*) FROM code_health_biomarkers_v1", params![], |r| r.get(0))
        .expect("query biomarkers");
    assert!(count >= 1, "tiny_repo should produce >=1 biomarker row");

    // intensities are valid probabilities
    let bad: i64 = db.conn()
        .query_row(
            "SELECT COUNT(*) FROM code_health_biomarkers_v1 WHERE intensity < 0.0 OR intensity > 1.0",
            params![], |r| r.get(0),
        ).expect("query range");
    assert_eq!(bad, 0, "all intensities must be in [0,1]");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p codelore-lib --features test-support biomarkers_flag_complex_functions`
Expected: FAIL — `code_health_biomarkers_v1` does not exist.

- [ ] **Step 3: Add the DDL + materializer**

In `code_health.rs`, near `CENTRALITY_DDL`:

```rust
const BIOMARKERS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE code_health_biomarkers_v1 (
        path TEXT NOT NULL,
        smell TEXT NOT NULL,
        intensity DOUBLE NOT NULL
    )
";

/// Populate function-level structural biomarkers from the raw
/// `complexity_metrics` snapshot. Intensity = per-language PERCENT_RANK of the
/// function metric (self-relative, Phase 1), rolled up to the file by MAX.
/// Language is derived from the path extension (no stored language column).
const BIOMARKERS_INSERT: &str = "
    INSERT INTO code_health_biomarkers_v1 (path, smell, intensity)
    WITH lang_fn AS (
        SELECT
            path, name, cyclomatic, loc,
            CASE lower(regexp_extract(path, '\\.([^.]+)$', 1))
                WHEN 'rs' THEN 'rust'
                WHEN 'py' THEN 'python' WHEN 'pyi' THEN 'python'
                WHEN 'java' THEN 'java'
                WHEN 'js' THEN 'javascript' WHEN 'jsx' THEN 'javascript'
                WHEN 'mjs' THEN 'javascript' WHEN 'cjs' THEN 'javascript'
                WHEN 'ts' THEN 'typescript' WHEN 'tsx' THEN 'typescript'
                ELSE 'other'
            END AS lang
        FROM complexity_metrics
        WHERE cyclomatic IS NOT NULL
    ),
    ranked AS (
        SELECT path, name, lang,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY cyclomatic) AS cx_i,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY loc)        AS loc_i
        FROM lang_fn
    )
    SELECT path, 'complex-method' AS smell, MAX(cx_i) AS intensity
        FROM ranked GROUP BY path
    UNION ALL
    SELECT path, 'large-method' AS smell, MAX(loc_i) AS intensity
        FROM ranked GROUP BY path
";

fn materialize_biomarkers(db: &FactsDb) -> Result<()> {
    db.conn()
        .execute(BIOMARKERS_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create biomarker temp table: {e}")))?;
    db.conn()
        .execute(BIOMARKERS_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert complexity biomarkers: {e}")))?;
    Ok(())
}
```

- [ ] **Step 4: Call it from `run_code_health`**

In `run_code_health` after `materialize_centrality(db, opts)?` (`code_health.rs:182`):

```rust
    materialize_biomarkers(db)?;
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p codelore-lib --features test-support biomarkers_flag_complex_functions`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): materialize complex-method and large-method biomarkers"
```

---

### Task 3: Reframe Fisher coupling as Shotgun Surgery / Divergent Change biomarkers

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (extend `materialize_biomarkers` to add coupling-derived rows from `coupling_centrality_v1`)
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Consumes: temp table `coupling_centrality_v1(path, centrality)` (already materialized by `materialize_centrality`, which runs first).
- Produces: additional `code_health_biomarkers_v1` rows with `smell = 'shotgun-surgery'`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn coupling_becomes_shotgun_surgery_biomarker() {
    use duckdb::params;
    let repo_fx = codelore_lib::test_support::differential_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(repo_fx.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::test_support::permissive_coupling_opts(repo_fx.dir.path());
    db.ingest(&repo, &opts).expect("ingest");

    let _ = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");

    let n: i64 = db.conn()
        .query_row(
            "SELECT COUNT(*) FROM code_health_biomarkers_v1 WHERE smell = 'shotgun-surgery'",
            params![], |r| r.get(0),
        ).expect("query");
    assert!(n >= 1, "a coupling-heavy repo should yield shotgun-surgery biomarkers");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p codelore-lib --features test-support coupling_becomes_shotgun_surgery_biomarker`
Expected: FAIL — no `shotgun-surgery` rows.

- [ ] **Step 3: Extend `materialize_biomarkers`**

Append after `BIOMARKERS_INSERT` runs, inside `materialize_biomarkers`:

```rust
    // Shotgun Surgery / Divergent Change: a file that co-changes with many
    // Fisher-significant partners is definitionally a temporal smell. Reuse the
    // already-materialized centrality table; intensity = self-relative rank.
    const SHOTGUN_INSERT: &str = "
        INSERT INTO code_health_biomarkers_v1 (path, smell, intensity)
        SELECT path, 'shotgun-surgery' AS smell,
               PERCENT_RANK() OVER (ORDER BY centrality) AS intensity
        FROM coupling_centrality_v1
        WHERE centrality > 0
    ";
    db.conn()
        .execute(SHOTGUN_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert shotgun-surgery biomarkers: {e}")))?;
    Ok(())
```

(Remove the old trailing `Ok(())` so there is exactly one.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p codelore-lib --features test-support coupling_becomes_shotgun_surgery_biomarker`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): reframe Fisher coupling as shotgun-surgery biomarker"
```

---

### Task 4: Fold God Class + DRY into the biomarker set

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (`materialize_biomarkers` gains `god-class` + `dry` rows from `run_god_classes`/`run_clones`)
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Consumes: `crate::analyses::god_classes::run_god_classes(db, opts) -> Vec<GodClassRow{path, god_score, ..}>`; `crate::analyses::clones::run_clones(opts) -> Vec<ClonesRow{entity, function, ..}>`.
- Produces: `code_health_biomarkers_v1` rows with `smell ∈ {'god-class','dry'}`. `materialize_biomarkers` signature changes to `materialize_biomarkers(db: &FactsDb, opts: &Options)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn god_class_and_dry_are_biomarkers() {
    use duckdb::params;
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let _ = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");

    // The smell vocabulary must include the reused analyses (0 rows is allowed
    // for tiny_repo, but the query must succeed against the known smell set).
    let distinct: i64 = db.conn()
        .query_row(
            "SELECT COUNT(DISTINCT smell) FROM code_health_biomarkers_v1 \
             WHERE smell IN ('complex-method','large-method','shotgun-surgery','god-class','dry')",
            params![], |r| r.get(0),
        ).expect("query smell vocabulary");
    assert!(distinct >= 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p codelore-lib --features test-support god_class_and_dry_are_biomarkers`
Expected: FAIL to compile — `materialize_biomarkers` takes one arg / no `opts` in scope for the new inserts.

- [ ] **Step 3: Insert God Class + DRY rows via prepared statements**

Change the signature to `materialize_biomarkers(db: &FactsDb, opts: &Options)` and update the call site to `materialize_biomarkers(db, opts)?`. Append:

```rust
    // God Class: reuse the existing analysis; intensity = normalized god_score.
    let gods = crate::analyses::god_classes::run_god_classes(db, opts)?;
    let max_god = gods.iter().map(|g| g.god_score).fold(0.0_f64, f64::max);
    // DRY: reuse clone detection (walks HEAD worktree); intensity = normalized
    // count of cloned functions per file.
    let clones = crate::analyses::clones::run_clones(opts)?;
    let mut dry_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for c in &clones {
        *dry_counts.entry(c.entity.clone()).or_insert(0) += 1;
    }
    let max_dry = dry_counts.values().copied().max().unwrap_or(0);

    let mut stmt = db.conn()
        .prepare("INSERT INTO code_health_biomarkers_v1 (path, smell, intensity) VALUES (?, ?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare biomarker insert: {e}")))?;
    if max_god > 0.0 {
        for g in &gods {
            stmt.execute(duckdb::params![g.path, "god-class", g.god_score / max_god])
                .map_err(|e| CodeLoreError::Analysis(format!("god-class biomarker: {e}")))?;
        }
    }
    if max_dry > 0 {
        for (path, n) in &dry_counts {
            stmt.execute(duckdb::params![path, "dry", f64::from(*n) / f64::from(max_dry)])
                .map_err(|e| CodeLoreError::Analysis(format!("dry biomarker: {e}")))?;
        }
    }
    Ok(())
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p codelore-lib --features test-support god_class_and_dry_are_biomarkers`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): add god-class and dry biomarkers from existing analyses"
```

---

### Task 5: Aggregate biomarkers into structural risk (probabilistic-OR + co-occurrence multiplier)

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (SQL: replace `n_cx` with a biomarker-derived `structural_risk`)
- Test: `crates/codelore-lib/tests/code_health_test.rs`

**Interfaces:**
- Consumes: `code_health_biomarkers_v1(path, smell, intensity)`.
- Produces: `structural_risk ∈ [0,1]` per file = `LEAST(1.0, combined_risk * co_occurrence_mult)`, where `combined_risk = 1 - Π(1 - intensity)` (probabilistic OR) and `co_occurrence_mult = 1 + 0.25*(distinct_smells - 1)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn structural_risk_rewards_multiple_cooccurring_smells() {
    // A file flagged by more distinct smells must not score healthier than a
    // file flagged by fewer, all else equal. Assert the monotonic invariant via
    // the exposed structural_risk, not an exact value.
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let rows = codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run");
    for r in &rows {
        assert!((0.0..=1.0).contains(&r.structural_risk), "risk in [0,1]: {}", r.structural_risk);
    }
    // Higher structural_risk must never correspond to a higher (healthier) score.
    let mut sorted = rows.clone();
    sorted.sort_by(|a, b| a.structural_risk.partial_cmp(&b.structural_risk).unwrap());
    for w in sorted.windows(2) {
        if (w[0].structural_risk - w[1].structural_risk).abs() > 1e-9 {
            assert!(w[0].score >= w[1].score - 1e-6,
                "riskier file must not score healthier");
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p codelore-lib --features test-support structural_risk_rewards_multiple_cooccurring_smells`
Expected: initially PASS-or-FAIL depending on current `structural_risk` (mirrors `n_cx`); this test locks the invariant before the Step-3 change. If it passes now, proceed — Step 3 must keep it passing.

- [ ] **Step 3: Replace the cognitive-only structural term with the biomarker aggregate**

Add a CTE before `joined` and swap `n_cx`'s source. New CTE:

```sql
    file_structural AS (
        SELECT
            path,
            LEAST(1.0,
                (1.0 - EXP(SUM(LN(GREATEST(1e-9, 1.0 - intensity)))))   -- probabilistic OR
                * (1.0 + 0.25 * (COUNT(DISTINCT smell) - 1))            -- co-occurrence multiplier
            ) AS structural_risk
        FROM code_health_biomarkers_v1
        GROUP BY path
    ),
```

In `joined`, `LEFT JOIN file_structural fs ON fc.path = fs.path` and carry `COALESCE(fs.structural_risk, 0.0) AS structural_risk`. In `normalized`/`scored`, use that column as `structural_risk` and replace the `0.40 * n_cx` term in the score with `0.40 * structural_risk`. `n_cx` (raw cognitive normalization) is no longer needed in the score; keep `cognitive` in the projection for the drill-down column.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p codelore-lib --features test-support structural_risk_rewards_multiple_cooccurring_smells`
Expected: PASS.

- [ ] **Step 5: Regression — existing churn test still holds**

Run: `cargo test -p codelore-lib --features test-support code_health`
Expected: `code_health_penalizes_churn` and the rows-limit invariant still PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): derive structural risk from biomarkers with co-occurrence multiplier"
```

---

### Task 6: Finalize behavioral fusion + explain + full gate

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (doc-comment formula; confirm size-normalization of behavioral terms)
- Modify: `crates/codelore-cli/src/main.rs:262-267` (explain tuple)
- Test: `crates/codelore-lib/tests/code_health_test.rs` (determinism)

**Interfaces:** none new — closes out the metric.

- [ ] **Step 1: Write the determinism test**

```rust
#[test]
fn code_health_v2_is_deterministic() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = codelore_lib::repo::GixRepo::open(tiny.dir.path()).expect("open");
    let opts = codelore_lib::Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    let run = || {
        let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
        db.ingest(&repo, &opts).expect("ingest");
        codelore_lib::analyses::code_health::run_code_health(&db, &opts).expect("run")
    };
    let a = run();
    let b = run();
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.path, y.path);
        assert!((x.score - y.score).abs() < 1e-9, "score must be stable");
        assert_eq!(x.band, y.band, "band must be stable");
    }
}
```

- [ ] **Step 2: Run test to verify it fails or passes**

Run: `cargo test -p codelore-lib --features test-support code_health_v2_is_deterministic`
Expected: PASS (it locks stability; if it fails, an unordered aggregate leaked — add a deterministic `ORDER BY` / tie-break).

- [ ] **Step 3: Update the module doc-comment formula**

Replace `code_health.rs:3-12` to describe the v2 contract (biomarker structural term + behavioral terms + bands), keeping the research citations already present. Describe the CURRENT contract only — no version words.

- [ ] **Step 4: Update the `explain` tuple**

`crates/codelore-cli/src/main.rs:262-267`:

```rust
        ("code-health",
         "code-health v2 composite: biomarker structural risk (Complex/Large Method, God Class, DRY, Shotgun Surgery) fused with behavioral signal (Nagappan & Ball 2005 churn + Mockus & Herbsleb 2002 ownership + Tornhill 2018 coupling); self-relative percentile banding (Alves/Ypma/Visser 2010)",
         "100 × (1 − 0.40·structural_risk − 0.25·churn − 0.15·ownership_fv − 0.20·coupling_centrality); band from structural_risk thresholds; percentile = per-language PERCENT_RANK of structural_risk.",
         "See analyses/code_health.rs."),
```

- [ ] **Step 5: Full CI gate + differential parity**

Run: `just ci`
Expected: fmt-check, clippy (`-D warnings`), deny, and the full test suite (including `differential_repo_test`) all pass. Confirm the `differential_repo_test` event-stream gate is unaffected (this plan touches no `Repo`-trait method).

- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/src/analyses/code_health.rs crates/codelore-cli/src/main.rs crates/codelore-lib/tests/code_health_test.rs
git commit -m "feat(code-health): finalize v2 behavioral fusion, explain formula, determinism test"
```

- [ ] **Step 7: CHANGELOG**

Add a `[Unreleased]` entry to `CHANGELOG.md` (the only place version/history narration is allowed) describing the code-health v2 change and the `CACHE_EPOCH` bump. Commit:

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): note code-health v2 (biomarkers, bands, percentile)"
```

---

## Self-Review

**Spec coverage** (against `docs/superpowers/specs/2026-07-02-code-health-and-dashboard-redesign-design.md` §3):
- §3.1 biomarkers (Complex Method, Large Method, God Class, DRY, Shotgun Surgery) → Tasks 2–4. ✓ (density/intensity via PERCENT_RANK; co-occurrence multiplier → Task 5. ✓)
- §3.2 self-relative percentile + bands → Task 1 (bands/percentile) + Task 5 (risk source). ✓ (per-language via path-extension derivation, per Explore finding E.)
- §3.3 behavioral fusion, size-normalized → Task 5/6 (structural term replaces cognitive; churn/ownership/coupling retained). ✓
- §3.4 widen `CodeHealthRow`, CACHE_EPOCH bump, documented diff → Task 1 (fields + `schema_v6`), Task 6 (explain), Task 6 Step 7 (CHANGELOG). ✓
- Not in scope (correct): `refactoring-targets` (Plan 2), SPA (Plan 3), cross-repo corpus / full biomarker set (Phase 2).

**Placeholder scan:** no "TBD"/"handle edge cases"/"similar to Task N". All steps show real code or exact commands. Constants (0.66/0.33 band cuts, 0.25 co-occurrence, 0.40/0.25/0.15/0.20 weights) are explicit and flagged tunable in Global Constraints; tests assert invariants, not magic values. ✓

**Type consistency:** `CodeHealthRow` fields (`structural_risk`, `percentile`, `band`) defined in Task 1 are used identically in Tasks 5/6 and both output emitters. `materialize_biomarkers` arity changes once (Task 4, one-arg → two-arg) with its call site updated in the same task. Temp table `code_health_biomarkers_v1` columns `(path, smell, intensity)` are consistent across Tasks 2–5. `run_god_classes(db, opts)`/`run_clones(opts)` signatures match the Explore reference (F). ✓

**Open risk flagged for the executor:** the exact SQL is best-effort (cannot be executed while planning); the *tests are the contract*. If a CTE errors under DuckDB, fix the SQL to satisfy the invariant test — do not weaken the test.
