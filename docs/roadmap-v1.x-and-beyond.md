# CodeLore — Roadmap (post-`0.1.0` and beyond)

**Status:** living document. Last updated 2026-06-11.


This doc is the prioritized backlog of *everything* proposed after the `0.1.0` tag. Each row links to a plan document when one exists.

> The "v1.x" in this file's path is a legacy artifact from when the first stable was planned as `1.0.0`; the project ultimately collapsed the alpha→beta→rc ladder and shipped `0.1.0` as the first stable. See [`RELEASING.md`](RELEASING.md) for the versioning policy.

## Decision rubric

Items are ranked by **leverage × risk**:
- **Leverage**: does this change make users more successful or open new use cases?
- **Risk**: implementation difficulty × blast radius of getting it wrong
- **Strategic**: does this differentiate CodeLore against code-maat / CodeScene / jscpd / CHM.

### Borrow-or-build principle (applies to every feature evaluation)

When evaluating a feature surfaced by another tool (code-maat, CodeScene,
SonarQube, CHM / code-health-meter, jscpd, etc.):

- **Never copy-paste.** No direct port of foreign idioms / data shapes.
- **Identify the signal**, not the implementation. The signal is the
  end-user value; the implementation must be re-derived against
  CodeLore's stack (Rust + DuckDB + tree-sitter + behavioral git
  history).
- **If we already have the data**, the work is SQL aggregation +
  emitter exposure, *not* new ingest. Latent-value surfacing beats
  bolt-on plumbing.
- **If we have a richer signal** (behavioral > static, polyglot > JS-only,
  AST > text, persistent > snapshot), adapt the algorithm to the
  richer signal. "Same algorithm, better feature."
- **Cite the research foundation** in `docs/research-foundations.md`.
  Brand promise: every metric is peer-reviewed-grounded.

This rule applies retroactively too — past code-maat borrowing was done
on this basis ([feedback memory: modernize-dont-migrate](../../.claude/memory/feedback_modernize_dont_migrate.md))
and the CHM borrow analysis (v0.4.5 / v0.5.x) is the most recent
application.

## Active plan (v0.4.5 → v0.6.x)

The shipped baseline is **v0.4.4** (released 2026-06-11). Current state:

- 23 analyses + `codelore diff` PR-mode
- SPA emitter behind `spa` Cargo feature (6-widget MVP shipped v0.4.0; W7-W11 widgets shipped v0.4.2; success message + dispose-on-rerender shipped v0.4.3; arg_max/first SQL planner sweep shipped v0.4.4)
- DuckDB persistent cache, mailmap, AI attribution, `.gitignore`-aware exclude defaults
- 5-binary release pipeline (macOS-arm64 / macOS-x86_64 / linux-x86_64-gnu / linux-arm64-gnu / windows-x86_64) + Homebrew + ghcr container
- ~520 tests across lib + CLI + differential + integration

### v0.4.5 — SQL planner finishing + cross-stack UI + first CHM-borrows

**Scope ceiling**: backend perf cleanup + cross-stack AI/MI surfacing + UI roadmap holdovers + the *latent-value* CHM borrows (data already in the fact store, never queried). One week of focused work.

Source-of-truth tracker: `docs/reports/deep_analysis_report.md` (F-findings) and `docs/ui-roadmap.md` §3b (UI items).

