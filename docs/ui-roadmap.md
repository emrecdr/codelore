# CodeLore — UI Roadmap

This document captures the technical decisions and current widget inventory for the CodeLore dashboard, plus the planned next direction. The brand axis is preserved throughout: **local-first, audit-trail, deterministic-formula**.

It is a **planning artefact**, not a contract. Re-validate every assumption before implementing each planned phase (see `feedback_validate_assumptions.md`).

For per-release history of widgets and stack changes, see `CHANGELOG.md`.

---

## 1. North star

A single self-contained HTML file (`codelore analyze --format spa -o codelore.html`) that opens in any browser, runs without a server, fits in a CI artefact, and surfaces a CodeScene-equivalent analytic surface — **plus** CodeLore's three differentiators that CodeScene does not have:

1. **Auto-detected knowledge islands** (departed-author × clones × co-change intersection) — CodeScene requires manual ex-developer marking.
2. **AI-attribution toggle** — filter hotspots by who wrote them (human, AI-assisted, AI-dominant). CodeScene has no such signal.
3. **Auditable formulas** — a `?` tooltip on every metric links to the SQL query, the research foundation (`docs/research-foundations.md`), and the provenance manifest. CodeScene's biomarkers are opaque; CodeLore's are inspectable.

---

## 2. Technical stack

| Layer | Choice | Justification |
|---|---|---|
| Charts (~95% of widgets) | **Apache ECharts 6.1.0** | Top-starred Apache Foundation project (66k+ stars); monthly minor releases; XSS-fix responsiveness validated in 6.1.0 + 5.5.1. Apache-2.0 license (deny.toml-allowed). Treemap, sunburst, sankey, chord, force-graph, heatmap, calendar heatmap, parallel coords, line/bar/area — all native. |
| Circle-pack layout (1 widget) | **d3-hierarchy 3.1.2** | ECharts has no native circle-pack series. `d3-hierarchy.pack()` computes the layout (~10 KB tree-shaken); ECharts `custom` series renders the result. ISC license (deny.toml-allowed). |
| Interactivity layer | **Alpine.js 3.15 + persist plugin** | ~46 KB minified core + sub-1 KB persist plugin; both single `<script>` embeds via the same SHA-256-pinned `build.rs` fetch pattern as ECharts. HTML-attribute reactivity (`x-data`, `x-show`, `x-on`, `x-model`, `x-cloak`, `x-transition`) matches the Rust-generated template philosophy — directives sit on existing DOM, no virtual DOM, no JSX/SFC compiler. Three production stores: `$store('detail')` (drawer open/close), `$store('filter')` (cross-widget filter text, `$persist`-backed), `$store('theme')` (light/dark toggle, `$persist`-backed). |
| CSS framework | **Tailwind v4 + DaisyUI 5** | Tailwind v4 utility-first layout (`grid-cols-1 xl:grid-cols-2 gap-7 p-7` etc.) + DaisyUI 5 themed components (`navbar`, `card`, `stat`, `table-zebra`, `badge`, `swap`). DaisyUI plugin config `themes: light --default, dark --prefersdark` makes first-paint match the OS `prefers-color-scheme` via CSS media query — no JS frame for the wrong theme. Pre-compiled CSS bundle (~78 KB minified) inlined into the SPA at build time via the same template-substitution layer as ECharts/Alpine. |
| Build | **`build.rs` + SHA-256-pinned CDN fetch behind `spa` Cargo feature** | Avoids committing minified JS into the repo. The `spa` feature is **opt-in**: default builds skip the fetch entirely (no internet needed). When enabled, `crates/codelore-lib/build.rs` fetches each pinned dep from jsDelivr at pinned URLs, verifies SHA-256 against the `ASSETS` table, and caches them in `OUT_DIR`. The pin table IS the supply-chain manifest. |
| Template | `include_str!` from `src/output/spa/template.html` | Same idiom as the existing HTML emitter; no runtime templating engine. |

### Why not Svelte / Solid / React?

- They each add 25–40 KB of runtime + a Vite-style build step.
- The CodeLore HTML emitter precedent is vanilla JS plus Alpine attribute directives — no SFC compiler.
- Single-file static output is more brittle when a framework is involved.
- The dashboard is not a long-lived SPA with routing, forms, or deep reactive state graphs — it renders a fixed widget set from one JSON blob.

### Why Alpine.js (re-validated against the size-class peers)

Alpine sits in the sweet spot between "raw DOM gets unmanageable past 2–3 cross-widget filters" and "real framework adds a build pipeline":

