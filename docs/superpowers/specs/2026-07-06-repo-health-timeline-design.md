# Repo Health Timeline — architectural + code + combined health over sampled history

**Status:** approved design. **Depends on sub-project 1** — see
`2026-07-06-rev-parameterizable-code-health-design.md`. This is piece 2 of 2.

## Decomposition (why two specs)

The design's code-health-over-time line was originally scoped as a "structural
proxy." Investigating the internals showed `code_health` is hardwired to HEAD
tables and cannot run at a historical rev without either a throwaway proxy metric
(a second, divergent definition of "code health") or copy-pasting private
biomarker SQL (drift risk). The clean, long-term choice is to fix the root cause:

- **Piece 1 — rev-parameterizable `code_health`** (its own spec): teach the
  existing `code_health` engine to run against a pluggable complexity/imports
  source + history cutoff + clone toggle, HEAD output byte-identical. One
  definition of code health, evaluable at any rev.
- **Piece 2 — this timeline**: a thin `health-trend` analysis that, at each
  sampled rev, builds the rev-scoped sources and calls the *same* engine, then
  composes the three scores and the SPA widget.

Build piece 1 first; piece 2 sits on top of it.

## Problem

CodeLore answers "how healthy is the code *now*" (`code-health`, HEAD-only) and
"is the architecture decaying, and when did it start" (`architecture-trend`, raw
propagation-cost / cycle-count / tangle over ≤12 sampled revs). It does **not**
produce a single architectural-health *score*, a code-health score *over time*,
or a *combined* health score at all. Users want one graph showing how
architectural health, code health, and an overall combined score move over the
project's history.

## Decisions (settled in brainstorming)

| Fork | Decision |
|---|---|
| Code-health-over-time | Computed by the **rev-parameterizable `code_health` engine** (piece 1) at each sampled rev — the *same* definition as HEAD, not a separate proxy. Per-rev complexity scan reuses the walker's blob reads; the DRY/clone biomarker is omitted at historical revs (re-normalized), per piece 1 |
| Complexity placement | Complexity lives in the **code-health** score only; architecture stays purely structural (dependency graph). Complexity reaches the combined score through the code-health half — the two components stay orthogonal |
| Combined weighting | Equal blend: `0.5·arch_health + 0.5·code_health` |
| Primary view | **Required**: one overlaid 3-line chart (combined bold) over time. **Optional**: a toggle to split into small multiples |
| Cost | On-demand, never cached, ≤12 samples, ~2× the existing `architecture-trend` cost |

## 1. Scoring model

Three scores, all **0–100, higher = healthier**, sharing one banding:
**green ≥ 70, yellow 40–69, red < 40**.

### Architectural health (purely structural — no complexity)

From the per-rev import-graph metrics the trend walker already computes
(`GraphMetrics`: `propagation_cost ∈ [0,1]`, `cyclic_nodes`, `largest_cycle`,
`n` = files/nodes):

```
arch_risk   = 0.5·propagation_cost              (a random change reaches this fraction of the system)
            + 0.3·(cyclic_nodes / n)            (fraction of the codebase tangled in dependency cycles)
            + 0.2·(largest_cycle / n)           (how much of it the single biggest tangle spans)
arch_health = 100 · (1 − min(1.0, arch_risk))
```

- `n == 0` (no resolvable graph at that rev) ⇒ `arch_health = 100` (nothing to
  be unhealthy about) — documented; matches "empty graph is trivially acyclic".
- Propagation cost is the dominant term (the validated DSM decoupling signal,
  already `[0,1]`); cycle terms are the acute problems.

### Code health (the rev-parameterizable engine — carries complexity)

Computed by piece 1's `run_code_health_scoped(db, opts, cx)` — the *same*
`code_health` engine as HEAD, not a separate metric. Per sample the timeline
builds a `HealthScanCtx` for that rev: a rev-scoped complexity source (from the
per-rev blob scan), a rev-scoped imports source (from `import_graph_at_rev`), a
`history_cutoff` at the sample's date (churn / author / coupling filtered to
`commits.date <= ts`), and `include_clones = false`. The score is the canonical
composite `100·(1 − 0.50·structural_risk − 0.30·churn − 0.20·author_fragmentation)`
with the DRY biomarker omitted and the remaining four biomarker weights
re-normalized to sum to 1.0 (see piece 1).

- **Consistency:** every sample — including HEAD — uses the same
  `include_clones = false` reduced form, so the timeline line is internally
  consistent. It may sit slightly above CodeLore's canonical HEAD `code-health`
  number (which includes DRY); documented in the analysis output and widget.
