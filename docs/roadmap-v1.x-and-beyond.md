# CodeLore — Roadmap

**Status:** living document.

This doc is the prioritized backlog of *everything* still open after the current shipped baseline. Each row links to a plan document when one exists.

> The `v1.x` in this file's path is a legacy artifact from when the first stable was planned as `1.0.0`; the project ultimately collapsed the alpha→beta→rc ladder and shipped its first stable as `0.1.0`. See [`RELEASING.md`](RELEASING.md) for the versioning policy. The path is preserved to avoid breaking cross-doc links.

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
on this basis ([feedback memory: modernize-dont-migrate](../../.claude/memory/feedback_modernize_dont_migrate.md)).

---

## Shipped (current state)

The current state of the codebase. For per-release history see `CHANGELOG.md`.

### Analysis surface

- **The full analysis registry** (enumerated by `AnalysisName::all()` / `codelore --list`) including hotspots, change-coupling, ownership, code-health, clones, live-clones, Kamei JIT-SDP features, knowledge-islands, centrality, behavioural communities (Leiden), god-classes, architecture-violations, stale-code, pair-programming, lead-time, bus-factor, and refactoring-targets
- **MI (Maintainability Index) surfacing** — `mi_sei()` per-function values from `codelore-rca` joined onto hotspots and code-health, banded into Low/Moderate/High via SQL `CASE WHEN` and rolled up on the KPI tiles (Coleman 1994 + SEI variant; polyglot across the languages `codelore-rca` parses)
- **Behavioural-graph centrality** as a first-class analysis (degree / in / out on the Fisher-significant co-change graph) — previously an internal TEMP TABLE, now a queryable output row type
- **Behavioural communities (Leiden)** over the Fisher-significant coupling graph — different signal vs static-import modularity tools (we cluster on git co-change). Output: per-file community ID + global Q score, surfaced on a KPI tile, in the sankey node coloring, and in the drawer's "behavioural module" section
- **Cross-stack AI-attribution** — `commits.ai_attribution` rolled up into a per-file percentage column on hotspots, threaded through the SpaDashboard JSON, and drives the SPA's AI Attribution toggle end-to-end

### CLI surface

- 1 primary subcommand (`codelore analyze`) plus a family of ancillary subcommands: `codelore diff <base>..<head>` (PR-mode, 4 output formats, `--fail-on` quality gate), `codelore check` (quality-gate validation against `.codelore-thresholds.toml`, `$GITHUB_OUTPUT`-integrated), `codelore gate` (agent-loop verdict on uncommitted changes), `codelore explain` (formula + citation for each analysis), `codelore profile` (operational telemetry), `codelore docs` (markdown analysis catalogue), `codelore ingest-sarif` (external-findings sidecar from SARIF documents), `codelore calibrate` (build org-specific reference corpus), `codelore completions <shell>` (bash | zsh | fish | powershell | elvish), `codelore schema <row-type>` (JSON Schema 2020-12), `codelore mcp` (MCP server over stdio)
- Cross-field validation at the CLI boundary via `Options::validate()` — rejects the four pathological combinations (`min_coupling_pct > max_coupling_pct`, `clone_similarity_floor ∉ [0, 1]`, `fisher_significance ∉ [0, 1]`, `after > before`)

### SPA dashboard (`--format spa`)

Single self-contained HTML file behind the `spa` Cargo feature. Tailwind v4 + DaisyUI 5 + Alpine.js 3.15 + Apache ECharts 6.1 + d3-hierarchy, all SHA-pinned at build time via `build.rs`. The detailed widget inventory and stack rationale lives in [`ui-roadmap.md`](./ui-roadmap.md). Highlights:

- **15+ interactive widgets** — KPI tiles, knowledge islands, hotspot circle-pack (7 color modes + ring overlay + coupling arc overlay), hotspot table, parallel DOM tree, change-coupling sankey, trends, calendar heatmap, X-Ray sunburst, Kamei Delivery-Risk Sparkline, treemap, parallel coordinates, boxplot, module chord, architecture force-graph, drawer-radar
- **CodeLore differentiators visible in the dashboard**: auto-detected knowledge islands (departed-author × clones × co-change), AI-attribution toggle, `?` tooltips on every metric linking to SQL + research citation (the "auditable formulas" brand)
- **Off-boarding scenario picker** — DaisyUI multi-select dropdown + `$persist`, runs entirely client-side over the embedded data
- **Modern web platform primitives** — View Transitions API, native `<dialog>`, PWA manifest, OKLCH `color-mix()`, WCAG-conformant parallel DOM tree

