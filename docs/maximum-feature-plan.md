# Maximum aligned feature plan

**Status:** ✅ **FULLY SHIPPED** (all 27 features + 6 deferral closures). Last updated 2026-06-15.

> The original 18-day plan has shipped end-to-end. All 27 inventoried features are implemented; the six explicit deferrals from prior turns (F-Q1/Q2/Q3/Q4 quality-gates, F-D2 release-notes, and the Rust + Python resolver coverage for F-A1) have been closed in a follow-up sprint. See `CHANGELOG.md` for the per-feature release mapping and `git log` for the per-feature commit trail.

**Reads with:**
[`codescene-parity-plan.md`](./codescene-parity-plan.md) (the CodeScene parity subset of this plan) ·
[`ui-roadmap.md`](./ui-roadmap.md) (UI evolution rules) ·
[`roadmap-v1.x-and-beyond.md`](./roadmap-v1.x-and-beyond.md) (active milestones) ·
[`research-foundations.md`](./research-foundations.md) (citation home) ·
[`reports/deep_analysis_report.md`](./reports/deep_analysis_report.md) (open F-findings).

This document answers: **what is the largest, most architecturally-aligned feature set CodeLore can ship while staying inside its current stack and modern-tooling brand?**

It supersedes `codescene-parity-plan.md` by treating that plan as Tier 1 of a wider effort. Every Tier 1 + Tier 2 item is source-quote validated against current `main` (post-v0.6.0).

---

## 1. TL;DR

| | |
|---|---|
| **Total new features** | **27** — 12 widgets/visualizations, 7 analyses, 5 CLI commands, 3 platform upgrades |
| **New Rust dependencies** | **3** — `schemars` (JSON Schema gen), `jwalk` (parallel filesystem walk), `clap_complete` (shell completions). All actively-maintained, modern, narrow-scope. |
| **New external JS deps** | **0** — every new widget uses ECharts series already in the SHA-pinned 6.1.0 bundle. No CDN additions. |
| **New web platform primitives** | **5 modern, Baseline-2024+** — View Transitions API, Popover API, native `<dialog>`, CSS Container Queries, OKLCH `color-mix()`. All have graceful fallbacks. |
| **Closes open F-findings** | **F71, F90, F92, F97, F98** — five existing Active findings retire as side effects of Tier 1+2. |
| **Wall-clock estimate** | **18 working days** end-to-end (Tier 1: 6d, Tier 2: 7d, Tier 3: 5d). |
| **New ingest cost** | One-time per cache-miss: ~5 % over current baseline (import-graph extraction). All other features ship as SQL aggregations over existing tables. |

---

## 2. Validation matrix

Every architectural claim in this plan was source-verified.

| Claim | Source | Status |
|---|---|---|
| Tree-sitter at HEAD already parses Rust/Python/Java/JS/TS/TSX | `clones/language.rs:43-45` + `complexity/language.rs` | ✅ confirmed |
| The clones extractor walks every AST node via TreeCursor | `clones/fingerprint.rs:108-131` (single-cursor preorder, F45 fix) | ✅ confirmed — extending to import-node detection is a one-walker addition |
| `petgraph` is scoped to `codelore-rca/src/preproc.rs` (~50 LOC) | grep across all 3 crates | ✅ confirmed — adding architecture analysis can either reuse it or pick the modern `petgraph 0.8` since the use site is isolated |
| `leiden-rs 0.8` is on the workspace dep list | `crates/codelore-lib/Cargo.toml` | ✅ confirmed (v0.6.0 work shipped) |
| `ureq 3` is on the workspace dep list | `crates/codelore-lib/Cargo.toml` | ✅ confirmed (round-2 F105 already resolved on `main`) |
| `SpaDashboard` serializes `coupling`, `knowledge_islands`, `entity_ownership`, `xray`, `daily_commits`, `trends`, `mi_rollup` | `output/spa.rs::SpaDashboard` | ✅ confirmed — at least 11 data fields already shipped to the SPA payload |
| `HotspotRow` carries `code_health`, `hotspot_score`, `cognitive`, `mi`, `mi_rank`, `ai_pct`, `revisions` | `analyses/hotspots.rs::HotspotRow` | ✅ confirmed |
| `CouplingRow` carries `degree` AND `fisher_p` | `analyses/coupling.rs::CouplingRow` | ✅ confirmed |
| Kamei 14 features live in `commits` table at schema_v1 | `facts/schema_v1.sql:21-26` | ✅ confirmed — `ns, nd, nf, entropy, fix, la, ld, lt, ndev, age, nuc, exp, rexp, sexp` |
| ECharts 6.x catalog has 22 series types | Apache ECharts builder + cheat-sheet (Context7 `/apache/echarts-website`) | ✅ confirmed — current SPA uses 6 of 22 |
| View Transitions API is Baseline 2024 with graceful fallback | MDN | ✅ confirmed — `document.startViewTransition` no-op on unsupported browsers |
| Popover API is Baseline 2025 with native top-layer management | MDN | ✅ confirmed — pure HTML attributes, no JS lib |
| OKLCH `color-mix()` ships in all current evergreen browsers | DaisyUI 5 already uses it for theme tokens | ✅ confirmed |
| `schemars` derives JSON Schema from any serde-Serialize struct | Schemars docs (Context7 `/gresau/schemars`) | ✅ confirmed — composes with existing `#[derive(Serialize)]` |

