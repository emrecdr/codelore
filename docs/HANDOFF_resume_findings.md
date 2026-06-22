# Handoff — resuming the active-findings sweep

> **Status update 2026-06-22 (continuation session).** Of the 7 findings
> this doc was written to resume, **4 are now closed and on `main`**:
> F111 (`7534025`), F94 (`99a06ef`), and F113 + F145 together (`b734d45`).
> **5 Active findings** now remain in
> `docs/reports/deep_analysis_report.md` (the source of truth): F119,
> F148, F161, F164, F165.
>
> - **F119 / F148 / F161** — the output-emitter cluster. Deliberately
>   deferred to its own focused session: validation showed the handoff's
>   "single generic `TabularEmit`" design is leakier than assumed (7 CSV
>   emitters carry a `code_maat_compat` flag that changes their schema at
>   runtime, `revisions` is a tuple, `hotspots` has heavy conditional
>   formatting), and the change is byte-identical-critical across 64
>   emitters. F145 was done WITHOUT touching the emitter internals, so
>   those internals stay byte-identical by construction.
> - **F164** (new) — task-ID (`F<NN>`) references in code comments,
>   codebase-wide (~48 sites). Documentation-hygiene sweep.
> - **F165** (new) — `--format ndjson`/`gha` on an unsupported analysis
>   panics via a reachable `unreachable!` (exit 101). Surfaced during the
>   F145 byte-identical verification; preserved verbatim there. Mirror the
>   SARIF guard to bail cleanly instead.
>
> The §F111 / §F113 / §F94 / cluster sections below are retained as
> historical reference; the first three are DONE. For the cluster, the
> §"F119 + F148 + F145 + F161" section's design sketch should be revisited
> against the leakiness notes above before implementing.
>
> Verification harness note for the cluster: a byte-identical baseline
> across all 228 analysis×format pairs is the right gate. Capture with a
> binary COPIED to a stable path (e.g. `/tmp/codelore_stable`) — the live
> `target/debug/codelore` gets wiped by concurrent rust-analyzer/cargo
> churn mid-capture, which silently produces all-empty (exit 127) outputs
> and a misleading diff. Compare stdout + exit codes; expect benign diffs
> in clones (working-tree-sensitive), delivery-friction (`wip_age_days`
> wall-clock drift), and SARIF (`run/<id>` per-run).

---

Session paused 2026-06-22. This document captures each remaining finding
with concrete implementation details, verification commands, and
prior-session notes — paste a finding ID + the relevant section into a
fresh session to pick up exactly where this one left off.

## Original session state (clean main, e848621 → 6218fd1 → 5c10a3e → 6218fd1)

Closed in the prior session (Wave 1 + Wave 2 + partial Wave 3): F123
(refuted), F162 (already-closed), F131, F137, V5, V6, F114, F115, F122,
F136, F144, F149, F121, V4, F132, F133, F97. **14 fixes + 2 prior
closures = 16 total**. Active 23 → 7.

The 7 findings this doc was written to resume, ranked by leverage
(✅ = closed in the continuation session):

| ID    | Effort | Blast        | Why it's last |
|-------|--------|--------------|---------------|
| ✅ F111 | S    | cross-cutting| Visibility + safe methods. **Done** (`7534025`). |
| ✅ F113 | M    | cross-cutting| `codelore_lib::cli_api` façade (additive). **Done** (`b734d45`, with F145). |
| ✅ F94  | L    | architectural| Split of `ingest.rs` into 7 submodules. **Done** (`99a06ef`). |
| F119  | L      | cross-cutting| `csv` crate sweep. Pairs with F148/F161 — deferred cluster. |
| F148  | L      | architectural| `TabularEmit` trait. Pairs with F119/F161 — deferred cluster. |
| ✅ F145 | L    | cross-cutting| `main.rs` dispatch collapse (per-analysis fns). **Done** (`b734d45`). |
| F161  | L      | cross-cutting| `EmitterStream` trait for streaming emit. Pairs with F148/F119 — deferred cluster. |

---

## F111 — `FactsDb::conn()` visibility + narrow safe methods

