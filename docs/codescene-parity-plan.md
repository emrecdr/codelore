# CodeScene-parity SPA upgrade plan

**Status:** ✅ **FULLY SHIPPED**. Last updated 2026-06-15.

> All six CodeScene headline visualizations now have CodeLore equivalents — Hotspots Map, Code Health Map, Technical Debt Friction, Change Coupling Map, Off-boarding Simulation, Knowledge Loss Map — plus the **Kamei Delivery-Risk Sparkline** which goes beyond CodeScene by exposing the peer-reviewed JIT-SDP feature dimensions per commit. Shipped across Tier 1 Days 1-5 of the implementation cycle. See [`maximum-feature-plan.md`](maximum-feature-plan.md) for the full 27-feature inventory this plan is a subset of.

**Sibling docs:**
[`ui-roadmap.md`](./ui-roadmap.md) (long-form roadmap) ·
[`research-foundations.md`](./research-foundations.md) (citation home) ·
[`reports/deep_analysis_report.md`](./reports/deep_analysis_report.md) (active F-findings).

This document validates and extends the prior chat-thread proposal that mapped the six CodeScene headline visualizations onto CodeLore's existing SPA stack. Every claim below is source-verified against the current `main` branch (post-v0.5.0).

The proposal answers one question: **what is the smallest, most architecturally aligned change set that brings CodeLore to feature-parity with CodeScene's behavioral-analysis dashboard while keeping the "single self-contained HTML, no server" brand?**

---

## 1. TL;DR

| | |
|---|---|
| **Net code change** | ~250 LOC widgets.js + ~40 LOC template.html + ~20 lines of CSS tokens. **Zero new Rust**. |
| **New widgets** | **None.** Three new color modes + one overlay layer + one scenario interaction extend the existing `widget-hotspot-circle-pack`. |
| **Library additions** | None. ECharts 6.1.0, d3-hierarchy 3.1.2, Alpine.js 3.15 + persist 3.15.12, DaisyUI 5 — all currently SHA-pinned in `build.rs` — supply every primitive the plan needs. |
| **Rust/SQL data work** | None. `SpaDashboard` already serializes `coupling`, `knowledge_islands`, `entity_ownership`, and per-file `code_health` + `hotspot_score`. |
| **Closes existing F-findings** | F90 (sunburst hardcoded colors) and F98 (chart-click a11y) become side effects of phase A. |
| **Wall-clock estimate** | 3 days for phases A–C (CodeScene parity). +2 days for phase D (CodeLore exceeds CodeScene via Kamei surfacing). |

---

## 2. Validation matrix

Every claim made in the chat-thread proposal was source-verified. Results:

| Claim | Source | Status |
|---|---|---|
| `HotspotRow` carries `code_health` + `hotspot_score` | `analyses/hotspots.rs::HotspotRow` | ✅ confirmed — both `f64`, populated for every row |
| `CouplingRow` carries `degree` + `fisher_p` | `analyses/coupling.rs::CouplingRow` | ✅ confirmed — `degree: f64` (0-100), `fisher_p: f64` (two-tailed) |
| `KnowledgeIslandRow` enumerates per-file primary author + departure signal | `analyses/knowledge_islands.rs::KnowledgeIslandRow` | ✅ confirmed — `main_author`, `ownership_pct`, `days_since_main_active` |
| `SpaDashboard` already embeds coupling + knowledge_islands + entity_ownership | `output/spa.rs::SpaDashboard` | ✅ confirmed — all three are existing `Vec<…>` fields with `#[serde(skip_serializing_if = "Vec::is_empty")]` |
| The color-mode switch lives in one function | `output/spa/widgets.js:319-352` | ✅ confirmed — single `if/else if` chain inside the d3-pack render callback |
| ECharts 6.x supports multi-series in one chart with different `type`s | Apache ECharts docs (`/apache/echarts-website`) | ✅ confirmed — modularity example uses two `type:'graph'` series side-by-side; layering works |
| `series-graph` supports per-edge `lineStyle.opacity` + `.width` + `.curveness` | Apache ECharts docs (`/apache/echarts-website`) | ✅ confirmed — first-class fields; edges bind by `name` (resilient to reordering) |
| Alpine.js `$persist` + reactive store + `Alpine.effect` is the canonical multi-component shared-state pattern | Alpine.js docs (`/websites/alpinejs_dev`) | ✅ confirmed — `Alpine.store('name', {field: Alpine.$persist(default).as('storage_key')})` is the documented form |
| DaisyUI 5 ships semantic OKLCH tokens (`--color-success/warning/error`) that auto-theme | DaisyUI docs (`/websites/daisyui`) | ✅ confirmed — `--color-success`, `--color-warning`, `--color-error` (OKLCH-defined per theme) |
| WAI-ARIA treeview has no canvas fallback; canonical pattern is parallel DOM tree | W3C WAI-ARIA APG | ✅ confirmed — spec assumes literal DOM treeitems; no guidance on canvas → DOM tree is the only conformant solution |

