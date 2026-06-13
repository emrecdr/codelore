# CodeLore — UI Roadmap (v0.4.x series)

This document captures the technical decisions and sequenced plan for
bringing a CodeScene-equivalent dashboard to CodeLore, while preserving
the local-first, audit-trail, deterministic-formula brand.

It is a **planning artefact**, not a contract. Re-validate every
assumption before implementing each phase (see
`feedback_validate_assumptions.md`).

---

## 1. North star

A single self-contained HTML file (`codelore analyze --format spa
-o codelore.html`) that opens in any browser, runs without a server,
fits in a CI artefact, and surfaces the full CodeScene-equivalent
analytic surface — **plus** CodeLore's three differentiators that
CodeScene does not have:

1. **Auto-detected knowledge islands** (departed-author × clones ×
   co-change intersection) — CodeScene requires manual ex-developer
   marking.
2. **AI-attribution toggle** — filter hotspots by who wrote them
   (human, AI-assisted, AI-dominant). CodeScene has no such signal.
3. **Auditable formulas** — a `?` tooltip on every metric links to
   the SQL query, the research foundation
   (`docs/research-foundations.md`), and the provenance manifest.
   CodeScene's biomarkers are opaque; CodeLore's are inspectable.

---

## 2. Technical stack (validated)

| Layer | Choice | Justification |
|---|---|---|
| Charts (~95% of widgets) | **Apache ECharts 6.1.0** | Top-starred Apache Foundation project (66k+ stars); monthly minor releases; XSS-fix responsiveness validated in 6.1.0 + 5.5.1. Apache-2.0 license (deny.toml-allowed). Treemap, sunburst, sankey, chord, force-graph, heatmap, calendar heatmap, parallel coords, line/bar/area — all native. |
| Circle-pack layout (1 widget) | **d3-hierarchy 3.1.2** | ECharts has no native circle-pack series. `d3-hierarchy.pack()` computes the layout (~10 KB tree-shaken); ECharts `custom` series renders the result. ISC license (deny.toml-allowed). |
| Framework (v0.4.x) | **None** (vanilla JS) | Existing `output/html.rs` uses vanilla JS and serves 525 LOC of paginated/sortable table — proves the pattern. Adding a framework (Svelte/Vue/React) would add ~30–40 KB and a build pipeline without commensurate benefit. |
| Interactivity layer (v0.5.x) | **Alpine.js 3.x** (scheduled, not yet wired) | ~15 KB, single `<script>` embed via the same SHA-256-pinned `build.rs` fetch pattern as ECharts. HTML-attribute reactivity (`x-data`, `x-show`, `x-on`) matches our Rust-generated template philosophy — directives sit on existing DOM, no virtual DOM, no JSX/SFC compiler. Re-validated 2026-06-11: still the right call for `codelore serve` cross-widget filter state. See "Why Alpine.js (re-validated)" subsection below. |
| Build | **`build.rs` + SHA-256-pinned CDN fetch behind `spa` Cargo feature** | Avoids committing minified JS into the repo. The `spa` feature is **opt-in**: default builds skip the fetch entirely (no internet needed). When enabled, `crates/codelore-lib/build.rs` fetches ECharts and d3-hierarchy from jsDelivr at pinned URLs, verifies SHA-256 against the table in `ASSETS`, and caches them in `OUT_DIR`. Subsequent builds with `spa` enabled hit the cache (no network) until either the pin changes or the cached bytes drift. Audit-trail: the build script's pin table IS the supply-chain manifest. |
| Template | `include_str!` from `src/output/spa/template.html` | Same idiom as the existing HTML emitter; no runtime templating engine. |

### Why not Svelte / Solid / React?

- They each add 25–40 KB of runtime + a Vite-style build step
- The CodeLore HTML emitter precedent is vanilla JS (no framework)
- Single-file static output is more brittle when a framework is involved
- We are not building a long-lived SPA with routing, forms, or
  reactive state graphs — we are rendering a fixed set of charts
  from one JSON blob

### Why Alpine.js at v0.5 (re-validated 2026-06-11)

Re-checked the landscape before locking the v0.5.x boundary — Alpine
is still the right answer:

- **Size budget**: 15 KB minified+gzipped (current 3.x). HTMX (14 KB)
  is the next-smallest with a similar philosophy but is request-oriented
  (every interaction is a server roundtrip), which conflicts with the
  "single static HTML file works offline" north star (§1). Petite-Vue
  (6 KB) is technically smaller but has been in maintenance-only mode
  since 2022 — Alpine is actively shipped against.
- **Programming model fit**: HTML-attribute directives (`x-data`,
  `x-show`, `x-bind`, `x-on`) sit on existing DOM nodes. Our
  templates are `include_str!`-shipped from Rust; no JSX, no SFC
  compiler, no Vite step required. The same template renders
  identically in `--format spa` (static) and `codelore serve` (live)
  by toggling whether Alpine is loaded.
- **Cross-widget filter state**: Alpine's reactive `$store` is the
  exact primitive v0.5.x needs — filter on hotspot table updates a
  store, every widget watching that store re-renders. Doing this in
  vanilla JS scales to 2–3 widgets; we have 11.
- **SHA-pin compatibility**: ships as a single minified `<script>`,
  drops into the same `build.rs` SHA-256 pin table that hosts ECharts
  and d3-hierarchy. No build pipeline added.
- **Trajectory check**: Alpine 3.x has been stable since 2020 with
  regular minor releases; not a flavor-of-the-quarter risk. Used by
  Laravel (Livewire), 37signals (Basecamp), and the Caleb Porzio
  ecosystem — long-tail mindshare.

Rejected alternatives at this size class:
- **HTMX** — request-oriented (kills offline mode).
- **Petite-Vue** — maintenance-only since 2022.
- **Lit** — Web Components add 8 KB and a class-based component model
  that fights our Rust-templated HTML.
- **Vue / React / Svelte** — already rejected at §2 for v0.4.x; the
  same arguments hold at v0.5 (build pipeline, framework chrome).

### Why not Observable Framework / Evidence.dev / Rill?

- They stamp their own chrome and project layout on the output
- "This IS CodeLore" identity is weaker
- All three are valid alternatives the user can reach via the
  `--format sqlite` path that **already ships**; see §6 below

---

## 3. Release sequence — calibrated against what actually shipped

| Version | Scope | Status |
|---|---|---|
| **v0.4.0** | 6-widget SPA-MVP: KPI tiles, hotspot circle-pack, hotspot table, change-coupling sankey, knowledge-islands ranked view, file detail drawer; F38 windowed Kamei rewrite as precondition; `--features spa` Cargo gate; SHA-pinned ECharts + d3-hierarchy build.rs fetch | **Shipped 2026-06-11** ✓ |
| **v0.4.1** | **Perf batch** — F43-F54 (DISTINCT cleanup, blob mem-move, empty-diff short-circuit, single-cursor AST walks, single-pass templating). No new widgets — the audit findings made this a backend-only release. | **Shipped 2026-06-11** ✓ |
| **v0.4.2** | Widget completeness — 5 new widgets (W7 knowledge map, W8 X-Ray sunburst, W9 trends multi-line, W10 calendar heatmap, W11 AI-attribution overlay) + CSS theming + light/dark mode toggle. | **Shipped 2026-06-11** ✓ |
| **v0.4.3** | Dashboard polish — success message on render, dispose-on-rerender (F64) to fix the chart-instance leak that bit `--repeat`-style rerenders. CI embed mode, per-metric provenance tooltips, and X-Ray live-at-HEAD pre-filter slipped to v0.4.5 (now tracked as UI-1/UI-2/UI-3 in the main roadmap). | **Shipped 2026-06-11** ✓ (partial — UI-1/2/3 deferred) |
| **v0.4.4** | SQL planner sweep — F61/F63 `arg_max` rewrite, F66 `bstr` SIMD line count. No widget changes; perf headroom for v0.4.5's X-Ray pre-filter (UI-3) reuses the `arg_max(change_type, ROW(date, -rowid))` pattern. | **Shipped 2026-06-11** ✓ |
| **v0.4.5** | **Active** — UI holdovers from v0.4.3 + CHM borrows. UI-1 `--embed` flag, UI-2 `?` tooltips with CHM-A4 per-metric description text on every KPI tile + table column, UI-3 X-Ray sunburst pre-filtered to live-at-HEAD via the F63 hash-aggregation pattern, F68 cross-stack AI-attribution toggle wired end-to-end (column → SQL → JSON → widgets.js), F71 per-container `ResizeObserver` replacing the leaking global `resize` listener. See main roadmap for the full F-finding + CHM-A1/A2/A3/A5 backend pairs. | **Active** (in development) |
| **v0.5.x** | `codelore serve` — local Axum web server, live SQL exploration, cross-widget filter state. Alpine.js added at this point. | Planned (research done) |
| **v0.6.x** | Tauri 2 desktop wrapper. Native filesystem, drag-drop folder, signed cross-platform installers. | Planned |