- **Size budget**: 15 KB minified+gzipped (3.x).
- **Programming model fit**: HTML-attribute directives sit on existing DOM nodes. The template is `include_str!`-shipped from Rust; no JSX, no SFC compiler, no Vite step. The same template renders identically in `--format spa` (static) and in any future served version.
- **Cross-widget filter state**: Alpine's reactive `$store` is the exact primitive multi-widget filtering needs — filter on hotspot table updates a store, every widget watching that store re-renders. Doing this in vanilla JS scales to 2–3 widgets; the dashboard has 15+.
- **SHA-pin compatibility**: ships as a single minified `<script>`, drops into the same `build.rs` SHA-256 pin table that hosts ECharts and d3-hierarchy. No build pipeline added.
- **Trajectory check**: Alpine 3.x has been stable since 2020 with regular minor releases; not a flavor-of-the-quarter risk. Used by Laravel (Livewire), 37signals (Basecamp), and the Caleb Porzio ecosystem — long-tail mindshare.

Rejected alternatives at this size class:

- **HTMX** — request-oriented (every interaction is a server roundtrip), conflicts with the "single static HTML works offline" north star.
- **Petite-Vue** — maintenance-only since 2022.
- **Lit** — Web Components add 8 KB and a class-based component model that fights Rust-templated HTML.
- **Vue / React / Svelte** — build pipeline + framework chrome; rejected above.

### Why Tailwind v4 + DaisyUI 5 over the CSS alternatives

