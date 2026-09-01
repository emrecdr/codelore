# CodeLore — Deep Analysis & Improvement Report

*Produced 2026-09-01 from 14 parallel read-only codebase audits plus 6 external research sweeps (competitor landscape, 2026 standards); every load-bearing claim was re-verified against source before inclusion. Findings the validation pass refuted are listed in §2 — they are part of the result. The §4 Phase-1 fixes shipped the same day (ledger F322, F324–F327; refutations F328–F329; new Active findings F323, F330–F335).*

---

## 1. Executive summary

CodeLore is in unusually good shape: the hardening campaign shows everywhere (cache atomicity, panic-aware MCP dispatch, anti-vacuous guard tests, SHA-pinned supply chain, an SPA with best-in-class empty states). The improvements that matter now fall into six themes:

1. **The flagship claim needs reframing, not the flagship.** "Behavioral facts → AI agents via MCP" is no longer unique: **Repowise** (AGPL, Python, ~6.3k stars, 5 months old, verified via GitHub API) ships 10 MCP tools including a per-change defect-risk score; **CodeScene** ships a CodeHealth MCP server (history tools paywalled). The defensible claim — verified against everything researched — is: *the only Rust-native, fully local, free tool exposing this breadth of git-history behavioral analytics to agents with no cloud dependency, and with materially deeper research grounding (Kamei JIT-SDP, Leiden, cycle-origins, clones × co-change) than any MCP-native OSS entrant.*
2. **A small set of validated correctness bugs** — headlined by MCP tool handlers silently ignoring the server's `--defect-calibration` (7 of 11 handlers), and the three language enums drifting apart (`.pyi`, uppercase extensions). Phase 1 below fixes them.
3. **Three big performance levers**, in ascending effort: warm-blob-reader gaps (S, two sites missed by the earlier campaign), eliminating repeated work in the SPA render (`run_code_health` executes 5× per render; M), and the fused single-parse HEAD scan (ledger F173's fix; M). Incremental ingest remains the only moonshot (L, design-first).
4. **The polyglot blind spot is a disclosure problem.** Architecture analyses describe only the Tier-1 subset with no coverage denominator; hotspots score unsupported-language files 0 and label them "100/100 healthy"; nothing anywhere says "this repo is 70% Go". Cheap first fix: a language census line in the pre-flight banner + coverage disclosure columns.
5. **The agent-integration surface is the highest-ROI product investment** given where competition moved: `codelore schema` is a stub (57/57 names, 0/57 row shapes), the GitHub Action is unpublished and swallows its own `result`/`violations` outputs, and MCP lacks corpus-lens calibration entirely.
6. **README/docs conversion quick wins**: the excellent Quick start sits 223 lines below 11 decorative badges; the project's best credibility asset (`research-foundations.md`, 797 lines) is linked from nowhere.

## 2. Validation discipline — what was refuted

Agent findings were treated as leads, not verdicts. Four were killed or downgraded on source evidence:

| Claimed finding | Verdict | Evidence |
|---|---|---|
| "churn analyses silently ignore `--time-bucket` (private lineage dispatcher lacks the branch)" | **Refuted as a bug.** The CLI hard-rejects `--time-bucket` outside coupling/soc/hotspots/code-health (`analyze.rs:141`), so the missing branch is unreachable. Survives only as a DRY/drift-risk item. | verified |
| "stale-code silently ignores `--min-revs` while its sibling honors it" | **Refuted.** The documented contract (advanced-usage.md:87) never promises min-revs; three other analyses carry `min_revs` in tracing spans as blanket convention without using it. Working as documented. | verified |
| "`duckdb::Connection` is `!Send + !Sync`" (stated in ~16 places incl. CLAUDE.md) | **Wrong for the pinned version.** `unsafe impl Send for Connection` at duckdb-1.10505.0 `lib.rs:277`; only `Appender`/`Statement` are `!Send`. The architecture stays right; the stated reason is wrong. Docs fix in Phase 1; parallel dispatch remains correctly rejected for temp-table-multiplication reasons (§5). | verified |
| "`is_shallow` backend asymmetry violates the differential gate" | **Downgraded.** The trait doc explicitly frames the default-false as a hint-not-contract opt-out. Still worth parity (GitCliRepo has a cheap `.git/shallow` check) — Phase 1 — but it is not a violation of the event-stream invariant. | verified |

Also verified healthy, explicitly: cache-write atomicity (PID-suffixed tmp + rename, stale sweep, zero-commit stores never persisted), bounded-channel producer join with panic propagation, MCP stdio hygiene (no stdout writes, no subprocesses in tool paths), digest-keyed O(n log n) clone grouping, `gix::open` (no discovery walk), zero full-table scans before the analysis query on a cache hit, parameter binding across all 66 sites (no SQL-injection shape anywhere), SARIF emitter richness above GitHub's baseline, and the zizmor/cargo-deny/SHA-pinning supply chain already exceeding the 2026 checklist.

## 3. Competitive picture (verified 2026-08-31)

- **Repowise** — the one to watch. AGPL-3.0, 6,278★/676 forks/79 contributors, created 2026-03-23, pushed daily. Hotspots, ownership, co-change, bus factor, clone detection, architecture cycles, delivery metrics, dead code, 49 deterministic risk detectors, 10 MCP tools, hosted+self-hosted. Its own competitive-landscape article does not mention CodeLore at all — the gap is visibility, not capability. Unverified: SARIF, CI gate, real JIT-SDP.
- **CodeScene** — CodeHealth MCP server: free tools are static-only; hotspots/ownership MCP tools require a paid Core subscription. The paywall is CodeLore's opening.
- **SonarQube** — shipped architecture analysis (beta 2025-12, GA Server 2026.4): zero-config dependency graph + declared intended architecture + drift detection. Structural snapshot only, 5 languages, commercial editions only, **no git history anywhere in Sonar** (their own forum confirms). CodeLore's historical architecture angle (trend, cycle-origins) remains differentiated.
- **GitClear** — closest commercial category-mate (Diff Delta, AI-tool attribution, the widely cited 2026 Maintainability Gap report), but proprietary SaaS, no architecture/clones/gate/MCP.
- **Engineering-intelligence platforms** (Swarmia/LinearB/Jellyfish/DX) — none compute behavioral code analysis; Swarmia is explicitly filename-only; DX's "code quality" is survey-derived. Macro story is consolidation (Atlassian bought DX ~$1B; Cursor's parent bought Graphite).
- **AI reviewers** — only Bito makes a primary-source-verified "hotspots from commit history" claim (one feature in a context-graph product). CodeRabbit/Greptile/Graphite/Sourcery: no verified behavioral-history signal.
- **Dead/stale**: Hercules, code-forensics, git-of-theseus. code-maat itself is alive but slow-moving. The Rust+gitoxide niche has no other occupant at scale.

