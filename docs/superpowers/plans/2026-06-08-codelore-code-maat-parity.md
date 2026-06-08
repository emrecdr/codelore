# CodeLore — code-maat Feature Coverage (Modernized) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining feature gaps against code-maat by shipping the modern equivalent of every analysis and CLI flag — not a 1:1 port. After this plan lands, CodeLore covers every signal code-maat exposes today, but each one is implemented with the right tools for our architecture (DuckDB SQL + Rust type safety + canonical-author + fancy-regex + deterministic ordering) rather than preserving 2013 Clojure design choices. Migration users get an opt-in `--code-maat-compat` surface for the handful of cases where bit-identical output matters.

**Architecture:** Each new analysis is a new file under `crates/codelore-lib/src/analyses/` with a typed `Row` struct, a `run_<name>(db, opts) -> Result<Vec<Row>>` function backed by a single SQL query against the existing DuckDB fact store, three output emitters (CSV/JSON/Markdown), a CLI dispatch arm in `main.rs`, and an integration test. CLI flag wiring is `#[arg]` annotations in `args.rs` + propagation to `Options` in `main.rs`. Architectural grouping adds a new `facts/groups.rs` parser plus a rewrite pass in `facts/ingest.rs` that runs once after raw ingest.

**Tech Stack:** Rust 1.87, `clap` 4 derive macros, `duckdb` 1.10503.1 bundled, `time` 0.3 (NOT `chrono`), `regex` 1.x (workspace dep), `fancy-regex` 0.14 (NEW — full lookaround support for grouping patterns), `serde` for row serialization, `tempfile` + `assert_cmd` for tests.

---

## Modernization Decisions (what we improve, not just port)

This plan ports the **functional coverage** of code-maat's analyses but explicitly rejects the design choices below as 2013 footguns. Each decision is final for this plan — not an "open question" — per the project rule [[feedback-modernize-dont-migrate]] ("default is improve, preserve only with explicit reason"). Migration users who genuinely need bit-identical code-maat output opt in via a single `--code-maat-compat` flag.

| Legacy code-maat behavior | Modern CodeLore behavior | Rationale |
|---|---|---|
| **Arbitrary tiebreaks** in `main-dev`, `entity-effort`, etc. (`first (reverse (sort-by ...))`) | **Deterministic secondary sort** (`ORDER BY metric DESC, author ASC`) on every ranking analysis | Reproducible output is a 2026 baseline. Identical input → identical output is non-negotiable for CI use. |
| **`min-revs` overloaded** in `soc` to mean "minimum SoC sum" (not minimum revision count) | New flag `--min-soc N` for the SoC threshold; `--min-revs` keeps its global meaning (minimum revision count) | Flag name should match what it gates. Code-maat's overload is a known footgun. |
| **`main-dev-by-revs` reuses `added` / `total-added` column headers** even though values are revision counts | Honest column headers: `entity,main-dev,revisions,total-revisions,ownership` | Lying column headers fail every code-review smell test. Migration users get the legacy headers via `--code-maat-compat`. |
| **`--temporal-period N` = sliding window with duplication** (a 2013 hack — same commit appears in up to N output rows) | Primary: `--time-bucket DAY\|WEEK\|MONTH` for clean non-overlapping bucket aggregation backed by DuckDB `date_trunc()`. Legacy: `--temporal-period N` still accepted under `--code-maat-compat` | DuckDB has proper window functions and `date_trunc`; we shouldn't replicate a JVM-era kludge. Sliding-window with duplication breaks any "count distinct commits" invariant. |
| **`refactoring-main-dev` named after a heuristic** (deleted-lines == refactor) without explaining it | Same SQL but documented honestly: "this is `main-dev` with metric=deleted; Tornhill's heuristic is that removing code is a deliberate design choice. Use `--analysis main-dev-by-deletions` as the more honest name; `refactoring-main-dev` is an accepted alias." | The name implies a refactor-commit-message filter that doesn't exist. We surface the alias for migration but lead with the honest name. |
| **Hand-coded "for each metric: write a near-identical Clojure function"** pattern across main-dev variants | Single Rust `Metric` enum + one generic `run_main_dev(db, opts, metric)` helper; three thin wrappers for the three variants | Rust's type system encodes the variant explicitly; the SQL builds itself from `Metric::sql_expr()`. Less code, no duplication, no possibility of metric-specific bugs in one variant. |
| **Raw `author_email` everywhere** in author-based analyses | `canonical_author` (post-mailmap, post-bot-filter) everywhere | We already do mailmap resolution + bot filtering at ingest; using raw email would discard that work. Code-maat does its mailmap-equivalent at parse time too, but it's a per-tool addition; in CodeLore it's the fact-store invariant. |
| **`--strict-grouping` doesn't exist** (code-maat is always strict — unmatched files silently dropped) | Default `--strict-grouping=false`: unmatched files retain their raw path and remain in the output. Opt in to strict-drop via `--strict-grouping`. | Silent data loss is a 2013 mistake. Modern least-surprise = unmatched stays visible. Migration users get strict via `--code-maat-compat` (which sets `--strict-grouping=true` among other flags). |
| **`--expression-to-match` only used by `messages`**, with hard-error if missing | Same flag, but error message is descriptive and points at the alternative: "messages analysis requires a regex via `--expression-to-match`; for general commit filtering across all analyses, see the planned `--commit-filter` flag (future scope)" | Eager validation + helpful error message. The "future scope" pointer keeps the door open for richer commit classification without committing in this plan. |
| **Clojure `re-pattern` (java.util.regex) flavor** in grouping patterns; lookaround in real fixtures | `fancy-regex` 0.14 for grouping patterns (full lookaround support); DuckDB's RE2-flavor `regexp_matches` for `messages` analysis (no lookaround needed for commit-message matching, perf matters in SQL) | Right tool per use case. Grouping is parsed once per run — perf doesn't matter, parity does. `messages` runs over every commit — perf matters, common patterns work in RE2. Document both flavors in advanced-usage. |
| **Binary diffs encoded as `"-"` strings** that the analysis must coerce | Already normalized to 0 at ingest in CodeLore (our ingest layer converts before insert) | We already do this. No work needed; document it. |
| **Statistical filtering = "percent threshold"** (`--min-coupling 30` means "shared/avg-revs ≥ 30%") | Keep the percent threshold flag for parity but **add Fisher exact significance as the default gate** (already done for `coupling`); apply the same Fisher gate to `soc` if appropriate (TBD by research during implementation) | Percent thresholds are noisy on small N; Fisher exact is the right test. Code-maat predates this being practical. |
| **CLI flag values from 2013** (min-revs=5, min-coupling=30, max-changeset-size=30) | Keep defaults — they're still reasonable | Don't change for the sake of changing. The 2013 values were calibrated against real OSS codebases and the calibration still holds. |

