# CodeLore — Modernization Sprint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **This plan is task-level, not step-level — each task has been validated against the current code but full TDD step lists are deferred until the bugfix sprint lands and the surrounding code stabilizes.** Re-read this plan and expand any task into TDD steps before executing it (apply [[feedback-improve-during-validation]] — the plan should reflect what the code looked like when execution starts, not when planning finished).

**Goal:** Close 16 validated 🟡 / 🔴-quality findings from `docs/modernization_audit_2026-06-08.md` that are quality / leverage improvements rather than correctness bugs (the correctness bugs go to the bugfix sprint). Each item makes the codebase more idiomatic Rust / DuckDB / 2026-best-practices, removes a code-maat-era ergonomic, or expands strategic surface area.

**Architecture:** Tasks group into 5 themes — SQL hygiene, schema, identity, CLI typing, output coverage. Each theme is a coherent commit cluster; some themes are single tasks. Land each task as an atomic commit. Re-run gates after every commit.

**Tech Stack:** Adds `duckdb::params!` macro usage (already in `facts/ingest.rs`), DuckDB `ENUM` types, `include_str!()` for SQL files, `clap::ValueEnum` derive. No new crates.

---

## Non-Goals

- **Bug fixes** — all 7 critical correctness fixes are in `2026-06-08-codelore-bugfix-sprint.md`. **Execute that plan first.** This plan assumes those landed.
- **code-maat feature coverage** — covered by `2026-06-08-codelore-code-maat-parity.md`. Independent of this plan.
- **Already-tracked backlog** — rename tracking, Options builder + cross-field validation, CSV crate migration, parallel clone walk. Listed in `docs/codebase_analysis_report.md`. None subsumed here.
- **SARIF coverage expansion (#23)** — large enough to deserve its own plan once the rule taxonomy is designed (`helpUri` per analysis, severity bands per dimension, fingerprint scheme per shape). Carve out into a follow-up plan rather than inflating this one.

---

## Tasks by theme

### Theme A: SQL hygiene — bind parameters + extract to .sql files (audit #10, #11)

**Validated state:** 12 analyses use `format!("…HAVING revs >= {min}…", min=opts.min_revs)`. No prepared-statement reuse; injection-shaped even when currently safe; SQL is buried in heredocs that escape editor highlighting.

**Why now:** Modernization plan precondition. Every subsequent SQL-touching task (indexes, change_type ENUM, Fisher in code_health) is cheaper if SQL is bind-parameter-shaped already.

#### Task A.1: Migrate all 12 analyses to `params!` bind parameters

- [ ] **Per-analysis:** Replace each `format!()` SQL string with `db.conn().prepare(STATIC_SQL_CONST)` + `stmt.query_map(params![bind1, bind2, ...], ...)`. The `format!()` for `{limit}` (which is a string suffix, not a value) becomes a separate prepared-statement variant — `LIMIT` clauses don't take bind parameters in most SQL dialects.
- [ ] **Per-analysis:** Extract the SQL constant either as a top-level `const X_SQL: &str = "…"` in the analysis module, or — better — as a sibling `.sql` file pulled in with `include_str!()`. The latter wins because editors apply SQL highlighting and `cargo-deny`-style SQL linters become possible.
- [ ] **Joint test:** Add `crates/codelore-lib/tests/analyses_use_bind_params_test.rs` that greps the analyses sources and fails if any `format!()` invocation contains "SELECT", "WHERE", "GROUP BY", etc. Lints the invariant going forward.

**Files:** 12 in `crates/codelore-lib/src/analyses/`, optionally 12 new `.sql` files in same directory. ~250 LOC churn.

**Effort:** L. Mostly mechanical; the lint test prevents regression.

#### Task A.2: `code_age.rs` now_str specifically (audit #10)

- [ ] Subset of A.1; can ship independently. The `now_str` interpolation is the most visible of the format!-SQL hotspots because it interpolates a string built from a Date type. Replace with `params![now_str.as_str(), opts.min_revs]`.

**Files:** 1.

**Effort:** S. Could be folded into A.1; calling out only because the audit flagged it specifically and it's the cleanest single-file example to lead with.

### Theme B: Schema — indexes + ENUM (audit #12, #13)

#### Task B.1: Add hot-path indexes to changes + commits

**Validated state:** Only `clones` has indexes (`idx_clones_group`, `idx_clones_fp`). Every analysis JOINs `changes ON commits.rev = changes.rev` with no index beyond the composite PK; GROUP BY `changes.path` is unindexed; `commits.canonical_author` is scanned in 5 analyses with no index.

- [ ] Add to `crates/codelore-lib/src/facts/schema_v1.sql`:
  ```sql
  CREATE INDEX IF NOT EXISTS idx_changes_path ON changes(path);
  CREATE INDEX IF NOT EXISTS idx_changes_rev  ON changes(rev);
  CREATE INDEX IF NOT EXISTS idx_commits_author ON commits(canonical_author);
  CREATE INDEX IF NOT EXISTS idx_commits_date ON commits(date);
  ```
- [ ] Bump schema version (or add a migration if we have a migrations runner — verify in `cache.rs::schema_version`).
- [ ] **Bench validation:** run the criterion bench harness on `medium_repo` before + after; the test should be hotspots + coupling + clone-coupling. Capture numbers in the commit message.

**Files:** 1.

**Effort:** S. Single migration commit; the bench tells us whether DuckDB benefits in our scale range.

> **Note:** DuckDB is columnar — indexes help with zone-map pruning + small-result JOINs more than with large scans. The win is measurable on the codescene workspace (~95 commits, 155 files); the win is structural at kernel scale. Land it regardless; measure for the changelog entry.

#### Task B.2: `change_type TEXT` → DuckDB ENUM

**Validated state:** `change_type TEXT NOT NULL` storing `"added" | "modified" | "deleted" | "renamed" | "copied" | "binary"`. C-string for what is structurally a closed enum. `code_age.rs:38` filters `change_type != 'deleted'` — typo waiting to happen.

- [ ] Add `CREATE TYPE change_type_enum AS ENUM(…)` declaration to schema_v1.sql.
- [ ] Change `change_type` column type from `TEXT` to `change_type_enum`.
- [ ] Verify ingest writes the enum values (DuckDB accepts string literal inserts against ENUM columns, so probably no Rust-side change needed — confirm during execution).
- [ ] Existing WHERE clauses (`change_type != 'deleted'` etc.) still work — DuckDB ENUM comparisons accept string literals.

**Files:** 1 schema + maybe `ingest.rs` if Appender needs an ENUM type hint.

**Effort:** M. Investigate DuckDB ENUM + Appender interaction first — if it doesn't play nicely with the Rust-side `duckdb::Appender`, fall back to keeping TEXT but adding a `CHECK` constraint.

### Theme C: Identity layer (audit #14, #16)

#### Task C.1: `bots.toml` extension hook for custom bot patterns

**Validated state:** `DEFAULT_BOT_PATTERNS: &[&str]` is compile-time. Users with internal bot accounts (`our-deploy-bot@example.com`) get them counted as humans.

- [ ] Read `bots.toml` at repo root + at `~/.config/codelore/bots.toml` (XDG). Format:
  ```toml
  # bots.toml — additions to CodeLore's default bot-detection list
  patterns = [
      "our-deploy-bot",
      "another-internal-bot",
  ]
  ```
- [ ] Merge with `DEFAULT_BOT_PATTERNS` at ingest start. User additions, no removals.
- [ ] Document in `docs/advanced-usage.md` §6.2.

**Files:** `identity/mod.rs` or new `identity/config.rs`. Ingest changes its bot-resolution path.

**Effort:** M. Cleanest if a single `BotPatternSet` struct handles default+user merge.

#### Task C.2: Lowercase + trim before bot match

**Validated state:** `is_bot` uses raw `email.contains(p) || name.contains(p)`. `Dependabot[Bot]@noreply.github.com` is NOT detected. (Already partially addressed by bugfix Task 6 which switched to lowercase — verify after bugfix lands; this task may then be a no-op or subsumed.)

- [ ] **Validation re-check** after bugfix Task 6 lands. If 6's lowercase fix covers this, mark C.2 as done-by-subsumption.

**Files:** 0–1.

**Effort:** S, possibly 0.

### Theme D: CLI typing — string → enum (audit #19, #20)

#### Task D.1: `--analysis` / `--format` / `--fail-on` in `codelore diff` as ValueEnum

**Validated state:** 5 `pub *: String` fields where typed enums would catch typos at parse time. Today `--analysis hotpsots` silently runs nothing (the three booleans stay false).

- [ ] Define `#[derive(clap::ValueEnum, Clone, Debug)]` enums:
  ```rust
  enum DiffAnalysisKind { Hotspots, Coupling, Clones, All }
  enum DiffFormat { Text, Json, Sarif, Markdown }
  enum DiffFailOn { None, RankEntrant, ScoreIncrease, Any }
  ```
- [ ] Replace `pub analysis: String` with `pub analysis: DiffAnalysisKind` (etc.).
- [ ] Update downstream `match` against the enum (cleaner; the compiler ensures coverage).
- [ ] Also apply to `analyze`'s `--format` and `--analysis` — `AnalysisName` already has `FromStr` but isn't wired through clap as `ValueEnum`. ~30 lines away from being typed; close the loop.

**Files:** `crates/codelore-cli/src/args.rs` + `main.rs` + `diff.rs` + `diff_output.rs`.

**Effort:** M. Mechanical but touches many sites because of the existing string-match cascade.

#### Task D.2: Coupling-absence thresholds as `DiffArgs` knobs

**Validated state:** `diff.rs:308-311` hard-codes `c.shared >= 5 && c.fisher_p < 0.05`. Locked-in research-brief defaults but not user-tunable, breaking the pattern that every other coupling knob (`min_shared_revs`, `fisher_significance`) exposes.

- [ ] Add `--absence-min-shared u32` (default 5) and reuse `Options::fisher_significance` for the p-gate.
- [ ] Default behavior unchanged; users can tune sensitivity.

**Files:** `args.rs` + `diff.rs` + 1 test.

**Effort:** S.

### Theme E: Output emitters (audit #6, #21, #22, #23 partial)

#### Task E.1: `main-author` CSV header (rename from `main-dev`)

**Validated state:** CSV header `entity,main-dev,total-revs,fractal-value` but the struct field is `main_author`. Mismatched naming costs grep-ability.

- [ ] Rename CSV/Markdown/JSON column header to `main-author`. Document the schema change in CHANGELOG.
- [ ] Migration: users with dashboards parsing `main-dev` literally must update. Provide a one-line `--compat code-maat` opt-in if migration pain is real (defer adding the flag until evidence of user complaint).

**Files:** `output/csv.rs` + `output/markdown.rs` + maybe an existing snapshot test.

**Effort:** S.

> **Note:** the code-maat parity plan already addresses the `main-dev-by-revs` column-name lie via `--code-maat-compat`. This task closes the symmetrical issue on `ownership`'s `main-dev` header.

#### Task E.2: Hotspot SARIF `level` derives from `security-severity`

**Validated state:** `sarif.rs:73` `let level = if row.hotspot_score >= 0.5 { "warning" } else { "note" };` — analysis-internal scale, not aligned with the SARIF severity bands. Live-clone SARIF (`build_live_clone_result:419`) already uses the modern pattern (level derived from severity).

- [ ] Replace the raw threshold with: `let level = if security_severity >= 7.0 { "error" } else if security_severity >= 4.0 { "warning" } else { "note" };`
- [ ] Already-set `security-severity` value (`(100 - code_health) / 10`) drives both fields → one source of truth.

**Files:** `sarif.rs` (one function).

**Effort:** S.

#### Task E.3: Refactor 14-arm `match (format, analysis)` ladder in main.rs

**Validated state:** `crates/codelore-cli/src/main.rs:187-487` is a 300-line match expression that exhaustively enumerates `(format, AnalysisName)` pairs. Adding an analysis or format = many new match arms.

- [ ] Replace with table-driven dispatch: a `Vec<(AnalysisName, Format, EmitterFn)>` registry built at startup; lookup is `O(1)`. The emitter fns capture `Row → impl Write → Result<()>` closures.
- [ ] Cuts ~250 LOC. Adding a new analysis means appending to the registry, not editing a 300-line match.

**Files:** `main.rs` mostly; possibly a new `dispatch.rs` module.

**Effort:** M. The refactor is large but the test surface is whether `cargo test --workspace` stays green and the CLI integration tests pass.

### Theme F: code_health composite improvements (audit #3, #4)

#### Task F.1: Materialize `coupling_pairs` view; use in code_health centrality

**Validated state:** `code_health.rs:72-84` has an inline 12-line `WITH file_coupling AS (...) UNION ALL (...)` that re-derives every coupling pair. `analyses/coupling.rs:168` does this once with the Fisher filter. Duplication + drift risk + JOIN runs twice per code_health invocation.

- [ ] Materialize a `coupling_pairs` DuckDB temp table (or view) in the ingest pipeline. Both `coupling::run_coupling` and `code_health::run_code_health` query it.
- [ ] Bonus: same approach as `clone_coupling.rs`'s probe-table pattern — already in the codebase as proven.

**Files:** `facts/ingest.rs` + `analyses/coupling.rs` + `analyses/code_health.rs`.

**Effort:** M. Requires deciding whether the view is built on-demand or eagerly at ingest. Eager is simpler; on-demand is cheaper for runs that don't need it.

#### Task F.2: code_health uses Fisher-filtered coupling centrality

**Validated state:** `code_health` linearly combines `n_cx`, `n_cn`, `n_au`, `n_cp`. `n_cp` is raw coupling-centrality (count of partners). No Fisher filter — a file with 50 spurious refactor-sweep partners scores the same as one with 50 genuine ones.

- [ ] Depends on F.1. Once `coupling_pairs` is a materialized view with the Fisher filter applied, `code_health`'s `n_cp` calculation queries it.
- [ ] **Behavior change:** existing code_health scores will shift. Document in CHANGELOG. Consider whether to expose a `--legacy-centrality` opt-out for users who anchored on the old score (probably overkill — composite scores aren't audited externally the way SARIF results are).

**Files:** `analyses/code_health.rs`.

**Effort:** S (after F.1).

---

## Theme deferred: SARIF coverage expansion (audit #23)

The audit flagged that only 3 of 14 analyses emit SARIF. Adding `CODELORE-CODE-HEALTH` and `CODELORE-COUPLING` rules is mechanical, but the design questions (rule taxonomy, severity bands, fingerprint scheme, `helpUri` per dimension) deserve their own plan + spec review. **Out of scope here.** Track in `docs/codebase_analysis_report.md` as a follow-up.

---

## Effort estimate

| Theme | Tasks | Files | LOC | Effort |
|---|---|---|---|---|
| A (SQL hygiene) | 2 | ~13 | ~350 | L (most of week 1) |
| B (Schema) | 2 | ~2 | ~50 | S |
| C (Identity) | 2 | ~2 | ~80 | M |
| D (CLI typing) | 2 | ~4 | ~120 | M |
| E (Output) | 3 | ~5 | ~300 | M |
| F (code_health) | 2 | ~3 | ~150 | M |
| **Total** | **13** | **~29** | **~1050** | **2-3 weeks** |

---

## Phase ordering for execution

Execute themes in this order:

1. **Theme A (SQL hygiene) first** — every other theme touches SQL; cleaner ground.
2. **Theme B (Schema)** — indexes + ENUM. Cheap. Benchmark before/after.
3. **Theme D (CLI typing)** — independent; small commits; user-visible improvement.
4. **Theme C (Identity)** — depends on whether bugfix Task 6 subsumed C.2.
5. **Theme F (code_health)** — has internal dependency F.1 → F.2.
6. **Theme E (Output)** — last; large refactor of main.rs (E.3); compose well with everything else landed.

---

## Self-review

**Spec coverage:** 16 audit findings get tasks here (1, 3, 4, 6, 8-14, 16, 18-22). Findings 1 (author_churn sort), 5 (empty name), 7 (p_value=0.0), 15 (AI patterns), and updated_analysis_report findings 3.1-3.5 are in the bugfix sprint. Findings 2, 17, 24 are 🟢 (already modern) — no work. Finding 23 (SARIF coverage) is a future plan. Total accounted: 24 of 24.

**Placeholder scan:** Each task has a "validated state" sentence with the exact code state + a clear deliverable. No TBD. SQL snippets are syntactically complete. Where execution-time details matter (e.g., DuckDB ENUM + Appender interaction in B.2), the task explicitly flags "investigate first; fall back to … if … ".

**Type consistency:** Where renames are proposed (`main-dev` → `main-author`), the change is consistent across all 3 emitters. Where bind parameters replace `format!()`, every analysis follows the same pattern.

**Improvability:** Per [[feedback-improve-during-validation]], every task should be revisited at execution time. The "Validated state" sentence may already be stale by the time A is done (B might trivially follow); collapse tasks if they fold cleanly.
