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

### Tier 1 — first-stable release readiness (Plan 8) ✅ shipped under `0.1.0`
Foundation for the first stable tag and clearing the validation-report findings. Plan 8 closed out before the `0.1.0` cut.

| Item | Why | Plan | Status |
|---|---|---|---|
| 5 pre-tag fixes (perf-evidence drift, README inconsistency, missing test, CLI error UX, spec note) | Validation report Findings S1, S2, S4, S7, S8 | Plan 8 §1 | ✅ shipped `3043a42` |
| `--analysis authors` standalone | Closes spec §1.1 gap; trivial SQL | Plan 8 §2 | ✅ shipped `0ce89ff` + mailmap fix `1154975` |
| `--group-file` clap flag exposure | Field exists in `Options`; not surfaced | Plan 8 §2 | ✅ shipped `af572cb` |
| `--exclude PATTERN` + `.codeloreignore` | Validation report Finding S9; needed before clones is usable on vendor-heavy repos | Plan 8 §2 | ✅ shipped `af572cb` |
| Clones JSON / Markdown / SARIF emitters (CODELORE-CLONE rule) | Plan 7 shipped CSV-only | Plan 8 §2 | ✅ shipped `0ce89ff` + `af572cb` |
| FactsDb integration for clones (write to clones table) | Closes validation Finding S3; foundation for clone-coupling | Plan 8 §4 | pending |

### Tier 2 — v1.x differentiators (Plan 8)
Where CodeLore visibly beats the field. Same plan as Tier 1; these are §5-§7.

| Item | Why | Plan | Status |
|---|---|---|---|
| **Persistent fact-store cache** (XDG-style, LRU-evicted) | 100×+ speedup on repeat runs; makes `codelore diff` viable in CI | Plan 8 §3 | ✅ shipped (`a6e8409`+3) |
| **Parallel complexity extraction** (Rayon `map_init`) | 3-5× wall-time speedup; closes Plan 4 footnote | Plan 8 §5 | ✅ shipped (`8ae2dd6` + T18 bench in flight) |
| **`clone-coupling` intersection** (the CodeScene X-Ray pattern with our published-formula transparency) | The single biggest differentiator from any existing tool | Plan 8 §6 | ✅ shipped (`49d1dcb` + `f63bcab` CLI/SARIF) |
| **`codelore diff <base>..<head>`** (PR-mode) | The form users actually deploy in CI | Plan 8 §7 | ✅ shipped (`b9bfdc7`) — full subcommand with 4 output formats |

### Tier 3 — v1.1+ (Plan 9, future)
Strategic features once v1.0 ships and the bench data is in.

| Item | Why | Plan | Status |
|---|---|---|---|
| PGO campaign + release pipeline rebuild | Spec §6.5 commits to v1.1 | future | pending |
| Tag `v1.0.0` and execute release pipeline | The 9 "implemented but not validated" items flush in one shot | Plan 8 §8 / decision | pending |
| Type 3 near-miss clones (MinHash + LSH @ Jaccard ≥ 0.8) | Plan 7 §2 Task 4; ~100 LOC; catches "renamed + restructured" code | future | pending |
| **Bus-factor / knowledge-island detector** (hotspots × single-owner × departed-author) | Plan 7 research surfaced this; we already have all the data | future | pending |
| **Live-clone × knowledge-loss intersection** (clones inside departed-contributor code) | Engineering-director-level signal nobody else produces | future | pending |
| Rename tracking via `gix_diff::tree::breaks::detect_renames` | Validation Finding S6; revisions/coupling/churn currently split on rename | future | pending |
| Bootstrap confidence intervals on hotspot scores | Methodological honesty wedge; CodeScene reports point estimates | future | pending |
| `--query SQL` escape hatch | Spec §5 reserved; power-user feature | future | pending |
| LCOV input → hotspot-weighted coverage | CodeScene shipped this in 2025 | future | pending |
| AI-authorship correlation reports | We tag commits; novel publishable signal | future | pending |
| Survival analysis on hotspots (how long do they stay hot?) | Temporal-extension research | future | pending |

### Tier 4 — quality and DX (continuous)
Always-on hygiene work; no plan required, weave into other plans.

| Item | Why | Status |
|---|---|---|
| `proptest` on parser + fingerprint walker | Catches edge cases | pending |
| `cargo-mutants` in CI | Hardens test assertion quality | pending |
| `cargo-fuzz` campaign (spec §6.7 → v1.5) | Parser hardening | pending |
| Switch CSV writer to `csv` crate | Hand-rolled quoting has bugs waiting | pending |
| Macro-driven CLI dispatch | Replaces 33-arm match | pending |
| Builder + validation for `Options` (18 fields, no cross-field checks) | Catches `min_revs > max_changeset_size` silently accepted today | pending |
| `gix-write` for test fixtures (5-10× faster than shell-out) | Spec §6 noted gix-write maturing | pending |
| Better error messages at CLI boundary | "find_parent_commit ..." → "shallow clone is missing parent ancestry" | pending |
| Reproducible-build verification in CI | Compare binary hashes across runs | pending |
| Snapshot tests for SARIF / JSON output | Catches silent format drift | pending |

### Tier 5 — operational (post-v1.0 launch)
Adoption levers; deferred until v1.0 actually ships and we have real users.

| Item | Why | Status |
|---|---|---|
| `codelore-action@v1` reusable GHA | Path of least resistance for adoption | pending |
| GitHub App for auto-PR comments | Biggest UX win at scale | pending |
| VS Code extension (hotspot gutter markers) | Surfaces findings where devs live | pending |
| Static-HTML report generator (`report.html`) | Web UI is out-of-scope per spec §1.2; a single file is in-scope | pending |
| Container variants: alpine + debian (in addition to distroless) | Different consumers, different tradeoffs | pending |

### Tier 6 — research-flavored / v2+
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

### Tier 7 — community / docs (continuous)

| Item | Why |
|---|---|
| Comparison matrix vs code-maat + CodeScene (measured numbers) | Honest positioning |
| "Anatomy of a hotspot" tutorial | Demystifies methodology |
| Real-world case studies (Rails, Linux, React) | Shows the tool at scale |
| ADRs for major design picks (gix, DuckDB, SARIF, RCA vendor) | Documents the *why* |
| Migration guide from code-maat | Lowers switching cost |
| Glossary (Fractal Value, Code Health, Behavioral SARIF, Kamei vector) | No current single source of truth |

---

## What's planned right now

**Plan 8** (`docs/superpowers/plans/2026-06-07-codelore-plan-8-v1.x-readiness.md`) covers Tier 1 + Tier 2 — the v1.x release scope. ~25 tasks across 7 phases.

**Beyond Plan 8**: each Tier 3+ item gets its own plan when scheduled. The rubric for scheduling is: **what user complaint or stakeholder ask does this address?** Build for measured pull, not anticipated need.
