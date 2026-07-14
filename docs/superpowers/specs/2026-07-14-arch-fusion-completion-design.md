# Architecture Fusion Completion — Design

Completes the structure×history fusion thesis over the architecture graph. The
SCC kernel, `dependency-cycles`, `architecture-roles`, topo-levels, and the DSM
widget all exist; what is missing is the behavioral half of the story in two
places — which tangles are *alive* and where to *cut* them, and the
structure-vs-history agreement view in the matrix — plus a corpus-relative
anchor for the repo-level architecture numbers.

Three units, each independently shippable. Everything is additive: one new
analysis, one optional calibration-artifact section, optional output rows on an
existing analysis, and one new SPA cell-mode. No fact-store schema change, no
shipped row shape changes, no new vendored libraries.

## Unit A — `cycle-health` analysis

One row per non-trivial SCC (size ≥ 2) of the resolved import graph, computed
in Rust from the existing `analyses/import_graph.rs` kernel. No temp tables.

Row shape:

| column | type | semantics |
|---|---|---|
| `cycle_id` | u32 | rank by `size` descending, ties by lexicographically smallest member — deterministic |
| `size` | u32 | member count |
| `members_preview` | String | first 3 members lexicographically, `+N more` suffix; full membership remains `dependency-cycles`' job |
| `heat_pct` | f64 | cycle members' share of repo LOC churn (`loc_added + loc_deleted`) over the trailing `--window-days` window; same window anchoring (repo's last commit date) and lineage-aware source table as `effort-exposure` |
| `verdict` | String | `live` — at least one member appears in a window commit; `fossil` otherwise |
| `extract_candidate` | String | the member whose removal minimizes the size of the largest surviving SCC of the induced subgraph (trial-removal Tarjan per member); ties broken by smallest resulting total cyclic-node count, then lexicographically by path |
| `predicted_pc_drop` | Option\<f64\> | full-graph propagation-cost delta with the chosen candidate node removed; computed only for the chosen candidate, and only when `size ≤ 64` — absent above the bound (honest absence), where `extract_candidate` falls back to highest in-SCC degree |

Sorted by `heat_pct` descending, then `size` descending — the live big tangles
first. Registered through the standard analysis recipe (registry variant +
`as_str`, CLI dispatch + explain entry, CSV/markdown emitters, JSON via the
generic writer). Reuses `Options::window_days`; no new CLI flags. No new
quality gate in this slice; feeding `arch_health` is an explicit follow-up
decision once heat semantics have been observed on real repositories.

## Unit B — DSM co-change overlay (SPA only)

`renderArchMatrix` (`output/spa/js/40_architecture.js`) gains a cell-mode
toggle rendered with the existing `wt-btn` toolbar pattern and persisted in the
Alpine layout store, mirroring the force-graph's layout toggle:

- **Structure** — today's rendering, unchanged, remains the default.
- **Fusion** — each module-pair cell is classified from data already present in
  the SPA payload (coupling rows + import edges), aggregated at the shared
  module depth:
  - *structural + co-change* — agreeing dependency; blended color graded by
    co-change degree;
  - *structural only* — candidate fossil edge; dimmed;
  - *co-change only* — modularity violation rendered in the matrix; amber, the
    same hue the force graph uses for violation edges.

Back-edge red below the diagonal is preserved in both modes. Cells are never
color-only: tooltips name the class, and a legend row is added to the widget.
The widget stays registered for theme-reactive re-render and the existing
depth selector. When the payload carries no coupling rows, Fusion mode renders
the structural view with a "no co-change data" hint.

## Unit C — corpus-relative architecture percentiles

**Artifact.** `CalibrationArtifact` gains an optional `repo_metrics` section:
for each of `propagation_cost` and `cycle_file_share` (files in non-trivial
SCCs ÷ total graph files), the sorted raw per-repo values from the corpus —
one value per included repo, so ~99 floats per metric. `format_version` stays
`1`; an absent section means no lens (the corpus-percentile precedent), so
existing artifacts remain fully valid.

**Calibrate.** Head-only ingest additionally runs the HEAD-time imports passes
(populate + resolve — both history-free, so shallow pinned checkouts keep
working). `codelore calibrate` computes `import_graph::graph_metrics()` per
repo and pools the two repo-level values. The embedded world artifact is
rebuilt within this slice — a minutes-scale job on the shallow head-only
pipeline — which doubles as corpus-scale validation of head-only imports.

**Output.** `architecture-metrics` — whose `(metric, value)` row shape absorbs
additive rows naturally — emits, when an active artifact carries
`repo_metrics`:

- `corpus_percentile:propagation_cost` — rank of this repo's value among the
  corpus values, `0..1`, midpoint rank for ties;
- `corpus_percentile:cycle_file_share` — same;
- `corpus_n` — the number of corpus observations backing the percentiles.

The percentile base is ~99 repo-level observations — coarse by construction;
`corpus_n` states it, and docs present the lens as "percentile among N corpus
repositories", never as a fine-grained calibration. Rows are absent when no
artifact is active or the artifact lacks `repo_metrics`. The SPA Architecture
factor tile's detail line includes the propagation-cost percentile when
present. Artifact resolution and `corpus_vintage` provenance stamping reuse
`calibration::load_active_artifact` unchanged.

## Cross-cutting

- **CACHE_EPOCH bump.** Existing head-only caches lack imports; the new reader
  expects them. One epoch bump orphans them cleanly.
- **Additivity contract.** Without an active artifact (or with one lacking
  `repo_metrics`), `architecture-metrics` output is byte-identical to today —
  enforced by a strip-and-compare test like the code-health corpus lens.
- **Honest absences.** No cycles → zero `cycle-health` rows. No coupling data
  → Fusion cell-mode falls back with a hint. No `repo_metrics` → no percentile
  rows.
- **Conventions.** No ticket IDs or version references in code or docs;
  CHANGELOG `[Unreleased]` carries one entry per user-visible change; no new
  vendored JS.

## Testing

- **cycle-health**: fixture with a constructed import cycle (reuse/extend the
  `dependency-cycles` test fixture); hand-computed `heat_pct` on fixed commit
  dates; extraction-candidate correctness on a known topology (a cycle with an
  articulation member); live/fossil via window placement; determinism of
  `cycle_id` and tie-breaks.
- **Head-only imports equivalence**: extend the existing full-vs-head-only
  equivalence test to assert identical `imports` (resolved) row sets.
- **Artifact**: roundtrip with and without `repo_metrics`; percentile lookup
  unit tests (ties, extremes); `calibrate` e2e on local fixtures asserts the
  section is populated.
- **Additivity**: `architecture-metrics` strip-and-compare with/without an
  active artifact.
- **SPA**: integration test asserts the payload fields the Fusion mode needs;
  browser test toggles the cell-mode and asserts zero console errors.
- **Real-CLI**: `cycle-health` and `architecture-metrics` (with the rebuilt
  world artifact) run against this repository; plausible rows pasted into the
  implementation report.

## Follow-ups (explicitly out of scope)

- `arch_health` / factor-tile formula integration of cycle heat — decide after
  observing real-repo values.
- A `max_live_cycles`-style quality gate — same reason.
- DSM row/column reordering by SCC membership — cheap once Fusion cells exist.
- Per-member `cycle-health` detail rows.
