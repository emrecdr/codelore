# Design: Code Health v2 + Refactoring-Targets + Dashboard IA overhaul

**Status:** Draft for review (brainstorming output)
**Origin:** Competitor-analysis session vs. CodeScene (which had analyzed CodeLore's own repo). Goal: adopt CodeScene's *legibility* without copying its model — beat it where CodeLore's architecture is structurally stronger.
**Relationship to existing roadmap:** Complements, but is distinct from, the architecture-graph fusion roadmap (modularity-violations / unstable-interface / DSM). This initiative is about the *code-health metric* and *dashboard information architecture*.

---

## 1. Motivation & strategic thesis

CodeScene's flagship is **Code Health**: a 1–10 score aggregating ~25 named threshold "biomarkers", normalized against in-repo maxima. Its dashboard is **one circle-pack "system map" recolored by one swappable lens at a time** (Health / Hotspots / Friction / Refactoring-Targets), with a tabbed file drill-down.

Two research findings (see §11) set the strategy:

1. **A static composite is already near the accuracy ceiling.** *Ghost Echoes Revealed* (Borg & Tornhill, ICSME 2024) found CodeScene Code Health matches state-of-the-art ML (F1≈0.96) and beats the average human expert. **Re-implementing their biomarkers ties at best.**
2. **CodeLore beats it only by attacking the two things their model structurally cannot do**, both of which are where CodeLore is already strongest:
   - **In-repo normalization** makes scores non-comparable across repos and lets a uniformly-bad codebase look "healthy." The peer-reviewed alternative is **corpus-relative percentile scoring** (Alves/Ypma/Visser, ICSM 2010). Nobody ships it in a local CLI.
   - **Static snapshots miss where bugs land.** Process/history metrics out-predict static structure for defects (Rahman & Devanbu, ICSE 2013). CodeScene keeps behavior (hotspots) and structure (health) in *separate* views. CodeLore already computes both at function granularity and can **fuse** them.

### The unifying model

The **health metric, the bivariate map, and the refactoring-targets ranking are the same object viewed three ways.** The map's two axes *are* the two research recommendations:

- **Fill hue = structural health** (percentile-relative, band-categorized, explained by evidence-selected biomarkers).
- **Fill saturation = behavioral activity** (churn × Fisher-significant coupling × ownership fragmentation).
- **The danger quadrant (sick + churning) = the top of the refactoring-targets funnel.**

CodeScene cannot draw this — it paints one lens at a time (its "Combined Aspects" toggle is an admission the single-lens model fails at comparison).

## 2. Goals / non-goals

**Goals (Phase 1):**
- Evolve `code-health` into a transparent, percentile-relative, behaviorally-fused score reported as **risk bands + per-entity percentile**, with an exposed biomarker/behavioral breakdown.
- Add a first-class, **effort-aware** `refactoring-targets` analysis that flows through every emitter (`csv/json/sarif/markdown/check`).
- Replace the flat 17-widget SPA with **one hero bivariate map + coordinated (linked-brushing) views + progressive-disclosure tabbed drill-down**.

**Non-goals (deferred — see §9):** cross-repo corpus table, full biomarker set (nesting/LCOM4/nargs/complex-conditional), test-coverage ingestion, 2D code-cartography layout, LLM enrichment, own-repo defect calibration.

## 3. Component 1 — Code Health v2

Evolve the existing composite (`analyses/code_health.rs`); do **not** replace it with a CodeScene clone and do **not** add a second parallel score. Keeping the behavioral core is the differentiator.

### 3.1 Biomarkers (Phase-1 set only — all from already-persisted data)
The raw `complexity_metrics` table already persists per-function `cyclomatic`, `loc`, `cognitive`, `nom`, `nexits` (schema `crates/codelore-lib/src/facts/schema_v1.sql:103`). Biomarkers must read the **raw** per-function table, not `complexity_metrics_grouped` (which carries only `cognitive`+`mi`, `grouping.rs:297`).

- **Complex Method** — cyclomatic (already persisted).
- **Large Method** — loc (already persisted).
- **God Class** — reuse existing `god-classes` analysis.
- **DRY / duplication** — reuse existing `clones` analysis.
- **Shotgun Surgery / Divergent Change** — **reframe the existing Fisher-significant temporal change-coupling** as named biomarkers. These smells are *definitionally* temporal, so a statistically-significant co-change signal is a better-grounded detector than any static heuristic. Near-zero cost; CodeScene can only approximate it.

Scoring rules (evidence-based, not CodeScene-cloned):
- **Density per LOC, never raw counts** — kills the size confound (Palomba EMSE; Olbrich: God-class harm largely disappears after size normalization).
- **Continuous intensity, not binary presence** (Palomba "Smell like teen spirit").
- **Co-occurrence multiplier** — ≥2 smells in one unit is the empirically highest-risk bucket (+100–350% fault-proneness).

### 3.2 Percentile-relative scoring
Replace in-repo min-max normalization with **percentile rank**. Phase 1 uses **self-relative** percentiles — a single DuckDB `PERCENT_RANK()` window over the repo's own per-language function distribution (free, no corpus). The score is *categorized into risk bands* (Red / Yellow / Green); the headline is the **band distribution (%R/Y/G) + per-entity percentile** ("worse than 92% of Rust functions"), **not a single scalar** — averaging hides the tail, and the tail is the signal (the core critique of the Maintainability Index).

Architecture speaks "percentile" from day one, so Phase 2's baked cross-repo corpus is a *data swap*, not a redesign.

### 3.3 Behavioral fusion
Weight structural risk by behavioral risk: a complex function that is *also* a high-churn hotspot with fragmented/departed ownership scores far worse than an equally-complex but stable, single-owner one. Terms already in the fact store: churn, Fisher-coupling centrality, ownership Herfindahl, knowledge-islands, Kamei features. **Must size-normalize** so the behavioral term adds orthogonal signal, not re-weighted bigness.

### 3.4 Output & cache
`CodeHealthRow` widens from `{path, cognitive, score}` to include `band`, `percentile`, and the biomarker + behavioral sub-component breakdown (currently computed in-SQL but discarded). This is an **intentional semantic change** → bump `CACHE_EPOCH` (`cache.rs`) and update fixtures with a documented, reviewed diff. This is *not* a byte-identical refactor; do not claim byte-parity.

## 4. Component 2 — `refactoring-targets` analysis

New first-class `--analysis refactoring-targets` (registered in `analysis.rs`; flows to all emitters + `check`).

- **Ranking = risk ÷ effort** (risk ÷ LOC touched), not raw risk — the accepted modern effort-aware framing (Popt / PofB20).
- Ship **ManualUp** (rank by ascending size) as a **built-in baseline the composite must beat**, surfaced in `explain`. If the composite can't beat "inspect the small dense files first," the tool says so. No competitor is this honest.
- Apply an **EA-Z probability floor** (`score / max(LOC, k)`) to avoid tiny-change ranking artifacts.
- Each target annotated with its **dominant biomarker** as the "type" (complex-hotspot / duplicated-and-coupled / knowledge-island-debt / shotgun-surgery).
- Compute thresholds per time-window, not one global cut (recent commits are under-labeled — verification latency).

## 5. Component 3 — SPA information-architecture overhaul

Retire the flat wall of ~17 widgets (`output/spa/widgets.js`). New IA: **KPI header → ONE hero circle-pack → tabbed file drawer** (Metrics / Biomarkers / Coupling / Trend). The other widgets become drill-down panels reachable from the map, not a scroll wall.

- **Circle-pack stays** (2026 perception studies still rank it highest for hierarchy accuracy; GitHub Next uses it) — but the **lens-swap is killed** via a **bivariate health×activity glyph** (3×3 blend; CVD-safe palette + colorblind toggle; cap 3 encodings/glyph; coverage ring reserved for Phase 2).
- **Linked brushing** via an **Alpine store** (zero new deps — already vendored): selecting a file/dir highlights it across map/DSM/coupling/ownership/trend simultaneously. One "focus entity" model; highlight (don't hide) non-matches.
- **On-demand hierarchical edge-bundled coupling** drawn *into* the map for the selected node (not a separate chord widget); fade unrelated edges.
- **Bivariate legend is the primary filter** — clicking a 3×3 cell brushes all matching files (collapses "filter" and "understand" into one gesture).
- **Deterministic node positions** so a file sits in the same place across every analysis (cheap spatial memory now; on-ramp to Phase-2 cartography).
- **Vendor Observable Plot** (UMD, load d3 first — verified single-file vendorable) for terse trend/boxplot sparklines in the drawer. **Do NOT** vendor cosmos/sigma.js (ESM-only, not vendorable via `include_str!`, and overkill at this data scale) or any 3D "software city" (gimmick for an at-a-glance analytics dashboard).

Vendoring follows the existing `build.rs` SHA-256-pin-from-jsDelivr pattern; Phase-1 net-new lib = Observable Plot only.

**Dogfood note:** `widgets.js` is CodeScene's #1 refactoring target (health 2.09, 2,840 LOC, complexity-67 render functions). Building this IA *is* the refactor that fixes CodeLore's own worst hotspot.

## 6. Data / schema / cache implications

- **No new ingested facts in Phase 1** — biomarkers and percentiles are computed in SQL over `complexity_metrics` (+ existing coupling/ownership/god-classes/clones outputs).
- `CACHE_EPOCH` bump (semantic change to `code-health`).
- Schema unchanged in Phase 1 (nargs/nesting deferred to Phase 2, which will migrate the schema and invalidate cache naturally).
- Percentile corpus vintage (Phase 2) stamped into the existing provenance sidecar manifest for reproducibility/audit.

## 7. Phasing

| | Phase 1 (this spec) | Phase 2 | Phase 3 |
|---|---|---|---|
| Metric | biomarker health v2, self-relative percentile, behavioral fusion, bands | **cross-repo corpus percentile table**; full biomarkers (nesting→Bumpy Road, nargs→Many Args, Complex Conditional, LCOM4) | own-repo defect calibration (SZZ-lite) |
| Analysis | effort-aware `refactoring-targets` | test-coverage (LCOV) ingestion | — |
| Viz | hero bivariate map + linked brushing + tabbed drawer | coverage ring; 2D code-cartography (Rust-side UMAP) alt layout | — |
| LLM | — | — | advisory `--llm` on `explain`/`diff`, off the scored path, content-hash cached |

## 8. Testing & validation strategy

- **Metric:** golden fixtures for the new `code-health` shape (bands + percentile + breakdown); assert the **ManualUp baseline comparison** is emitted and that the composite's effort-aware ranking beats it on the fixture repo.
- **Differential backend parity:** any change touching change-coupling reframing must keep `GixRepo`/`GitCliRepo` event streams identical (existing `differential_repo_test.rs` gate).
- **Determinism:** percentile + biomarker SQL must be stable across runs (byte-diff a repeat run on the fixture).
- **SPA:** snapshot the `SpaDashboard` JSON contract; smoke-test the map renders + linked-brushing store wiring.
- **`just lint` / `just ci` must match CI exactly** (`cargo clippy --workspace --all-targets --all-features -- -D warnings`).

## 9. Risks & mitigations

- **Behavioral/structural double-counting** (churn ≈ LOC ≈ complexity) → size-normalize; validate orthogonality on fixtures.
- **Self-relative percentiles jitter on small repos** → document; Phase-2 corpus stabilizes. Offer band-based headline (robust) over scalar.
- **SPA rewrite risk** (widgets.js is already the worst hotspot) → incremental: new hero map + drawer land first; legacy widgets demoted to drill-down panels, removed only once replaced.
- **Framing honesty:** *Code Red*'s 15×/124% figures are vendor-authored (no independent replication) → frame CodeLore's metric as *prediction/association*, not causation; attribute directionally.

## 10. What we explicitly do NOT copy from CodeScene

1. **In-repo maxima normalization** — the core weakness we're beating (§3.2).
2. **The ~25-biomarker list wholesale** — several are size-confounded/weak (Feature Envy, Data Class, Lazy Class, Long Parameter List). Pick by evidence, not catalog. (Also don't cargo-cult SonarQube rules — small effects, severity ≠ fault-proneness.)
3. **A purely static composite** — ties at best; behavioral+corpus signal is the only path to beating it.
4. **MI / raw cyclomatic as a headline** — LOC-dominated, average-blurred; keep only as drill-down annotations.
5. **The single-lens-swap map** — replaced by bivariate + coordinated views.
6. **LLM auto-refactor as a scoring input** — advisory only, out of the deterministic score.

## 11. Research sources (key)

- *Ghost Echoes Revealed* (Borg, Ezzouhri, Tornhill, ICSME 2024) — https://arxiv.org/html/2408.10754v1
- Alves, Ypma, Visser, *Deriving Metric Thresholds from Benchmark Data* (ICSM 2010) — https://webarchive.di.uminho.pt/wiki.di.uminho.pt/twiki/pub/Personal/Joost/PublicationList/AlvesYpmaVisserICSM2010.pdf
- Rahman & Devanbu, *Revisiting Process vs Product Metrics* — https://arxiv.org/pdf/2008.09569
- Palomba et al., diffuseness & impact of code smells (EMSE) — https://link.springer.com/article/10.1007/s10664-017-9535-z
- Olbrich, *Are all code smells harmful?* (size confound) — https://www.semanticscholar.org/paper/171dbc23ef96bc6c418c9ecc1d1036a4b6f6da6e
- Effort-aware metrics (EMSE 2022; Popt/PofB20/IFA) — https://link.springer.com/article/10.1007/s10664-022-10186-7 · ManualUp/Down — https://ieeexplore.ieee.org/document/9115238/
- SIG maintainability model (yearly recalibration) — https://www.softwareimprovementgroup.com/blog/maintainability-model-2024-update/
- Coordinated multiple views (Roberts SOTA) — https://www.semanticscholar.org/paper/State-of-the-Art:-Coordinated-&-Multiple-Views-in-Roberts/5fabb8fe27edd41b61d6231318f5479f299c8388
- Bivariate choropleth (Josh Stevens) — https://www.joshuastevens.net/cartography/make-a-bivariate-choropleth-map/
- Hierarchical edge bundling — https://www.data-to-viz.com/graph/edge_bundling.html
- Software cartography / Codemap (Kuhn) — https://scg.unibe.ch/archive/papers/Kuhn10bSoftwareMaps.pdf
- Charting libs 2026 vendorability — https://www.youngju.dev/blog/culture/2026-05-14-data-visualization-libraries-2026-d3-plot-visx-recharts-echarts-vega-comparison-deep-dive-2026.en
- GitHub Next repo-visualization — https://githubnext.com/projects/repo-visualization/