### Architecture + ingest

- **Architecture import-graph** (`schema_v3` adds the `imports` table) populated via tree-sitter walks across the Tier-1 languages
- **Per-language path resolvers** for Rust (`crate::`), Python (`.`), JS/TS (`./`), TypeScript declaration types, and the JS/TS resolver pack
- **Layered-architecture rule validation** via `.codelore-arch-rules.toml` — declarative allow/forbid edges, evaluated by the `architecture-violations` analysis
- **Quality gates** via `.codelore-thresholds.toml` consumed by `codelore check` with `$GITHUB_OUTPUT` integration for CI step summaries
- **Persistent DuckDB cache** keyed on `SHA256(canonical_repo_path || head_sha || pkg_version || opts_hash || CACHE_EPOCH)` — `CACHE_EPOCH` is a manually-bumped sentinel that orphans stale cache files (the on-disk schema version is tracked separately)
- **Mailmap + bot patterns + AI-author attribution** at the identity layer; `.gitignore`-aware exclude defaults

### Release pipeline

- 5-binary release matrix — `macOS-arm64`, `macOS-x86_64`, `linux-x86_64-gnu`, `linux-arm64-gnu`, `windows-x86_64`
- Homebrew tap + ghcr container
- `scripts/cut-release.sh` orchestrates the full release-cut including the ruleset-disable/restore dance documented in `docs/RELEASING.md`

---

## Planned (next direction)

Forward-looking work not yet in tree. Items are sorted by tier; each is committed only when there's measured user pull or a clear adoption lever behind it.

### Tier 1 — strategic differentiators

| Item | Why |
|---|---|
| `codelore serve` — interactive Axum web mode wrapping the existing fact-store query layer as REST endpoints; reuses the SPA frontend served live | Cross-widget filter state, threshold sliders, and live SQL exploration all want reactivity that pure static HTML can't deliver. The Alpine stores already in the SPA generalize naturally to the served version. |
| Tauri 2 desktop wrap — reuses the served frontend, `codelore-lib` linked directly into `src-tauri/`; signed cross-platform installers (`.dmg` / `.msi` / `.AppImage`) | Discovery-surface widening ("install CodeLore.app, drag your repo onto it"). Tauri 2 installer is <5 MB vs Electron's 300+ MB; the `duckdb` Rust crate is known to work inside Tauri's Rust backend. |
| `--query SQL` escape hatch | Spec §5 reserved; power-user feature. Natural fit for the served UI. |
| Java FQN resolver pack | The import-graph ingest has a placeholder branch for Java today; a Java-specific resolver mapping `com.foo.Bar` → repo-relative path would complete the polyglot architecture analysis. |
| `schema_v4` author_date column | Current lead-time analysis uses commit date as a proxy for the in-flight wall-clock window. Adding `commits.author_date` alongside `commits.committer_date` would let `lead-time` differentiate authoring vs landing time. |
| Type 3 near-miss clones (MinHash + LSH @ Jaccard ≥ 0.8) | ~100 LOC on top of the existing structural fingerprinting; catches "renamed + restructured" code that Type 1/2 detection misses. |
| Bootstrap confidence intervals on hotspot scores | Methodological-honesty wedge; CodeScene reports point estimates. |
| LCOV input → hotspot-weighted coverage | CodeScene shipped this in 2025. |
| Survival analysis on hotspots (how long do they stay hot?) | Temporal-extension research. |
| PGO campaign + release pipeline rebuild | 5–15% perf headroom on real workloads. |

### Tier 2 — quality and DX (continuous)

Always-on hygiene work; no plan required, weave into other plans.