**Positioning actions**: reframe the README differentiator claim (drop any "only ones feeding behavioral facts to agents" implication); add a comparison page vs Repowise + CodeScene MCP (roadmap Tier 5 already wants a comparison matrix); treat schema/action/MCP-completeness (Phase 2) as the competitive response.

## 4. Validated correctness findings

### Phase 1 — shipped alongside this report (all re-verified at source before implementation)

| # | Fix | Evidence |
|---|---|---|
| P1.1 | **MCP calibration drift**: 7 of 11 tool handlers (`repo_overview`, `hotspots`, `code_health`, `delta_health`, `refactoring_targets`, `function_xray`, `finding_hotspot_overlap`) never thread the server's `defect_calibration`/`allow_foreign_calibration` into their `Options`; `code_health` vs `check_gates` on the same server use different smell weights. Fix mirrors the 4 correct handlers incl. the memo-key fragment. | mcp.rs:394 vs 681–1309 |
| P1.2 | **Language-enum parity** (closes ledger F311 + F321): `CloneLanguage`/`ImportLanguage` match raw extensions (uppercase skipped) and clones drop `.pyi`; complexity lowercases and accepts `.pyi`. Align all three + add a parity test; requires `CACHE_EPOCH` bump (ingested tables change). | clones/language.rs:36, imports/language.rs:33, complexity/language.rs:27 |
| P1.3 | **Warm blob reader misses**: `ingest_complexity_at_rev` uses `map_init(|| ())` + cold `read_blob_at` per file (the one rayon scan the F253 campaign missed — its sibling in the same feature does it right and names this cost); `effort_exposure` cold-reads per path in a serial loop at one rev. | at_rev.rs:66, effort_exposure.rs:505 |
| P1.4 | **delivery-friction is not rename-aware**: aggregates `FROM changes` by path with no lineage opt-in, while correctly routing its complexity axis; revisions/lead-time/WIP stats silently split across renames. | delivery_friction.rs:92,147–183 |
| P1.5 | **`is_shallow` parity**: give `GitCliRepo` the cheap `.git/shallow` check + differential coverage on a `--depth=1` clone. | repo/mod.rs:106, gix_repo/mod.rs:282 |
| P1.6 | **Bare `cargo test -p codelore-lib` compile failure**: `options::tests` uses `tempfile` at 15 sites without the `test-support` gate. Gate the module. | options.rs:709 |
| P1.7 | **`paths_filter` has zero unit tests** despite being load-bearing for the cache key; `.gitignore`/`.git/info/exclude`/negation/precedence never exercised anywhere. Add a direct test module. | paths_filter.rs |
| P1.8 | **Action swallows its own verdict**: `check`/`gate` write `result=pass|fail` + `violations=N` to `$GITHUB_OUTPUT`, but `action.yml` declares only `result-path`/`version-used`. Declare them. | action.yml outputs, check.rs:61–349 |
| P1.9 | **Docs/ledger hygiene**: correct the `Connection` `!Send` claim (~16 sites, key ones first); fix "twelve path-aggregating analyses" (actual ≥21); add `research-foundations.md` + `github-action.md` to the README doc table; fix the dead `improvement_suggestions.md` link; reconcile the 57/54/56 analysis-count drift (drop hard numbers per house rule); fix 4 stale ledger pointers (F215 self-contradiction, calibrate-defects "remains open", §9 cross-ref, F288 next-ID marker); record the §2 refutations and these fixes in the ledger. | multiple |