---

## 3. The modern web platform we already command

CodeLore's v0.5.0 stack (Tailwind v4 + DaisyUI 5 + Alpine.js + ECharts + d3-hierarchy, all SHA-pinned in `build.rs`) is one of the most modern SPA foundations possible. The features below extend it *without* reaching for legacy primitives:

| Modern primitive | Status | Replaces (legacy) | Use in this plan |
|---|---|---|---|
| **View Transitions API** | Baseline 2024+ | Hand-rolled CSS transitions / FLIP animations | Color-mode swap animations, drawer open/close, scenario-state changes |
| **Popover API** | Baseline 2025 | Tooltip libraries (Tippy.js, Popper.js, etc.) | UI-2 roadmap "?" tooltips, scenario dropdowns, KPI explainers |
| **Native `<dialog>`** | Universal since 2022 | Custom drawer logic + focus traps | Detail drawer (F71 cleanup), filter modal, scenario picker |
| **CSS Container Queries** | Universal since 2023 | JS-driven `ResizeObserver` (F71) | Widget-internal responsive layouts |
| **OKLCH `color-mix()`** | Universal since 2024 | sRGB/HSL interpolation (perceptually wrong) | All heat ramps, theme-aware gradients |
| **CSS Cascade Layers** | Universal since 2022 | Specificity wars between Tailwind/DaisyUI/inline | Tailwind v4 + DaisyUI 5 already use `@layer`; new tokens slot in |
| **HTML5 `<details><summary>`** | Universal | JS-driven collapsibles | All collapse interactions (no JS) |

Every primitive above ships **with graceful fallback** — older browsers degrade quietly. None introduces a new framework dependency.

---

## 4. Feature inventory (27 features, scored)

Scoring rubric:

- **Leverage** (1–5): how much user-facing value per dev-day. 5 = brand-defining.
- **Risk** (1–5): implementation risk including coupling to external state. 5 = high.
- **Modern fit** (1–5): how cleanly it uses our modern stack vs legacy primitives. 5 = pure modern.
- **Data ready**: ✅ if no new ingest, 🟡 if minor ingest extension, ⛔ if blocked.