| ID | Scope | Adaptation principle | Effort |
|---|---|---|---|
| **F67** | `coupling.rs`: `filtered_changes` CTE forces deterministic pre-filter (good_commits ⨝ {src}) before the self-join. | Aligns with v0.4.4's hash-aggregation rewrite pattern. | ~half day |
| **F68** | Cross-stack AI attribution: `HotspotRow.ai_pct` column + hotspots SQL `COUNT(CASE WHEN ai_attribution IN ('ai-assisted','ai-authored') THEN 1 END) / COUNT(*)` + SpaDashboard JSON + widgets.js color band. Activates the AI Attribution toggle that's currently a placeholder. | Already-collected `commits.ai_attribution` becomes a queryable per-file signal. Latent-value surfacing. | ~1 day |
| **F71** | `widgets.js`: replace `window.addEventListener('resize', …)` with per-container `ResizeObserver`. F64 fixed chart disposal but left the resize listener leaking; ResizeObserver auto-cleans when target node detaches. | Modern browser API (ResizeObserver is everywhere we support). Avoids manual `removeEventListener` bookkeeping. | ~half day |
| **UI-1** | `--embed` flag strips full-page shell for `$GITHUB_STEP_SUMMARY` and SARIF code-scanning embed contexts. | Builds on the existing `include_str!` template; conditional shell. | ~half day |
| **UI-2** | `?` tooltips on every KPI tile + table column. Tooltip text = SQL query + link to `docs/research-foundations.md`. **Augmented with CHM A4 borrow**: per-metric educational description text (title, range, interpretation) joins the existing provenance manifest payload, surfaced inline. | Brand-defining: "auditable formulas" promise made visual. | ~1 day |
| **UI-3** | X-Ray sunburst entities pre-filtered to live-at-HEAD via the same `arg_max(change_type, ROW(date, -rowid))` pattern from F63. | Reuses v0.4.4 hash-aggregation primitive. | ~half day |
| **CHM-A1** | Surface CodeLore's *already-computed polyglot MI* (`complexity_metrics.mi` via `mi_sei()`) on hotspots, KPI tiles, code-health. | We've computed it via Mozilla rust-code-analysis since v0.1.0; never queried. Pure SQL + emitter exposure. We're 8+ languages vs CHM's JS/TS-only — strictly richer. | ~half day |
| **CHM-A2** | MI band buckets: `MiBand { Low (<65), Moderate (65-85), High (≥85) }` Rust enum + SQL `CASE WHEN` + KPI tile rollup ("234 files high, 41 moderate, 9 low"). | Industry-consensus thresholds (Coleman 1994 / SEI / SonarQube / CHM converge). Our typed enum + SQL idiom; not CHM's `Matcher()` chain. | ~50 LOC |
| **CHM-A3** | Behavioral coupling graph **density** scalar on KPI tiles. `edges / (n·(n-1)/2)` over the Fisher-significant pairs. | CHM does density on static dep graph (JS-only); we do it on the behavioral graph — different signal. | ~30 LOC |
| **CHM-A5** | Cite Coleman 1994 (MI), Ben Khalfallah 2025 TOSEM (CHM), Newman 2008 (modularity, for forward reference) in `docs/research-foundations.md`. | Brand promise: every metric peer-reviewed-grounded. | docs only |
| **F60** | _Deferred to v0.4.6 hotfix._ `parse_git_log_stream` needs incremental parser rewrite (two-record lookahead pairs pretty blocks with name-status chunks). Carries forward. | Out of v0.4.5 scope. | — |
| **F69 / F70** | _Conditional._ Run `EXPLAIN ANALYZE` benchmark on a 100k-commit fixture; if measurable win, fold into v0.4.5. If not, defer or close. | Gated on data, not speculation. | TBD |

**Cumulative net change**: ~3 days work + ~1 day testing. No new dependencies. No new ingest. The biggest change is F68 (cross-stack AI rollup) which touches Rust → SQL → JSON → JS.

**Goal-backward gate**: A v0.4.5 dashboard on the CodeLore repo itself should surface (1) MI band breakdown on the KPI tiles, (2) AI Attribution toggle that *works*, (3) `?` tooltips on every metric showing the SQL formula + Coleman/Ben Khalfallah/Blondel citations, (4) coupling density score. If any of those are missing or placeholder-state, the release isn't ready.

### v0.5.x — `codelore serve` (interactive mode) + the big CHM borrow

**Scope ceiling**: this is where vanilla JS starts to hurt. Cross-widget
filter state, threshold sliders, live SQL exploration — they all want
reactivity that pure JS makes painful. Framework decision (Alpine.js,
~15 KB, SHA-pinned via the same `build.rs` pattern as ECharts) was
validated in earlier UI roadmap research; locked in for this boundary.

The Tauri 2 desktop wrap is the *next* step after this lands.

