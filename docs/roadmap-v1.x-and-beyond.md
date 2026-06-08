# CodeLore — Roadmap (post-`0.1.0` and beyond)

**Status:** living document. Last updated 2026-06-08.

This doc is the prioritized backlog of *everything* proposed after the `0.1.0` tag. Each row links to a plan document when one exists.

> The "v1.x" in this file's path is a legacy artifact from when the first stable was planned as `1.0.0`; the project ultimately collapsed the alpha→beta→rc ladder and shipped `0.1.0` as the first stable. See [`RELEASING.md`](RELEASING.md) for the versioning policy.

## Decision rubric

Items are ranked by **leverage × risk**:
- **Leverage**: does this change make users more successful or open new use cases?
- **Risk**: implementation difficulty × blast radius of getting it wrong
- **Strategic**: does this differentiate CodeLore against code-maat / CodeScene / jscpd?

## Priority queue

### ✅ Shipped under `0.1.0`

The original "Tier 1" (release readiness) and "Tier 2" (v1.x differentiators) lists have all landed in the first stable cut and are no longer roadmap items. The full set, with the commits that delivered them, is preserved in `CHANGELOG.md`'s `[0.1.0]` entry. Headline shipped items:

- Persistent fact-store cache (XDG-style, LRU-evicted) — 100×+ speedup on repeat runs
- Parallel complexity extraction via Rayon — 3–5× wall-time speedup on cold runs
- `clone-coupling` intersection (the strategic differentiator with the `CODELORE-LIVE-CLONE` SARIF rule)
- `codelore diff <base>..<head>` PR-mode subcommand — 4 output formats, `--fail-on` quality gate
- All 21 code-maat-parity analyses + 4 SARIF rules
- 6 verified correctness fixes (R1, R2, R3, R4, R6, R12 — negative hotspot scores, GixRepo date/merge filters, `--after`/`--before`/`--include-merges` CLI surface, Kamei O(N²) → hash-joined UPDATE, Parquet schema completion)

### Tier 1 — v0.2 differentiators (Plan 9, next-up)
Strategic features for the next minor. Promote when there's measured user pull.