### Recorded as new ledger findings (validated, not fixed inline)

- **MCP has no corpus-lens `calibration` support at all** — `CodeLoreServer` lacks the field; every MCP call uses the embedded world artifact only (S to add; separate decision because it extends the MCP flag surface).
- **`knowledge_shares` built-guard ignores `opts`** — a bare bool; per-analysis `opts` divergence inside one dashboard build is a live pattern (delivery-metrics clones opts with `include_merges=true`). Flagged by the DuckDB audit as a correctness smell; needs validation of whether any current caller pair actually diverges on shares-affecting fields.
- **`refactoring_targets` depends on the side effect** that `run_code_health` leaves `code_health_biomarkers_v1` at HEAD, while the trend loop overwrites it with historical samples — currently correct only by call order. Blocks the naive code-health memo (§5).
- **`ignored_flag_warnings` misses 9 analysis-scoped flags** (coupling + clone families); **`CalibrateDefectsArgs::window_days` shares a name with `Options::window_days`** but different ranges/semantics; **no test ties `Gates` fields to evaluator branches or `RatchetMetrics`**.
- **Hotspots scores non-Tier-1 files 0/"100 healthy"** (LEFT JOIN + COALESCE(cognitive,0)) while code-health drops them (INNER JOIN) — two opposite silent semantics side by side; needs a design decision (exclude-with-disclosure recommended, mirroring the mi_rank care already taken).
- **`Repo::diff_hunks` is a required trait method with zero production callers** (hunks attach inline during the walk) plus an untested backend divergence in `FileChange.hunks` (GitCliRepo always emits empty). Needs a remove-or-repurpose decision.
- **AST-cap files are invisible** — over-2MiB skips are `NotCounted` (excluded from the coverage denominator) and logged only at debug; a bundle-heavy repo reports 100% clone coverage with an empty clones table. Add a disclosed skip category + document the cap; bytes-per-line minified heuristic as the sub-cap backstop.

## 5. Performance levers (ranked, beyond Phase 1)

1. **SPA render de-duplication (M)** — `run_code_health` executes 5× per `--format spa` (dashboard, effort-exposure, marginal-owner-risk, refactoring-targets, coordination-needs): 15 full lineage-table scans and 5 five-way window sorts where 3 and 1 would do. Fix = `CodeHealthMemo` mirroring `CouplingMemo`, keyed on `HealthScanCtx` + health-affecting opts, **with the biomarker temp-table re-assertion** (see §4). `run_hotspots` 2× is the same fix in small.
2. **Coupling semi-join pre-gate (S)** — the O(Σ n_rev²) self-join runs over un-gated changes; semi-joining against the min-revs-gated file set first is provably output-identical and shrinks the quadratic input by the long tail. Land behind the byte-identical baseline gate.
3. **Fused HEAD scan (M)** — ledger F173: complexity/clones/imports each blob-read + parse the same files (3× decode, 3× parse; 16.9MB parsed instead of 5.6MB on this repo). `codelore-rca` already exposes the raw tree-sitter node, so clone/import walkers can reuse one parse. Prerequisite: P1.2 (the three passes must agree on the file set first). Also fixes health-trend's double parse per sample.
4. **Unusable indexes (S + benchmark)** — all 5 `schema_v1.sql` secondary indexes + 2 per-run lineage indexes serve only join/GROUP-BY predicates, which DuckDB's ART matcher never uses (verified against the vendored `art.cpp`). Pure build/maintenance cost. Benchmark, then drop + `CACHE_EPOCH` bump. Keep every PRIMARY KEY.
5. **MCP resource multiplication (M)** — 4 concurrent tool calls × own DuckDB instance = up to 4×cores threads and 4×4GB limits. Either divide `threads`/`memory_limit` by intended concurrency, or share one DB via `Connection::try_clone()` (now known to be possible: `Connection` is `Send`).
6. **Row-at-a-time temp-table INSERTs (S)** — two sites (`eh_bands_v1`, `coupling_centrality_v1`) do the exact pattern `at_rev.rs` documents as pathological, which already has the chunked-VALUES helper to reuse. Function-xray's 20 per-path scans batch into 2 `IN`-list queries (S).
7. **Analysis-connection PRAGMAs (S, byte-identical-gated)** — `preserve_insertion_order=false` on the read-only connection (never the ingest one — the lineage rowid tiebreak is load-bearing); `checkpoint_threshold=1GB` during file-backed ingest.
8. **TTY dirty-check narrowing (M)** — every interactive cache hit pays an O(tracked-files) `git status` to print a warning that applies to ~4 of 64 analyses.
9. **Incremental ingest (L, design-first)** — the cache key includes `head_sha`, so any new commit = full re-walk; confirmed absent, Tier-4 roadmap. The design must answer append-only commits/changes + HEAD-scan refresh + lineage invalidation. This is the only lever that changes the product's steady-state latency class. Recommend a design doc before any code.