| Phase | Item | Adaptation principle | Effort |
|---|---|---|---|
| **5.1** | Axum-based `codelore serve --port 4242` — wraps the existing fact-store query layer as REST endpoints. Reuses the SPA frontend; same emitter, served live. | Pure plumbing; no algorithm change. | ~1 week |
| **5.2** | Cross-widget filter state via **Alpine.js** (SHA-pinned via `build.rs`). Filter on hotspots table → highlight matching circles → restrict knowledge-islands → restrict sankey. | Locked at the v0.5 boundary per `docs/ui-roadmap.md` §3c. | ~1 week |
| **5.3** | Threshold sliders re-run analyses on the server with debouncing. Surfaces the deterministic-formula brand promise interactively. | Leverages the persistent DuckDB cache; no new analysis code. | ~3 days |
| **5.4** | Live SQL exploration (`--query` escape hatch with a UI). Power-user feature; audit-trail brand. | Already partly in Tier 1 (`--query SQL`). Wires into the serve UI. | ~3 days |
| **5.5** | `codelore diff` PR-mode UI — same Axum + SPA frontend, diff-anchored. | Reuses the existing `codelore diff` subcommand. | ~3 days |
| **CHM-B1** | **Louvain modularity on the behavioral coupling graph** — NOT on a static dep graph. New analysis `run_behavioral_modules` over the Fisher-significant coupling output (run_coupling). Pure Rust implementation (Newman 2008 + Blondel et al. 2008), ~200 LOC; no `graphology`-equivalent dep pulled in. Output: per-file community ID + global Q score. Surface: new analysis row type + KPI tile + sankey node coloring by community + drawer "behavioral module" section. | **Genuinely different signal** vs CHM (they Louvain on static imports; we Louvain on git-co-change). Same algorithm, philosophically different feature. | ~3 days |
| **CHM-B2** | Promote `coupling_centrality_v1` from internal TEMP TABLE to first-class analysis output. Adds degree/in-degree/out-degree as queryable columns on a `centrality` analysis. | CHM has degree centrality on static graph; we expose it on behavioral graph — richer signal. Already 90% there; just promotes the existing computation. | ~half day |
| **Tier-1 holdovers picked up here** | `--query SQL` escape hatch (was Tier 1), `codelore-action@v1` reusable GHA (was Tier 3), GitHub App auto-PR comments (was Tier 3). Bundle naturally with `codelore serve`. | Architectural fit: all are HTTP-shaped clients of the same Axum routes. | varies |

**Goal-backward gate**: a v0.5 user clicks a file in the SPA → drawer shows "Behavioral module: M3 (Q=0.42)" → clicks the module ID → sankey re-highlights with that community's edges → threshold slider changes the Fisher cutoff → all four widgets re-render in <500ms. If interactivity is laggy or the modularity score is not citable to Newman+Blondel, the release isn't ready.

### v0.6.x — Tauri 2 desktop wrap

**Scope ceiling**: reuse the v0.5.x frontend; Tauri 2 wraps the Axum
server + SPA frontend into a native desktop app. Validated as the
right desktop story in earlier session research:

- Tauri 2 installer size: <5 MB (vs Electron's 300+ MB)
- `codelore-lib` links directly into `src-tauri/` — no shell-out
- Native filesystem: drag-drop a folder onto the window
- Signed cross-platform installers (`.dmg` / `.msi` / `.AppImage`) via Tauri's bundler
- `duckdb` Rust crate works inside Tauri's Rust backend (Duckling reference confirms)

**Adaptation principle**: this is the same SPA, just locally-installed.
No new metrics, no new analyses; the value-add is the discovery surface
("install CodeLore.app, drag your repo onto it"). Skipping it would
cap reach at CLI users.

**v0.6.x effort estimate**: 4-6 weeks on top of v0.5.x. See
`docs/ui-roadmap.md` §3d.

---

### ✅ Shipped under `0.1.0`

The original "Tier 1" (release readiness) and "Tier 2" (v1.x differentiators) lists have all landed in the first stable cut and are no longer roadmap items. The full set, with the commits that delivered them, is preserved in `CHANGELOG.md`'s `[0.1.0]` entry. Headline shipped items:

- Persistent fact-store cache (XDG-style, LRU-evicted) — 100×+ speedup on repeat runs
- Parallel complexity extraction via Rayon — 3–5× wall-time speedup on cold runs
- `clone-coupling` intersection (the strategic differentiator with the `CODELORE-LIVE-CLONE` SARIF rule)
- `codelore diff <base>..<head>` PR-mode subcommand — 4 output formats, `--fail-on` quality gate
- All 21 code-maat-parity analyses + 4 SARIF rules
- 6 verified correctness fixes (R1, R2, R3, R4, R6, R12 — negative hotspot scores, GixRepo date/merge filters, `--after`/`--before`/`--include-merges` CLI surface, Kamei O(N²) → hash-joined UPDATE, Parquet schema completion)

### Tier 1 — strategic differentiators (forward-looking)
Strategic features for upcoming minors. Promote when there's measured user pull.

| Item | Why | Plan | Status |
|---|---|---|---|
| PGO campaign + release pipeline rebuild | Spec §6.5; 5–15% perf headroom on real workloads | future | pending |
| Type 3 near-miss clones (MinHash + LSH @ Jaccard ≥ 0.8) | Plan 7 §2 Task 4; ~100 LOC; catches "renamed + restructured" code | future | pending |
| **Bus-factor / knowledge-island detector** (hotspots × single-owner × departed-author) | Plan 7 research surfaced this; we already have all the data | shipped (v0.3.x) — `run_knowledge_islands` is a first-class analysis with KPI tile + SPA widget | **shipped** |
| **Live-clone × knowledge-loss intersection** (clones inside departed-contributor code) | Engineering-director-level signal nobody else produces | shipped (v0.3.x) — `knowledge_islands.rs` joins departed-author × clones × co-change in one CTE chain | **shipped** |
| Rename tracking via `gix_diff::tree::breaks::detect_renames` | Validation Finding S6. `ChangeType::Renamed { from, similarity }` is captured at ingest by both `GixRepo` and `GitCliRepo`. **Now shipped end-to-end**: the canonical lineage CTE (`facts/ingest.rs::materialize_path_lineage`) joins commit dates to deterministically break recycled-filename cycles; 12 path-aggregating analyses opt into rename-aware aggregation via the `analyses/lineage.rs::rewrite` SQL helper. | shipped (v0.2.x) | **shipped** |
| Bootstrap confidence intervals on hotspot scores | Methodological honesty wedge; CodeScene reports point estimates | future | pending |
| `--query SQL` escape hatch | Spec §5 reserved; power-user feature. **Now folded into v0.5.x serve** (live SQL exploration in the dashboard). | v0.5.x | pending |
| LCOV input → hotspot-weighted coverage | CodeScene shipped this in 2025 | future | pending |
| AI-authorship correlation reports | We tag commits; novel publishable signal. **Partial shipped**: `commits.ai_attribution` captured at ingest, surfaced in `commits` + per-author rollup. **F68 in v0.4.5** wires per-file AI percentage into the SPA's AI Attribution toggle. | v0.4.5 (F68) | **partial → shipped v0.4.5** |
| Survival analysis on hotspots (how long do they stay hot?) | Temporal-extension research | future | pending |
| **MI (Maintainability Index) surfacing** (polyglot via `codelore-rca`) — Coleman 1994 + SEI variant | We already compute `mi_sei()` per function and ingest into `complexity_metrics.mi`; never queried. CHM-A1/A2/A3 in v0.4.5 fix that. | v0.4.5 (CHM-A1/A2/A3) | pending |
| **Behavioral Louvain modularity** (Newman 2008 + Blondel 2008 on our coupling graph) | Novel signal vs CHM (they Louvain on static dep graph; we Louvain on git-co-change). | v0.5.x (CHM-B1) | pending |
| **First-class centrality analysis** (degree / in / out on behavioral coupling graph) | `coupling_centrality_v1` exists internally; promote to a queryable output. | v0.5.x (CHM-B2) | pending |

### Tier 2 — quality and DX (continuous)
Always-on hygiene work; no plan required, weave into other plans.

| Item | Why | Status |
|---|---|---|
| `proptest` on parser + fingerprint walker | Catches edge cases | pending |
| `cargo-mutants` in CI | Hardens test assertion quality | pending |
| `cargo-fuzz` campaign (spec §6.7 → v1.5) | Parser hardening | pending |
| Switch CSV writer to `csv` crate | `output/csv.rs` has 38 hand-rolled `writeln!` calls and zero `csv::Writer`. The `quote_if_needed` helper triggers on `,`, `"`, `\n`, and `\r` (RFC 4180 §2.5 complete — the missing-`\r` bug was closed in the deep-analysis follow-up). Full `csv::Writer` migration was considered and rejected: regenerates 14+ golden snapshots for zero correctness gain (CSV injection is a downstream-Excel concern neither approach addresses). Revisit only if a future emitter needs variable-width records or BOM emission. | rejected (Unreleased) |
| Macro-driven CLI dispatch | Replaces 66-arm `match (format, &analysis)` ladder in `main.rs` (grew from 14 as analyses landed) | pending |
| Builder + validation for `Options` (28 fields, no cross-field checks) | The 4 pathological combinations (`min_coupling_pct > max_coupling_pct`, `clone_similarity_floor` outside `[0, 1]`, `fisher_significance` outside `[0, 1]`, `after > before`) are now rejected at the CLI boundary via `Options::validate()`, called immediately after construction in `codelore-cli::main`. Full builder pattern was rejected as over-architecture (would have forced every callsite to migrate to a new construction path) — `validate()` gives the same coverage with zero callsite churn. Revisit if/when the field-addition-funnel argument starts paying its keep (currently it doesn't). | shipped (Unreleased) — `validate()` |
| Parallelize clone extraction (`populate_clones_at_head`) | Split into serial walk (cheap `WalkDir` + exclude-globset filter) feeding a `rayon::into_par_iter()` phase that reads + tree-sitter-fingerprints each candidate. Fail-fast `extract_functions` error semantics preserved via `collect::<Result<Vec<_>>>`. Mirrors the existing `ingest_complexity_at_head` pattern. | shipped (Unreleased) |
| `gix-write` for test fixtures (5-10× faster than shell-out) | Spec §6 noted gix-write maturing | pending |
| Better error messages at CLI boundary | "find_parent_commit ..." → "shallow clone is missing parent ancestry" | pending |
| Reproducible-build verification in CI | Compare binary hashes across runs | pending |
| Snapshot tests for SARIF / JSON output | Catches silent format drift | pending |
| **CI speedup — `cargo-nextest`** | Drop-in replacement for `cargo test` with ~20-30% faster test-phase execution (smarter scheduling, faster output, better failure aggregation). One-line workflow change. Mainly helps the test-phase wall-clock; doesn't touch the compile-phase dominator. | pending |
| **CI speedup — sccache 0% hit-rate investigation** | `mozilla-actions/sccache-action@v0.0.6` is wired in `ci.yml`, but the v0.1.0 CI run on Windows reported `Cache hits: 0 / Cache misses: 392 / Cache hits rate: 0.00%`. The sccache key is hashing something that changes on every run (likely env-var-derived). Diagnosis + fix could save up to ~5 min off the Windows test job (which is the wall-clock bottleneck). | pending |
| **CI speedup — bundled DuckDB compile dominates** | `libduckdb-sys` with the `bundled` feature compiles ~6000 .cpp files via `cc-rs` from scratch every run (~5-7 min on every OS). The 3 OS jobs already parallel-execute, so wall-clock is bounded by Windows. Three options: (a) keep `bundled` + improve sccache C++ object-cache hit rate (low-risk, medium-payoff), (b) switch to `dynamic` + ship pre-built DuckDB on runners (medium-risk, high-payoff but loses single-binary portability), (c) split a "build DuckDB once, cache the artifact" job that all 3 OS test jobs depend on (medium-risk, high-payoff, no portability loss). | pending |
| **CI speedup — path filters** | `.github/workflows/ci.yml` runs the full matrix on every push, including docs-only changes. Adding `on.push.paths: ['!docs/**', '!*.md']` (or similar) skips the heavy test matrix when only docs change. Modest win in absolute time (~15 min per docs-only push) but meaningful for developer flow. | pending |
| **Re-add `x86_64-unknown-linux-musl` release target** | Dropped from the release matrix for `v0.1.0` because Ubuntu's `musl-tools` package ships `musl-gcc` (C) but not `musl-g++` (C++) — `bca-tree-sitter-preproc`'s `scanner.cc` and bundled DuckDB's ~6000 .cpp files have nowhere to compile to. Two routes: (a) add a musl-targeted build job to `release.yml` that runs inside a `messense/rust-musl-cross`-style Docker image with a full C++ cross-toolchain, or (b) install a `musl-cross-make`-built toolchain into the existing Ubuntu runner. Either gives Alpine users a true static-musl binary. Interim workaround: `cargo install codelore` (links static libgcc) or run the gnu binary under `gcompat`. | pending |
| **`petgraph` 0.6 → 0.8 bump** | Single consumer (`codelore-rca/src/preproc.rs`, ~50 LOC) uses `algo::kosaraju_scc`, `StableGraph`, `Dfs`, `Direction`, `NodeIndex` — each sits on a 0.7 or 0.8 API break line. Real risk is silent: `kosaraju_scc` returns SCCs in implementation-defined order, and the include-graph collapse depends on which component's macros "win" during transitive-include resolution. Fix recipe: (1) add a regression test pinning macro-resolution output for a known include-cycle fixture; (2) bump `petgraph = "0.8"` in `codelore-rca/Cargo.toml`; (3) if the test fails, sort SCC outputs explicitly by path-string before collapse. Currently ignored in `.github/dependabot.yml` to prevent piecemeal bot PRs from landing the silent ordering shift. Half-day focused work. | pending |
| **`tree-sitter*` coordinated grammar sweep** | The Dependabot ignore on `tree-sitter*` blocks single-grammar bumps because `codelore-rca/src/languages/language_*.rs` contains node-ID enum tables generated against specific grammar versions — a bumped grammar still compiles but matches against renumbered AST node IDs, producing silently-wrong complexity metrics. Sweep recipe: (1) bump `tree-sitter` core + every `tree-sitter-<lang>` dep in lockstep; (2) regenerate every `language_*.rs` against the new grammar versions (one file per supported language, ~10 languages); (3) re-run complexity-metric golden fixtures and review any drift. ~1 day focused sprint. Forcing functions worth waiting for: a CVE in a pinned grammar, a grammar bugfix codelore needs, or new-language-support work that requires the newer core. | pending |

### Tier 3 — operational (adoption levers)
Lower priority until `v0.1.0` has measurable real-world traction.

| Item | Why | Status |
|---|---|---|
| `codelore-action@v1` reusable GHA | Path of least resistance for adoption | pending |
| GitHub App for auto-PR comments | Biggest UX win at scale | pending |
| VS Code extension (hotspot gutter markers) | Surfaces findings where devs live | pending |
| Static-HTML report generator (`report.html`) | Web UI is out-of-scope per spec §1.2; a single file is in-scope | pending |
| Container variants: alpine + debian (in addition to distroless) | Different consumers, different tradeoffs | pending |

### Tier 4 — research-flavored / v2+
Long-term work. Listed for completeness.

| Item | Plan / spec reference |
|---|---|
| Pluggable SZZ (start AG-SZZ; allow Neural-SZZ later) | spec §8 |
| Pluggable tangled-commit untangling (SmartCommit pass-through) | spec §8 |
| Salsa-style incremental memoization | spec §6 + Plan 5 design |
| LSP server mode | spec §1.2 (deferred) |
| LLM-based commit classification (pluggable model interface) | spec §8 |
| PDG-based Type 4 semantic clone detection | NP-hard; long horizon |
| Cross-language clone detection (JS↔TS↔Rust shape equivalence) | Plan 7 §2 deferred |
| Knowledge-graph JSON output (for Greptile-style consumers) | spec §8 |
| DORA-adjacent delivery flow metric | spec §8 |
| Code coverage analysis (LCOV input, hotspot-weighted) | spec §8 |

### Tier 5 — community / docs (continuous)

| Item | Why |
|---|---|
| Comparison matrix vs code-maat + CodeScene (measured numbers) | Honest positioning |
| "Anatomy of a hotspot" tutorial | Demystifies methodology |
| Real-world case studies (Rails, Linux, React) | Shows the tool at scale |
| ADRs for major design picks (gix, DuckDB, SARIF, RCA vendor) | Documents the *why* |
| Migration guide from code-maat | Lowers switching cost |
| Glossary (Fractal Value, Code Health, Behavioral SARIF, Kamei vector) | No current single source of truth |

---

## How to use this document

Plan 8 (Tier 1 + Tier 2 of the original roadmap) shipped under `v0.1.0`; what's left is the forward-looking work above. Each new Tier 1 item gets its own plan document under `docs/superpowers/plans/` when scheduled. The scheduling rubric:

- **What user complaint or stakeholder ask does this address?** Build for measured pull, not anticipated need.
- **Does this differentiate CodeLore?** Items in Tier 1 (vs. Tier 2 quality work) should advance the strategic position vs. code-maat / CodeScene / jscpd.
- **Is the risk understood?** Items with "Hard" implementation difficulty (rename tracking, PGO campaign) deserve a design phase before coding.