- Only the complexity biomarkers require the per-rev blob parse; shotgun /
  churn / author are SQL. This is the ~2× cost over `architecture-trend`.

### Combined health

```
combined_health = 0.5·arch_health + 0.5·code_health
```

Equal weighting — systemic (architecture) and local (code) health are both
first-class; a documented constant, retunable later.

## 2. Architecture & reuse

New analysis module `analyses/health_trend.rs` reusing the
`architecture_trend.rs` sampler wholesale:

- **Same sampling:** `SAMPLE_POINTS = 12` (reuse the constant), same
  `evenly_spaced_indices` over date-ordered commits, oldest→newest, both ends
  included.
- **Same per-rev walk:** for each sampled `(rev, date)` the existing walker
  already reads each live-at-rev blob to resolve imports (`resolve_imports_at_rev`
  → `graph_metrics`). Extend the per-rev step to **also** run the tree-sitter
  complexity analyzer on the same blobs (`crate::complexity::compute_for_file` +
  `Tier1Language::from_path`, the primitives `ingest_complexity_at_head` already
  uses) into an in-memory per-rev complexity set, then compute the biomarker
  intensities + code-health proxy. Architectural metrics come from the graph as
  today.
- **SQL-derived terms** (shotgun coupling, churn, author fragmentation) are
  computed per rev with a date filter — no extra blob work.

The shared per-rev scoring is factored into one helper so `health-trend` and any
future consumer compute the three scores identically.

### Output row

```rust
pub struct HealthTrendRow {
    pub date: String,        // YYYY-MM-DD
    pub rev: String,         // short SHA
    pub files: u32,
    pub arch_health: f64,    // 0..=100
    pub code_health: f64,    // 0..=100
    pub combined_health: f64,// 0..=100
    pub arch_band: String,   // "red" | "yellow" | "green"
    pub code_band: String,
    pub combined_band: String,
}
```

Rows emitted oldest-first. Banding via one shared `health_band(score) -> &str`
helper (green ≥ 70, yellow ≥ 40, else red), used for all three.

## 3. SPA graph

New `SpaDashboard` field `health_trend: Vec<HealthTrendRow>`, populated in
`build_spa_dashboard` following the exact `architecture_trend` pattern (opens its
own repo, degrades to empty on failure), consumed by a new `renderHealthTrend`
widget in `widgets.js`.

- **Default view (required):** one ECharts line chart, x = sample dates, y =
  0–100, three series — **Combined** (bold, emphasized), Architectural, Code
  (lighter). Faint red/yellow/green horizontal band background (< 40 / 40–69 /
  ≥ 70). Tooltip shows date, short SHA, and all three scores.
- **Split toggle (optional):** an Alpine-backed toggle re-renders as three
  stacked small multiples (Combined / Architectural / Code), each its own
  0–100 mini-chart with the same band background — for reading each line in
  isolation. Same data, no recompute.
- Empty/degraded data (fewer than 2 samples, e.g. a 1-commit repo) renders a
  "not enough history" placeholder, matching how other trend widgets handle
  sparse input.

## 4. CLI / output

`health-trend` is a normal analysis name: `codelore analyze --analysis
health-trend` emits CSV/JSON/etc. via the standard emitters (columns = the
`HealthTrendRow` fields). It is **on-demand, never cached** (like
`architecture-trend`), and carries the same module-doc cost warning
("re-parses source at many revisions … markedly heavier … computed on demand").

## 5. Testing

- Unit: `health_band` boundaries (39.9/40/69/70); `arch_health` on a known
  graph (acyclic ⇒ 100·(1−0.5·pc); fully-tangled ⇒ low); the code-health proxy
  matches the canonical formula-minus-DRY on a fixture; combined = mean of the
  two.
- Integration (`test-support` fixture): a repo with ≥2 commits produces a
  `HealthTrendRow` per sample, oldest-first, all scores in `[0,100]`, bands
  consistent with scores; a repo whose architecture degrades across commits
  shows `arch_health` decreasing.
- SPA integration: `health_trend` appears in the embedded dashboard JSON; the
  widget renders (extend the existing `spa_integration_test` shape assertions).
- No `CACHE_EPOCH` bump (on-demand analysis, no schema change); no `Repo` trait
  change (reuses `read_blob_at`).

## Out of scope (v1)

Per-commit (unsampled) resolution; caching the trend; the DRY/clone biomarker at
historical revs; cross-consistency with the canonical HEAD `code-health` number
(the timeline is internally consistent by design); configurable combined
weighting or band thresholds (documented constants).