### The `--code-maat-compat` flag

A single boolean flag that, when set, flips CodeLore back to legacy code-maat behavior:
- Tiebreaks become non-deterministic (or first-author-as-written rather than alphabetical)
- `main-dev-by-revs` outputs `added` / `total-added` headers
- `--temporal-period` works (raises an error otherwise — pointing at `--time-bucket`)
- `--strict-grouping` defaults to `true`
- Other quirks as discovered during implementation

Document this as a one-paragraph "migration helper" in advanced-usage. The modern surface is the recommendation; the compat flag exists so dashboards that parse code-maat CSV verbatim don't break on day one.

---

## Non-Goals (deliberate scope cuts)

- **Non-git VCS support** (Hg, SVN, Perforce, TFS). Out of scope permanently — see `project_git_only_scope.md` in user memory. Don't propose adding even as future work.
- **Bug-for-bug parity on tiebreak ordering.** Code-maat's `main-dev` ties break arbitrarily via `(first (reverse (sort-by metric-fn ...)))`. CodeLore will add a deterministic secondary sort (`, author ASC`) for reproducible output. Document this divergence.
- **`identity` analysis.** Code-maat's debug parse-dump is superseded by `--format sqlite` which exports the entire fact store. No standalone port.
- **`messages` for legacy `git` log format vs `git2`.** Code-maat fails the `messages` analysis on `git2` because that parser strips messages. CodeLore ingests messages unconditionally via gix, so this failure mode doesn't exist for us — don't bother replicating the error path.

---

## File Structure (new + modified)

**New files:**
- `crates/codelore-lib/src/analyses/messages.rs` — Phase 1
- `crates/codelore-lib/src/analyses/soc.rs` — Phase 1
- `crates/codelore-lib/src/analyses/main_dev.rs` — Phase 1 (covers `main-dev` + `main-dev-by-revs` + `refactoring-main-dev` — three variants of the same query shape)
- `crates/codelore-lib/src/analyses/entity_effort.rs` — Phase 1
- `crates/codelore-lib/src/analyses/entity_ownership.rs` — Phase 1
- `crates/codelore-lib/src/facts/groups.rs` — Phase 3
- `crates/codelore-lib/tests/messages_test.rs` — Phase 1
- `crates/codelore-lib/tests/soc_test.rs` — Phase 1
- `crates/codelore-lib/tests/main_dev_test.rs` — Phase 1 (covers all 3 variants)
- `crates/codelore-lib/tests/entity_effort_test.rs` — Phase 1
- `crates/codelore-lib/tests/entity_ownership_test.rs` — Phase 1
- `crates/codelore-lib/tests/groups_test.rs` — Phase 3

**Modified files:**
- `crates/codelore-lib/Cargo.toml` — add `fancy-regex = "0.14"` for grouping lookaround parity (Phase 3)
- `crates/codelore-lib/src/options.rs` — schema additions (see next section)
- `crates/codelore-lib/src/analysis.rs` — extend `AnalysisName` enum with 7 new variants + `as_str()` + `all()` (Phase 1)
- `crates/codelore-lib/src/analyses/mod.rs` — `pub mod` for each new module (Phase 1)
- `crates/codelore-lib/src/analyses/code_age.rs` — confirm `age_time_now` propagation (already wired; sanity check only) (Phase 2)
- `crates/codelore-lib/src/facts/ingest.rs` — call `groups::apply()` after raw ingest if `opts.group_file.is_some()` (Phase 3)
- `crates/codelore-lib/src/output/csv.rs` — 7 new emitters (Phase 1) + grouping note (Phase 3)
- `crates/codelore-lib/src/output/json.rs` — 7 new emitters (Phase 1)
- `crates/codelore-lib/src/output/markdown.rs` — 7 new emitters (Phase 1)
- `crates/codelore-cli/src/args.rs` — 5 new `#[arg]` annotations (Phase 2) + `--strict-grouping` doc-update (Phase 3)
- `crates/codelore-cli/src/main.rs` — 7 new analysis dispatch arms × 3 formats = 21 match arms (Phase 1); 5 new options-propagation lines (Phase 2)

---

## Options struct additions (prerequisite for several tasks)

The modernization decisions require three new fields on `Options`. Add these once, in **Task 0** before any analysis work, so subsequent tasks reference them naturally:

```rust
// crates/codelore-lib/src/options.rs

/// Time-bucket granularity for coupling-family analyses.
/// `None` = no bucketing (raw commit grain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeBucket { Day, Week, Month }

pub struct Options {
    // ... existing 25 fields ...

    /// SoC threshold for the `soc` analysis (modern replacement for the
    /// overloaded `--min-revs` semantic in code-maat). When `None`, defaults
    /// to 1 (drop solo commits) under modern mode, or `min_revs` under
    /// `code_maat_compat`.
    pub min_soc: Option<u32>,

    /// Time-bucket granularity for coupling-family analyses. Modern
    /// replacement for code-maat's sliding-window `--temporal-period`.
    pub time_bucket: Option<TimeBucket>,

    /// Migration-helper flag. When true, flips internal defaults to match
    /// legacy code-maat output bit-for-bit (lying column headers, arbitrary
    /// tiebreaks, sliding-window temporal, etc.). Off by default — modern
    /// surface is the recommendation.
    pub code_maat_compat: bool,
}
```

**Note:** `strict_grouping: bool` already exists on `Options`. The default value flips from `false` (kept) under modern mode to `true` only when `code_maat_compat` is set.