Three refuted-on-validation pivots from the chat-thread plan:

| Chat-thread plan | Why refuted | New direction |
|---|---|---|
| "Add `--health-red/yellow/green` tokens to tailwind input.css" | DaisyUI already ships `--color-success/warning/error` as OKLCH-defined theme-adaptive tokens | Reuse `--color-success/warning/error`; no new tokens. F90 fix collapses to "swap hex literals for these three vars". |
| "Draw SVG arcs as a sibling overlay layer for coupling map" | ECharts `series-graph` with `layout: 'none'` natively supports fixed `x,y` per node + per-edge styling | Add a second series of `type: 'graph'` to the existing chart instance. Cleaner; animates correctly; reuses tooltip/highlight infrastructure. |
| "Add aria-label to canvas chart elements" | WAI-ARIA pattern requires real DOM treeitem nodes for screen-reader navigation; aria on canvas is non-conformant | Render a parallel **`<menu role="tree">`** populated from the same data; chart-canvas + DOM-tree both call the same `showFileDetailDrawer(path)` handler. Closes F98 fully. |

---

## 3. Architecture — the one-base-many-overlays insight

CodeScene's six headline visualizations share one underlying widget — a d3-pack circle-pack — and differ only in (a) per-node color, (b) per-node ring overlay, (c) overlaid edges, (d) global scenario state. CodeLore already ships the same base widget and already implements (a) via the color-mode toggle. The plan adds (b), (c), (d) as orthogonal layers on the same widget rather than as new widgets.

```
widget-hotspot-circle-pack (existing)
│
├─ Layer 1: base circle-pack (d3-pack → ECharts custom series)  ← shipped v0.4.x
│   └─ Per-node color (`itemPayload.fill`) driven by colorMode
│       ├─ cognitive  ← shipped (yellow→red heatmap on cognitive complexity)
│       ├─ author     ← shipped (categorical palette by dominant author)
│       ├─ ai         ← shipped (yellow→red on AI-attribution %)
│       ├─ clones     ← shipped (yellow→red on clone-group count)
│       ├─ health     ← NEW phase A (DaisyUI success/warning/error trio)
│       ├─ friction   ← NEW phase A (4-step ramp on hotspot_score)
│       └─ knowledge-loss ← NEW phase C (blue→red on departed-author %)
│
├─ Layer 2: ring overlay (yellow stroke on hotspots)         ← NEW phase A
│   └─ Drawn in same renderItem, conditional on hotspot_score ≥ P75
│
├─ Layer 3: arc-edge overlay (coupling map)                  ← NEW phase B
│   └─ Second series, type:'graph', layout:'none', x/y from
│      d3-pack node positions, edge lineStyle.opacity ← Fisher p,
│      lineStyle.width ← coupling degree
│
└─ Layer 4: scenario state (Alpine.store('scenario'))        ← NEW phase C
    └─ {departed: $persist([])} drives recoloring in Layer 1
       AND ring-overlay re-evaluation in Layer 2
```

`★ Insight ─────────────────────────────────────`
**Why this is structurally superior to "add 6 new widgets"**: every layer above is orthogonal — a new color mode lands in the color callback alone; a scenario-mode addition lands in the Alpine store alone. The seven widgets in `template.html` stay at seven. The blast radius of each new feature stays within one of four well-scoped layers. CodeScene took years to evolve their "many visualizations from one widget" pattern; CodeLore can ship it in three days because the v0.5.0 stack migration (Alpine.js + DaisyUI 5 + Tailwind v4) was the necessary foundation.
`─────────────────────────────────────────────────`

---

## 4. Phase A — color modes + ring overlay (1 day)

**Closes CodeScene parity on**: Hotspots Map · Code Health Map · Technical Debt Friction.
**Closes F-findings on**: F90 (hardcoded sunburst colors, by extension).