## 6. Standards alignment (2026)

Already at/above baseline (verified): zizmor with reasoned policy, cargo-deny, SHA-pinning + drift-guard test, Dependabot grouping/ignores with rationale, SARIF 2.1.0 emitter with fingerprints/severity/automationDetails, five-shell completions, artifact attestation for release binaries, no telemetry (verified claim).

Adopt, in value order:
1. **crates.io Trusted Publishing** (S; requires per-crate configuration on crates.io by the maintainer — the ledger's old build.rs blocker was already disproved). Kills the long-lived `CARGO_REGISTRY_TOKEN`.
2. **OpenSSF Scorecard workflow** (S) — repo would score well immediately; visible trust signal for a trust-selling tool.
3. **cargo-nextest** in CI (S) — process-per-test isolation + sharding across the existing 3-OS matrix; keep `cargo test --doc` separately.
4. **proptest** first targets (S each): `lineage::rewrite`, `Options::validate`, `paths_filter` precedence.
5. **In-repo cargo-fuzz targets** (S each): thresholds-TOML, `.codelore-teams`, group-file DSL, SARIF ingest. (Scorecard's Fuzzing check won't credit them for Rust — value is bug-finding, not badge.)
6. **divan + CodSpeed** (M) — free for OSS; instruction-count gating for CPU-bound paths (complexity, fingerprinting, rewriter), walltime as advisory trend for DuckDB/IO paths. Directly answers ledger F186 (bench gate never on PRs). criterion's maintenance moved orgs — use divan for new benches.
7. **cargo-mutants `--in-diff`** advisory PR job (M) — targets the query-logic bug class `CACHE_EPOCH` exists to mop up.
8. **Renovate app install** (user action; config already exists) — closes the Containerfile digest gap Dependabot can't see.
9. **cargo-vet** (M) — pre-disclosure supply-chain insertion coverage; import Mozilla/Google audit sets.
10. **Explicit Dependabot `cooldown:` block** (S) — make the 3-day default visible and tunable.

## 7. Feature gaps & opportunities

**Agent/CI surface (the competitive response — do first):**
- Real `codelore schema` via `schemars` derives (M) — 57 names exist, 0 row shapes; agents and integrators currently guess.
- GitHub Action: publish to Marketplace + automate the `v1` retag on release (S–M) + P1.8 outputs.
- MCP corpus-lens calibration field (S, from §4).
- **F315** (your standing decision): persist ScanCoverage → `degraded` gate verdict. Converts disclosure into enforcement (`fail_on_degraded` defaults true — blobless-partial-clone repos start failing). The clones pass is the sharpest case (thin scan reads as improvement today).
- PR-comment posting: the roadmap's GitHub-App path is right; a cheaper interim is a documented `gh pr comment --body-file` recipe on the step-summary markdown (S, docs-only).

**Polyglot disclosure (cheap, high-trust):**
- Language census line in the pre-flight banner ("312 files: 61% Rust, 22% Go (no parser), …") (S–M).
- Coverage denominators on architecture-metrics output ("graph covers N of M live source files") (S).
- Hotspots: exclude non-Tier-1 from cognitive ranking with a disclosed count instead of scoring them healthy (design decision, then S).
- Language expansion when ready: C++/Kotlin are vendor-drop reverts (grammars were deliberately deleted; enum regeneration tooling must be written first — it's absent from the repo); Go is the highest-demand net-new. Each bump must follow the documented 4-site coordination.

**Dashboard representation** (34 of 57 analyses have no SPA surface): highest-value additions are dependency-cycles/cycle-health (the arch graph currently shows imports but never the SCC tangles), Leiden communities (a stated differentiator, invisible), architecture-violations (the rules engine has zero surface), and a gate/check verdict band.

**Onboarding**: `codelore init --thresholds` (the repo's own F4 proposal, still open) + glossary page (roadmap Tier 5, largely liftable from research-foundations.md).

**Explicitly NOT proposed** (honoring documented rejections): SLSA "L4", `unsafe-inline` CSP, cargo-machete/udeps gates, Louvain, online JIT-SDP, composite 4-factor score, LLM auto-refactoring, burnout signals, Mermaid emitter, `cs rules-config` clone, hosted SaaS/team server, non-git VCS, Options builder, automatic worktree cache invalidation, domain newtypes, Type-3 clones as a "~100 LOC quick win" (known-wrong estimate; needs new shingled ingest).

## 8. UX

**README/docs (S batch, high conversion impact):** move Quick start above the pitch wall; trim the 11 topic badges; add a 15-word pitch line; link research-foundations + github-action from the doc table (split user/contributor); one install path marked "start here"; expand the advanced-usage TOC to two levels; per-analysis output-column documentation (or make `codelore schema` the canonical answer once real).

**CLI:** exit codes and error text are strong; the missing pieces are `codelore init` and error-message "next step" hints (the miette-style investment was reviewed and is *not* recommended — errors here are config/repo-shaped, not source-span-shaped).

**SPA (three sprints, from the verified audit):**
- *Sprint 1 (all S):* `aria-describedby` on 41 tooltips; keyboard-operable sort + `aria-sort`; `aria-current` on nav; repo-derived `<title>` + HEAD SHA stamp (currently every dashboard is "CodeLore Dashboard" over an absolute local path); restore the search-box focus ring; rebuild the friction ramp on monotonic lightness (band 5 is currently darker than band 3, and bands 2/5 collide under deuteranopia — computed, not guessed); commit the missing landing-page assets.
- *Sprint 2 (M):* replace `role="button"` on `<tr>` (500 tab stops, destroyed table semantics); a <768px breakpoint; cap the circle-pack and `run_imports_for_arch_graph` payloads with disclosed "top N of M"; serialize selection/filter/colorMode into the URL hash and stop clobbering pasted anchors.
- *Sprint 3 (L):* wire the dead `filter` store into every path-aware widget (one search box scopes the dashboard — the top monorepo ask); a verdict + "top 3 actions" band; CVD-safe author palette.

## 9. Recommended plan

- **Phase 1 (shipped):** §4's fix clusters landed as small PRs, each with tests, CHANGELOG and ledger entries; anti-vacuity probes where a test could pass vacuously.
- **Phase 2 (next sessions):** SPA render memo + coupling semi-join + INSERT batching (perf, measured); schemars schema; Action marketplace; Scorecard + nextest + first proptest/fuzz targets; SPA Sprint 1; README conversion batch; MCP calibration field; language census.
- **Phase 3 (strategic, each behind a design doc):** fused HEAD scan (F173); incremental ingest; MCP shared-DB; language expansion (Go first, tooling first); SPA Sprints 2–3; dashboard coverage for cycles/communities/gate; F244 registry (absorbs the emitter cluster); GitHub App.
- **Maintainer decisions, open:** F315 gate wiring; Renovate install; Trusted Publishing per-crate setup; hotspots non-Tier-1 semantics; `diff_hunks` remove-or-repurpose.

## 10. Further-improvement pass (what this analysis itself suggests next)

- The session's recurring defect shape — *hand-mirrored things drifting apart* (three language enums, gates↔evaluators↔ratchet, exemption lists, doc counts) — suggests a standing pattern: every hand-mirrored pair in the codebase should carry a parity test or a single source of truth. A one-page inventory of remaining mirrors would be a cheap next audit.
- The coverage sentinel design (Scored/Lost/NotCounted) is good enough to extend product-wide: the same "found-nothing vs couldn't-look" discipline would fit SARIF ingest (absent vs unreadable — already done), the import resolver (resolution-rate disclosure exists), and the language census (proposed above). One shared vocabulary, four surfaces.
- Competitive monitoring: Repowise moves fast; a quarterly re-check of its SARIF/gate/JIT-SDP gaps (the three unverified items) keeps the comparison page honest.