| Item | Why | Plan | Status |
|---|---|---|---|
| PGO campaign + release pipeline rebuild | Spec §6.5; 5–15% perf headroom on real workloads | future | pending |
| Type 3 near-miss clones (MinHash + LSH @ Jaccard ≥ 0.8) | Plan 7 §2 Task 4; ~100 LOC; catches "renamed + restructured" code | future | pending |
| **Bus-factor / knowledge-island detector** (hotspots × single-owner × departed-author) | Plan 7 research surfaced this; we already have all the data | future | pending |
| **Live-clone × knowledge-loss intersection** (clones inside departed-contributor code) | Engineering-director-level signal nobody else produces | future | pending |
| Rename tracking via `gix_diff::tree::breaks::detect_renames` | Validation Finding S6. `ChangeType::Renamed { from, similarity }` is captured at ingest by both `GixRepo` (gix_repo.rs:260) and `GitCliRepo` (git_cli_repo.rs:400). What's missing: no analysis queries the `from` field — `revisions` / `coupling` / `churn` SQL views all `GROUP BY` raw post-rename path, so a renamed file's history splits. Needs a canonical-lineage DuckDB view; rename chains can have cycles so the resolver must detect and break them deterministically. | future | partial (data captured, queries don't follow yet) |
| Bootstrap confidence intervals on hotspot scores | Methodological honesty wedge; CodeScene reports point estimates | future | pending |
| `--query SQL` escape hatch | Spec §5 reserved; power-user feature | future | pending |
| LCOV input → hotspot-weighted coverage | CodeScene shipped this in 2025 | future | pending |
| AI-authorship correlation reports | We tag commits; novel publishable signal | future | pending |
| Survival analysis on hotspots (how long do they stay hot?) | Temporal-extension research | future | pending |

### Tier 2 — quality and DX (continuous)
Always-on hygiene work; no plan required, weave into other plans.

| Item | Why | Status |
|---|---|---|
| `proptest` on parser + fingerprint walker | Catches edge cases | pending |
| `cargo-mutants` in CI | Hardens test assertion quality | pending |
| `cargo-fuzz` campaign (spec §6.7 → v1.5) | Parser hardening | pending |
| Switch CSV writer to `csv` crate | `output/csv.rs` has 28 hand-rolled `writeln!` calls and zero `csv::Writer`. The `quote_if_needed` helper at the top closes the acute injection vector, but any new emitter forgetting to call it silently breaks valid CSV. Migrate to `csv::WriterBuilder::flexible(false)` with per-emitter round-trip snapshot tests. | pending |
| Macro-driven CLI dispatch | Replaces 66-arm `match (format, &analysis)` ladder in `main.rs` (grew from 14 as analyses landed) | pending |
| Builder + validation for `Options` (28 fields, no cross-field checks) | 4 verified pathological combinations silently produce empty results today: `min_revs > max_changeset_size`, `clone_similarity_floor > 1.0`, `after > before`, `fisher_significance > 1.0`. `OptionsBuilder::build() -> Result<Options, OptionsError>` would catch all four at the boundary; also gates future field-additions through a single funnel (currently new fields require updating ~6 struct-literal call sites). | pending |
| Parallelize clone extraction (`populate_clones_at_head`) | `facts/ingest.rs::populate_clones_at_head` walks Tier-1 files sequentially. Same `rayon::par_iter().map_init` pattern that complexity extraction already uses. `tree_sitter::Parser` is `Send + Sync` so no thread-local pool. ~30 LOC change, low risk. Win is smaller than complexity parallelization (hash-only, no Halstead/MI) but linear in file count. | pending |
| `gix-write` for test fixtures (5-10× faster than shell-out) | Spec §6 noted gix-write maturing | pending |
| Better error messages at CLI boundary | "find_parent_commit ..." → "shallow clone is missing parent ancestry" | pending |
| Reproducible-build verification in CI | Compare binary hashes across runs | pending |
| Snapshot tests for SARIF / JSON output | Catches silent format drift | pending |
| **CI speedup — `cargo-nextest`** | Drop-in replacement for `cargo test` with ~20-30% faster test-phase execution (smarter scheduling, faster output, better failure aggregation). One-line workflow change. Mainly helps the test-phase wall-clock; doesn't touch the compile-phase dominator. | pending |
| **CI speedup — sccache 0% hit-rate investigation** | `mozilla-actions/sccache-action@v0.0.6` is wired in `ci.yml`, but the v0.1.0 CI run on Windows reported `Cache hits: 0 / Cache misses: 392 / Cache hits rate: 0.00%`. The sccache key is hashing something that changes on every run (likely env-var-derived). Diagnosis + fix could save up to ~5 min off the Windows test job (which is the wall-clock bottleneck). | pending |
| **CI speedup — bundled DuckDB compile dominates** | `libduckdb-sys` with the `bundled` feature compiles ~6000 .cpp files via `cc-rs` from scratch every run (~5-7 min on every OS). The 3 OS jobs already parallel-execute, so wall-clock is bounded by Windows. Three options: (a) keep `bundled` + improve sccache C++ object-cache hit rate (low-risk, medium-payoff), (b) switch to `dynamic` + ship pre-built DuckDB on runners (medium-risk, high-payoff but loses single-binary portability), (c) split a "build DuckDB once, cache the artifact" job that all 3 OS test jobs depend on (medium-risk, high-payoff, no portability loss). | pending |
| **CI speedup — path filters** | `.github/workflows/ci.yml` runs the full matrix on every push, including docs-only changes. Adding `on.push.paths: ['!docs/**', '!*.md']` (or similar) skips the heavy test matrix when only docs change. Modest win in absolute time (~15 min per docs-only push) but meaningful for developer flow. | pending |
| **Re-add `x86_64-unknown-linux-musl` release target** | Dropped from `Cargo.toml::[workspace.metadata.dist].targets` for `v0.1.0` because Ubuntu's `musl-tools` package ships `musl-gcc` (C) but not `musl-g++` (C++) — `bca-tree-sitter-preproc`'s `scanner.cc` and bundled DuckDB's ~6000 .cpp files have nowhere to compile to. Two routes: (a) replace cargo-dist's default Ubuntu+`musl-tools` env with a `messense/rust-musl-cross`-style Docker per target (cargo-dist supports custom containers via `[workspace.metadata.dist.github-custom-runners]`), or (b) install a `musl-cross-make`-built toolchain in the runner. Either gives Alpine users a true static-musl binary. Interim workaround: `cargo install codelore` (links static libgcc) or run the gnu binary under `gcompat`. | pending |

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