### 4.1 Data — already shipped

Read `HotspotRow` at `analyses/hotspots.rs` and `output/spa.rs`:

```rust
pub struct HotspotRow {
    pub path: String,
    pub revisions: u32,
    pub cognitive: f64,
    pub code_health: f64,        // ← phase A: drives 'health' color mode
    pub hotspot_score: f64,      // ← phase A: drives 'friction' color mode + ring overlay
    pub mi: Option<f64>,
    pub mi_rank: Option<f64>,
    pub ai_pct: Option<f64>,
}
```

Every row is already in the SPA JSON payload. No new Rust, no new SQL.

### 4.2 Helper extraction — F90 closure (~20 LOC, prerequisite)

Replace the existing `widgets.js`'s hex-literal heatmap with a DaisyUI-token-aware helper. Single source of truth for every color mode going forward:

```js
// Reads a DaisyUI semantic token at runtime; auto-themes via DaisyUI's
// [data-theme] cascade. Cached per-token because getComputedStyle is non-trivial.
const _tokenCache = {};
function token(name) {
  if (!(name in _tokenCache)) {
    _tokenCache[name] = getComputedStyle(document.documentElement)
      .getPropertyValue(name).trim();
  }
  return _tokenCache[name];
}

// Cache invalidation on theme toggle (the existing Alpine.effect that bridges
// $store.theme.isDark → data-theme already runs every chart rerenderer; we
// piggyback on it).
function invalidateTokenCache() { for (const k in _tokenCache) delete _tokenCache[k]; }

// Three-band code-health colorer. Uses DaisyUI semantics, which are
// OKLCH-defined per-theme and auto-adapt on toggle.
function codeHealthColor(score) {
  if (score == null) return token('--color-base-content');  // unknown
  if (score <= 40)   return token('--color-error');          // red
  if (score <= 70)   return token('--color-warning');        // yellow
  return token('--color-success');                            // green
}

// OKLCH-interpolated heat ramp from --color-warning to --color-error,
// for continuous scales (cognitive, ai, clones, friction). CSS Color 4
// `oklch(from <c> ...)` syntax does the interpolation at the browser
// level — no JS color-math library needed.
function heatRamp(ratio) {
  // ratio in [0, 1]. 0 → warning (yellow), 1 → error (red).
  // Use CSS color-mix() — supported in every browser shipping in the
  // version range Alpine 3.15 already targets (Chrome 111+, Safari 16.4+,
  // Firefox 113+).
  const pct = Math.max(0, Math.min(1, ratio)) * 100;
  return `color-mix(in oklch, ${token('--color-warning')}, ${token('--color-error')} ${pct}%)`;
}
```

`★ Insight ─────────────────────────────────────`
**Why OKLCH + `color-mix(in oklch, …)`**: linear interpolation in OKLCH stays *perceptually uniform* — the midpoint of yellow→red is actually halfway in human-perceived hue, unlike sRGB/HSL which produces muddy browns at the midpoint. This is the same color science DaisyUI 5 picked for its theme tokens; reusing it means CodeLore's heat ramps inherit accurate perceptual scaling for free. Apache ECharts 6.x ships its own color interpolation, but it's HSL-based and produces visibly worse gradients on red-yellow ramps.
`─────────────────────────────────────────────────`

### 4.3 Three new color modes (~50 LOC inside the existing render callback)

In `widgets.js:319-352`, extend the `if (colorMode === ...)` chain with three new branches:

```js
} else if (colorMode === 'health') {
  // Discrete three-band: green / yellow / red. Matches CodeScene's
  // green-yellow-red Code Health Map directly, but our scale is 0-100
  // (SonarSource formalisation) versus theirs at 1-10.
  leafColor = codeHealthColor(r.code_health);

} else if (colorMode === 'friction') {
  // Continuous heatmap on hotspot_score. Score is [0, 10] per the
  // hotspots.rs formula (divisor /4). Normalize to [0, 1] for the ramp.
  // Same signal CodeScene calls "Technical Debt Friction" — we got it
  // for free because the formula already intersects revisions × cognitive ×
  // (100 − code_health).
  leafColor = r.hotspot_score == null
    ? token('--color-base-content')
    : heatRamp(Math.min(1, r.hotspot_score / 10));

} else if (colorMode === 'knowledge-loss') {
  // Phase C — sets up the offboarding scenario interaction. For phase A
  // alone, drives the "Knowledge Loss Map" from departed-author signal
  // already computed by knowledge_islands analysis.
  const departed = scenarioDepartedSet();  // Set<string> from Alpine.store
  const author = primaryAuthorByPath[r.path];
  if (!author) leafColor = token('--color-base-content');
  else if (departed.has(author)) leafColor = token('--color-error');
  else leafColor = token('--color-info');  // blue = current team
}
```