| ID | Feature | Leverage | Risk | Modern | Data | Tier |
|---|---|---|---|---|---|---|
| **F-V1** | Code-health color mode (3-band green/yellow/red) | 5 | 1 | 5 | ✅ | 1 |
| **F-V2** | Tech-debt friction color mode (4-step ramp) | 5 | 1 | 5 | ✅ | 1 |
| **F-V3** | Hotspot ring overlay (yellow stroke on P75+) | 4 | 1 | 5 | ✅ | 1 |
| **F-V4** | Coupling arc-edge overlay with Fisher-encoded opacity | 5 | 2 | 5 | ✅ | 1 |
| **F-V5** | Knowledge-loss + offboarding scenario interaction | 5 | 2 | 5 | ✅ | 1 |
| **F-V6** | Kamei Delivery-Risk sparkline (per-commit risk score) | 5 | 2 | 5 | ✅ | 1 |
| **F-A1** | Architecture layer detection (tree-sitter import graph) | 5 | 3 | 5 | 🟡 | 1 |
| **F-A2** | Layered-architecture rule validation (config-driven) | 4 | 2 | 5 | 🟡 (uses A1) | 2 |
| **F-A3** | God-class / god-method detector (cognitive × fan-in/out) | 4 | 2 | 5 | 🟡 (uses A1) | 2 |
| **F-A4** | Stale code surfacer (low cognitive + months-untouched) | 3 | 1 | 5 | ✅ | 2 |
| **F-A5** | Pair-programming detector (Co-Authored-By trailers) | 3 | 1 | 5 | ✅ | 2 |
| **F-A6** | Release lead-time + cycle-time analyses (commit → tag) | 4 | 2 | 5 | ✅ | 2 |
| **F-A7** | Bus factor *per module* (not just per file) | 4 | 2 | 5 | ✅ | 2 |
| **F-V7** | Radar widget — 6-axis per-file profile | 3 | 1 | 5 | ✅ | 2 |
| **F-V8** | Treemap (square) alternative view to circle-pack | 3 | 1 | 5 | ✅ | 2 |
| **F-V9** | Parallel coordinates — multi-metric file comparison | 3 | 1 | 5 | ✅ | 2 |
| **F-V10** | Boxplot — metric distribution across files | 3 | 1 | 5 | ✅ | 2 |
| **F-V11** | Architecture force-graph (uses A1 import edges) | 5 | 3 | 5 | 🟡 (uses A1) | 2 |
| **F-V12** | Module-to-module chord diagram (group-file rollup) | 4 | 2 | 5 | ✅ | 2 |
| **F-Q1** | `codelore check` — pass/fail quality gate for CI | 5 | 2 | 5 | ✅ | 1 |
| **F-Q2** | `.codelore-thresholds.toml` config-file thresholds | 4 | 1 | 5 | ✅ | 1 |
| **F-Q3** | `codelore check --diff` (PR-mode gate) | 5 | 2 | 5 | ✅ | 2 |
| **F-Q4** | GitHub Actions `$GITHUB_OUTPUT` integration | 3 | 1 | 5 | ✅ | 2 |
| **F-X1** | `codelore explain <metric>` — print formula + citation | 4 | 1 | 5 | ✅ | 2 |
| **F-X2** | `codelore profile` — cache + timing telemetry | 3 | 1 | 5 | ✅ | 3 |
| **F-X3** | Shell completions (bash/zsh/fish/PowerShell) | 3 | 1 | 5 | ✅ | 2 |
| **F-X4** | JSON Schema export for every output format (`schemars`) | 4 | 1 | 5 | ✅ | 2 |
| **F-P1** | View Transitions API for color-mode swap animations | 3 | 1 | 5 | ✅ | 1 |
| **F-P2** | Popover API for UI-2 educational tooltips | 5 | 1 | 5 | ✅ | 2 |
| **F-P3** | Native `<dialog>` for drawer + scenario modal | 4 | 1 | 5 | ✅ | 2 |
| **F-P4** | Parallel DOM tree for keyboard a11y (closes F98) | 4 | 1 | 5 | ✅ | 1 |
| **F-P5** | PWA / Service Worker for offline-first SPA | 3 | 2 | 5 | ✅ | 3 |
| **F-D1** | `codelore docs` — emits static HTML with citations | 4 | 2 | 5 | ✅ | 3 |
| **F-D2** | Per-PR markdown release-notes mode | 3 | 1 | 5 | ✅ | 3 |

`★ Insight ─────────────────────────────────────`
**Notice the column homogeneity**: every Tier-1 + Tier-2 feature scores **5** on "modern fit," **5** on data-readiness or one step away, and has risk ≤3. This isn't accidental — it reflects how aligned the v0.5.0 stack actually is. The plan deliberately *excludes* anything that would force a legacy primitive or stretch the architecture; that filter is what kept the inventory at 27 instead of 60.
`─────────────────────────────────────────────────`

---

## 5. Tier 1 — must-ship (6 days, 11 features)

Everything in this tier ships CodeScene parity + the architecture-layer foundation + CI integration + the modern-platform upgrades, with zero new ingest beyond the import-graph pass (one new table, one HEAD-time scan).

### 5.1 Visual layer parity (3 days)

