# Rev-parameterizable code_health — evaluate code health at any revision

**Status:** approved direction, spec for review.

**Sub-project 1 of 2** for the Repo Health Timeline
(`2026-07-06-repo-health-timeline-design.md`). This piece makes `code-health`
computable at an arbitrary historical revision; the timeline (piece 2) is then a
thin consumer that calls this engine at each sampled commit.

## Problem

`run_code_health` is hardwired to HEAD. Its biomarker pipeline reads HEAD-only
sources with no rev/date parameter:

- **complex-method / large-method** — the private `BIOMARKERS_INSERT` SQL bakes
  in `FROM complexity_metrics` (the HEAD complexity table); the biomarker
  "universe" query does the same.
- **god-class** — `run_god_classes` reads the HEAD `imports` table (fan-in/out)
  and `complexity_metrics` (cognitive).
- **DRY** — `run_clones` walks the working tree on disk; cannot target a rev.
- **shotgun-surgery** — `run_coupling` mines *full* co-change history, never
  date-filtered.

Only churn / author-fragmentation already flow from parameterized history
sources (`lineage::source_table`). This means "code health at commit X" — needed
by the timeline and any future point-in-history health feature — is impossible
without either duplicating the biomarker SQL (drift risk; `refactoring_targets`
shares `code_health_biomarkers_v1`) or fixing the root cause.

## Goal

Extend the source-table placeholder convention the main `code_health` SQL
*already* uses (`{cm_src}`, `{src}`) to the three hardwired spots, plus an
explicit clone/DRY toggle, so one engine computes code health at HEAD or any
rev. **HEAD output stays byte-identical** (parameters default to today's table
names — a no-op at HEAD).

## Design

### 1. The scan context

```rust
/// What revision / sources a code-health scan runs against. `head()` resolves
/// to today's HEAD tables so the existing behaviour is unchanged.
#[derive(Debug, Clone)]
pub struct HealthScanCtx {
    /// Complexity source table (default `"complexity_metrics"`).
    pub complexity_source: String,
    /// Imports source table for god-class fan-in/out (default `"imports"`).
    pub imports_source: String,
    /// When `Some(ts)`, history-derived terms (churn, author fragmentation,
    /// coupling) are restricted to `commits.date <= ts`.
    pub history_cutoff: Option<String>,
    /// Include the clone/DRY biomarker. `true` at HEAD (canonical); `false`
    /// at a historical rev, where clone detection is not available.
    pub include_clones: bool,
}

impl HealthScanCtx {
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
```

### 2. Entry points (public API unchanged)

```rust
// Existing signature preserved — delegates to the scoped form with a HEAD ctx.
pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    run_code_health_scoped(db, opts, &HealthScanCtx::head())
}

// New: the parameterized engine.
pub fn run_code_health_scoped(
    db: &FactsDb,
    opts: &Options,
    cx: &HealthScanCtx,
) -> Result<Vec<CodeHealthRow>>;
```

Every current caller (SPA, quality gates, `refactoring_targets`, CLI dispatch)
keeps calling `run_code_health` and is unaffected.

### 3. Parameterization, spot by spot

- **Biomarker SQL (`BIOMARKERS_INSERT`, universe query):** replace the literal
  `FROM complexity_metrics` with `{cm_src}`, substituted from
  `cx.complexity_source` — mirroring how the main `SQL` already substitutes
  `{cm_src}`. At HEAD `cx.complexity_source == "complexity_metrics"` ⇒ identical
  string ⇒ byte-identical output.
- **god-class:** add `run_god_classes_scoped(db, opts, complexity_source,
  imports_source)`; `run_god_classes` delegates with the HEAD defaults. Its two
  CTEs swap `complexity_metrics`/`imports` for the passed names.
- **shotgun-surgery / coupling:** `materialize_centrality` calls a coupling pass
  that honours `cx.history_cutoff`. Realized by wrapping the changes source in a
  date-filtered view when a cutoff is set (reusing the `lineage` source-table
  seam — a `commits.date <= ts` filter), not by forking `run_coupling`'s logic.