**Status**: in-progress when this session ended. WIP stashed; this
section is a clean restart plan.

**Goal**: tighten `pub fn conn(&self) -> &Connection` to `pub(crate)`
so external callers can't bypass FactsDb's safety surface. Add narrow
safe methods (`prepare`, `execute_batch`, `query_row`) so the existing
external uses (all in `tests/`) have a sanctioned migration path.

### Recon (done)

```bash
# Zero CLI uses:
/usr/bin/grep -rn '\.conn()' crates/codelore-cli/src/   # empty

# Eight external test sites:
/usr/bin/grep -rn '\.conn()' crates/codelore-lib/tests/
# clones_factsdb_test.rs:56  .conn()
# team_map_test.rs:46        .conn()
# team_map_test.rs:82        .conn()
# f69_window_spike_test.rs:172  db.conn().prepare(sql)
# f69_window_spike_test.rs:264  db.conn().prepare(&explain_sql)
# imports_factsdb_test.rs:61   let conn = db.conn();   (used for multi-query)
# imports_factsdb_test.rs:214  let conn = db.conn();   (same)
# imports_factsdb_test.rs:321  .conn()
# facts_test.rs:57           db.conn().execute_batch(...)
```

### Implementation

**Step 1.** Add three safe methods + tighten `conn()` in
`crates/codelore-lib/src/facts/mod.rs` (replace the existing `pub fn
conn` at line ~307):

```rust
/// Prepare a SQL statement against the underlying connection. Returns
/// a `duckdb::Statement<'_>` whose lifetime is tied to `&self`. Use
/// for the `prepare → query_map / query_row → collect` pattern when
/// the caller needs multi-row iteration. Errors wrapped in
/// [`CodeLoreError::Analysis`] so they share exit code 4.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] if statement preparation fails.
pub fn prepare<'a>(&'a self, sql: &str) -> Result<duckdb::Statement<'a>> {
    self.conn
        .prepare(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare: {e}")))
}

/// Run multiple SQL statements separated by `;`. Useful for test
/// fixtures and one-shot DDL/DML. Single-statement SQL also works.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL error.
pub fn execute_batch(&self, sql: &str) -> Result<()> {
    self.conn
        .execute_batch(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("execute_batch: {e}")))
}

/// Run a single SQL statement that returns exactly one row, mapping
/// it via the caller-supplied closure. Mirrors `rusqlite`'s shape so
/// migration from `db.conn().query_row(...)` is mechanical.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on prepare / execute / no-rows
/// error.
pub fn query_row<T, P, F>(&self, sql: &str, params: P, mapper: F) -> Result<T>
where
    P: duckdb::Params,
    F: FnOnce(&duckdb::Row<'_>) -> duckdb::Result<T>,
{
    self.conn
        .query_row(sql, params, mapper)
        .map_err(|e| CodeLoreError::Analysis(format!("query_row: {e}")))
}

/// Internal raw-connection accessor. `pub(crate)` so the rest of
/// `codelore-lib` (kamei, quality_gates, output::spa, ingest, etc.)
/// can still reach the underlying `duckdb::Connection` for
/// Appender / multi-statement transactions / etc. without
/// re-implementing every primitive on `FactsDb`. External callers
/// must use the narrow safe methods above (`prepare`,
/// `execute_batch`, `query_row`, `query_one_value`, `list_tables`,
/// `explain_sql`, `flush`) — see F111 in the deep analysis report.
pub(crate) fn conn(&self) -> &Connection {
    &self.conn
}
```

**Step 2.** Bulk-migrate test sites with this Python script:

```python
import re, pathlib

files = [
    'crates/codelore-lib/tests/clones_factsdb_test.rs',
    'crates/codelore-lib/tests/team_map_test.rs',
    'crates/codelore-lib/tests/f69_window_spike_test.rs',
    'crates/codelore-lib/tests/imports_factsdb_test.rs',
    'crates/codelore-lib/tests/facts_test.rs',
]
for f in files:
    p = pathlib.Path(f)
    src = p.read_text()
    # Multi-line chained `.conn()\n.<method>` form
    src = re.sub(r'(\w+)\s*\n\s*\.conn\(\)\s*\n\s*\.(query_row|prepare|execute_batch|execute)\b',
                 r'\1.\2', src)
    # Single-line `db.conn().<method>` form
    src = re.sub(r'(\w+)\.conn\(\)\.(query_row|prepare|execute_batch|execute)\b',
                 r'\1.\2', src)
    p.write_text(src)
```

**Step 3.** Two LEFTOVER `let conn = db.conn();` sites in
`imports_factsdb_test.rs` at lines 61 and 214. These bind the
connection to a local then run multiple `conn.query_row(...)` /
`conn.prepare(...)` calls in sequence. Manual fix: replace the
binding with direct `db.query_row(...)` / `db.prepare(...)` calls on
each downstream use. Example at line 61:

```rust
// before:
let conn = db.conn();
let total: i64 = conn
    .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
    .expect("count imports");
let unresolved: i64 = conn
    .query_row(
        "SELECT COUNT(*) FROM imports WHERE resolved = FALSE",
        [],
        |r| r.get(0),
    )
    .expect("count unresolved");

// after:
let total: i64 = db
    .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))
    .expect("count imports");