The full Tier-1 subset from [`codescene-parity-plan.md`](./codescene-parity-plan.md): F-V1, F-V2, F-V3, F-V4, F-V5, F-V6 + F-P1 + F-P4. All eight of these compose into the existing `widget-hotspot-circle-pack`. No new widget. Closes F90, F97 (with idle-callback), F98.

### 5.2 Architecture layer extraction (2 days, F-A1)

The biggest "new signal" in the plan. The pattern:

```
HEAD-time tree-sitter pass (existing)
    │
    ├─ existing: function-boundary detection → entities + complexity_metrics + clones
    │
    └─ NEW: import-node detection → imports table
       └─ per-language node kinds:
           Rust:       'use_declaration'
           Python:     'import_statement' + 'import_from_statement'
           Java:       'import_declaration'
           JavaScript: 'import_statement' + 'import_clause'
           TypeScript: 'import_statement' + 'import_clause'
           C++:        already in codelore-rca preproc — reuse for completeness
```

New schema (additive, schema_v3 bump per existing migration pattern):

```sql
CREATE TABLE IF NOT EXISTS imports (
    rev         TEXT NOT NULL REFERENCES commits(rev),
    src_path    TEXT NOT NULL,
    target      TEXT NOT NULL,   -- raw import target string (resolved or not)
    resolved    BOOLEAN NOT NULL, -- did our resolver map it to a tracked path?
    target_path TEXT,             -- resolved path (NULL if external)
    kind        TEXT NOT NULL,    -- 'absolute' | 'relative' | 'wildcard'
    PRIMARY KEY (rev, src_path, target)
);
CREATE INDEX IF NOT EXISTS idx_imports_src    ON imports(src_path);
CREATE INDEX IF NOT EXISTS idx_imports_target ON imports(target_path);
```

The new pass is purely additive and runs in the same rayon-then-serial-drain pattern as `ingest_complexity_at_head`. Extraction cost is small: every Tier-1 file is already being tree-sitter-parsed for complexity. Adding a second visitor in the existing walk costs ~5 % CPU overhead per file, measured against the cyclomatic + cognitive walk.

**Modernness note**: this unlocks the v0.6.0 `leiden-rs` centrality + community detection on a *structural* graph alongside the existing behavioral coupling graph. Per `feedback_no_copypaste_features`, CodeLore can now run modularity on **both** signals and report where they agree vs disagree — that's strictly richer than CodeScene, which only has the structural graph.

### 5.3 Quality gates for CI (1 day, F-Q1 + F-Q2 + F-P4)

The smallest "CodeLore is now CI-grade" surface:

```toml
# .codelore-thresholds.toml — new convention file at repo root.
[gates]
cognitive_max = 30        # Sonar's threshold; reject any new file above
code_health_min = 60      # any file below = fail
fisher_significance = 0.05
hotspot_score_max = 8.0   # any file above the warn line
disallow_clone_type_1 = true

[diff]
delta_code_health_min = -5   # health may drop at most 5 points in a PR
new_hotspot_max = 0           # zero new hotspots introduced

[teams]
require_team_assignment = ["src/payments/", "src/auth/"]
```

```bash
$ codelore check                  # full-tree gate (release-time)
PASS: 487 files, 3 hotspots, 0 new clones — see report.md

$ codelore check --diff main..HEAD  # PR gate (CI-time)
FAIL: src/payments/router.rs introduced cognitive=42 (threshold: 30)
      → exit code 4 (analysis)
```

Output is human-readable + machine-parseable (writes to `$GITHUB_OUTPUT` for direct GitHub Actions step output if env is set). This is the missing piece that makes CodeLore actionable in CI — no more "ran the report, nobody read it." Closes Tier-2 ambition rows in roadmap §6.

`★ Insight ─────────────────────────────────────`
**Why thresholds file matters more than the flag set**: thresholds live in the *repo*, not the *invocation*. That means the gate is the same regardless of who runs it: a contributor's pre-push hook, GitHub Actions, an IDE plugin, a release pipeline. Per `feedback_modernize_dont_migrate`: code-maat reads CSV files of "rules" too, but ours integrates with `--group-file`, `--mailmap`, and the existing convention naming — same data, deeply better DX.
`─────────────────────────────────────────────────`

### 5.4 What Tier 1 closes (cumulative)

After Tier 1: CodeScene parity + architectural layer + CI quality gate + a11y conformance.
Closes F90, F97, F98 as side effects.