| Item | Why |
|---|---|
| `proptest` on parser + fingerprint walker | Catches edge cases. |
| `cargo-mutants` in CI | Hardens test-assertion quality. |
| `cargo-fuzz` campaign | Parser hardening. |
| Switch CSV writer to `csv` crate | `output/csv.rs` has 38 hand-rolled `writeln!` calls. The `quote_if_needed` helper covers RFC 4180 §2.5. Full migration regenerates 14+ golden snapshots for no correctness gain — revisit only when an emitter needs variable-width records or BOM emission. |
| Macro-driven CLI dispatch | Replaces the wide `match (format, &analysis)` ladder in `main.rs`. |
| `gix-write` for test fixtures | 5–10× faster than the current shell-out to `git`. |
| Better error messages at CLI boundary | "find_parent_commit ..." → "shallow clone is missing parent ancestry". |
| Reproducible-build verification in CI | Compare binary hashes across runs. |
| Snapshot tests for SARIF / JSON output | Catches silent format drift. |
| `cargo-nextest` adoption | Drop-in replacement for `cargo test` with ~20–30% faster test-phase execution. |
| sccache 0% hit-rate investigation | `mozilla-actions/sccache-action` is wired in `ci.yml` but the key hashes something that changes every run. Diagnosis + fix could save ~5 min off the Windows test job. |
| Bundled-DuckDB compile dominator | `libduckdb-sys` `bundled` compiles ~6000 .cpp files via `cc-rs` every run (~5–7 min). Options: improve sccache C++ object-cache hit rate, switch to `dynamic` + ship pre-built DuckDB on runners, or split a "build DuckDB once, cache the artifact" job. |
| CI path filters | `ci.yml` runs the full matrix on every push, including docs-only changes. `on.push.paths: ['!docs/**', '!*.md']` skips the heavy matrix for docs-only pushes. |
| Re-add `x86_64-unknown-linux-musl` release target | Dropped because Ubuntu's `musl-tools` ships `musl-gcc` but not `musl-g++`; `bca-tree-sitter-preproc`'s `scanner.cc` and bundled DuckDB's .cpp files need a C++ cross-toolchain. Two routes: a musl-targeted build job inside a `messense/rust-musl-cross`-style Docker image, or a `musl-cross-make`-built toolchain on the existing runner. |
| `petgraph` 0.6 → 0.8 bump | Single consumer (`codelore-rca/src/preproc.rs`, ~50 LOC) uses `algo::kosaraju_scc`, `StableGraph`, `Dfs`, `Direction`, `NodeIndex` — each sits on a 0.7 or 0.8 break line. `kosaraju_scc`'s implementation-defined SCC ordering can silently shift macro-resolution. Fix recipe: regression test pinning macro-resolution output → bump → if test fails, sort SCC outputs explicitly by path-string before collapse. Pinned in `dependabot.yml` for this reason. |
| `tree-sitter*` coordinated grammar sweep | Single-grammar bumps either ABI-mismatch at load or silently produce wrong complexity metrics (renumbered node IDs). Sweep recipe: lockstep-bump core + every `tree-sitter-<lang>` → regenerate every `language_*.rs` → re-run complexity golden fixtures. ~1-day focused sprint. |

### Tier 3 — operational (adoption levers)

Lower priority until real-world traction is measured.

| Item | Why |
|---|---|
| `codelore-action@v1` reusable GHA | Path of least resistance for adoption. |
| GitHub App for auto-PR comments | Biggest UX win at scale. |
| VS Code extension (hotspot gutter markers) | Surfaces findings where devs live. |
| Container variants: alpine + debian (in addition to distroless) | Different consumers, different tradeoffs. |

### Tier 4 — research-flavored / long horizon

| Item | Plan / spec reference |
|---|---|
| Pluggable SZZ (start AG-SZZ; allow Neural-SZZ later) | spec §8 |
| Pluggable tangled-commit untangling (SmartCommit pass-through) | spec §8 |
| Salsa-style incremental memoization | spec §6 |
| LSP server mode | spec §1.2 (deferred) |
| LLM-based commit classification (pluggable model interface) | spec §8 |
| PDG-based Type 4 semantic clone detection | NP-hard; long horizon. |
| Cross-language clone detection (JS ↔ TS ↔ Rust shape equivalence) | deferred. |
| Knowledge-graph JSON output (for Greptile-style consumers) | spec §8 |
| DORA-adjacent delivery flow metric | spec §8 |
| Code coverage analysis (LCOV input, hotspot-weighted) | spec §8 — see Tier 1 above for the smaller-bite version. |