- **churn / author fragmentation:** the main `SQL` already reads `{src}` =
  `lineage::source_table(opts)`. When `cx.history_cutoff` is set, `{src}`
  resolves to a date-filtered changes view so these terms are rev-scoped too.
- **DRY / clones:** when `cx.include_clones` is false, skip `run_clones` and the
  `dry` biomarker rows entirely.

### 4. Weight re-normalization when DRY is excluded

`structural_risk` weights today: complex-method 0.30, god-class 0.25,
large-method 0.15, **dry 0.15**, shotgun 0.15 (capped at 1.0).

- `include_clones = true` (HEAD): weights unchanged ⇒ byte-identical.
- `include_clones = false` (historical rev): the four retained weights are
  **re-normalized to sum to 1.0** (each divided by 0.85: complex-method 0.353,
  god-class 0.294, large-method 0.176, shotgun 0.176). This keeps the
  `structural_risk` *scale* consistent whether or not DRY is present, so a
  historical score is comparable to a HEAD score computed the same way. The
  renormalized weights are documented constants. (Chosen over "keep weights,
  cap at 0.85" because a systematically-lower ceiling would bias every
  historical point downward vs HEAD.)

The band thresholds (`>= 0.55` red, `>= 0.28` yellow) and the composite formula
(`100·(1 − 0.50·structural_risk − 0.30·n_cn − 0.20·n_au)`) are unchanged.

### 5. Rev-scoped source construction (helpers this piece provides)

So piece 2 (and tests) can build the sources a non-HEAD `HealthScanCtx` points
at, this piece ships two small helpers on `FactsDb` (or a new
`facts::ingest::at_rev` module):

- `ingest_complexity_at_rev(repo, rev, live_paths, temp_table)` — mirrors
  `ingest_complexity_at_head` but reads `repo.read_blob_at(rev, path)` and writes
  to a caller-named temp table with the `complexity_metrics` column shape.
- `materialize_imports_at_rev(graph, temp_table)` — writes the edges of an
  in-memory `ImportGraph` (from `import_graph_at_rev`) into a temp table with the
  `imports` column shape, for god-class fan-in/out.

These are the only genuinely new compute; everything else is table-name
substitution.

## Byte-identical guarantee (hard requirement)

Per the repo's SQL-refactor rule, HEAD `code-health` output MUST be
byte-identical before/after. Proof obligation in the plan: capture
`codelore analyze --analysis code-health --format csv` (and `--format json`) on
a fixture at the pre-refactor commit, run twice post-refactor, and `diff` —
attach the result to the refactor commit message. The `HealthScanCtx::head()`
path resolves every placeholder to its current literal, so the emitted SQL
strings are unchanged.

## Testing

- Unit: `HealthScanCtx::head()` field values; the renormalized-weight constants
  sum to 1.0.
- Golden/byte-identical: `run_code_health` vs `run_code_health_scoped(…head())`
  produce identical `Vec<CodeHealthRow>` on the `biomarker_repo` fixture.
- Scoped: on `biomarker_repo`, `run_code_health_scoped` with a temp complexity
  source + `include_clones=false` returns rows whose `structural_risk` excludes
  the DRY contribution and uses renormalized weights (assert a known file's
  score matches the hand-computed no-DRY value).
- `ingest_complexity_at_rev` / `materialize_imports_at_rev`: on a 2-commit
  fixture, the rev temp tables match what a HEAD scan of that same tree would
  produce.
- No `CACHE_EPOCH` bump (no schema change — temp tables are session-scoped);
  no `Repo` trait change (uses existing `read_blob_at`).

## Out of scope (this piece)

The timeline analysis, the three composite scores, the SPA widget (all piece 2);
clone/DRY detection at historical revs (deliberately omitted, re-normalized
around); making `refactoring_targets` rev-aware (it keeps consuming the HEAD
biomarker table unchanged).
