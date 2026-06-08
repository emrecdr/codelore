# CodeLore Modernization Audit — 2026-06-08

Lens: *is each design choice still correct in 2026, given our own architecture?*
Scope: 14 shipped analyses, ingest pipeline, identity layer, Options struct,
output emitters, `codelore diff`. Backlog items (rename tracking, Options
builder, CSV crate, parallel clone extraction) intentionally skipped.

Severity legend: 🔴 real footgun shipped · 🟡 minor improvement · 🟢 already modern.

## Summary

| # | File:Line | Finding | Sev | Effort |
|---|-----------|---------|-----|--------|
| 1 | `analyses/churn.rs:81` | `author_churn` ORDER BY non-deterministic on author name | 🔴 | S |
| 2 | `analyses/churn.rs:47` | `abs_churn` no secondary sort (date ties impossible but fragile) | 🟢 | — |
| 3 | `analyses/code_health.rs:30+` | `code_health` emits no Fisher significance signal | 🟡 | M |
| 4 | `analyses/code_health.rs:72-84` | Coupling centrality recomputed inline | 🟡 | M |
| 5 | `analyses/hotspots.rs:99` | `name` column always empty `''` — code-maat-compat-lie | 🔴 | S |
| 6 | `output/csv.rs:148` | Header `main-dev` for `main_author` field — naming honesty | 🟡 | S |
| 7 | `analyses/clone_coupling.rs:186` | `p_value = 0.0` hard-coded after gate | 🔴 | S |
| 8 | `analyses/clone_coupling.rs:229` | `same_parent_dir` ignores Windows `\` separators | 🟡 | S |
| 9 | `analyses/coupling.rs:31` | `average_revs: u32` is integer-truncated `(a+b)/2` | 🟡 | S |
| 10 | `analyses/code_age.rs:34` | `now_str` interpolated into SQL — bind parameter instead | 🟡 | S |
| 11 | `analyses/*` (12 sites) | `format!()` SQL interpolation everywhere — uniform style | 🔴 | M |
| 12 | `facts/schema_v1.sql` | No indexes on `changes(rev)`, `changes(path)`, `commits(canonical_author)` | 🔴 | S |
| 13 | `facts/schema_v1.sql:25` | `change_type TEXT` (string enum) — DuckDB has `ENUM` | 🟡 | M |
| 14 | `identity/bots.rs:4` | Static const bot list — no `bots.toml` / `.codeloreignore`-style config | 🟡 | M |
| 15 | `identity/bots.rs:27` | AI-attribution misses `Cursor`, `Cody`, `Aider`, `Continue` | 🔴 | S |
| 16 | `identity/bots.rs:14` | Email matched case-sensitively; whitespace not normalized | 🟡 | S |
| 17 | `options.rs:35` | Doc-comment "code-maat parity" tag on knobs that diverge | 🟢 | — |
| 18 | `options.rs:35` | `min_revs` overloaded — same knob gates 9 analyses with different semantics | 🔴 | M |
| 19 | `cli/args.rs:42,59,108,131,140` | String-typed `--format`, `--analysis`, `--fail-on` in clap | 🔴 | M |
| 20 | `cli/diff.rs:308,311` | Coupling-absence thresholds `shared>=5`, `p<0.05` hard-coded | 🟡 | S |
| 21 | `cli/main.rs:187-487` | 14-arm `match (format, analysis)` ladder — should be table-driven | 🟡 | M |
| 22 | `output/sarif.rs:73` | Hotspot SARIF level uses raw `hotspot_score>=0.5` (analysis-internal scale) | 🟡 | S |
| 23 | `output/sarif.rs` | Only 3 of 14 analyses emit SARIF (hotspots, clones, clone-coupling) | 🟡 | L |
| 24 | `output/json.rs:32-37` | Revisions JSON ad-hoc shim — should derive Serialize on a real row type | 🟢 | — |

---

## 1. The 14 Shipped Analyses

### `analyses/churn.rs:81` — `author_churn` non-deterministic tie-break  🔴 S

**Current.** `ORDER BY added DESC, commits DESC{limit}`. No tertiary tie-break on author name. Two authors with identical `added` and `commits` (extremely common for low-traffic repos and merge-bot accounts) flip positions between runs, breaking golden tests and SARIF fingerprints. `abs_churn` and `author_churn` both lack the `, author ASC` / `, date ASC` tertiary clause that `entity_churn`, `coupling`, `hotspots`, `ownership`, `communication`, `code_health` and `code_age` all have.

**Modern.** Add `, author ASC` / `, date ASC`. The pattern is already inconsistent in this very file — line 115 has it, line 81 doesn't. One-line fix; the diligence cost is zero once you've made determinism a project invariant. (`abs_churn` line 47 is technically safe — `GROUP BY date` produces unique dates — but adding the secondary sort matches the project's stated style anyway.)

### `analyses/code_health.rs:30+` — No Fisher signal in composite score  🟡 M

**Current.** `code_health` linearly combines four 0..1-normalized inputs (`n_cx`, `n_cn`, `n_au`, `n_cp`) and clamps. `n_cp` is raw coupling-centrality (count of partners). No Fisher-significance filter — a file with 50 spurious refactor-sweep coupling partners scores identically to a file with 50 genuine ones.

**Modern.** Compute `n_cp` from the *Fisher-filtered* coupling set, the same one `analyses/coupling.rs:168` builds. Today it's a degenerate "anyone who ever touched together" centrality, which contradicts the spec §4.6 spirit of basing the score on real signal. The 2025 MSR research the project already cites (and uses in `coupling.rs`) makes this asymmetry weird to leave in.

### `analyses/code_health.rs:72-84` — Coupling centrality recomputed  🟡 M

**Current.** Inline 12-line `WITH file_coupling AS (...) UNION ALL (...)` that re-derives every couplng pair via self-join. `analyses/coupling.rs` already does this once with the Fisher filter. The duplication means schema drift between the two is a real risk and the JOIN runs twice per `code_health` invocation.

**Modern.** Materialize a `coupling_pairs` view (or DuckDB temp table) in the ingest pipeline; both `coupling::run_coupling` and `code_health::run_code_health` query it. Same shape as `clone_coupling.rs`'s probe-table reuse pattern, already present in the codebase.

### `analyses/hotspots.rs:99` — `name` column always `''`  🔴 S

**Current.** `SELECT path, '' AS name, revs, ...` — the row type carries a `name: String` but the SQL hard-codes empty string. CSV header `entity,name,revisions,...` advertises a column we never populate. Same pattern in `code_health.rs:113`.

**Modern.** Either drop the column from the row struct + CSV header + SARIF, or wire the actual entity name (function/class) when scope is per-entity rather than per-file. The current `'' AS name` is a 2020-era code-maat hangover where row shapes were tuple-typed and aliased. In a Rust codebase with typed row structs there's no reason to carry a dead column.

### `output/csv.rs:148` — `main-dev` header, `main_author` field  🟡 S

**Current.** CSV header `entity,main-dev,total-revs,fractal-value` but the Rust struct field is `main_author`. This is a code-maat-compat lie — the original `main-dev-by-revs` analysis emitted *developer-with-most-revs*, which we call `main_author` in Rust. Mismatched naming costs grep-ability and onboarding clarity.

**Modern.** Rename the CSV header to `main-author` (we are 14 analyses past v1 launch, this is our spelling now). If users want code-maat-compat output, that belongs behind an `--compat code-maat` flag, not in the default schema.

### `analyses/clone_coupling.rs:186` — `p_value = 0.0` after Fisher gate  🔴 S

**Current.** `let approximated_p = 0.0; // already passed Fisher filter`. The row's `p_value` field — exposed in CSV, JSON, and SARIF `properties.codelore/p-value` — is structurally always 0.0. SARIF severity `combined_score = similarity * degree_pct * (1 − 0)` is just `similarity * degree_pct`.

**Modern.** `analyses/coupling.rs:164` already computes `fisher_p` per pair and stores it on `CouplingRow.fisher_p`. The HashMap probe at line 89-94 carries `&CouplingRow`, so the real `cp.fisher_p` is in hand. The fix is `let approximated_p = cp.fisher_p;`. The `combined_score` ranking changes meaningfully — pairs at p=0.04 now score lower than pairs at p=0.001, which is the whole point of carrying the field.

### `analyses/clone_coupling.rs:229` — `same_parent_dir` is Unix-only  🟡 S

**Current.** `let parent = |p: &str| p.rfind('/')...`. Windows clones with `src\foo\a.rs` paths never match, so the same-dir mitigation silently disables on Windows.

**Modern.** Use `std::path::Path::parent()` — handles both separators and is more honest about what "parent dir" means.

### `analyses/coupling.rs:31` — `average_revs: u32` integer truncation  🟡 S

**Current.** `(fr_a.revs + fr_b.revs) / 2 AS average_revs` — integer division. Pair with revs 9 and 10 reports `average_revs=9`, but `degree` uses the float version `(a+b)/2.0`. So `degree * average_revs / 100 != shared` and the integer is misleading enough to surface in CSV inspection.

**Modern.** Either drop the integer field (it's redundant with `revs_a`/`revs_b`) or emit `f64`. Cheap; the SARIF surface doesn't carry this field today so the change is contained.

### `analyses/code_age.rs:34` — String-interpolated date  🟡 S

**Current.** `format!("DATE '{now}' ...", now = now_str)` interpolates an attacker-controllable (CLI flag) string into SQL. The `now_str` is built from `Date::year/month/day` so it can't actually inject — but the same pattern at coupling.rs:124 with `max = opts.max_changeset_size` and 11 other call sites is one CLI flag away from being unsafe.

**Modern.** Bind parameters: `db.conn().prepare(sql)` then `stmt.query_row(params![now_str, min_revs], ...)`. DuckDB's `params!` macro is already in use at `facts/ingest.rs:223`. Project-wide cleanup of all 12 analyses + the SQL ladder pattern is more uniform & faster (prepared statement reuse).

### `analyses/*` (12 sites) — Uniform `format!` SQL pattern  🔴 M

**Current.** Every analysis uses `format!("...HAVING revs >= {min}...", min=opts.min_revs)`. No prepared-statement reuse; injection-shaped even when the value is currently safe; SQL is a giant heredoc that escapes editor highlighting.

**Modern.** Bind parameters everywhere. Plus: extract the SQL into top-level `const REVISIONS_SQL: &str` / `const COUPLING_SQL: &str` modules. Makes them grep-able, lint-able, and snapshot-testable (`include_str!` from `.sql` files is even better — DuckDB-aware editors then highlight them).

## 2. Ingest Pipeline

### `facts/schema_v1.sql` — Missing hot-path indexes  🔴 S

**Current.** Only `clones` has indexes (`idx_clones_group`, `idx_clones_fp`). Every analysis JOINs `changes ON commits.rev = changes.rev` (no index on `changes.rev` other than the composite PK); GROUP BY `changes.path` (no index); and `commits.canonical_author` is scanned in `authors`, `communication`, `ownership`, `code_health`, `author_churn` (no index).

**Modern.** Add:
```sql
CREATE INDEX IF NOT EXISTS idx_changes_path ON changes(path);
CREATE INDEX IF NOT EXISTS idx_changes_rev  ON changes(rev);
CREATE INDEX IF NOT EXISTS idx_commits_author ON commits(canonical_author);
CREATE INDEX IF NOT EXISTS idx_commits_date ON commits(date);
```
DuckDB benefits less from B-tree indexes than row-stores, but it does use them to prune zone-maps in JOINs and `changes` will hit millions of rows on real repos. Measure on a 50k-commit repo before declaring victory — but the omission is a 2020s SQLite-era reflex.

### `facts/schema_v1.sql:25` — `change_type TEXT`  🟡 M

**Current.** `change_type TEXT NOT NULL` storing `"added" | "modified" | "deleted" | "renamed" | "copied" | "binary"`. A C-string for what is structurally a closed enum. Type drift if a new variant is added in code but not in queries (`code_age.rs:38` filters `change_type != 'deleted'` — typo waiting to happen).

**Modern.** DuckDB supports `CREATE TYPE change_type_enum AS ENUM('added','modified','deleted','renamed','copied','binary')`. Zero-cost in storage, validated at insert time, queryable with the same WHERE clauses. Matches Rust's `ChangeType` enum 1:1.

## 3. Identity Layer

### `identity/bots.rs:15` — Static const bot list  🟡 M

**Current.** `DEFAULT_BOT_PATTERNS: &[&str]` — compile-time, no extension hook. Users with internal bot accounts (`our-deploy-bot@example.com`) get them counted as humans in all author analyses.

**Modern.** Mirror the project's existing `.codeloreignore` convention with `.codelorebots` (or section in a single `codelore.toml`). Default list stays as it is; user file adds substrings. Already proven pattern in `analyses/clones.rs:117`.

### `identity/bots.rs:27` — Outdated AI-attribution patterns  🔴 S

**Current.** Detects `Co-Authored-By: Claude | Copilot | GitHub Copilot`. Misses every other 2024+ AI coder: **Cursor** (`Co-Authored-By: Cursor`), **Aider** (`(aider)` in message body), **Cody** (`Co-Authored-By: Sourcegraph Cody`), **Continue** (`Co-Authored-By: Continue`), **Codeium / Windsurf**, **Devin** (`Co-Authored-By: Devin <devin-ai-integration[bot]@users.noreply.github.com>`). Today a repo with 100% Cursor-assisted commits reads as 100% human.

**Modern.** Extend the substring list and ship a regex (`(?i)co-authored-by:\s*(claude|copilot|cursor|cody|aider|continue|codeium|windsurf|devin|tabnine)`) — case-insensitive, single match. Put the list in the same `bots.toml` from finding #14 so it evolves without a release.

### `identity/bots.rs:14` — Case-sensitive substring match  🟡 S

**Current.** `email.contains(p) || name.contains(p)`. `Dependabot[Bot]@noreply.github.com` (GitHub returns mixed case for some bots) is NOT a bot under our rule. Also: leading/trailing whitespace on commit author lines (rare but happens in `git filter-repo` outputs) defeats the substring.

**Modern.** Lowercase both sides; trim before comparing. Two lines.

## 4. Options

### `options.rs:35` — `min_revs` semantically overloaded  🔴 M

**Current.** One knob, nine analyses, three meanings: (a) per-file revision floor in `revisions`/`hotspots`/`code_age`/`entity_churn`/`code_health`/`ownership`; (b) `min_shared_revs` floor in `communication` (different field, accidentally semi-aligned); (c) silently ignored in `summary`/`authors`/`abs_churn`/`author_churn`. Setting `--min-revs 100` for a large repo correctly thins `hotspots` but also wipes 90% of `ownership` rows even though that analysis's "show me Fractal Value" reading wants a low floor.

**Modern.** Per-analysis floors: `min_hotspot_revs`, `min_coupling_revs` (already separate as `min_shared_revs`), `min_ownership_revs`. Or — and this is the structural fix — drop "thresholds in Options" entirely and accept them as a per-analysis struct passed at call time. The Options builder backlog item is the natural moment to do this; flagging because it's *more* than a builder, it's a semantic refactor.

## 5. Output Emitters

### `output/sarif.rs:73` — Hotspot SARIF level uses analysis-internal score  🟡 S

**Current.** `let level = if row.hotspot_score >= 0.5 { "warning" } else { "note" };`. The 0.5 threshold is bound to the analysis's percentile-rank scale — meaningful only relative to the current run's repo. A small repo where the top hotspot scores 0.4 emits zero warnings; a large repo emits many. Plus SARIF consumers compare `security-severity` (which IS already computed via `(100 - code_health)/10`), so duplicating that decision in `level` is redundant.

**Modern.** Derive `level` from `security-severity` ranges (≥7 error, ≥4 warning, else note) — the pattern `build_live_clone_result:419` *already* uses. Three-line unification.

### `output/sarif.rs` — SARIF coverage gap  🟡 L

**Current.** SARIF supports only `hotspots`, `clones`, `clone-coupling`. The 11 other analyses don't emit SARIF, so `code-health` (the headline composite) can't surface in GitHub Code Scanning at all. Modern CI integrations are SARIF-first.

**Modern.** Add `CODELORE-CODE-HEALTH` (one result per low-scoring file, severity = `(100-score)/10`) and `CODELORE-COUPLING` (one result per Fisher-significant pair, severity from `degree`). These are mechanical given the existing pattern, but it's L because the rule taxonomy needs design — what's a `helpUri` for code-age? what's "level" for ownership?

## 6. `codelore diff`

### `cli/args.rs:108` — String-typed `--analysis hotspots|coupling|clones|all`  🔴 M

**Current.** `pub analysis: String` with downstream `args.analysis.as_str()` matches. The codebase already has `enum AnalysisName` for exactly this purpose — `cli/main.rs:73` parses it via `FromStr`. The diff command doesn't, so a typo `--analysis hotpsots` silently runs *nothing* (no error, no warning — the three booleans all stay `false`).

**Modern.** Mirror the analyze path: `#[derive(clap::ValueEnum)] enum DiffAnalysisKind { Hotspots, Coupling, Clones, All }`. Clap validates at parse-time. Same fix for `--fail-on` (line 140) and `--format` (line 131). Removes 4 match-on-string sites.

### `cli/diff.rs:308-311` — Coupling-absence magic numbers  🟡 S

**Current.** `c.shared >= 5 && c.fisher_p < 0.05`. Locked from the research brief but not threaded through `DiffArgs`. Users can't tune the absence-warning sensitivity even though every other coupling threshold (`min_shared_revs`, `fisher_significance`) is a knob.

**Modern.** Add `--absence-min-shared u32` (default 5) and reuse `Options::fisher_significance` for the p-value gate. Already the pattern for `--score-threshold` two args earlier.

---

## Top 5 to fix first (leverage × effort)

1. **`hotspots.rs:99` empty `name` column** (🔴 S) — visible in every CSV/JSON/SARIF output today, costs 5 minutes, fixes a project-wide schema lie.
2. **`clone_coupling.rs:186` `p_value=0.0` hard-coded** (🔴 S) — silently mis-ranks the headline "live clone" output; the real value is already in scope. 10 minutes; meaningfully changes what users see at the top of the report.
3. **`identity/bots.rs:27` outdated AI patterns** (🔴 S) — every Cursor/Aider/Cody repo reads as 100% human. 15 minutes to add patterns; the analysis becomes accurate for the modern AI-tooling era the project explicitly targets.
4. **`schema_v1.sql` missing indexes** (🔴 S) — single migration commit, measurable cold-run speedup on real repos, removes a hot-path embarrassment.
5. **`cli/args.rs:108` `--analysis` string typing in diff** (🔴 M) — typo-silent failure in PR mode, where it matters most. Pattern already exists 30 lines away in `analyze`; the migration is 90% rote.

Items 6–10 are the `format!` SQL → bind-parameter sweep (#11), the `min_revs` semantic split (#18), `same_parent_dir` cross-platform fix (#8), `change_type` DuckDB ENUM (#13), and the SARIF coverage expansion (#23) — none individually dramatic, but together they retire the bulk of the code-maat-era ergonomics.