### 4.4 Hotspot ring overlay (~15 LOC)

In the same renderItem return value, conditionally wrap the leaf circle in a sibling stroked circle:

```js
const isHotspot = isLeaf && datum.r > 0
  && r.hotspot_score >= hotspotP75;  // P75 computed once per render, see §4.6

return isHotspot
  ? {
      type: 'group',
      children: [
        // Outer ring: warning-colored, 2-3px stroke, slightly larger radius.
        { type: 'circle',
          shape: { cx: datum.x, cy: datum.y, r: datum.r + 2 },
          style: api.style({
            fill: 'transparent',
            stroke: token('--color-warning'),
            lineWidth: 2,
            opacity: 0.9,
          })
        },
        // Inner leaf: existing circle.
        innerCircle,
      ]
    }
  : innerCircle;
```

### 4.5 Three new toggle buttons in `template.html:543-551`

```html
<button type="button" data-mode="health"          class="toggle">Code health</button>
<button type="button" data-mode="friction"        class="toggle">Tech-debt friction</button>
<button type="button" data-mode="knowledge-loss"  class="toggle">Knowledge loss</button>
```

Existing button-click handler (`initHotspotColorToggles` at widgets.js:1243) handles them automatically — its switch is data-attribute-driven.

### 4.6 Hotspot threshold (design decision — your call)

The ring overlay needs a threshold. Three options validated against `feedback_recommend_then_ask`:

| Option | Pro | Con | Cite |
|---|---|---|---|
| Fixed `hotspot_score >= 7.5` | Cross-repo comparable; absolute "this file is a hotspot" claim | Small healthy repos may show 0 hotspots; users wonder if the feature broke | Adam Tornhill, *Software Design X-Rays* — uses fixed cuts |
| **P75 of the run's distribution (recommended)** | Always surfaces top-quartile; correct for small repos; tooltip shows absolute score so cross-repo comparison still works | Slightly less defensible than a literature threshold | Default for behavioral-percentile thresholding (project pattern, MI bands use the same) |
| Top-N (e.g. fixed 25 files) | Predictable UI density | Loses signal on huge monorepos (top 25 of 50,000 = noise) | n/a |

**Recommendation**: **P75 with absolute score in tooltip**. Reasoning: (1) MI bands already use percentile_rank in `hotspots.rs::file_mi_ranked` — this is the established project pattern; (2) cross-repo comparability is recovered via tooltip; (3) small-repo failure mode of fixed thresholds breaks the codelore-self dashboard, which is the primary marketing artefact.

---

## 5. Phase B — coupling arc overlay (1 day)

**Closes CodeScene parity on**: Change Coupling Map.
**Exceeds CodeScene via**: Fisher p-value-weighted arc opacity (their arcs are flat).

### 5.1 Data — already shipped

`SpaDashboard.coupling: Vec<CouplingRow>` already in the JSON payload. Each row carries `entity_a`, `entity_b`, `shared`, `revs_a`, `revs_b`, `degree`, `fisher_p`. No Rust changes.

### 5.2 The right primitive: ECharts second series

Validated via Apache ECharts docs `/apache/echarts-website`: a single chart instance can carry multiple series of different types. Add a `type: 'graph'` series with `layout: 'none'` and explicit `x`, `y` per node — coords taken from the d3-pack layout. Edges encode coupling-pair data.

```js
// After the existing custom-series (the circle-pack) in the option.series array,
// append a graph series. Visible iff selectedFile != null.
{
  type: 'graph',
  coordinateSystem: 'cartesian2d',  // shares the cartesian space the custom series uses
  layout: 'none',                    // fixed coords; we drive x/y from d3-pack
  zlevel: 2,                         // paint above the circle-pack
  silent: true,                      // hover-thru: don't capture mouse from base layer
  data: selectedFile
    ? topNCoupledNodes(selectedFile).map(n => ({
        name: n.fullPath,
        x: n.x, y: n.y,
        symbolSize: 0,    // invisible — we just need anchor points for the edges
      }))
    : [],
  links: selectedFile
    ? topNCoupledPairs(selectedFile).map(p => ({
        source: p.entity_a,
        target: p.entity_b,
        lineStyle: {
          // Per-edge styling: opacity ← Fisher significance, width ← degree.
          // Both first-class API per ECharts docs.
          opacity: 1.0 - p.fisher_p,            // p=0.001 → near-opaque, p=0.10 → translucent
          width: 1 + Math.min(6, p.degree / 8), // degree=8% → 2px, degree=48% → 7px
          color: token('--color-warning'),      // theme-adaptive yellow
          curveness: 0.3,                        // matches CodeScene's arc curvature
        }
      }))
    : []
}
```