Tailwind v4 dropped the production-grade runtime CDN script that v3 shipped (the [Play CDN](https://tailwindcss.com/docs/installation/play-cdn) is explicitly dev-only); the recommended distribution is the standalone CLI binary that compiles a pruned CSS file at build time. For CodeLore's offline-first build flow this means the CSS is a *compiled artefact*, not a runtime asset.

Three handling options were evaluated:

| Option | Trade-off | Verdict |
|---|---|---|
| A. Precompile + commit | One-time CLI install per contributor; rebuild is a chore when DaisyUI bumps. CSS becomes a `git diff`-reviewable source artefact. | **Chosen.** |
| B. `build.rs` fetches + runs the standalone CLI | Zero check-in surface, BUT adds ~30 MB binary to every contributor's build cache + makes `cargo build` depend on running a foreign executable across all five release-target platforms. Trust surface larger than every other dep combined. | Rejected. |
| C. Vanilla CSS (extend the prior `:root` custom-property tokens) | No build step, no checked-in CSS. Loses DaisyUI's admin-portal component vocabulary; turns the redesign into a sustained hand-rolling effort. | Rejected. |
| D. Runtime-CDN framework alternative (Pico / Beer / Open Props) | Stays within the existing `build.rs` SHA-pin pattern with no compilation step. Different design vocabulary than DaisyUI; smaller component coverage. | Rejected. |

DaisyUI 5's component vocabulary maps directly onto the widget set:

| Dashboard element | DaisyUI 5 component |
|---|---|
| `.kpi-grid` + tile divs | `stat` / `stats` |
| `.drawer-body` | `drawer` |
| Hotspot sortable table | `table table-zebra` + `badge` |
| Header + footer chrome | `navbar` + `footer` |

### Why not Observable Framework / Evidence.dev / Rill?

- They stamp their own chrome and project layout on the output.
- "This IS CodeLore" identity is weaker.
- All three are valid alternatives the user can reach via the `--format sqlite` path that already ships; see §6 below.

### Stack pin table

| Layer | Pin | Distribution |
|---|---|---|
| Interactivity | Alpine.js 3.15.12 | jsDelivr `npm/alpinejs@3.15.12/dist/cdn.min.js` — `build.rs` SHA-pin |
| State persistence | Alpine persist 3.15.12 | jsDelivr `npm/@alpinejs/persist@3.15.12/dist/cdn.min.js` — `build.rs` SHA-pin |
| Charts | ECharts 6.1.0 | jsDelivr `npm/echarts@6.1.0/dist/echarts.min.js` — `build.rs` SHA-pin |
| Layout helper | d3-hierarchy 3.1.2 | jsDelivr `npm/d3-hierarchy@3.1.2/dist/d3-hierarchy.min.js` — `build.rs` SHA-pin |
| CSS framework | Tailwind v4 (CLI v4.3.1) | Standalone CLI compiles `tailwind-src/input.css` → `tailwind.daisyui.min.css` (committed) |
| Component library | DaisyUI 5 | Bundled with Tailwind v4 standalone CLI; activated via `@plugin "daisyui"` in `input.css` |

---

## 3. Current widgets

The SPA emitter follows the same multi-source pattern as `write_full_fact_store_sqlite` (ignores `--analysis`): it runs the required analyses in sequence, builds a `SpaDashboard`, and renders the embedded template.

| # | Widget | Data source(s) | ECharts series | Notes |
|---|---|---|---|---|
| 1 | **Hotspot circle-pack** | `run_hotspots` + filesystem hierarchy | `custom` + `d3.pack()` layout | Signature CodeScene look. Supports 7 color modes (cognitive, code health, tech-debt friction, knowledge loss, AI attribution, MI band, clones overlay) plus a ring overlay and coupling-arc overlay (Fisher p-value-encoded opacity + degree-encoded width). |
| 2 | **Hotspot table** | same `HotspotRow[]` | sortable paginated table (500/page) + 80 ms debounced filter | Drill-down from #1 click; row click → detail drawer. MI-band emoji → DaisyUI badge; AI percentage → outline badge. |
| 3 | **KPI tiles** | `run_summary` + `run_code_health` + `run_hotspots` + `run_knowledge_islands` + `run_coupling` + `run_communities` | DaisyUI `stat` / `stats` | Files analyzed, commits, distinct authors, median code health, cognitive p95, knowledge-island count, coupling pair count, MI band breakdown, behavioural-community count + global Q. |
| 4 | **File detail drawer** | per-path slice of all run_* | DaisyUI `drawer` + Alpine `x-show="$store.detail.open"` with `x-transition.opacity` + `@keydown.escape.window` close | Click any circle / table row / sankey / chord / force-graph node → drawer with that path's hotspot, knowledge-island, code-health, coupling-partner, behavioural-module, and per-file radar data. |
| 5 | **Change coupling sankey** | `run_coupling` top pairs by combined score | native `sankey` | Node click → drawer. Node coloring drives off the behavioural community ID. |
| 6 | **Knowledge islands** (CodeLore differentiator) | `run_knowledge_islands` | ranked HTML table | Auto-detected departed-author files. Surfaced with a "CodeLore differentiator" badge. |
| 7 | **Knowledge map** (author-colored treemap) | `run_hotspots` + `run_entity_ownership` | native `treemap` | Palette swap by `primary_author`. |
| 8 | **X-Ray sunburst** (function-level) | `entities` + `complexity_metrics` (raw SQL aggregation in `run_xray`) | native `sunburst` | Per-leaf `itemStyle.color` driven off `cognitive / maxCognitive` via the same `heatmapColor(ratio)` helper as the circle-pack — one visual vocabulary across the dashboard. Entities pre-filtered to live-at-HEAD. |
| 9 | **Trends multi-line** | hotspots over time, top-N files | native `line` | `run_hotspots` with `--time-bucket month`. |
| 10 | **Calendar heatmap** | raw `SELECT date_trunc('day', date) AS d, COUNT(*) FROM commits GROUP BY d` | native `heatmap` on `calendar` coord | Single-glance authoring cadence. |
| 11 | **AI-attribution overlay** | client-side filter on `commits.ai_attribution` band | re-renders #1 + #2 with filtered data | Toggle UI + JS. |
| 12 | **Kamei Delivery-Risk Sparkline** | per-commit Kamei JIT-SDP feature vector (LA, LD, NS, ND, NF, Entropy, LT, NDEV, AGE, NUC, EXP, REXP, SEXP) | native `line` with multi-axis overlay | Beyond CodeScene — exposes the peer-reviewed JIT-SDP feature dimensions per commit. |
| 13 | **Hotspot treemap** | `run_hotspots` | native `treemap` | Alternate framing of the circle-pack data for users who prefer rectangular density. |
| 14 | **Parallel coordinates** | hotspots × cognitive × MI × knowledge-loss × AI% | native `parallel` | Multi-metric exploration. |
| 15 | **Cognitive boxplot** | `complexity_metrics.cognitive` per language / module | native `boxplot` | Distribution view; surfaces tail-risk that means/medians hide. |
| 16 | **Module chord** | `run_communities` edges between top modules | native `chord` | Behavioural-community inter-edge density. |
| 17 | **Architecture force-graph** | `imports` table + `architecture-violations` analysis | native `graph` (force-directed) | Layered-architecture rule overlay; violating edges highlighted. |
| 18 | **Drawer radar** | per-file `cognitive / code-health / MI / knowledge-loss / AI% / coupling-degree` vector | native `radar` inside the detail drawer | Compact per-file profile. |

### Cross-widget interactions

- **Off-boarding scenario picker** — DaisyUI multi-select dropdown + `$persist`. Runs entirely client-side over the embedded `run_knowledge_islands` payload; selected ex-author set re-colours the circle-pack and re-ranks the knowledge-islands table.
- **Cross-widget filter store** — `Alpine.store('filter', { text, set, clear })` with `Alpine.$persist` for `localStorage` round-trip. Hotspot table filter input is the primary writer; the circle-pack and sankey watch the store and dim non-matching nodes.
- **Theme controller** — `Alpine.store('theme', { isDark: $persist(initialDark).as('codelore_theme_is_dark') })` + `Alpine.effect` bridge mirrors the boolean to `<html data-theme>` AND fires every ECharts re-renderer in `_codeloreRerenderers`. Defense-in-depth via DaisyUI `<label class="swap swap-rotate">` + `class="theme-controller" value="dark"` checkbox (CSS-only swap if Alpine fails to load).
- **First-paint guard** — DaisyUI `themes: light --default, dark --prefersdark` plus an anti-flash inline `<head>` script that reads persisted preference and sets `data-theme` synchronously before first paint.

### Modern web-platform primitives used

- **View Transitions API** — smooth drawer-open and color-mode-swap transitions where the browser supports it.
- **Native `<dialog>`** — drawer chrome falls back gracefully where unsupported.
- **PWA manifest** — install-as-app affordance.
- **OKLCH `color-mix()`** — tech-debt friction heat ramp.
- **WCAG-conformant parallel DOM tree** — keyboard-accessible mirror of the circle-pack hierarchy.

### Bundle reference

- ECharts 6.1.0 minified: ~1.1 MB
- d3-hierarchy 3.1.2 minified: ~14 KB
- Alpine.js 3.15.12 core + persist: ~47 KB
- Tailwind v4 + DaisyUI 5 compiled bundle: ~78 KB
- CodeLore SPA glue (template.html + widgets.js): ~30 KB
- Per-analysis JSON payload on a 300-file repo: ~50 KB
- **Typical emitted size on the CodeLore repo itself**: ~1.5 MB (~400 KB gzipped over the wire)

---

## 4. Planned widgets and dashboard work

Forward-looking dashboard items. Cross-reference [`roadmap-v1.x-and-beyond.md`](./roadmap-v1.x-and-beyond.md) for the broader product-level direction.

### Embed mode

`codelore analyze --format spa --embed` strips the full-page shell and produces an HTML fragment suitable for embedding in `$GITHUB_STEP_SUMMARY` and SARIF code-scanning result pages. Keeps the widgets, drops the header/footer/styling that conflicts with embedding contexts.

### Interactive served mode

`codelore serve` (Axum) wraps the existing fact-store query layer as REST endpoints and serves the SPA frontend live. Reuses every widget unchanged; the value-add is:

- **Threshold sliders** that re-run analyses on the server with debouncing — surfaces the deterministic-formula brand promise interactively. Leverages the persistent DuckDB cache; no new analysis code.
- **Live SQL exploration** — `--query SQL` escape hatch with a UI; power-user feature; audit-trail brand. Wires into the served UI.
- **`codelore diff` PR-mode UI** — same Axum + SPA frontend, diff-anchored. Reuses the existing `codelore diff` subcommand.

No new metrics, no new analyses; same widget set served live instead of dumped as a static file.

### Desktop wrap

Tauri 2 wraps the served mode into a native desktop app:

- Tauri 2 installer size: <5 MB (vs Electron's 300+ MB)
- `codelore-lib` links directly into `src-tauri/` — no shell-out
- Native filesystem: drag-drop a folder onto the window
- Signed cross-platform installers (`.dmg` / `.msi` / `.AppImage`) via Tauri's bundler
- The `duckdb` Rust crate works inside Tauri's Rust backend (Duckling reference confirms this pattern)

Same SPA, just locally-installed. The value-add is the discovery surface ("install CodeLore.app, drag your repo onto it"). Skipping it would cap reach at CLI users.

---

## 5. Emitter shape

Pattern mirrors `write_full_fact_store_sqlite` (multi-source, ignores `--analysis`), not `write_html` (per-row-type single analysis):

```rust
// crates/codelore-lib/src/output/spa.rs

pub struct SpaDashboard {
    pub hotspots: Vec<HotspotRow>,
    pub coupling: Vec<CouplingRow>,
    pub code_health: Vec<CodeHealthRow>,
    pub summary: Vec<SummaryRow>,
    pub knowledge_islands: Vec<KnowledgeIslandRow>,
    pub communities: Vec<CommunityRow>,
    pub centrality: Vec<CentralityRow>,
    pub clones: Vec<CloneSummary>,
    pub entities: Vec<XrayRow>,
    pub provenance: ProvenanceManifest,
    // ...
}

pub fn write_spa<W: Write>(
    dash: &SpaDashboard,
    repo_path: &str,
    generated_at: &str,
    w: &mut W,
) -> Result<()> { ... }
```

CLI dispatch in `codelore-cli/src/main.rs` carries a `format == "spa"` branch alongside the `format == "html"` block. It bypasses the `--analysis` match entirely (like `--format sqlite` does), runs the required analyses in sequence, builds the `SpaDashboard`, and calls `write_spa`.

### Why not a new subcommand (`codelore dashboard`)?

The `--format sqlite` precedent already broke the "`--analysis X --format Y`" mental model — it ignores `--analysis` and exports the whole fact store. `--format spa` follows the same precedent. If usage feedback shows the flag is awkward, a `codelore dashboard` alias can be added later without changing the underlying emitter.

---

## 6. Build-time CDN fetch with SHA-pinning

`crates/codelore-lib/build.rs` fetches each minified JS dep from jsDelivr at the pinned URL, verifies SHA-256 against a hardcoded table, and writes to `OUT_DIR`. The emitter `include_str!`s from `OUT_DIR`. Subsequent builds skip the fetch when the file already exists and its hash still matches.

**The fetch only fires when the `spa` Cargo feature is enabled.** Default builds (`cargo build`, `cargo install codelore`) do NOT include `spa`, do NOT fetch anything from the network, and do NOT need internet — offline-clean and audit-trail-clean. Released binaries shipped via Homebrew / ghcr / GitHub Releases enable `spa` at the release-workflow level.

To build CodeLore with the dashboard emitter:

```bash
cargo install codelore --features spa
# or:
cargo build --features spa
```

The build script:

1. Reads each pin in the `ASSETS` table.
2. Checks `OUT_DIR/{name}` exists AND SHA matches.
3. If yes — done.
4. If no — fetches URL, verifies SHA, writes to `OUT_DIR/{name}`.
5. Emits `cargo:rerun-if-changed=build.rs`.

Failure modes:

- Network unavailable + asset not cached → build fails with a clear "first build requires internet for SPA assets" error. The user can disable the SPA emitter at compile time via the feature flag if needed.
- CDN returns content with wrong SHA → build fails. SHA-pinning is the supply-chain control.

### Why not vendor the JS into the repo?

- Repository hygiene — ~250 KB+ of minified JS in git history per dep bump.
- Easier upgrades — bump the URL + SHA in `build.rs` instead of re-vendoring.
- The pin table in `build.rs` IS the audit manifest.

---

## 7. The "already-shipped UI path" — SQLite + your favourite BI tool

CodeLore already ships `--format sqlite`, which exports the full DuckDB fact store (commits, changes, hunks, entities, complexity_metrics, author_aliases, provenance, clones, imports) to a portable `.sqlite` file.

Any SQL-native dashboard tool can open it:

```bash
codelore analyze --analysis summary --format sqlite -o codelore.sqlite

# Then any of:
datasette codelore.sqlite        # browse + plugins
duckdb codelore.sqlite           # SQL REPL
rill start codelore.sqlite       # local-first BI
# point Metabase / Superset / Evidence.dev / Observable Framework at it
```

`docs/bi-integration.md` (planned) will document this with example queries that reproduce each of the 43 analyses. This path is complementary to the SPA emitter — users who want raw exploration can stay here; users who want the curated CodeLore narrative get the SPA.

---

## 8. Out of scope

- **Hosted SaaS** — conflicts with the local-first audit-trail brand. Not planned.
- **Cloud sync / team server** — possibly later if user feedback asks for it; not on the active roadmap.
- **VS Code extension** — different surface, different repo. The SPA emitter does not preclude this work; SARIF already covers editor integration via the GitHub Code Scanning channel.
- **Alternative dashboard frameworks** (Observable Framework / Evidence.dev / Rill) — the SPA emitter delivers the curated CodeLore experience; users who want raw SQL exploration use the already-shipped `--format sqlite` output with Datasette / Rill / Metabase. No parallel emitter to maintain.

In the planned column (was out-of-scope in earlier drafts):

- `codelore serve` — see §4.
- Tauri desktop app — see §4.