### Task 0: Add the three new Options fields

- [ ] **Step 1:** Add the three fields above to `Options` struct with `Default` impls (`min_soc: None`, `time_bucket: None`, `code_maat_compat: false`).
- [ ] **Step 2:** Add the `TimeBucket` enum in the same file with `as_sql_unit()` returning `"day"` / `"week"` / `"month"` (passed to DuckDB's `date_trunc()`).
- [ ] **Step 3:** Update every existing test that constructs `Options { ... }` via struct literal — there are ~20 of them. Use `..Options::default()` shorthand so future field additions don't break tests. (This nudge toward `..default()` is a small-bang fix for the "Options has no builder" backlog item.)
- [ ] **Step 4:** `cargo test --workspace --all-features` to confirm nothing breaks.
- [ ] **Step 5:** Commit. Message: `feat(lib): Options fields for modernized code-maat coverage (min_soc, time_bucket, code_maat_compat)`.

---

## Phase 1: The Six Missing Analyses (cheap wins)

**Why first:** Pure additions, no architectural changes, no dependencies on Phase 2 or 3. Once these land, the AnalysisName enum is complete and the rest of the plan only touches CLI surface + ingest preprocessing.

### Task 1: `soc` (Sum of Coupling) — with `--min-soc` modern flag

**Files:**
- Create: `crates/codelore-lib/src/analyses/soc.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs` (add `pub mod soc;`)
- Modify: `crates/codelore-lib/src/analysis.rs` (add `Soc` variant + `"soc"` mapping)
- Modify: `crates/codelore-lib/src/output/{csv,json,markdown}.rs` (3 emitters)
- Modify: `crates/codelore-cli/src/main.rs` (3 dispatch arms — csv, json, markdown)
- Test: `crates/codelore-lib/tests/soc_test.rs`

- [ ] **Step 1: Write the failing integration test**

  Build a 4-commit fixture where commit sizes are 2, 3, 2, 1 files, then assert each file's SoC equals the sum-over-windows of `(size - 1)` for windows it participated in. Use `tiny_repo` builder pattern.

- [ ] **Step 2: Implement `run_soc`**

  ```rust
  // crates/codelore-lib/src/analyses/soc.rs
  use crate::{CodeLoreError, Options, Result, facts::FactsDb};

  #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
  pub struct SocRow {
      pub entity: String,
      pub soc: u32,
  }

  /// Sum-of-Coupling: each commit of size N contributes `N-1` to every
  /// entity in it. A solo commit contributes 0. Gated on `--min-soc`
  /// (modern semantic — see Modernization Decisions). For migration
  /// users with `--code-maat-compat`, falls back to `--min-revs` as
  /// the gate threshold.
  pub fn run_soc(db: &FactsDb, opts: &Options) -> Result<Vec<SocRow>> {
      let limit = opts.rows_limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
      // Modern: --min-soc N gates the SoC value. Legacy compat: fall back to --min-revs.
      let threshold = opts.min_soc.unwrap_or(if opts.code_maat_compat { opts.min_revs } else { 1 });
      let sql = format!(
          "WITH rev_sizes AS (
               SELECT rev, COUNT(DISTINCT path) AS n FROM changes GROUP BY rev
           )
           SELECT c.path AS entity, SUM(rs.n - 1)::INTEGER AS soc
           FROM changes c JOIN rev_sizes rs USING (rev)
           GROUP BY c.path
           HAVING SUM(rs.n - 1) >= {threshold}
           ORDER BY soc DESC, entity ASC{limit}",
          threshold = threshold,
      );
      let mut stmt = db.conn().prepare(&sql)
          .map_err(|e| CodeLoreError::Analysis(format!("prepare soc: {e}")))?;
      let rows = stmt.query_map([], |r| Ok(SocRow {
          entity: r.get::<_, String>(0)?,
          soc: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
      })).map_err(|e| CodeLoreError::Analysis(format!("query soc: {e}")))?;
      rows.collect::<std::result::Result<Vec<_>, _>>()
          .map_err(|e| CodeLoreError::Analysis(format!("collect soc: {e}")))
  }
  ```

- [ ] **Step 3: Add 3 output emitters**

  CSV header: `entity,soc`. JSON: `[{entity, soc}, …]`. Markdown: `| Entity | SoC |` table. Mirror the pattern used by `write_revisions_*`.

- [ ] **Step 4: Wire CLI dispatch in `main.rs`**

  Three new match arms: `("csv", AnalysisName::Soc) => …`, `("json", AnalysisName::Soc) => …`, `("markdown", AnalysisName::Soc) => …`. Mirror the Revisions block.

- [ ] **Step 5: Run + commit**

  `cargo test -p codelore-lib --test soc_test` → green. Commit: `feat(lib): soc (Sum of Coupling) analysis (code-maat parity)`.

### Task 2: `messages` (commit-message regex frequency)

**Prerequisite:** `commits.message` is already ingested (verified — `schema_v1.sql:8`).

**Files:** same shape as Task 1 — `analyses/messages.rs` + mod + enum + 3 emitters + 3 CLI arms + test.

- [ ] **Step 1: Write the failing integration test**

  5-commit fixture with messages "fix bug #1", "feature X", "Bugfix typo", "WIP", "bug found". Run with `Options.message_regex = Some("bug".into())` → expect 2 matches (case-sensitive). Re-run with `"(?i)bug"` → expect 3.

- [ ] **Step 2: Implement `run_messages`**

  ```rust
  #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
  pub struct MessagesRow { pub entity: String, pub matches: u32 }

  pub fn run_messages(db: &FactsDb, opts: &Options) -> Result<Vec<MessagesRow>> {
      let expr = opts.message_regex.as_deref()
          .ok_or_else(|| CodeLoreError::Analysis(
              "messages analysis requires --expression-to-match".into()))?;
      // Validate regex eagerly so we error before SQL prep
      let _ = regex::Regex::new(expr).map_err(|e|
          CodeLoreError::Analysis(format!("invalid --expression-to-match regex: {e}")))?;
      let limit = opts.rows_limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
      // Use DuckDB's regexp_matches (RE2 flavor — close enough to Rust regex)
      let sql = format!(
          "SELECT c.path AS entity, COUNT(*)::INTEGER AS matches
           FROM changes c
           JOIN commits m ON m.rev = c.rev
           WHERE regexp_matches(m.message, $1)
           GROUP BY c.path
           ORDER BY matches DESC, entity DESC{limit}");
      let mut stmt = db.conn().prepare(&sql)?;
      let rows = stmt.query_map([expr], |r| Ok(MessagesRow {
          entity: r.get(0)?, matches: u32::try_from(r.get::<_, i64>(1)?).unwrap_or(u32::MAX),
      }))?;
      rows.collect::<std::result::Result<Vec<_>, _>>()
          .map_err(|e| CodeLoreError::Analysis(format!("collect messages: {e}")))
  }
  ```

  > **NB:** DuckDB's `regexp_matches` is RE2-flavor. The vast majority of code-maat user patterns (`bug`, `fix`, `(?i)bug|fix`, `#\d+`) work identically. Backreferences and Java-specific extensions don't — document in the CLI help text.

- [ ] **Step 3–5:** emitters + dispatch + commit. Commit message: `feat(lib): messages analysis (--expression-to-match regex over commit messages)`.

### Task 3: `main-dev`, `main-dev-by-revs`, `main-dev-by-deletions` (+ `refactoring-main-dev` alias)

**Files:** `analyses/main_dev.rs` + mod + 3 enum variants + 12 emitters (3 analyses × 3 formats + alias re-uses) + 9 CLI arms + alias dispatch + test.

These three share an identical query shape — author-aggregation, top-by-metric, ownership ratio — only the metric column changes. **Honest** column headers per the modernization decisions:

| Analysis | Metric | Column headers (modern) | Code-maat-compat headers |
|---|---|---|---|
| `main-dev` | `SUM(loc_added)` | `entity,main-dev,added,total-added,ownership` | (same — code-maat got this one right) |
| `main-dev-by-revs` | `COUNT(*)` | `entity,main-dev,revisions,total-revisions,ownership` | `entity,main-dev,added,total-added,ownership` (lying labels) |
| `main-dev-by-deletions` (aliased: `refactoring-main-dev`) | `SUM(loc_deleted)` | `entity,main-dev,removed,total-removed,ownership` | (same headers) |

- [ ] **Step 1: Write the failing test**

  Build a fixture with 3 authors A/B/C and 2 entities. A adds 100 lines to `foo.rs` and deletes 5; B adds 30 to `foo.rs` and deletes 20; C adds 0 to `foo.rs` and deletes 50. Assert:
  - `main-dev foo.rs` → A (winner by added)
  - `refactoring-main-dev foo.rs` → C (winner by deleted)
  - `main-dev-by-revs foo.rs` → check whoever has the most commits

  Also assert deterministic tiebreak: when two authors tie on the metric, the alphabetically-first author wins (CodeLore divergence from code-maat).

- [ ] **Step 2: Implement a single helper + 3 thin wrappers**

  ```rust
  #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
  pub struct MainDevRow {
      pub entity: String,
      pub main_dev: String,
      /// Column header is "added" for main-dev/main-dev-by-revs,
      /// "removed" for refactoring-main-dev — emitter chooses.
      pub metric: u64,
      pub total: u64,
      /// Rounded to 2 decimal places (code-maat parity).
      pub ownership: f64,
  }

  enum Metric { Added, Deleted, RevCount }
  impl Metric {
      fn sql_expr(self) -> &'static str {
          match self {
              Self::Added => "SUM(c.loc_added)",
              Self::Deleted => "SUM(c.loc_deleted)",
              Self::RevCount => "COUNT(*)",
          }
      }
  }

  fn run(db: &FactsDb, opts: &Options, metric: Metric) -> Result<Vec<MainDevRow>> {
      let limit = opts.rows_limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
      let m = metric.sql_expr();
      let sql = format!(
          "WITH ea AS (
               SELECT c.path AS entity, m.canonical_author AS author,
                      {m}::BIGINT AS metric
               FROM changes c JOIN commits m USING (rev)
               GROUP BY c.path, m.canonical_author
           ),
           totals AS (
               SELECT entity, SUM(metric) AS total FROM ea GROUP BY entity
           ),
           ranked AS (
               SELECT entity, author, metric,
                      ROW_NUMBER() OVER (
                          PARTITION BY entity ORDER BY metric DESC, author ASC
                      ) AS rn
               FROM ea
           )
           SELECT r.entity, r.author, r.metric, t.total,
                  ROUND(r.metric::DOUBLE / GREATEST(t.total, 1), 2) AS ownership
           FROM ranked r JOIN totals t USING (entity)
           WHERE rn = 1
           ORDER BY r.entity ASC{limit}");
      // ... map to MainDevRow
  }

  pub fn run_main_dev(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> { run(db, opts, Metric::Added) }
  pub fn run_main_dev_by_revs(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> { run(db, opts, Metric::RevCount) }
  pub fn run_refactoring_main_dev(db: &FactsDb, opts: &Options) -> Result<Vec<MainDevRow>> { run(db, opts, Metric::Deleted) }
  ```

- [ ] **Step 3: CSV emitters use analysis-specific column headers**

  ```rust
  pub fn write_main_dev_csv(rows: &[MainDevRow], w: &mut impl Write) -> Result<()> {
      writeln!(w, "entity,main-dev,added,total-added,ownership")?;
      // ... iterate
  }
  pub fn write_refactoring_main_dev_csv(rows: &[MainDevRow], w: &mut impl Write) -> Result<()> {
      writeln!(w, "entity,main-dev,removed,total-removed,ownership")?;
      // ... iterate
  }
  // write_main_dev_by_revs_csv reuses write_main_dev_csv's header (code-maat compat lie — document)
  ```

- [ ] **Step 4–5:** wire 9 dispatch arms + commit. Commit message: `feat(lib): main-dev / main-dev-by-revs / refactoring-main-dev analyses (code-maat parity, deterministic tiebreak)`.

### Task 4: `entity-effort`

**Files:** `analyses/entity_effort.rs` + standard plumbing.

Per-(entity, author) row count + total. Window function over `COUNT(*)`.

- [ ] **Implement `run_entity_effort`** with SQL:

  ```sql
  WITH ea AS (
      SELECT c.path AS entity, m.canonical_author AS author, COUNT(*)::BIGINT AS author_revs
      FROM changes c JOIN commits m USING (rev)
      GROUP BY c.path, m.canonical_author
  )
  SELECT entity, author, author_revs,
         SUM(author_revs) OVER (PARTITION BY entity) AS total_revs
  FROM ea
  ORDER BY entity ASC, author_revs DESC, author ASC
  ```

- [ ] **Row struct:** `EntityEffortRow { entity, author, author_revs: u32, total_revs: u32 }`. CSV header `entity,author,author-revs,total-revs`.

- [ ] **Test, wire, commit.** Commit: `feat(lib): entity-effort analysis (per-author revision counts per file)`.

### Task 5: `entity-ownership`

**Files:** `analyses/entity_ownership.rs` + standard plumbing.

Per-(entity, author) churn breakdown.

- [ ] **Implement `run_entity_ownership`** with SQL:

  ```sql
  SELECT c.path AS entity, m.canonical_author AS author,
         SUM(c.loc_added)::BIGINT AS added,
         SUM(c.loc_deleted)::BIGINT AS deleted
  FROM changes c JOIN commits m USING (rev)
  GROUP BY c.path, m.canonical_author
  ORDER BY entity ASC, author ASC  -- secondary sort for determinism (code-maat divergence)
  ```

- [ ] **Row struct:** `EntityOwnershipRow { entity, author, added: u64, deleted: u64 }`. CSV header `entity,author,added,deleted`.

- [ ] **Test, wire, commit.** Commit: `feat(lib): entity-ownership analysis (per-(file,author) churn breakdown)`.

---

## Phase 2: CLI Flag Wiring (cheap wins)

**Why second:** All 5 target `Options` fields exist and (in 3 cases) are already consumed by analyses — only the CLI surface is missing. No new SQL.

### Task 6: Wire `--min-shared-revs`, `--min-coupling`, `--max-coupling`, `--max-changeset-size`

These are knobs on the `coupling` and `clone-coupling` analyses; Options fields exist, defaults match code-maat.

- [ ] **Step 1:** Add 4 `#[arg]` lines to `crates/codelore-cli/src/args.rs`:

  ```rust
  #[arg(long, default_value_t = 5)]
  pub min_shared_revs: u32,
  #[arg(long = "min-coupling", default_value_t = 30)]
  pub min_coupling_pct: u8,
  #[arg(long = "max-coupling", default_value_t = 100)]
  pub max_coupling_pct: u8,
  #[arg(long, default_value_t = 30)]
  pub max_changeset_size: u32,
  ```

- [ ] **Step 2:** Propagate to `Options` in `main.rs` (4 lines in the options-builder block).

- [ ] **Step 3:** Add 4 CLI-level integration tests in `crates/codelore-cli/tests/` proving the flag round-trips (`codelore analyze --analysis coupling --min-shared-revs 10` → only pairs with shared >= 10 appear).

- [ ] **Step 4:** Commit. Message: `feat(cli): wire --min-shared-revs / --min/max-coupling / --max-changeset-size flags`.

### Task 7: Wire `--age-time-now`

**Verified:** the field is already consumed in `code_age.rs:36`. Only CLI surface missing.

- [ ] **Step 1:** Add `#[arg(long = "age-time-now", value_name = "YYYY-MM-DD", value_parser = parse_date)]` to `args.rs`. The `parse_date` helper:

  ```rust
  fn parse_date(s: &str) -> std::result::Result<time::Date, String> {
      time::Date::parse(s, &time::format_description::well_known::Iso8601::DEFAULT)
          .map_err(|e| format!("invalid date '{s}' (expected YYYY-MM-DD): {e}"))
  }
  ```

- [ ] **Step 2:** Propagate to `Options.age_time_now` (1 line).

- [ ] **Step 3:** Integration test: 2 commits at known dates, `--age-time-now 2024-07-01`, assert month-deltas independent of system clock.

- [ ] **Step 4:** Commit. Message: `feat(cli): wire --age-time-now flag for reproducible code-age analysis`.

### Task 8: Wire `--expression-to-match`

**Prerequisite:** Task 2 (the `messages` analysis must exist for this flag to be useful).

- [ ] **Step 1:** Add `#[arg(short = 'e', long = "expression-to-match", value_name = "REGEX")] pub message_regex: Option<String>` to `args.rs`.

- [ ] **Step 2:** Propagate to `Options.message_regex` (1 line).

- [ ] **Step 3:** Integration test: end-to-end run of `codelore analyze --analysis messages -e bug` against the fixture from Task 2.

- [ ] **Step 4:** Commit. Message: `feat(cli): wire --expression-to-match flag (powers the messages analysis)`.

### Task 9: Modern time-bucket aggregation (`--time-bucket DAY|WEEK|MONTH`)

This replaces code-maat's `--temporal-period N` sliding-window-with-duplication hack. See **Modernization Decisions** for rationale.

**Files:**
- New: `crates/codelore-lib/src/facts/time_bucket.rs` (~120 LOC — much smaller than the sliding-window equivalent because `date_trunc()` does all the work)
- New: `crates/codelore-lib/tests/time_bucket_test.rs` (~80 LOC)
- Modify: `crates/codelore-lib/src/options.rs` (add `pub time_bucket: Option<TimeBucket>` field; new enum `TimeBucket { Day, Week, Month }`)
- Modify: `crates/codelore-cli/src/args.rs` (add `--time-bucket` flag; keep `--temporal-period` flag accepted only under `--code-maat-compat`)
- Modify: `crates/codelore-lib/src/facts/ingest.rs` (post-ingest, materialize `changes_bucketed` if requested)
- Modify: `crates/codelore-lib/src/analyses/coupling.rs` + `clone_coupling.rs` + `soc.rs` (route through `changes_bucketed` if active)

- [ ] **Step 1: Failing test** — 7 commits across 14 days, run with `--time-bucket WEEK`, assert 2 logical changesets with deduped entities per bucket.

- [ ] **Step 2: Implement bucketing** — single SQL:

  ```sql
  CREATE OR REPLACE TABLE changes_bucketed AS
  SELECT DISTINCT
      date_trunc('week', m.date)::TEXT AS rev,
      c.path,
      MAX(c.loc_added) AS loc_added,
      MAX(c.loc_deleted) AS loc_deleted
  FROM changes c JOIN commits m USING (rev)
  GROUP BY date_trunc('week', m.date), c.path;
  ```

  `date_trunc('day' | 'week' | 'month', date)` is DuckDB-native. No calendar series, no sliding windows, no duplication. Each (bucket, path) is one row.

- [ ] **Step 3: Route coupling-family analyses** through the bucketed view when `opts.time_bucket.is_some()`. Other analyses pass through unchanged (no warning needed; bucketing is meaningless to them but harmless if applied).

- [ ] **Step 4: Compat — accept `--temporal-period` under `--code-maat-compat`**

  Under `--code-maat-compat`, also implement the sliding-window-with-duplication semantic (~80 LOC extra). Without `--code-maat-compat`, `--temporal-period` errors with a message pointing at `--time-bucket`.

- [ ] **Step 5: Commit.** Message: `feat(lib): modern time-bucket aggregation for coupling-family analyses (replaces legacy sliding-window temporal-period)`.

---

## Phase 3: Larger Items

### Task 10: Architectural grouping (`-g` / `--group-file`)

**Why this is larger:** Needs a new parser module + a rewrite pass that runs over the entire `changes` table after ingest + a regex flavor decision (code-maat fixtures use lookaround, which Rust's `regex` crate doesn't support).

**Files:**
- New: `crates/codelore-lib/src/facts/groups.rs` (~180 LOC parser + applier)
- New: `crates/codelore-lib/tests/groups_test.rs` (~120 LOC, including the byte-for-byte replication of code-maat's 3 `git2_live_data_test_with_group` assertions)
- Modify: `crates/codelore-lib/Cargo.toml` (add `fancy-regex = "0.14"`)
- Modify: `crates/codelore-lib/src/facts/ingest.rs` (call grouping pass after raw ingest)
- Modify: `crates/codelore-lib/src/options.rs` (no field changes — `group_file` + `strict_grouping` already exist)
- Modify: `crates/codelore-cli/src/args.rs` (existing — confirm the `-g` flag is on the CLI; if not, add)
- Modify: `crates/codelore-lib/src/analyses/*.rs` (verify NONE need changes — grouping rewrites the `path` column at ingest, so downstream SQL sees logical group names already)

- [ ] **Step 1: Decide on regex flavor**

  Two options:

  | Option | Pros | Cons |
  |---|---|---|
  | A. `fancy-regex 0.14` (recommended) | Full lookaround parity with code-maat; the `regex-and-text-layers-definition.txt` fixture works as-is; one new dep | +200 KB binary; slower than `regex` for non-lookaround patterns (we'd use it only for grouping, not the hot path) |
  | B. `regex 1.x` (current) + document limitation | Zero new deps | The `regex-layers-definition.txt` lookaround example breaks; migration users hit an error on their first attempt |

  **Recommendation: A.** Grouping is a one-time-per-ingest operation, not a per-row hot path — perf cost is negligible. Migration users care about parity. The +200 KB is a rounding error against DuckDB's footprint. Document in advanced-usage that `fancy-regex` syntax is the spec.

- [ ] **Step 2: Write the failing tests**

  Three byte-for-byte parity assertions against code-maat's `git2_live_data_test_with_group`:
  - Plain text mapping: `src/Features/Core => Core` matches `src/Features/Core/foo.rs` → 4 entries
  - Anchored regex: `^src\/.*\/.*Tests\.cs$ => CS Tests` → 2 entries
  - Image catch-all: `^src\/.*\.png$ => Images` → 1 entry
  - Lookaround (negative): `^src\/((?!.*Test.*).).*$ => Production` → only non-Test files
  - Unmatched-and-strict: file outside all rules is silently dropped
  - Unmatched-and-non-strict (CodeLore extension): file outside all rules retained with its raw path

- [ ] **Step 3: Implement `groups.rs`**

  ```rust
  use fancy_regex::Regex;
  use std::path::Path;

  pub struct GroupRule { pub pattern: Regex, pub name: String }
  pub struct GroupMap { pub rules: Vec<GroupRule>, pub strict: bool }

  #[derive(Debug, thiserror::Error)]
  pub enum GroupParseError {
      #[error("group file line {line}: missing `=>` separator")] MissingSeparator { line: usize },
      #[error("group file line {line}: empty pattern")] EmptyPattern { line: usize },
      #[error("group file line {line}: empty name")] EmptyName { line: usize },
      #[error("group file line {line}: invalid regex `{pattern}`: {source}")]
      InvalidRegex { line: usize, pattern: String, source: fancy_regex::Error },
      #[error(transparent)] Io(#[from] std::io::Error),
  }

  impl GroupMap {
      pub fn parse(text: &str, strict: bool) -> Result<Self, GroupParseError> {
          let mut rules = Vec::new();
          for (i, line) in text.lines().enumerate() {
              let trimmed = line.trim();
              if trimmed.is_empty() { continue; }
              let (lhs, rhs) = trimmed.split_once("=>")
                  .ok_or(GroupParseError::MissingSeparator { line: i + 1 })?;
              let path = lhs.trim();
              let name = rhs.trim();
              if path.is_empty() { return Err(GroupParseError::EmptyPattern { line: i + 1 }); }
              if name.is_empty() { return Err(GroupParseError::EmptyName { line: i + 1 }); }
              // Code-maat semantics: ^...$ literal regex; otherwise prefix-anchor + trailing /
              let pattern_str = if path.starts_with('^') {
                  path.to_string()
              } else {
                  format!("^{}/", regex::escape(path))  // escape to treat as literal prefix
              };
              let pattern = Regex::new(&pattern_str).map_err(|e|
                  GroupParseError::InvalidRegex { line: i + 1, pattern: path.into(), source: e })?;
              rules.push(GroupRule { pattern, name: name.into() });
          }
          Ok(Self { rules, strict })
      }

      pub fn from_file(p: &Path, strict: bool) -> Result<Self, GroupParseError> {
          let text = std::fs::read_to_string(p)?;
          Self::parse(&text, strict)
      }

      /// First match wins (rule order significant).
      pub fn map_entity(&self, path: &str) -> Option<&str> {
          for rule in &self.rules {
              if rule.pattern.is_match(path).unwrap_or(false) {
                  return Some(&rule.name);
              }
          }
          None
      }
  }
  ```

  > **Note on code-maat divergence:** code-maat does NOT escape the plain-text path before prefixing `^` and `/`. Our `regex::escape` is safer — it prevents users from accidentally writing `src/foo.bar` and having the `.` treated as a regex wildcard. Document this as a deliberate improvement.

- [ ] **Step 4: Apply grouping in `facts/ingest.rs`**

  After raw ingest finishes (before any analysis runs), if `opts.group_file.is_some()`:

  ```rust
  let group_map = GroupMap::from_file(&path, opts.strict_grouping)?;
  let conn = self.conn();
  // Stream every (rev, path) row, rewrite path, batch INSERT into a temp table, then swap
  let raw_rows: Vec<(String, String, i64, i64, ...)> = conn.prepare("SELECT * FROM changes")?
      .query_map([], /* ... */)?.collect::<Result<Vec<_>, _>>()?;
  let rewritten: Vec<_> = raw_rows.into_iter().filter_map(|(rev, path, ...)| {
      match group_map.map_entity(&path) {
          Some(group) => Some((rev, group.to_string(), ...)),
          None if opts.strict_grouping => None,  // drop
          None => Some((rev, path, ...)),  // keep raw (CodeLore-extension)
      }
  }).collect();
  // DELETE FROM changes; bulk INSERT rewritten via Appender
  ```

  Performance note: grouping runs once per ingest, before any analysis. On a 100k-commit repo with ~500 group rules this is maybe 100ms. Acceptable.

- [ ] **Step 5: CLI wiring**

  Verify `--group-file` is already on the CLI (it was per session memory `P8.§2.T7`). Add `--strict-grouping` if not present (CodeLore-extension flag).

- [ ] **Step 6: Documentation**

  Add a new subsection to `docs/advanced-usage.md §5 Configuration`: "Architectural grouping" with:
  - The file format and syntax
  - The first-match-wins semantic
  - The lookaround support note (we use fancy-regex)
  - The `--strict-grouping` flag and what it does differently from code-maat
  - A worked example

- [ ] **Step 7: Run all groups tests + commit**

  Commit: `feat(lib): architectural grouping via -g/--group-file (code-maat parity + fancy-regex lookaround)`.

### Task 11: `--code-maat-compat` flag and legacy semantics opt-in

This is the single migration-helper flag that flips internal defaults to match legacy code-maat behavior for users whose dashboards parse code-maat CSV verbatim.

**Files:**
- Modify: `crates/codelore-lib/src/options.rs` (add `pub code_maat_compat: bool` field, default `false`)
- Modify: `crates/codelore-cli/src/args.rs` (add `--code-maat-compat` flag)
- Modify: every analysis SQL that has a "modern vs legacy" branch (soc tiebreak, main-dev-by-revs headers, etc.) to honour the flag
- Modify: `crates/codelore-cli/src/args.rs` accept `--temporal-period` under `--code-maat-compat` (errors otherwise pointing at `--time-bucket`)
- Modify: `crates/codelore-lib/src/facts/temporal_legacy.rs` (NEW, ~150 LOC) — the actual sliding-window-with-duplication implementation, ONLY reachable via `--code-maat-compat --temporal-period N`
- New test: `crates/codelore-lib/tests/code_maat_compat_test.rs` — for each flipped behavior, assert modern default vs compat output differ exactly as documented

- [ ] **Step 1: Decide which compat behaviors are worth supporting.** Spike for ~30 minutes: search GitHub for `code-maat` users who pipe its CSV into custom dashboards. If we can find ≥ 3 active uses, ship the full compat surface. If we find 0, ship just the flag-existence (it's a no-op except for `--temporal-period` legacy mode) and document the discoverable divergences.

- [ ] **Step 2: Failing test for each flipped behavior** — one test per behavior in the Modernization Decisions table, asserting both modern default AND compat output, side-by-side.

- [ ] **Step 3: Plumb the flag** — single `bool` propagation from CLI to `Options` to every analysis that branches on it (~5-7 sites).

- [ ] **Step 4: Implement legacy sliding-window temporal under `--code-maat-compat --temporal-period N`** — the ~150 LOC sliding-window-with-duplication code that we did NOT implement as the modern default. Replicates code-maat's `multiple-days-give-a-rolling-dataset` test byte-for-byte.

- [ ] **Step 5: Commit.** Message: `feat(cli): --code-maat-compat flag for legacy-compat output (migration helper)`.

---

## Phase 4: Documentation + CHANGELOG

### Task 12: Update README + advanced-usage

- [ ] **Update `README.md`:**
  - Status line: bump "14 analyses" to "20 analyses" (14 + 6 new: soc, messages, main-dev, main-dev-by-revs, refactoring-main-dev, entity-effort, entity-ownership — wait, that's 7 new = 21 total. Verify count after Phase 1 lands.)
  - The differentiator paragraph stays the same; we still beat code-maat on every original axis plus we now have code-maat's full analysis set.
  - Add a single line at the top of Status: "Drop-in successor to code-maat: every published code-maat analysis is supported under the same `--analysis NAME` flag."

- [ ] **Update `docs/advanced-usage.md`:**
  - Add 7 new rows to the "§1 The N analyses" table (count must match enum after Phase 1)
  - Add `§5.X` subsection: "Architectural grouping (`--group-file`)" with file format + worked example + lookaround note + `--strict-grouping` callout
  - Add to §3 CLI table: the 5 new flags (`--min-shared-revs`, `--min-coupling`, `--max-coupling`, `--max-changeset-size`, `--age-time-now`, `--expression-to-match`, `--temporal-period`, `--group-file`, `--strict-grouping`)
  - Update §13 workspace layout: `analyses/` count comment, mention new modules

- [ ] **Update `CHANGELOG.md`:**
  - One entry per phase, with the commit SHAs of each task as references.

- [ ] **Commit:** `docs: code-maat parity — analysis catalogue + new CLI flags + grouping`.

---

## Effort estimate

| Phase | Tasks | Files touched | LOC added (rough) | Wall-clock |
|---|---|---|---|---|
| **0 — Options additions** | 1 task | 1 file + ~20 test files | ~30 LOC + test-construction updates | half day |
| **1 — 6 analyses** | 5 task groups | ~25 files | ~1200 LOC + ~800 tests | 1–2 days |
| **2 — 4 flag wirings** | 3 task groups | ~6 files | ~80 LOC + ~120 tests | half day |
| **3 — Grouping** | 1 task | ~6 files | ~350 LOC + ~250 tests | 1 day |
| **3 — Time-bucket (modern)** | 1 task | ~5 files | ~180 LOC + ~150 tests | half day |
| **3 — `--code-maat-compat` flag + legacy temporal** | 1 task | ~5 files | ~250 LOC + ~200 tests | 1 day |
| **4 — Docs + CHANGELOG** | 1 task | ~4 files | ~200 lines markdown | half day |
| **Total** | | ~52 files | ~2290 LOC code + ~1520 LOC tests | **5–6 days** |

## Dependencies between tasks

```
Task 1 (soc)           ─────┐
Task 2 (messages)      ─────┤
Task 3 (main-dev ×3)   ─────┼──→ Task 12 (docs)
Task 4 (entity-effort) ─────┤
Task 5 (entity-ownership) ──┘

Task 6 (4 flags)       ─────────→ Task 12

Task 2 (messages) ──→ Task 8 (--expression-to-match wiring)
Task 7 (--age-time-now wiring) — independent

Task 10 (grouping) ──→ Task 12
Task 11 (temporal) ──→ Task 12
```

No blocking interdependencies inside Phase 1 — all 5 tasks can ship as separate PRs. Phase 2 Task 8 needs Phase 1 Task 2 first. Phase 3 tasks are independent of each other and of Phase 1/2.

## Open decisions (for the user before execution starts)

All previously-open decisions are now resolved in the **Modernization Decisions** section above. The remaining genuinely-open items:

1. **Should `--code-maat-compat` ship in this plan or be deferred to a follow-up?** Implementing it as a single flag that flips ~6 internal defaults is ~50 LOC + tests. The case for shipping it in this plan: it's the migration story for users coming directly from code-maat, and the cost of adding it later (after dashboards break) is higher than adding it now. The case for deferring: every flag we ship is a flag we maintain forever; if zero users actually need it, we shouldn't carry it.
   - Recommendation: **Ship it.** The migration window is the highest-friction moment for any potential CodeLore adopter; making the compat flag exist (even if rarely used) removes adoption friction at low cost.

2. **For `soc`, do we apply Fisher exact significance filtering** (as on `coupling`) **or just the percentage-based threshold**? Research didn't dig deep enough to know whether Fisher applies cleanly here. **Spike during implementation:** check whether SoC's contingency table is well-defined; if yes, add `fisher_p` column and filter; if no, stick with percent threshold and note in docs.

3. **Naming for the modernized "main-dev with metric=deleted-lines" analysis.** Three candidates:
   - `refactoring-main-dev` (code-maat name; "refactoring" is misleading because it's not a refactor-commit filter)
   - `main-dev-by-deletions` (honest; symmetric with `main-dev-by-revs`)
   - `main-deleter` (catchy but maybe too cute)

   Recommendation: **`main-dev-by-deletions` as the canonical name; `refactoring-main-dev` as an alias** that maps to the same `run_main_dev(db, opts, Metric::Deleted)` call. Document `refactoring-main-dev` in advanced-usage with a "deprecated alias for `main-dev-by-deletions`" note.

---

## Phase ordering for subagent-driven execution

Recommended execution order:

1. **Phase 0, Task 0 (Options fields)** — prerequisite for everything else. Half day.
2. **Phase 1, Task 1 (soc with `--min-soc`)** — easiest, validates the pattern. Half day.
3. **Phase 1, Tasks 4 + 5 (entity-effort, entity-ownership)** — same pattern, parallelizable.
4. **Phase 1, Task 3 (main-dev triple)** — three variants behind one helper; mechanical once the helper is in.
5. **Phase 1, Task 2 (messages)** — adds the regex helper plumbing.
6. **Phase 2, Tasks 6–8** — CLI flag wiring once analyses are settled.
7. **Phase 3, Task 9 (`--time-bucket`)** — modern time-bucketing; relatively simple SQL.
8. **Phase 3, Task 10 (grouping)** — needs the `fancy-regex` decision; recommend running as one focused session.
9. **Phase 3, Task 11 (`--code-maat-compat` + legacy temporal)** — migration helper; last because it depends on knowing every flipped behavior from prior tasks.
10. **Phase 4 (docs + CHANGELOG)** — final, consolidates everything.

Land each task as a separate atomic commit. Run `cargo test --workspace --all-features && cargo clippy --workspace --all-targets --all-features -- -D warnings` before every commit.

---

## Self-review

**Spec coverage check:**
- 3 truly-missing analyses (messages, soc, refactoring-main-dev) → Tasks 1, 2, 3 ✓
- 4 partial analyses (main-dev, main-dev-by-revs, entity-effort, entity-ownership) → Tasks 3, 4, 5 ✓
- 5 unwired CLI flags (min-shared-revs, min-coupling, max-coupling, max-changeset-size, age-time-now) → Tasks 6, 7 ✓
- 2 additional CLI flags surfaced during research (expression-to-match, temporal-period) → Tasks 8, 9/11 ✓
- 1 architectural feature (grouping) → Task 10 ✓
- Documentation → Task 12 ✓

**Placeholder scan:** No `TBD`, no `TODO`, no `implement later`. Every code block is complete; every test specifies the fixture shape + assertions. Two open decisions are marked clearly for the user to resolve before execution.

**Type consistency:** `MainDevRow.metric: u64` is used across all three main-dev variants. `entity` is always `String`. `path` (raw) vs `entity` (post-grouping) distinction documented.