let unresolved: i64 = db
    .query_row(
        "SELECT COUNT(*) FROM imports WHERE resolved = FALSE",
        [],
        |r| r.get(0),
    )
    .expect("count unresolved");
```

Note the test wraps errors with `.expect`, not `?`. The new `db.query_row`
returns `Result<T, CodeLoreError>` (different from raw `Result<T,
duckdb::Error>`), so the `.expect("...")` call site works identically.

**Step 4.** Verify:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features 2>&1 | awk '/^test result/' \
  | awk '{p+=$4; f+=$6} END{print p " passed, " f " failed"}'
# Expected: 653 passed (or higher if F111 adds tests), 0 failed.
```

### Report + CHANGELOG updates

- `docs/reports/deep_analysis_report.md` §3 closure-log table: add
  ```
  | F111 | `FactsDb::conn()` leaks `&duckdb::Connection` into public API | **Fixed** | This session. Tightened to `pub(crate)`; new narrow safe methods `prepare`/`execute_batch`/`query_row` on FactsDb cover every external test-site need; 8 test sites migrated (4 via Python regex, 2 manual `let conn = db.conn();` expansions, 2 already used a single call). Zero CLI call sites — finding was about API hygiene, not breakage. |
  ```
- §4 — remove the `#### F111 — `FactsDb::conn()` …` block.
- §4½ — flip F111 row to **Fixed (this session)**.
- §5 — update Active list: drop F111, change count `7 → 6`.
- `CHANGELOG.md` `[Unreleased]` — add a `### Changed` entry:
  ```
  - **`FactsDb::conn()` tightened to `pub(crate)`; new narrow safe
    methods.** Pre-fix the raw `&duckdb::Connection` was reachable
    from any external consumer of `codelore-lib`, encouraging
    direct connection mutation that bypassed FactsDb's invariants
    (schema version stamp, FK clean-up, append-only ingest path).
    New `FactsDb::prepare(sql)`, `FactsDb::execute_batch(sql)`, and
    `FactsDb::query_row(sql, params, mapper)` cover every external
    test-site need; the eight external sites in `tests/` migrated
    mechanically. The CLI has zero `.conn()` uses, so production
    callers see no API change. Closes F111.
  ```

### Estimated scope

~80 LOC in mod.rs + ~10 mechanical test-site migrations. Single commit.

---

## F113 — `codelore_lib::cli_api` façade

**Goal**: stop the CLI from reaching into 7 distinct `codelore_lib`
submodules. Introduce `codelore_lib::cli_api` as the single `pub`
surface CLI imports. Internal modules become `pub(crate)`.

### Recon (current state)

The 2026-06-21 validation pass found exactly 7 first-level paths the
CLI imports from: `analyses`, `analysis`, `facts`, `options`,
`output`, `provenance`, `quality_gates`, `repo`, plus root-level
`CodeLoreError`, `AnalysisName`, `Options`. Run this to refresh:

```bash
awk '/use codelore_lib::/{print NR": "$0}' \
  crates/codelore-cli/src/main.rs | head -30
```

### Implementation sketch

**Step 1.** Inventory imports. Per CLI file, list every
`codelore_lib::<module>::<item>` import. Group by item type
(analysis names, output writers, options, facts, error, repo trait,
provenance bits).

**Step 2.** Design the façade. `codelore_lib::cli_api` should
re-export every CLI-needed item under stable names:

```rust
// crates/codelore-lib/src/cli_api.rs
//! The ONLY API surface `codelore-cli` (and other future
//! `codelore-lib` consumers) should reach into. Each internal
//! module remains `pub(crate)`; consumers go through here.

pub use crate::AnalysisName;
pub use crate::CodeLoreError;
pub use crate::Options;
pub use crate::Result;

pub mod analysis {
    pub use crate::analyses::*;  // narrow this per actual use
}

pub mod facts {
    pub use crate::facts::FactsDb;
    pub use crate::facts::IngestStats;
    // ... only what CLI needs
}

pub mod output {
    pub use crate::output::csv;
    pub use crate::output::json;
    pub use crate::output::markdown;
    pub use crate::output::sarif;
    pub use crate::output::ndjson;
    pub use crate::output::gha;
    pub use crate::output::html;
    pub use crate::output::parquet;
    pub use crate::output::sqlite;
    pub use crate::output::spa;
    pub use crate::output::step_summary;
}

pub mod repo {
    pub use crate::repo::{GitCliRepo, GixRepo, Repo};
}

pub mod provenance {
    pub use crate::provenance::Manifest;
}

pub mod quality_gates {
    pub use crate::quality_gates::{Gates, Thresholds};
}
```

**Step 3.** Update `crates/codelore-cli/src/main.rs` and `args.rs` to
import from `codelore_lib::cli_api::*` only. The current `use
codelore_lib::output::spa::SpaOptionsSnapshot;` becomes
`codelore_lib::cli_api::output::spa::SpaOptionsSnapshot;` etc.

**Step 4.** Tighten internal visibility. Change `pub mod analyses;`
etc. in `lib.rs` to `pub(crate) mod analyses;` ONLY if no external
crate consumer needs them. Keep `pub` if the test files in
`tests/` import them — they should migrate to `cli_api` too.

**Step 5.** Re-run full gate.

### Risks / tradeoffs

- **Test files** (`crates/codelore-lib/tests/*.rs`) currently import
  many internal modules. They'd need to either migrate to `cli_api`
  (architecturally correct) OR continue importing internal paths
  (which contradicts the encapsulation goal).
- **`pub(crate)`** on the internal modules means external crates
  (not just `codelore-cli` but any future `codelore-lib` consumer)
  ONLY see `cli_api`. This is the right shape but is a real
  breaking change for any out-of-tree consumer.
- The audit estimated **M cross-cutting**. Realistically this is
  closer to L-architectural because the entire CLI import shape
  changes.

### Estimated scope

~200-400 LOC of import migration + ~50 LOC façade module. Single PR
but touches every CLI file + every test that imports lib internals.

---

## F94 — Split `ingest.rs` (1455 LOC) into submodules

**Goal**: mechanical split of `crates/codelore-lib/src/facts/ingest.rs`
into the six topical submodules the audit suggested. No behavior
change; pure reorganization.

### Current shape

```
crates/codelore-lib/src/facts/ingest.rs   (1455 lines after F149)
```

Contains, roughly in source order:

- Producer/consumer crossbeam-channel + `ingest_loop` (the connection-owning thread)
- `ingest_complexity_at_head` (parallel-then-serial-drain pattern)
- `populate_clones_at_head` (tree-sitter fingerprinting at HEAD)
- `populate_imports_at_head` (parser-driven import edge insertion)
- `materialize_path_lineage` (canonical lineage CTE materialization)
- `apply_grouping` (changes-table rewrite under `--group-file`)
- `materialize_changes_bucketed` (time-bucket changeset collapse)
- `append_commit`, `append_change`, `append_entity_row`, `append_metric_row`, `append_alias_row`
- F149 `count_loc_and_hunks` deletion: the function lives in `gix_repo.rs`, not ingest.

