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
| Framework | **None** (vanilla JS) | Existing `output/html.rs` uses vanilla JS and serves 525 LOC of paginated/sortable table — proves the pattern. Adding a framework (Svelte/Vue/React) would add ~30–40 KB and a build pipeline without commensurate benefit. |
| Build | **`build.rs` + SHA-256-pinned CDN fetch behind `spa` Cargo feature** | Avoids committing minified JS into the repo. The `spa` feature is **opt-in**: default builds skip the fetch entirely (no internet needed). When enabled, `crates/codelore-lib/build.rs` fetches ECharts and d3-hierarchy from jsDelivr at pinned URLs, verifies SHA-256 against the table in `ASSETS`, and caches them in `OUT_DIR`. Subsequent builds with `spa` enabled hit the cache (no network) until either the pin changes or the cached bytes drift. Audit-trail: the build script's pin table IS the supply-chain manifest. |
| Template | `include_str!` from `src/output/spa/template.html` | Same idiom as the existing HTML emitter; no runtime templating engine. |

### Why not Svelte / Solid / React?

- They each add 25–40 KB of runtime + a Vite-style build step
- The CodeLore HTML emitter precedent is vanilla JS (no framework)
- Single-file static output is more brittle when a framework is involved
- We are not building a long-lived SPA with routing, forms, or
  reactive state graphs — we are rendering a fixed set of charts
  from one JSON blob

### Why not Observable Framework / Evidence.dev / Rill?

- They stamp their own chrome and project layout on the output
- "This IS CodeLore" identity is weaker
- All three are valid alternatives the user can reach via the
  `--format sqlite` path that **already ships**; see §6 below

---

## 3. v0.4.x release sequence

| Version | Scope | Status |
|---|---|---|
| **v0.4.0** | F38 perf fix + 6-widget SPA-MVP (KPI tiles, hotspot circle-pack, hotspot table, change-coupling sankey, knowledge-islands ranked view, file detail drawer) | **Shipped 2026-06-11** ✓ |
| **v0.4.1** | 3 more widgets: knowledge map (author-colored treemap), function-level X-Ray sunburst, trends multi-line | Planned |
| **v0.4.2** | Calendar heatmap, alternate coupling viz (network/chord), theming + dark mode | Planned |
| **v0.4.3+** | Embed mode for CI artefacts, perf tuning on > 100k-commit repos, AI-attribution overlay | Planned |

### v0.4.0 widget detail

| # | Widget | Data source(s) | ECharts series | Notes |
|---|---|---|---|---|
| 1 | **Hotspot circle-pack** | `run_hotspots` + filesystem hierarchy | `custom` + `d3.pack()` layout | The signature CodeScene look. Color = combined churn × complexity. |
| 2 | **Hotspot table** | same `HotspotRow[]` | existing paginated table | Drill-down from #1 click. |
| 3 | **Code-health KPI tiles** | `run_summary` + `run_code_health` | HTML cards | Total files, p95 cognitive, churn rate, AI-attribution %. |
| 4 | **File detail drawer** | per-path slice of all run_* | HTML drawer | Click any file/cell → drawer with that path's metrics across analyses. |
| 5 | **Change coupling sankey** | `run_coupling` top-N | native `sankey` | Quick to land; alternate `graph` (force network) view comes in v0.4.2. |
| 6 | **Knowledge islands** | `run_knowledge_islands` | native `treemap` + ranked list | CodeLore's differentiator. Surface prominently. |

**Bundle estimate**:
- ECharts 6.1.0 minified: ~250 KB
- d3-hierarchy 3.1.2 minified: ~10 KB
- CodeLore SPA glue (template + widgets.js): ~20 KB
- Per-analysis JSON data (typical mid-size repo): 50 KB – 2 MB
- **Total single-file HTML**: ~330 KB + data; ~110 KB gzipped + data-gzipped

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

## 8. Out of scope for v0.4.x

- **Hosted SaaS** — out of scope for the local-first CLI brand.
- **Cloud sync / team server** — possibly v0.5.x if user feedback
  asks for it.
- **VSCode extension** — different surface, different repo. The
  SPA emitter does not preclude this work; SARIF already covers
  editor integration via the Code Scanning channel.
- **Tauri desktop app** — once the SPA matures (v0.5.x).
- **`codelore serve`** — local Axum server with live SQL. Once
  the SPA is stable (v0.5.x).