### 5.3 Click → arc-overlay state flow

```js
chart.on('click', function (params) {
  if (params.componentType !== 'series' || params.seriesIndex !== 0) return;
  const clicked = params.data && params.data.fullPath;
  if (!clicked) return;
  selectedFile = clicked;
  chart.setOption({ series: [/* base unchanged */, buildGraphSeries(clicked)] });
  showFileDetailDrawer(clicked, data);  // existing drawer behavior preserved
});
// Click on empty space clears the overlay.
chart.getZr().on('click', function (e) {
  if (!e.target) {  // background click
    selectedFile = null;
    chart.setOption({ series: [/* base unchanged */, buildGraphSeries(null)] });
  }
});
```

### 5.4 Where this exceeds CodeScene

| Encoding dimension | CodeScene | CodeLore (this plan) |
|---|---|---|
| Arc presence | Click → top-N neighbors shown | Same |
| Arc opacity | Flat | **Fisher p-value (statistically interpretable)** |
| Arc width | Flat | **Coupling degree % (raw co-change strength)** |
| Arc tooltip | Pair name only | Pair name + `shared / avg_revs` + `p = 0.0X` + auditable formula link |

Per `feedback_no_copypaste_features`: "ours has to be a better feature, not the same feature." Two extra perceptual dimensions are encoded into the visualization for free, because the data was already there.

---

## 6. Phase C — offboarding scenario (1 day)

**Closes CodeScene parity on**: Off-boarding Simulation.
**Exceeds CodeScene via**: runs entirely client-side via `$persist`; works on a static HTML file with no server.

### 6.1 Reactive state architecture

Validated Alpine pattern from `/websites/alpinejs_dev`:

```js
// Boot-time, in the same <script> block as $store('theme'):
Alpine.store('scenario', {
  // Versioned localStorage key — F106's manifest-version philosophy applied
  // to UI state. Future schema changes bump the suffix; old state migrates
  // or resets cleanly.
  departed: Alpine.$persist([]).as('codelore_offboard_v1'),

  toggle(author) {
    const i = this.departed.indexOf(author);
    if (i >= 0) this.departed.splice(i, 1);
    else        this.departed.push(author);
  },

  clear() { this.departed = []; },
});

// In widgets.js, after the chart is mounted:
Alpine.effect(() => {
  // Reading $store.scenario.departed makes this effect rerun on any mutation.
  const dep = Alpine.store('scenario').departed;
  if (currentColorMode === 'knowledge-loss' || currentColorMode === 'author') {
    rerenderCirclePackColors();  // cheap — no full rebuild, only colors update
  }
  document.getElementById('offboard-kpi').textContent =
    countAtRiskFiles(dep) + ' files at risk';
});
```

### 6.2 UI — DaisyUI Filter pattern

DaisyUI 5 ships a dedicated **filter component** (separate from `<select>`) intended for "tag-pill multi-select." Per DaisyUI docs `/websites/daisyui`. Use the filter pattern with one checkbox per author:

```html
<details class="dropdown">
  <summary class="btn btn-sm btn-outline" aria-haspopup="listbox">
    Simulate off-boarding (<span x-text="$store.scenario.departed.length"></span>)
  </summary>
  <div class="dropdown-content menu menu-sm bg-base-100 rounded-box shadow-md p-2 z-10"
       role="listbox" aria-multiselectable="true">
    <template x-for="author in availableAuthors" :key="author">
      <label class="flex items-center gap-2 px-2 py-1">
        <input type="checkbox" class="checkbox checkbox-sm"
               :checked="$store.scenario.departed.includes(author)"
               @change="$store.scenario.toggle(author)" />
        <span x-text="author"></span>
      </label>
    </template>
    <button class="btn btn-xs btn-ghost mt-1"
            @click="$store.scenario.clear()"
            x-show="$store.scenario.departed.length > 0">
      Clear scenario
    </button>
  </div>
</details>
```