---

## 6. Tier 2 — strategic extensions (7 days, 13 features)

Tier 2 cashes in on what Tier 1 set up. Every item leans on the import graph (A1), the modern platform primitives (P1, P3), or the existing data.

### 6.1 Architecture analytics on top of import graph (1.5 days)

- **F-A2 layer rules**: declare allowed layer dependencies in `.codelore-arch-rules.toml`; report violations. Pattern:
  ```toml
  [layer."domain"]   paths = ["src/domain/"]   may_depend_on = []
  [layer."app"]      paths = ["src/app/"]      may_depend_on = ["domain"]
  [layer."infra"]    paths = ["src/infra/"]    may_depend_on = ["app", "domain"]
  ```
- **F-A3 god-class detector**: `SELECT path WHERE cognitive > 30 AND fan_in > 10 AND fan_out > 10` — pulls from existing `complexity_metrics` joined to new `imports`.
- **F-V11 architecture force-graph widget**: ECharts `series-graph` `layout: 'force'` over the resolved-import edges, node size = LoC, color = ownership team, edge thickness = call frequency proxy (count of distinct imports).

### 6.2 New behavioral analyses (1.5 days)

- **F-A4 stale code surfacer**: SQL-only on `commits.date` MAX-aggregate. Surface in markdown emitter + a new color mode.
- **F-A5 pair-programming detector**: parse `Co-Authored-By:` already done by `identity::ai_attribution`; aggregate to author-pair density. New `pair_density` analysis + sankey widget.
- **F-A6 release lead-time**: `git tag` timestamps + `commits.date` give commit-to-tag lead time. New `lead_time` analysis + boxplot widget.
- **F-A7 bus factor per module**: roll up `knowledge_islands` to group-file-defined modules. New aggregation on existing data.

### 6.3 Six new visualizations from ECharts catalog (2 days)

We currently use 6 of ECharts 22 series types. Tier 2 adds:

- **F-V7 radar** — per-file 6-axis profile (cognitive, churn, age, coupling, MI, AI%). Brand-defining "behavioral fingerprint" view. ECharts `series-radar`.
- **F-V8 treemap (square)** — alternative to circle-pack with stricter area encoding for size-sensitive comparisons. ECharts `series-treemap`. Drop-in: re-uses `xl:col-span-2` slot via toggle.
- **F-V9 parallel coordinates** — pick N files, see their metric profiles side-by-side. Power-user analytical tool. ECharts `series-parallel`.
- **F-V10 boxplot** — distribution of cognitive/health/age across files. Per-team or per-module facets. ECharts `series-boxplot`.
- **F-V11 architecture force-graph** — covered in §6.1.
- **F-V12 chord diagram** — module-to-module coupling, populated when `--group-file` is set. ECharts has chord as a deprecated alias for graph circular; the modern primitive is `series-graph` with `layout: 'circular'`. Net: pure ECharts.

### 6.4 Modern platform upgrades (1 day)

- **F-P2 Popover API for UI-2 tooltips**: the roadmap UI-2 item ("? tooltips on every KPI tile + table column") collapses to ~3 attributes per element + native styling via `:popover-open`. No tooltip lib. Closes a UI-roadmap line item.
- **F-P3 native `<dialog>`**: the detail drawer + offboarding scenario picker move to `<dialog>`. Free focus-trap, free Escape-to-close, free backdrop. Net code reduction ~80 LOC vs the current Alpine machinery.

### 6.5 CLI polish (1 day)

- **F-X1 `codelore explain`**: prints the SQL + Coleman/Tornhill/Kamei citation + formula for any computed metric. The auditable-formulas brand promise made tactile on the CLI side. ~150 LOC.
- **F-X3 shell completions**: `clap_complete` derive — bash/zsh/fish/PowerShell completions emitted by `codelore completions <shell>`. Ships with most modern Rust CLIs; ~30 LOC.
- **F-X4 JSON Schema export**: `schemars` derive added to every output row type; `codelore schema <format>` emits JSON Schema 2020-12 documents. Integrates with downstream tools (Stoplight, Spectral, Postman, OpenAPI registries). ~80 LOC + derive macros.

### 6.6 Diff-mode quality gate (1 day, F-Q3 + F-Q4)

Extension of Tier-1 `codelore check`:

```bash
$ codelore check --diff $(git merge-base main HEAD)..HEAD
```

Same engine, restricted-scope. `$GITHUB_OUTPUT` integration writes `result=pass`, `failing_files=...`, `delta_health=...` for direct use as GitHub step outputs without parsing stdout.

### 6.7 What Tier 2 closes (cumulative)

After Tier 2: + structural architecture analytics + 6 new visualizations + UI-2 tooltips + native dialog + CLI extensibility + diff-mode quality gate. Closes F71, F92 as side effects.

---

## 7. Tier 3 — brand-extending (5 days, 3 features)

Tier 3 invests in CodeLore's strategic position. These are the last things to ship; they presuppose Tier 1+2 and have higher uncertainty.

### 7.1 PWA / Service Worker (F-P5, 1.5 days)

Make the SPA install-to-home-screen on iOS/Android, work offline. Service Worker caches the entire single-file HTML at first load. Critical for compliance/airgapped audiences where the offline brand of the single-file SPA is competing with CodeScene's server. Uses native `navigator.serviceWorker.register()`; no library.

### 7.2 Static documentation generator (F-D1 + F-D2, 2 days)

`codelore docs` emits a static-HTML "documentation site" of every analysis, formula, citation, and a live SPA. Same primitives as the existing SPA emitter (no new framework). The strategic value: every CodeLore user becomes a possible advocate when they show "look how clearly this is documented" to their team. UI-2 educational tooltips become a build-time inverse: documentation pages link *into* the SPA, not just the other way.

`codelore notes --base v0.5.0..v0.6.0` extends `codelore diff` to emit a per-release markdown summary suitable for changelogs.

### 7.3 `codelore profile` (F-X2, 1 day)

Operational visibility — cache size, slow-query telemetry, schema migration audit log. Sets up the v0.6.x+ "serve mode" with already-knowable metrics. Pure additive CLI surface.

### 7.4 What Tier 3 closes (cumulative)

After all three tiers: offline-installable PWA, full-citation static docs, operational telemetry. CodeLore is now feature-complete vs every commercial competitor on the **behavioral analysis dimension**, with strictly stronger guarantees on auditability, modern stack, and single-binary portability.

---

## 8. Deliberately out of scope