### Target shape

```
crates/codelore-lib/src/facts/ingest/
  mod.rs            — re-exports + the `FactsDb::ingest` entry point
  loop.rs           — `ingest_loop`, the producer/consumer pump
                      (uses `crossbeam_channel::bounded(channel_capacity())`)
  complexity.rs     — `ingest_complexity_at_head` + helpers
  clones_head.rs    — `populate_clones_at_head` + clone-extraction helpers
  imports_head.rs   — `populate_imports_at_head`, `resolve_imports`,
                      `_resolved_imports` Appender
  lineage.rs        — `materialize_path_lineage` (the canonical lineage CTE)
  grouping.rs       — `apply_grouping` + `_changes_grouped` swap
  rows.rs           — `append_commit`, `append_change`, `append_entity_row`,
                      `append_metric_row`, `append_alias_row`
```

Note the audit's original list didn't include `rows.rs` but the
five `append_*` functions are now ~80 LOC and naturally cluster.
Group them or inline them into `loop.rs` per taste.

### Implementation

**Step 1.** Create `crates/codelore-lib/src/facts/ingest/` and move
`ingest.rs` to `ingest/mod.rs` initially. Verify compiles:

```bash
mkdir crates/codelore-lib/src/facts/ingest
git mv crates/codelore-lib/src/facts/ingest.rs crates/codelore-lib/src/facts/ingest/mod.rs
cargo build -p codelore-lib --features test-support
```

**Step 2.** Carve out each submodule. Move the relevant functions
+ their private helpers + their imports. The pattern for each
extraction:

1. Identify the function group + any private helpers only used by them.
2. Create the new file with module-level docs explaining the topical scope.
3. Move the code + adjust imports.
4. In `mod.rs`: add `mod <new_submod>;` and re-export anything the
   parent module's public API needs (e.g. `pub use loop::ingest_loop;`).
5. Build between each extraction so a bad cut surfaces immediately.

**Step 3.** Cross-check the `CommitEvent`-producing thread + the
`Connection`-owning consumer + the rayon HEAD-time scans stay on
their respective threads. The `Connection` is `!Send + !Sync` — if a
function moves into a submodule that's later called from rayon, the
type system will catch it.

**Step 4.** `mod.rs` ends up at ~100 LOC: just the entry-point
`FactsDb::ingest` + the public re-exports. The submodules pick up
the rest.

### Risks

- **Test compatibility**: every external test currently imports
  `codelore_lib::facts::ingest::IngestStats` (via the `pub use
  ingest::IngestStats;` in `facts/mod.rs`). The re-export keeps the
  external path stable.
- **Conditional compilation** (`#[cfg(feature = "spa")]` etc.) needs
  to follow the moved functions. Search for `#[cfg(` markers in the
  current file before splitting.
- **Differential test** (`tests/differential_repo_test.rs`) — no
  splits affect it directly; it tests the `Repo` trait, not ingest.

### Estimated scope

~1500 LOC moved, ~50 LOC new docs + re-exports. No tests need
migration (re-exports preserve all paths). Single commit, but
review will want a side-by-side. Recommend a "split-only" PR with
a diff comment confirming no functional changes.

### Verification