### v0.4.0 widgets — what's actually in production

| # | Widget | Data source(s) | ECharts series | Notes |
|---|---|---|---|---|
| 1 | **Hotspot circle-pack** | `run_hotspots` + filesystem hierarchy | `custom` + `d3.pack()` layout | The signature CodeScene look. Color = cognitive on yellow→red ramp. |
| 2 | **Hotspot table** | same `HotspotRow[]` | sortable paginated table (500/page) + 80 ms debounced filter | Drill-down from #1 click; row click → detail drawer. |
| 3 | **KPI tiles** | `run_summary` + `run_code_health` + `run_hotspots` + `run_knowledge_islands` + `run_coupling` | HTML cards | Files analyzed, commits, distinct authors, median code health, cognitive p95, knowledge-island count, coupling pair count. |
| 4 | **File detail drawer** | per-path slice of all run_* | HTML side panel | Click any circle / table row / sankey node → drawer with that path's hotspot, knowledge-island, code-health, and coupling-partner data. ESC or × closes. |
| 5 | **Change coupling sankey** | `run_coupling` top-30 by combined score | native `sankey` | Node click → drawer. |
| 6 | **Knowledge islands** (CodeLore differentiator) | `run_knowledge_islands` | ranked HTML table | Auto-detected departed-author files. Surfaced with a "CodeLore differentiator" badge. |