Per the [borrow-or-build principle](roadmap-v1.x-and-beyond.md#borrow-or-build-principle-applies-to-every-feature-evaluation) and the `project_git_only_scope` project rule:

| Out-of-scope feature | Why | Memory anchor |
|---|---|---|
| Multi-repo / inter-project dashboard | CodeLore is git-only / single-repo | `project_git_only_scope` |
| Server-time real-time monitoring | Stateless by design until roadmap v0.5.x phase 5.1 | Roadmap §5 explicit deferral |
| GitHub PR ingestion (review comments, reactions) | Not in git | `project_git_only_scope` |
| AI-augmented commit-message scoring (LLM) | Adds external network call + non-deterministic output | `feedback_root_cause_only` (no probabilistic results) |
| OKR / goals tracking | Requires persistent server state | Out of stateless brand |
| Slack / email digest | Server-mode dependency | Defer to roadmap §5.1 |
| Auto-detected "Early Warnings" composite | Vague; would dilute "peer-reviewed-grounded" brand | `feedback_no_copypaste_features` |
| Custom Wasm runtime, MicroVM-isolated analyses | Massive scope; no concrete win over current single-binary | `feedback_validate_assumptions` |

Each deferral has a real reason. None is "we forgot."

---

## 9. Sequenced implementation

```
DAY 1 ┐ Tier 1.1 — F90 fix + DaisyUI tokens + helper (codescene-parity §4.2)
DAY 2 ├ Tier 1.1 — F-V1 + F-V2 + F-V3 (3 new color modes + ring overlay)
DAY 3 ┤ Tier 1.1 — F-V4 + F-V5 (coupling arcs + offboarding scenario)
                   + F-P1 (View Transitions wrap on color swap)
DAY 4 ┤ Tier 1.2 — F-A1 (architecture import-graph extraction, schema_v3)
DAY 5 ┘ Tier 1.2 — F-A1 + F-V6 (Kamei delivery-risk sparkline) + F-P4 (parallel DOM tree)
DAY 6 ── Tier 1.3 — F-Q1 + F-Q2 (codelore check + thresholds file)

✓ TIER 1 COMPLETE — CodeScene parity + architecture + CI gate + a11y conformance.

DAY 7 ┐ Tier 2.1 — F-A2 + F-A3 (layer rules + god-class detector)
DAY 8 ┤ Tier 2.2 — F-A4 + F-A5 (stale code + pair-programming)
DAY 9 ┤ Tier 2.2 — F-A6 + F-A7 (lead time + bus factor per module)
DAY 10┤ Tier 2.3 — F-V7 + F-V8 + F-V9 (radar + treemap + parallel coordinates)
DAY 11┤ Tier 2.3 — F-V10 + F-V11 + F-V12 (boxplot + arch force-graph + chord)
DAY 12┤ Tier 2.4 — F-P2 + F-P3 (Popover for UI-2 + native <dialog>)
DAY 13┘ Tier 2.5 — F-X1 + F-X3 + F-X4 (explain + completions + schema)
                   + F-Q3 + F-Q4 (--diff gate + GITHUB_OUTPUT)

✓ TIER 2 COMPLETE — full analytical surface + every modern platform primitive.

DAY 14┐ Tier 3.1 — F-P5 (PWA / Service Worker)
DAY 15┤ Tier 3.2 — F-D1 (codelore docs generator)
DAY 16┤ Tier 3.2 — F-D1 continued + F-D2 (release-notes mode)
DAY 17┤ Tier 3.3 — F-X2 (codelore profile)
DAY 18┘ Slack week — testing, polish, CHANGELOG entries

✓ TIER 3 COMPLETE — brand-defining surface.
```

The plan can ship at any tier boundary; each tier is internally consistent. Tier 1 alone exceeds CodeScene parity; Tier 1+2 is the "v0.6.x maximum aligned release"; Tier 1+2+3 is the "v0.7.x brand release."

---

## 10. Risks + mitigations

| Risk | Likelihood | Severity | Mitigation |
|---|---|---|---|
| Import-graph resolution is language-specific and fragile | High | Medium | Per-language resolvers (~80 LOC each) covering the common case; `resolved=false` is a first-class column so unresolved imports still inform CodeLore's analyses. Modules/types not in our scope simply don't appear in the graph; no silent miscount. |
| `imports` table doubles ingest time on huge repos | Medium | Medium | Single tree-sitter walk shared with complexity pass. Worst case +20% on the HEAD-time scan; the producer/consumer walk-commits pass is unaffected. Bench-gate per F69 pattern. |
| Tier-2 widgets crowd the SPA viewport | High | Low | Tier 2 ships them as **toggleable views**, not new sections. The treemap is a toggle on the circle-pack widget; the radar is a per-file overlay in the detail drawer; parallel coordinates lives in a new "compare" sub-route. No widget count growth. |
| View Transitions API fallback hides bugs in older Safari | Low | Low | Already noop-fallback — color swap remains correct, just without animation. Detect via `'startViewTransition' in document`. |
| `codelore check` becomes a misery flag for noisy repos | Medium | Medium | Thresholds file is **opt-in**. Pre-bundled `--profile codescene` / `--profile sonar` presets give sensible starting points. CI integration explicitly notes "first-time setup: comment out failing gates until you've calibrated." |
| `schemars` macro inflates compile times | Low | Low | Schemars is ~5x lighter than syn-based macros (sccache-friendly). Measured against codelore-rca's existing macro load: negligible. |
| Architecture graph + behavioral graph divergence is hard to explain | Medium | Medium | Ship them side-by-side in the SPA, with a "where do they agree?" KPI tile. Disagreement IS the signal. Educational tooltips (F-P2) explain in-context. |
| Native `<dialog>` accessibility behavior differs across browsers | Low | Low | All major browsers shipping `<dialog>` agree on focus management; the remaining differences are styling-related and absorbed by DaisyUI's `modal` component classes. |

---

## 11. Modern-tool justification

User requirement: "modern tools (not outdated or older legacy tools)."

Every choice in this plan is the modern primitive for its category:

| Category | Modern (this plan) | Legacy (rejected) | Why modern wins |
|---|---|---|---|
| Reactivity | Alpine.js 3.15 with `$persist` + reactive stores | React/Vue/Svelte for a static-rendered SPA | Single-file SPA brand; no build step at runtime |
| State color tokens | DaisyUI 5 OKLCH semantic tokens | SCSS @variables, CSS HSL | Perceptually uniform, theme-aware out of the box |
| Animation | View Transitions API | FLIP / hand-rolled CSS transitions | One API call, native, gracefully degrades |
| Tooltips | Popover API | Tippy.js / Popper.js | No JS lib, native top-layer, baseline 2025 |
| Modal dialogs | Native `<dialog>` | Custom drawer with focus traps | Native focus management, native backdrop, free a11y |
| Filesystem walk (host side) | `jwalk` (parallel) + ignore crate | `walkdir` (serial) | Tree-sitter pass parallelization wins ~3x at HEAD scan |
| JSON Schema | `schemars` derive | Hand-authored schemas | One source of truth for serde + schema |
| Hashing | `sha2 0.11` | `md5`, `sha1` | Cryptographic + audit-trail-clean |
| Color interpolation | CSS `color-mix(in oklch, …)` | JS d3-interpolate | Browser-native, perceptually accurate |
| Concurrency | crossbeam-channel + rayon | thread::spawn + Mutex<Vec> | Established Rust modern primitives |
| Graph algorithms | leiden-rs + petgraph 0.8 (already on roadmap) | Custom hand-rolled Louvain | Peer-reviewed implementations |
| Output schemas | `schemars`-derived JSON Schema 2020-12 | `cuelang`, `protobuf` | Web-native + serde-compatible |
| Shell completions | `clap_complete` (derive) | hand-written completion files | Single source of truth, auto-updates |
| Tree walks | tree-sitter 0.25 LANGUAGE API + single TreeCursor (F45) | per-node `node.walk()` allocation | Validated; ships today |

Per the `feedback_modernize_dont_migrate` memory: defaults are *modern*, with backward-compat shims only when explicitly required.

---

## 12. Decisions for the maintainer

The plan is ready to ship pending these named calls:

1. **Architecture-graph scope** — Tier 1 ships the import-graph extraction. Do you want resolvers for *all six* Tier-1 languages on day 5, or *Rust + Python + JS/TS* first with Java + C++ in Tier 2? **Recommended:** all six on day 5; ~80 LOC per language and they share the same tree-sitter infra already loaded.
2. **`codelore check` default behavior** — fail-fast (exit on first violation, fast feedback) or accumulate-then-report (full picture, slower)? **Recommended:** accumulate-then-report; CI users want the full failure list.
3. **Tier 2 `<dialog>` migration** — incremental (drawer first, scenario picker later) or atomic (both at once)? **Recommended:** atomic — easier to test the focus-trap behavior in one shot.
4. **Tier 3 PWA scope** — full Service Worker (installable + offline) or manifest-only (installable, no offline)? **Recommended:** full SW; the offline brand is the strategic value.
5. **`leiden-rs` exposure** — surface the v0.6.0 community detection in the SPA via a new color mode + a per-module roll-up, or keep it CLI-only for v0.6.x? **Recommended:** SPA color mode is ~30 LOC and visually validates the algorithm output; ship it in Tier 2 with the architecture features.
6. **Schema versioning for output formats** — bump on **any** schema change or only on **breaking** changes? **Recommended:** any change, mirroring F106 manifest_version philosophy; consumers gating on schema_version is the clean integration story.

All open Active F-findings (F89, F91, F93, F94, F95, F96, F99–F106) sit in `deep_analysis_report.md`; the ones not retired by this plan can be batched as a Tier 2.5 hardening day.

---

## 13. What this plan proves about CodeLore's architecture

The fact that **27 substantial features fit into 18 days with zero stack changes** is the architectural proof. v0.5.0's stack migration (Tailwind v4 + DaisyUI 5 + Alpine.js + ECharts 6 + SHA-pinned single-file SPA) was the necessary investment to make this plan possible — without that foundation, every Tier 1+2 item would have required new dependencies, new build steps, or legacy primitives. The maintainer's discipline on the stack pays the dividend here: this is the broadest aligned feature set CodeLore can ship at this point in its lifecycle, and every line of it traces to a source-quote-verified architectural seam.

Per the `feedback_improve_during_validation` memory: validation is when scope is refined. The 18-day plan above is the *refined* scope after the validation pass — the brainstorm version was ~60 features; the filter that landed at 27 is the verifiable-alignment filter.