After the split:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
# Compare ingest LOC before/after:
git show <commit>:crates/codelore-lib/src/facts/ingest.rs | wc -l   # 1455
find crates/codelore-lib/src/facts/ingest -name '*.rs' | xargs wc -l | tail -1
# After: total should be ~1500 (some growth from new module docs +
# `mod` declarations).
```

---

## F119 + F148 + F145 + F161 — Output emitter refactor cluster

**Strategic note**: the audit triage and validation both flagged
these four findings as best done together. They touch the same
surface — the `format × analysis` dispatch matrix in
`crates/codelore-cli/src/main.rs` (2047 LOC, ~59% of which is one
giant `match` block) plus the per-analysis CSV and Markdown emitters
in `output/csv.rs` (826 LOC) and `output/markdown.rs` (similar
size). Shipping them as 4 separate PRs would force three of them
to live alongside the unmigrated emitters, creating an awkward
split-shape that lasts months.

### Recommended unified shape

**Target architecture (one PR, ~1500 LOC delta net deletion)**:

```
crates/codelore-lib/src/output/
  emit.rs              — `TabularEmit` trait + `EmitterStream` trait
                         (F148 + F161)
  csv.rs               — single `write_csv<W, R: TabularEmit>(rows, w)`
                         + ~33 hand-rolled functions DELETED (F119)
  markdown.rs          — single `write_markdown<W, R: TabularEmit>(rows, w)`
                         + ~33 hand-rolled functions DELETED (F148)
  json.rs              — already shipped F146 generic write_json (this
                         is the pattern to follow for csv + markdown)
  sarif.rs             — UNCHANGED (audit explicitly noted SARIF is
                         structured rule/result objects, not flat rows
                         — wrong fit for TabularEmit)
  gha.rs               — UNCHANGED (annotation lines, not rows)
  ndjson.rs            — already generic; stream-friendly
  ...
crates/codelore-cli/src/main.rs
  — `match (format, analysis)` collapsed to `dispatcher.emit(format,
    writer)` calls (F145). ~1200 LOC of the dispatch arm body deleted.
```

### Key design pieces

**`TabularEmit` trait** (F148) — one impl per Row struct:

```rust
pub trait TabularEmit {
    fn columns() -> &'static [Column];
    fn row(&self, push: &mut dyn FnMut(Cell));
}

pub struct Column {
    pub name: &'static str,
    pub align: Alignment,  // for markdown
}

pub enum Cell {
    Str(String),
    Int(i64),
    Float(f64, u8),       // value + decimal places
    Percent(f64, u8),
    Empty,
}
```

Each Row struct (`HotspotRow`, `CodeHealthRow`, etc.) gets a small
manual impl. The CSV and Markdown emitters consume this trait
uniformly; per-format formatting (CSV escaping, Markdown pipe
escaping, alignment) lives in the generic writer.

**`EmitterStream` trait** (F161) — for stream-friendly emitters:

```rust
pub trait EmitterStream<W: Write> {
    fn emit_header(&mut self) -> Result<()>;
    fn emit_row<R: TabularEmit>(&mut self, row: &R) -> Result<()>;
    fn finish(self) -> Result<()>;
}
```

CSV is mechanical; JSON needs array streaming; SARIF stays batch
(needs run-level totals); GHA already streams.

**Dispatcher registry** (F145) — one entry per analysis:

```rust
struct AnalysisDispatcher {
    name: AnalysisName,
    rows: Box<dyn FnOnce(&FactsDb, &Options) -> Result<Vec<Box<dyn TabularEmit>>>>,
}

const DISPATCHERS: &[AnalysisDispatcher] = &[
    AnalysisDispatcher { name: AnalysisName::Hotspots,    rows: ... },
    AnalysisDispatcher { name: AnalysisName::CodeHealth,  rows: ... },
    // ... 32 entries total
];

// main.rs analyze dispatch becomes:
let dispatcher = DISPATCHERS.iter().find(|d| d.name == analysis)
    .ok_or_else(|| CodeLoreError::UnknownAnalysisName{ ... })?;
let rows = (dispatcher.rows)(&db, &opts)?;
write_for_format(format, &rows, &mut writer)?;
```

(The above is a sketch — `Vec<Box<dyn TabularEmit>>` has lifetime
quirks; the real impl might use a generic over the Row type via
const-genericss or a macro that pairs `AnalysisName` with its row
type at compile time. The F147 `registry!` macro for `AnalysisName`
provides a template.)

### Byte-identical-output regression test

Per session memory `feedback_byte_identical_baseline_for_sql_refactors`,
every SQL refactor claimed semantic-preserving must prove
byte-identical output. The same applies here:

1. Before any code change, run the current binary against a test
   fixture for every (analysis, format) pair.
2. Capture all outputs.
3. After the refactor, re-run.
4. Diff. ANY difference fails the gate.

Script (run before + after):

```bash
mkdir -p /tmp/codelore_emit_baseline
for analysis in hotspots code-health coupling revisions ownership \
                authors xray knowledge-islands stale-code god-classes \
                arch-violations lead-time delivery-friction \
                pair-programming bus-factor clones live-clones \
                clone-coupling communities centrality summary ; do
  for format in csv json markdown ; do
    ./target/release/codelore analyze --analysis $analysis \
      --format $format --repo crates/codelore-lib \
      --no-cache 2>/dev/null \
      > /tmp/codelore_emit_baseline/${analysis}.${format}
  done