### Tier 5 — community / docs (continuous)

| Item | Why |
|---|---|
| Comparison matrix vs code-maat + CodeScene (measured numbers) | Honest positioning. |
| "Anatomy of a hotspot" tutorial | Demystifies methodology. |
| Real-world case studies (Rails, Linux, React) | Shows the tool at scale. |
| ADRs for major design picks (gix, DuckDB, SARIF, RCA vendor) | Documents the *why*. |
| Migration guide from code-maat | Lowers switching cost. |
| Glossary (Fractal Value, Code Health, Behavioral SARIF, Kamei vector) | No current single source of truth. |

---

## Deliberately out of scope

Forward-looking exclusions surfaced from competitive + literature audits. Each entry names the competitor / paper that does the thing, why it looked attractive, and the binding reason CodeLore won't follow. Capturing them here prevents rediscovery in the next audit cycle.

### LLM-based PR Refactoring Agent

CodeScene v7.5.0 combines AI-assisted refactoring with a local deterministic CodeHealth MCP server to auto-fix PRs ([CodeScene changelog](https://community.codescene.com/changelog), [CodeScene blog](https://codescene.com/blog/deterministic-pr-refactoring-agents), [Code Health MCP](https://codescene.com/product/code-health-mcp)). Well-engineered for their SaaS product surface. CodeLore declines because the README's stated brand is "no LLM-based scoring" + CLI-only / self-contained-SPA — auto-fix requires a SaaS or persistent agent runtime. Anchored by [`feedback_no_copypaste_features`](../../.claude/memory/feedback_no_copypaste_features.md).

### Synthetic "4-factor" composite score (Code Health + Knowledge Distribution + Team/Code Alignment + Delivery)

CodeScene 6.0's executive-dashboard framing bundles four behavioral axes into one synthetic composite ([CodeScene 6.0 release notes](https://docs.enterprise.codescene.io/versions/6.0.0/release-notes/history.html)). CodeLore already exposes every substantive axis discretely: hotspots/code-health, knowledge-islands/bus-factor, ownership/pair-programming, lead-time. Bundling them into a composite would (a) hide signal behind a black-box weighting, (b) duplicate per-analysis output the user can already aggregate, (c) lock in vendor-marketing semantics that drift each upstream release. Keep the analyses discrete.

### Online / streaming JIT-SDP mode

Tempting under the "concept-drift adaptation" framing, but Cabral & Minku (EMSE 2023, [paper](https://link.springer.com/article/10.1007/s10664-023-10335-6) / [PDF](https://minkull.github.io/publications/CabralEMSE2023.pdf)) show offline BORB-MLP beats the top online ORB approach by 2.76–23.19% absolute G-mean (median 6.36%) across projects. CodeLore's post-ingest SQL UPDATE pass is already structurally batch-mode and the cost is paid once per cache key. Adding online learning would multiply ingest complexity for a measured loss in predictive performance.

### Contributor-burnout / contributor-health signals from git history

The most-cited recent qualitative work on the topic — Heath's 2025 OSS Burnout report ([PDF](https://mirandaheath.website/static/oss_burnout_report_mh_25.pdf)) — explicitly scopes itself away from quantitative git-mineable signals and proposes only system-level interventions (pay developers, foster recognition, grow community, advocate). No validated commit-history-derived burnout signal exists in the surveyed literature; any commit-cadence-as-burnout proxy would be speculative inference about humans from machine data — a known false-positive trap.

### Louvain modularity Q as parallel community-detection pipeline

The Code Health Meter paper ([CHM, ACM TOSEM 2025, DOI 10.1145/3737670](https://dl.acm.org/doi/10.1145/3737670)) ships a 6-dimensional module signature with Louvain modularity as an "architectural soundness" score. CodeLore already ships Leiden communities — a strict improvement (no resolution-limit pathology, deterministic, peer-reviewed superior partition quality). Adding a second community-detection pipeline for a 6D composite would (a) violate [`feedback_use_existing_conventions`](../../.claude/memory/feedback_use_existing_conventions.md) (no parallel patterns), (b) confuse users about which output to trust when Leiden and Louvain disagree on a partition, (c) most of CHM's other dimensions (Maintainability Index, Cyclomatic Complexity, duplication) are already covered by CodeLore's tree-sitter complexity scan + AST-hash clone detection. Skip.

### AI-attribution productivity-impact / "AI-code ages differently" columns

Tempting given CodeLore's existing `commits.ai_attribution` column, but the strongest research to date refutes the framing. Borg et al. *Echoes of AI* RCT (N=151, [arxiv 2507.00788](https://arxiv.org/pdf/2507.00788)) found no significant downstream-maintainability difference (completion time or code quality) when developers evolved AI-co-developed vs human-only code; Bayesian analysis suggests any effect is small and highly uncertain. Borg/Tornhill *Code for Machines* ([arxiv 2601.02200](https://arxiv.org/abs/2601.02200)) does show CodeHealth associates with AI refactoring success, but only on 5,000 short Python competitive-programming files — explicitly NOT representative of industrial monorepos per the authors' own caveat. Shipping aging/impact columns would overclaim without controlled comparison data and contradict the "no LLM-based scoring as primary signal" constraint by proxy. The existing authorship-only attribution is the correct scope.

### `cs rules-config` CLI command family clone

CodeScene 7.4.7 introduced a `cs rules-config` CLI command family ([changelog](https://community.codescene.com/changelog)) for managing custom rules from the CLI. CodeLore already has `.codelore-thresholds.toml` + `codelore check` covering the same surface. Adding a parallel `codelore rules` family would duplicate the existing path and create a "legacy thresholds.toml vs new rules-config" migration trap. The valuable signal from CodeScene's move is *validation* that vendors are moving toward CLI-first config management — which CodeLore already does. Treat as architectural confirmation, not a feature to copy.

### "SLSA Build L4" as a meaningful target

The original supply-chain research framing assumed SLSA Build L4 existed and was the next step beyond CodeLore's current L3 attestation. SLSA v1.0 defines only L0–L3; there is no L4 ([SLSA v1.0 spec](https://slsa.dev/spec/v1.0/levels)). Chasing a phantom level is a feature-flag-graveyard pattern. The credible "beyond L3" hardening is **reproducible-builds bit-identical verification** ([reproducible-builds.org](https://reproducible-builds.org/docs/definition/)) — a second builder rebuilds each release and asserts byte-for-byte equality against the published artifact. That is the next supply-chain target when prioritised; the SLSA L4 phrase should never appear in CodeLore roadmap entries.

### Mermaid emitter for architecture or change-coupling graphs

Mermaid's C4 diagram syntax is officially experimental ([Mermaid C4 docs](https://mermaid.js.org/syntax/c4.html)) and carries an explicit syntax-change risk warning. Shipping a Mermaid emitter for behavioural-coupling graphs would lock CodeLore output into a moving target whose breaking changes are not on a SemVer cadence. The right alternative is the Cytoscape.js elements-JSON format ([js.cytoscape.org/#notation/elements-json](https://js.cytoscape.org/#notation/elements-json)) — stable spec, supports directed multigraphs and hierarchical compound nodes, has multiple downstream renderers (graphviz, sigma.js, vis.js). If a Mermaid alternative ever materialises in this space with a stable spec, this entry gets revisited.

---

## Active F-findings

Latent bugs and improvement candidates are tracked in [`reports/deep_analysis_report.md`](./reports/deep_analysis_report.md). That report is the canonical home; this roadmap intentionally does not duplicate the list. When work scheduled here would close one or more F-findings as a side effect, note them in the implementation commit (see `CHANGELOG.md` for the historical pattern).

---

## How to use this document

Each new Tier 1 item gets its own plan document under `docs/superpowers/plans/` when scheduled. The scheduling rubric:

- **What user complaint or stakeholder ask does this address?** Build for measured pull, not anticipated need.
- **Does this differentiate CodeLore?** Items in Tier 1 (vs. Tier 2 quality work) should advance the strategic position vs. code-maat / CodeScene / jscpd.
- **Is the risk understood?** Items with "Hard" implementation difficulty (rename tracking, PGO campaign) deserve a design phase before coding.