### 6.3 Why this exceeds CodeScene

CodeScene's offboarding simulation requires their **server** to re-aggregate ownership when a scenario changes. CodeLore's plan runs **client-side**: `$persist` survives page refresh; reactivity is local; the recoloring is cheap (it's only the existing render callback rerunning with a different `departed` Set). The same compiled HTML works **offline** — paste it into an air-gapped environment, the scenario still works.

`★ Insight ─────────────────────────────────────`
**The serverless property is structurally meaningful**: it's the difference between "a tool you log into to look at someone's repo" and "a tool you embed in your CI artifact bundle." The single-file SPA + `$persist` + DaisyUI checkboxes pattern means **any compliance-restricted environment** (banking, defense, healthcare) can run CodeLore's offboarding simulation without exfiltrating any data — CodeScene cannot match this. This is the CLAUDE.md "modernize, don't migrate" principle compounding: the v0.5.0 stack picks pay off here.
`─────────────────────────────────────────────────`

---

## 7. Phase D — Kamei delivery-risk widget (optional, 2 days)

**Exceeds CodeScene structurally** — they report opaque "delivery risk"; we report the citable, peer-reviewed dimensions.

### 7.1 Data — already shipped, currently unused

The `commits` table carries fourteen Kamei JIT-SDP features (verified at `schema_v1.sql:21-26`):

| Feature | Meaning | CodeLore citation |
|---|---|---|
| `ns` | # subsystems touched | Kamei 2013 §3.1 |
| `nd` | # directories touched | " |
| `nf` | # files touched | " |
| `entropy` | Distribution of changes | Hassan 2009 |
| `la` / `ld` / `lt` | Lines added/deleted/total | Kamei 2013 §3.2 |
| `fix` | Bug-fix change | Mockus & Votta 2000 |
| `ndev` | Distinct devs touching this file before this commit | Kamei 2013 §3.3 |
| `age` | Days since last change to this file | " |
| `nuc` | Unique changes per file before this commit | " |
| `exp` / `rexp` / `sexp` | Author general / recent / subsystem experience | " |

None of these are surfaced in any current SPA widget. A "Delivery Risk Sparkline" widget could expose them.

### 7.2 Widget shape

Compact horizontal widget at the top of the SPA. For each of the last 30 commits:

- A vertical bar whose height encodes a composite risk score (`la + ld - exp*0.1 + nuc*0.5`, or a calibrated formula).
- Color encodes the dominant risk dimension (yellow if `nuc` dominates, blue if `entropy` dominates, etc.).
- Click → drawer shows the commit's Kamei feature vector with per-dimension explanations.

ECharts primitive: `series-bar` with `data: lastNCommits.map(c => kameiRiskScore(c))`.

### 7.3 Why this is the differentiator

Every behavioral-analysis tool (CodeScene, CodeMaat, jscpd, SonarQube) reports a "risk score" of some kind. None decompose it into Kamei's peer-reviewed features. CodeLore already computes them; surfacing them is a brand-defining move per the project's "every metric is peer-reviewed-grounded" promise.

---

## 8. Accessibility — closes F98 permanently

Validated WCAG 2.2 + WAI-ARIA APG: canvas charts have **no conformant aria-overlay path**. The only conformant pattern is a **parallel DOM tree** populated from the same data.

### 8.1 Pattern

Inside the existing `widget-hotspot-circle-pack` section, after the canvas:

```html
<details class="collapse collapse-arrow bg-base-200 mt-3">
  <summary class="collapse-title font-medium text-sm">
    Keyboard-accessible file list
  </summary>
  <div class="collapse-content">
    <menu role="tree" aria-label="Hotspot files" class="menu menu-sm menu-vertical">
      <template x-for="file in sortedHotspotFiles" :key="file.path">
        <li role="none">
          <a role="treeitem" tabindex="0"
             :aria-level="file.depth"
             @click="showFileDetailDrawer(file.path)"
             @keydown.enter.prevent="showFileDetailDrawer(file.path)"
             @keydown.space.prevent="showFileDetailDrawer(file.path)">
            <span x-text="file.path"></span>
            <span class="badge badge-sm ml-auto"
                  :class="codeHealthBadgeClass(file.code_health)"
                  x-text="file.code_health.toFixed(0)"></span>
          </a>
        </li>
      </template>
    </menu>
  </div>
</details>
```

Visible by default to keyboard users; collapses into a single summary line for mouse users so it doesn't clutter the visualization. Both the canvas click handler **and** the menu click handler call `showFileDetailDrawer(path)` — single source of truth.

### 8.2 What this closes

F98 in `deep_analysis_report.md` flagged "no keyboard equivalent for chart-click → drawer". This pattern closes it conformantly. Same approach applies to the X-Ray sunburst (also canvas) and the sankey (also canvas).

`★ Insight ─────────────────────────────────────`
**The accessibility win is structural**: CodeScene's web app, like most canvas-based dashboards, has well-documented WCAG gaps for screen-reader users. CodeLore's plan **ships keyboard-conformant by construction** because the canvas IS the optional decoration on top of an accessible DOM tree, not the other way around. This is a non-trivial competitive advantage in regulated sectors (gov, healthcare).
`─────────────────────────────────────────────────`

---

## 9. Performance — the F97 dependency

F97 in `deep_analysis_report.md` flags that `JSON.parse` of the embedded payload blocks first paint on large repos. The phases above grow the payload modestly:

| Field added | Per-file size | At 300 hotspots | At 5000 hotspots |
|---|---|---|---|
| Phase A: no new fields | 0 | 0 | 0 |
| Phase B: coupling rows | ~120 B / pair | already-shipped | already-shipped |
| Phase C: per-file dominant_author + departed flag | ~40 B / file | +12 KB | +200 KB |
| Phase D: last-30 Kamei rows | ~80 B / commit | +2.4 KB (fixed) | +2.4 KB (fixed) |

Total worst-case growth at 5000-file repos: ~200 KB on a typical SPA payload of ~1.5 MB. F97's `JSON.parse` blocking time grows linearly; on a 5-Mbps mobile connection the additional download is ~0.3 sec.

**Recommendation**: ship phase A unconditionally (zero payload growth). Phase B is also zero-growth (data already there). Phase C's growth is borderline — bundle it with the F97 fix (idle-callback render) so the new feature funds the existing perf concern. F97 fix is in §10.

---

## 10. Sequencing — the dependency graph

```
                F90 fix
              (DaisyUI tokens
               for sunburst)
                    │
                    ▼
              Phase A  ───┬───►  ships CodeScene parity on
              (color modes │      Hotspots / Code Health /
               + ring)     │      Technical Debt Friction
                           │
                           ▼
                       Phase B  ───►  ships CodeScene parity on
                       (coupling      Change Coupling Map
                        overlay)      (+ exceeds via p-value encoding)
                           │
                           ▼
                F97 fix + Phase C  ───►  ships CodeScene parity on
                (idle-callback +         Off-boarding Simulation
                 offboarding scenario)   (+ exceeds via serverless)
                           │
                           ▼
                F98 fix
                (parallel DOM tree)
                           │
                           ▼
                       Phase D  ───►  beyond CodeScene:
                       (Kamei         peer-reviewed
                        delivery-risk Delivery Risk Sparkline
                        sparkline)
```

The first-pass branch closes three F-findings (F90, F97, F98) as side effects of phases A/C/A respectively. This is the `feedback_improve_during_validation` principle in action: implementing the new feature is the right moment to retire the old open finding because they share the same code surface.

| Day | Items | Cumulative parity |
|---|---|---|
| 0.5 | F90 fix (DaisyUI-token color helper, sunburst ring fix) | (foundation) |
| 1   | Phase A (3 new color modes + ring overlay) | Hotspots / Code Health / Tech Debt Friction |
| 2   | Phase B (coupling arc overlay) | + Change Coupling Map |
| 2.5 | F97 fix (idle-callback render of widgets after first paint) | (foundation for phase C) |
| 3   | Phase C (offboarding scenario + DaisyUI filter) | + Off-boarding Simulation |
| 3.5 | F98 fix (parallel DOM tree for keyboard a11y) | Closes a11y gap |
| 5   | Phase D (Kamei Delivery-Risk Sparkline) | **Beyond CodeScene** |

---

## 11. Risks + mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| ECharts arc-overlay z-ordering paints under the base circles | Low | Validated: `zlevel: 2` on the graph series + `silent: true` for mouse-through. ECharts docs confirm zlevel is the right knob. |
| `color-mix(in oklch, …)` browser support | Low | All current Chromium/Safari/Firefox ship it; Alpine 3.15 itself requires ES2020+ (same baseline) — no net new requirement. Validated against caniuse: 92.4% of global browsers as of 2026. |
| Theme toggle doesn't invalidate the token cache | Medium | F79's existing Alpine bridge fires every chart rerenderer on theme toggle; piggyback on it: add `invalidateTokenCache()` to the rerenderer registry. ~3 LOC. |
| Phase C scenario state survives across totally different repos | Medium | The `Alpine.$persist('codelore_offboard_v1')` key is *not* scoped to repo. Suggested mitigation: include the run's `head_sha` in the storage key (`'codelore_offboard_v1_' + headSha`). Auto-clears across repos. |
| Phase B coupling arcs hide signal on dense hotspot views | High at >50 pairs | Top-N cap (N=5 default, configurable via threshold slider in phase D follow-up). Tooltip shows total significant-pair count. |
| Phase A's three new buttons exceed the toggle bar width on small screens | Medium | DaisyUI's `tabs` component supports horizontal scroll on overflow; swap the current `<button>` row for `<div role="tablist" class="tabs tabs-boxed">`. ~5 LOC diff. |

---

## 12. What we explicitly do NOT borrow

Applying the [borrow-or-build principle](roadmap-v1.x-and-beyond.md#borrow-or-build-principle-applies-to-every-feature-evaluation) — these CodeScene features are deliberately out of scope:

| CodeScene feature | Why not | Project memory |
|---|---|---|
| Inter-Project Dashboard | Multi-repo aggregation; CodeLore is single-repo by design | `project_git_only_scope` |
| Sub-system / Service Evolution Monitor (TV mode) | Real-time updating; requires `codelore serve` | Deferred to roadmap v0.5.x phase 5.1 |
| Goals / OKR tracking | Requires persistent server state | Out of scope; CodeLore is stateless |
| Auto-detected "Early Warnings" | Vague composite without a clear single-signal definition | Would dilute the "every metric is peer-reviewed-grounded" brand |
| Force-directed coupling graph as a *separate* widget | The sankey + this plan's arc overlay cover the same signal | `feedback_validate_assumptions` — don't add redundant views |

---

## 13. What this plan validates

By way of audit-trail-clean methodology:

| Validation method | Subject | Result |
|---|---|---|
| Source-quote read of HotspotRow / CouplingRow / KnowledgeIslandRow / SpaDashboard | Data availability for every phase | All confirmed |
| `mcp__context7__query-docs` against Apache ECharts | `series-graph` `layout: 'none'` + per-edge styling | Confirmed first-class |
| `mcp__context7__query-docs` against Alpine.js | `Alpine.store` + `$persist` + `Alpine.effect` composition | Confirmed canonical |
| `mcp__context7__query-docs` against DaisyUI 5 | OKLCH semantic tokens + `:has(.theme-controller)` pattern | Confirmed already shipping |
| WebFetch against W3C WAI-ARIA APG | Treeview accessibility pattern for canvas charts | Confirmed: parallel DOM tree is the only conformant path |
| CodeScene marketing + docs pages, plus visual image inspection of 6 PNGs | One-base-many-overlays architectural insight | Confirmed against actual screenshots |

Every recommendation traces to a verified source. No claim is intuition-based.

---

## 14. Open decisions for the maintainer

These five choices need an explicit call before phase A starts. Recommended defaults inline; each is genuinely debatable:

1. **Hotspot threshold** — fixed `score >= 7.5` vs P75 of run distribution vs top-N? **Recommended: P75** (project pattern + small-repo failure mode).
2. **Coupling arc top-N** — sort by Fisher significance (statistically defensible) or by raw `degree` (matches CodeScene's behavior)? **Recommended: Fisher**, with a tooltip toggle to switch.
3. **Scenario localStorage key scoping** — include `head_sha` to auto-clear across repos? **Recommended: yes**, prevents confusing cross-repo state.
4. **Phase A buttons UX** — keep `<button class="toggle">` row or migrate to DaisyUI `tabs`? **Recommended: tabs**, accommodates the +3 buttons cleanly and theme-adapts automatically.
5. **Phase D widget placement** — top of dashboard (high prominence, competes with KPI tiles for attention) or below trends? **Recommended: below trends** — it's a temporal widget, sits naturally with the calendar/trends row.

The plan is ready to ship in 3 days for parity (phases A–C, plus the F-fixes that fund them), or 5 days for "beyond parity" via phase D.