done
```

(`sarif` and `gha` are excluded since they're not changing.)

### Estimated scope

~1500 LOC net deletion (the 826-LOC CSV emitter + similar markdown
+ ~1200 LOC of dispatch boilerplate). ~500 LOC added (traits + impls
+ dispatcher registry). **Single PR is the right shape**; serialising
this across 4 PRs is busy-work without intermediate value.

### Risks

- Numeric formatting drift — clippy's `excessive_precision` lint
  triggered on Fisher tests this session because the upstream's
  output had trailing zeros that Rust required deduping. Same kind
  of subtle f64 → string rounding can drift on CSV emission. The
  byte-identical baseline catches this BEFORE merge.
- SARIF + GHA staying out of the trait is correct (different shape)
  but must be explicit in the code — leave a doc comment in
  `output/emit.rs` listing what's IN scope and what's NOT.

### Sequencing within the PR

1. Land `TabularEmit` trait + per-Row impls (no consumer changes
   yet — coexists with hand-rolled emitters).
2. Switch CSV emitter to consume the trait. Run baseline diff.
3. Switch Markdown emitter. Run baseline diff.
4. Collapse `main.rs` dispatch matrix using the new generic emitters.
   Run baseline diff.
5. Add `EmitterStream` trait + per-format implementations. Verify
   stream-vs-batch outputs match.
6. Delete the now-orphaned hand-rolled emitters.

---

## Report bookkeeping after each finding

For every closure:

1. **`docs/reports/deep_analysis_report.md`**:
   - §3 closure-log table → add a row.
   - §4 body → remove the `#### F<ID>` block (or move to "Fixed" if
     it lived in a partial-fix bucket).
   - §4½ validation table → flip the row to "Fixed (this session)".
   - §5 Active list → drop the F-ID, decrement count.

2. **`CHANGELOG.md`** `[Unreleased]`:
   - Add a one-paragraph entry in the appropriate `### Added` /
     `### Changed` / `### Fixed` / `### Performance` / `### Security`
     / `### Accessibility / UI polish` subsection.
   - End with `Closes F<ID>.`

## Per-commit workflow

The session-wide pattern that worked well:

```bash
# Stage + diff-stat the commit shape:
git add <files>
git diff --cached --stat

# Commit with detailed message (HEREDOC for multiline):
git commit -m "$(cat <<'EOF'
type(scope): short subject line

Detailed body...
Bullet points...

Closes F<ID>. N Active findings remaining.
EOF
)"

# Push immediately so CI runs:
git push 2>&1 | tail -3
```

**Important rules from session memory** (do NOT include in commits
unless they apply):

- **NEVER append `Co-Authored-By: Claude` to any commit message** —
  HARD RULE.
- No F-IDs in source code comments or docs (only in CHANGELOG + this
  handoff doc).
- No version numbers in code/docstrings/README — describe CURRENT
  contract.
- No static test counts in code comments — they drift.

## Local gate (matches CI exactly)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

If any step fails, fix BEFORE pushing. The session pre-flight script
`scripts/cut-release.sh` runs this same gate before tagging — narrower
local invocations have missed lints CI then caught.

## Final state

When all 7 remaining findings close:

- §5 Active count: `0 Active findings`.
- §3 closure-log table: 7 new rows.
- `CHANGELOG.md` `[Unreleased]`: 7 new entries.
- Total session closures: 16 (this session) + 7 (next session) = 23,
  i.e. every Active finding the 2026-06-21 validation pass enumerated.