**Actual bundle**:
- ECharts 6.1.0 minified: 1.1 MB (full feature set; tree-shaken would be ~250 KB but we'd lose runtime widget swapping)
- d3-hierarchy 3.1.2 minified: 14 KB
- CodeLore SPA glue (template.html + widgets.js + CSS): ~30 KB
- Per-analysis JSON data on a 300-file repo: ~50 KB
- **Verified emitted size on the CodeLore repo itself**: 1.2 MB (~400 KB gzipped over the wire)

---

## 3a. v0.4.2 widget plan — **shipped 2026-06-11**

Five new widgets + theming. None require build-system changes; all are
JS + template extensions + (for two) new `SpaDashboard` fields and one
or two new `run_*` calls in `run_spa_dispatch`.

| # | Widget | Data source(s) | ECharts series | New scaffold | Est. LOC |
|---|---|---|---|---|---|
| W7 | **Knowledge map** (author-colored treemap) | reuse `run_hotspots` data; client-side palette swap by `primary_author` from `run_entity_ownership` | native `treemap` (palette toggle on the existing circle-pack data) | new `entity_ownership` field on `SpaDashboard`; toggle UI in template | ~50 LOC |
| W8 | **X-Ray sunburst** (function-level) | `entities` + `complexity_metrics` (already in DuckDB; needs a new raw SQL aggregation) | native `sunburst` | new `entities` field; new `run_xray` helper that emits `{path, function, cognitive}` rows | ~100 LOC |
| W9 | **Trends multi-line** | hotspots over time — `run_hotspots` with `--time-bucket month`, top-N files | native `line` | new `trends` field; second `run_hotspots` call inside `run_spa_dispatch` with bucketing opts | ~80 LOC |
| W10 | **Calendar heatmap** | raw `SELECT date_trunc('day', date) AS d, COUNT(*) FROM commits GROUP BY d` | native `heatmap` on `calendar` coord | new `daily_commits` field; new tiny SQL helper | ~60 LOC |
| W11 | **AI-attribution overlay** | toggle that filters the circle-pack + table by `commits.ai_attribution` band (human / assisted / dominant) | client-side filter on existing data | toggle UI + JS that re-renders the circle-pack with filtered data | ~50 LOC |

Plus:
- **CSS theming via `:root` variables** — already partially in place; complete the audit so every color references a variable.
- **Light/dark toggle** — single button in the header; preference stored in `localStorage`.

**Cumulative SPA HTML size estimate**: 1.2 MB → ~1.3 MB (the extra is data, not glue).

**v0.4.2 effort estimate**: ~1 week of focused work.

---

## 3b. v0.4.3 — polish + CI embed — **shipped 2026-06-11 (partial)**

> Dashboard polish (success message + dispose-on-rerender / F64) landed in v0.4.3. The CI embed mode, per-metric provenance tooltips, and X-Ray live-at-HEAD pre-filter were re-scoped to **v0.4.5** under IDs **UI-1 / UI-2 / UI-3** (see main roadmap). The original §3b plan below documents what those holdovers were intended to do.

- **Embed mode** — `codelore analyze --format spa --embed` strips the
  full-page shell and produces an HTML fragment suitable for
  embedding in `$GITHUB_STEP_SUMMARY` and SARIF code-scanning result
  pages. Keeps the widgets, drops the header/footer/styling that
  conflicts with embedding contexts.
- **Per-metric provenance tooltips** — `?` icons on every KPI tile
  and table column. Tooltip shows the SQL query that produced the
  value, plus a link to the research foundation in
  `docs/research-foundations.md`. This is the "deterministic
  published formulas" brand differentiator surfaced visually.
- **Perf tuning** — `entities` table query for X-Ray currently does a
  full scan; if v0.4.2 telemetry shows it as the long pole on
  100k+ commit repos, add an `--analysis xray` boundary that
  pre-filters to live-at-HEAD entities.

**v0.4.3 effort estimate**: ~3 days.

---

## 3c. v0.5.x — `codelore serve` (interactive mode)

This is where vanilla JS starts to hurt — cross-widget filter state
(filter on the hotspot table → highlight matching circles → restrict
the knowledge-islands view → restrict the sankey) needs reactivity.

**Framework decision — revisited at the v0.5 boundary** (see §2.1):
**Alpine.js** is the chosen upgrade. Validated as the best in-structure
choice: ~15 KB, single `<script>` embed via the same `build.rs`
SHA-pin pattern as ECharts, HTML-attribute syntax that matches our
Rust-generated template philosophy. We stay vanilla until v0.5
because v0.4.x widgets are independent — adding Alpine earlier would
pay for capability we don't use yet.

| Feature | Stack |
|---|---|
| Local web server | **Axum** (Rust) + `tower-http` for static |
| Live SQL exploration | **REST** API over DuckDB cache; user can run custom queries (audit-trail brand) |
| Cross-widget filter state | **Alpine.js** (new; SHA-pinned via build.rs) |
| Threshold sliders | Re-run analyses on slider change (debounced) |
| `codelore diff` PR-mode UI | Same Axum + the SPA frontend |
| Optional auth | None for v0.5.x (local-first); team server is v0.6+ |

**v0.5.x effort estimate**: 3-5 weeks. Mostly Axum routing + glue;
the frontend is the v0.4.x SPA with Alpine sprinkled on.

### 3c.1 v0.5.x CSS / component stack — locked

Decided 2026-06-13 after a research spike (see PR-1 commit on the
branch `feat/v0.5x-ui-redesign-pr1`): **Tailwind v4 + DaisyUI 5**,
precompiled to a single CSS bundle and committed to the repo.

| Layer | Pin | Distribution |
|---|---|---|
| Interactivity (locked earlier — §3c) | Alpine.js 3.15.8 | jsDelivr `npm/alpinejs@3.15.8/dist/cdn.min.js` (~46 KB) — `build.rs` SHA-pin |
| State persistence | Alpine persist 3.15.8 | jsDelivr `npm/@alpinejs/persist@3.15.8/dist/cdn.min.js` (~1 KB) — `build.rs` SHA-pin |
| CSS framework | Tailwind v4 (CLI v4.3.1) | Standalone CLI compiles `tailwind-src/input.css` → `tailwind.daisyui.min.css` (committed) |
| Component library | DaisyUI 5 | Bundled with Tailwind v4 standalone CLI; activated via `@plugin "daisyui"` in `input.css` |
| Charts | ECharts 6.1.0 (unchanged) | already locked at §2 |
| Layout helper | d3-hierarchy 3.1.2 (unchanged) | already locked at §2 |
| Plotly basic (azure-dashboard reference candidate) | DEFERRED to per-widget PRs | per-widget call: pick when a specific chart actually needs a Plotly capability ECharts doesn't have. Not added speculatively. |

#### Why Tailwind v4 + DaisyUI 5 over the alternatives

Tailwind v4 dropped the production-grade runtime CDN script that v3
shipped (the [Play CDN](https://tailwindcss.com/docs/installation/play-cdn)
is explicitly dev-only); the recommended distribution is the
standalone CLI binary that compiles a pruned CSS file at build time.
For `CodeLore`'s offline-first build flow this means the CSS is a
*compiled artefact*, not a runtime asset.

Three handling options were evaluated:

| Option | Trade-off | Verdict |
|---|---|---|
| A. Precompile + commit | One-time CLI install per contributor; rebuild is a chore when DaisyUI bumps. CSS becomes a `git diff`-reviewable source artefact. | **Chosen.** |
| B. `build.rs` fetches + runs the standalone CLI | Zero check-in surface, BUT adds ~30 MB binary to every contributor's build cache + makes `cargo build` depend on running a foreign executable across all five release-target platforms. Trust surface larger than every other dep combined. | Rejected. |
| C. Vanilla CSS (extend the existing `:root` custom-property tokens) | No build step, no checked-in CSS. Loses DaisyUI's admin-portal component vocabulary; turns the v0.5.x redesign into a sustained hand-rolling effort. | Rejected (loses the redesign polish the user asked for). |
| D. Runtime-CDN framework alternative (Pico / Beer / Open Props) | Stays within the existing `build.rs` SHA-pin pattern with no compilation step. Different design vocabulary than DaisyUI; smaller component coverage. | Rejected (substitutes away from a stack the user already validated on `gf-azure-ops/azure-dashboard`). |

DaisyUI 5's component set maps directly to v0.4.x's existing shapes
and v0.5.x's planned ones:

| v0.4.x element | DaisyUI 5 component |
|---|---|
| `.kpi-grid` + tile divs | `stat` / `stats` |
| `.drawer-body` | `drawer` |
| Hotspot sortable table | `table table-zebra` + `badge` |
| Header + footer chrome | `navbar` + `footer` |

The framework wasn't picked for v0.4.x because the widget set was
too small to amortise the cost; v0.5.x adds Alpine reactivity +
per-widget controls, which IS where DaisyUI starts paying off.

#### PR-1 — infrastructure (shipped)

Branch: `feat/v0.5x-ui-redesign-pr1` · commit `387dc9a` on origin.

Wires the build/CSS plumbing **without any visual change**:

- `build.rs` gains two `AssetPin` entries for Alpine.js + Alpine
  persist.
- `output/spa.rs` adds three `include_str!` constants (Alpine,
  persist, CSS) into the template-substitution table.
- `template.html` gains three new slot markers; the existing
  inline `<style>` block stays in place so v0.4.x rendering is
  byte-identical.
- `spa/tailwind-src/input.css` + `README.md` document the
  rebuild workflow.
- `spa/tailwind.daisyui.min.css` ships as a STUB — a fresh
  checkout `cargo build`s without anyone needing the CLI; the stub
  carries v0.4.x defaults so the dashboard renders unstyled-but-
  readable. The stub gets replaced via `just spa-css-rebuild`.
- `justfile` gains `spa-css-rebuild` recipe.
- `tests/spa_integration_test.rs` adds six assertions covering the
  new payloads + placeholder substitution.

One human-driven step gates the PR from being fully complete:
install the Tailwind v4 standalone CLI, run `just spa-css-rebuild`,
commit the resulting real CSS file. The Claude Code classifier
blocked autonomous fetching/execution of the ~76 MB CLI binary —
this step is appropriately gated behind a human authorisation.

#### PR-2+ — per-widget DaisyUI conversion

Each follow-up PR migrates one widget at a time. Rough sequence:

1. **PR-2: chrome** — header (`navbar`), footer (`footer`), main
   `<main>` grid → DaisyUI containers. No widget internals touched.
2. **PR-3: KPI tiles** — `.kpi-grid` + `.kpi-tile` → `stats` +
   `stat` components. Inherits the new accent colour from the
   `@theme` block.
3. **PR-4: drawer** — file-detail drawer → DaisyUI `drawer` with
   right-side overlay. Adds Alpine `x-data` for the open/close
   state instead of the imperative JS in `widgets.js`.
4. **PR-5: hotspot table** — `.table-container` → `table
   table-zebra` + `badge` for MI-band / AI-attribution cells.
5. **PR-6: cross-widget filter store** — Alpine `$store` for filter
   state, persisted via the persist plugin. First widget to consume:
   hotspot table filter.

Each PR is self-contained, reviewable in one sitting, and revertable
without untangling the others.

---

## 3d. v0.6.x — Tauri 2 desktop app

Reuses the v0.5.x frontend. Tauri 2 wraps the Axum server + the SPA
frontend into a native desktop app. Validated as the right desktop
story earlier in the session research:

- Tauri 2 installer size: <5 MB (vs Electron's 300+ MB)
- `codelore-lib` links directly into `src-tauri/` — no shell-out
- Native filesystem: drag-drop a folder onto the window
- Signed cross-platform installers (`.dmg` / `.msi` / `.AppImage`)
  via Tauri's bundler
- The `duckdb` Rust crate works inside Tauri's Rust backend
  (Duckling reference confirms this pattern)

**v0.6.x effort estimate**: 4-6 weeks on top of v0.5.x.

---

## 4. Emitter shape

Pattern mirrors `write_full_fact_store_sqlite` (multi-source, ignores
`--analysis`), not `write_html` (per-row-type single analysis):

```rust
// crates/codelore-lib/src/output/spa.rs

pub struct SpaDashboard {
    pub hotspots: Vec<HotspotRow>,
    pub coupling: Vec<CouplingRow>,
    pub code_health: Vec<CodeHealthRow>,
    pub summary: Vec<SummaryRow>,
    pub knowledge_islands: Vec<KnowledgeIslandRow>,
    pub provenance: ProvenanceManifest,
}

pub fn write_spa<W: Write>(
    dash: &SpaDashboard,
    repo_path: &str,
    generated_at: &str,
    w: &mut W,
) -> Result<()> { ... }
```

CLI dispatch in `codelore-cli/src/main.rs` adds a `format == "spa"`
branch alongside the existing `format == "html"` block. It bypasses
the `--analysis` match entirely (like `--format sqlite` does today),
runs the required analyses in sequence, builds the `SpaDashboard`,
and calls `write_spa`.

### Why not a new subcommand (`codelore dashboard`)?

The `--format sqlite` precedent already broke the
"`--analysis X --format Y`" mental model — it ignores `--analysis`
and exports the whole fact store. `--format spa` follows the same
precedent. If usage feedback shows the flag is awkward, we can add
a `codelore dashboard` alias later without changing the underlying
emitter.

---

## 5. Build-time CDN fetch with SHA-pinning, gated behind the `spa` feature

`crates/codelore-lib/build.rs` fetches each minified JS dep from
jsDelivr at the pinned URL, verifies SHA-256 against a hardcoded
table, and writes to `OUT_DIR`. The emitter `include_str!`s from
`OUT_DIR`. Subsequent builds skip the fetch when the file already
exists and its hash still matches.

**The fetch only fires when the `spa` Cargo feature is enabled.**
Default builds (`cargo build`, `cargo install codelore`) do NOT
include `spa`, do NOT fetch anything from the network, and do NOT
need internet — offline-clean and audit-trail-clean. Released
binaries shipped via Homebrew / ghcr / GitHub Releases enable
`spa` at the release-workflow level.

To build CodeLore with the dashboard emitter:

```bash
cargo install codelore --features spa
# or:
cargo build --features spa
```

The first build with `spa` enabled fetches `echarts.min.js` (~1.1 MB)
and `d3-hierarchy.min.js` (~14 KB) from jsDelivr and SHA-pins
them. Subsequent builds hit the `OUT_DIR` cache without any
network call. If the cached bytes ever drift from the pin
(corruption, tampering, manual replacement), the build fails loud
with a SHA-mismatch error and refuses to embed the unverified
bytes.

```rust
// crates/codelore-lib/build.rs

struct AssetPin {
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
}

const ASSETS: &[AssetPin] = &[
    AssetPin {
        name: "echarts.min.js",
        url: "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js",
        sha256: "<pin here at first build>",
    },
    AssetPin {
        name: "d3-hierarchy.min.js",
        url: "https://cdn.jsdelivr.net/npm/d3-hierarchy@3.1.2/dist/d3-hierarchy.min.js",
        sha256: "<pin here at first build>",
    },
];
```

The build script:
1. Reads each pin
2. Checks `OUT_DIR/{name}` exists AND SHA matches
3. If yes — done
4. If no — fetches URL, verifies SHA, writes to `OUT_DIR/{name}`
5. Emits `cargo:rerun-if-changed=build.rs`

Failure modes:
- Network unavailable + asset not cached → build fails with a clear
  "first build requires internet for SPA assets" error. The user
  can disable the SPA emitter at compile time via a feature flag
  if needed.
- CDN returns content with wrong SHA → build fails. SHA-pinning is
  the supply-chain control.

### Why not vendor the JS into the repo?

- Repository hygiene — 250 KB of minified JS in git history per
  dep bump
- Easier upgrades — bump the URL + SHA in `build.rs` instead of
  re-vendoring two files
- The pin table in `build.rs` IS the audit manifest

---

## 6. The "already-shipped UI path" — SQLite + your favourite BI tool

CodeLore already ships `--format sqlite`, which exports the full
DuckDB fact store (commits, changes, hunks, entities,
complexity_metrics, author_aliases, provenance, clones) to a
portable `.sqlite` file.

Any SQL-native dashboard tool can open it today:

```bash
codelore analyze --analysis summary --format sqlite -o codelore.sqlite

# Then any of:
datasette codelore.sqlite        # browse + plugins
duckdb codelore.sqlite           # SQL REPL
rill start codelore.sqlite       # local-first BI
# point Metabase / Superset / Evidence.dev / Observable Framework at it
```

`docs/bi-integration.md` (planned) will document this with example
queries that reproduce each of the 23 analyses. This path is
complementary to the SPA emitter — users who want raw exploration
can stay here; users who want the curated CodeLore narrative get
the SPA.

---

## 7. F38 fold-in (perf precondition for the SPA)

F38 is the Kamei `enrich_history` / `enrich_experience` quadratic
self-join. It produces the `ndev`, `nuc`, `age`, `exp`, `rexp`,
`sexp` columns on `commits` — which feed the code-health analysis,
which feeds the v0.4.0 KPI tiles.

Fixing F38 before the SPA work ensures:
- The KPI tiles surface correct numbers
- Large repos don't time out on ingest when the SPA emitter calls
  the code-health analysis
- The v0.4.0 release ships one cohesive story (perf + UI), not
  split releases

The fix replaces the cartesian self-join (`pchg.path = cchg.path`,
multiplied N-fold per commit) with window-function aggregations
that compute per-path cumulative-distinct counts in a single pass.
See the F38 implementation commit for the SQL rewrite + the
fixture-based correctness validation against the current
implementation.

---

## 8. Out of scope (across all v0.4.x-v0.6.x phases)

- **Hosted SaaS** — out of scope; conflicts with the local-first
  audit-trail brand. Not planned.
- **Cloud sync / team server** — possibly v0.7.x+ if user feedback
  asks for it; not on the active roadmap.
- **VSCode extension** — different surface, different repo. The
  SPA emitter does not preclude this work; SARIF already covers
  editor integration via the GitHub Code Scanning channel.
- **Alternative dashboard frameworks** (Observable Framework /
  Evidence.dev / Rill) — explicitly dropped. The v0.4.0 SPA emitter
  delivers the curated CodeLore experience; users who want raw SQL
  exploration use the already-shipped `--format sqlite` output with
  Datasette / Rill / Metabase. No parallel emitter to maintain.

**Now PLANNED (was out-of-scope in the original roadmap):**
- `codelore serve` — moved into **v0.5.x** above
- Tauri desktop app — moved into **v0.6.x** above
- Alpine.js framework upgrade — locked in for **v0.5.x** boundary
