# Positioning Cycle Implementation Plan (self-gating + comparison content)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make codelore's own repo gate itself with `codelore check` in CI (blocking), and add an honest "vs GitHub Code Quality" comparison to the README — landing before GitHub Code Quality's general availability.

**Architecture:** Task 1 is config + CI + one README line; Task 2 is README content plus a prepared landing-page section (published to the `gh-pages` branch after the PR merges). No Rust code changes.

## Global Constraints

- Branch feat/positioning-ga (base 0e2c3e8), append-only, `git log --oneline -1` before committing, Conventional Commits, NEVER Co-Authored-By, stage only intended files by name.
- No ticket IDs / plan refs / codelore version numbers in docs. Competitor claims must be capability-based and durable (no hard-coded competitor prices/dates in README — they go stale; "per-committer subscription plus metered CI compute" not "$10 + Actions minutes").
- Honest-marketing voice (established README convention): concede what the competitor does, then state the specific verifiable delta. No hyperbole, no "better than X". Disambiguate: GitHub Code **Scanning** is an integration target (SARIF, README line ~59); GitHub Code **Quality** is the compared product.
- CHANGELOG `[Unreleased]` gets one entry per user-visible change (Added subsection needed — currently only Changed exists).
- Gates for CI-file changes: YAML must be valid (`python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"` or actionlint if available); TOML must parse (`codelore check --repo .` run locally proves it end-to-end — release binary at /Users/emrec/.cache/cargo-target/release/codelore).

## Validated facts (answer sheet, verified at 0e2c3e8)

- Thresholds: `.codelore-thresholds.toml` at repo root, discovered by `Thresholds::discover`; `[gates]`/`[diff]` schemas with deny_unknown_fields; `_max` fails strictly-greater, `_min` fails strictly-less; `fail_on_degraded` defaults true; gate failure exits 1; `$GITHUB_OUTPUT` result/violations + `::error` annotations emitted under GITHUB_ACTIONS with text format.
- Current repo values: worst code-health 15.85 (crates/codelore-cli/src/main.rs, red; 11 red files); worst cognitive 139 (spa/js/10_helpers_drawer.js, present in hotspot rows so `cognitive_max` binds on it); max hotspot_score 2.9742; dependency_cycles 1 (the 9-file codelore-rca SCC); propagation_cost 0.0432; red-band churn share 9.64%; Type-1 clone families EXIST (so `disallow_clone_type_1` must stay unset).
- CI: `.github/workflows/ci.yml` job `dogfood` (lines ~258-357) has `continue-on-error: true` (deliberate bake-in) — a check step inside it cannot block. It builds `./target/release/codelore` (cargo build --release -p codelore-cli --features spa) with rust-cache shared-key `release-dogfood`.
- README: differentiator block "What makes CodeLore different" at lines ~53-67 (names code-maat, CodeScene, jscpd); comparison content slots after line ~67. No existing self-gating claim anywhere (fresh). CHANGELOG [Unreleased] has only a `### Changed` subsection.
- Landing page: hand-maintained `index.html` on the `gh-pages` branch (NOT generated from main; dogfood job only regenerates `demo/index.html`). Sections: "What it answers", "In your workflow", `#install`, "Benchmarks". No comparison section yet.

---

### Task 1: Self-gating — thresholds file + blocking CI job + claim

**Files:** Create `.codelore-thresholds.toml` (repo root); Modify `.github/workflows/ci.yml`, `README.md` (one claim line), `CHANGELOG.md`.

- [ ] **Step 1: Write `.codelore-thresholds.toml`** exactly (comments are current-state rationale, no history):

```toml
# Quality gates for CodeLore's own repository — evaluated by `codelore check`
# in CI on every push and pull request. Ceilings sit above today's worst
# measured values so the gates bind on regressions, not on the status quo;
# floors sit just below today's worst file.

[gates]
# Worst file today: crates/codelore-cli/src/main.rs at 15.85. Any file
# decaying below this floor fails the gate.
code_health_min = 15.0
# Worst hotspot-row cognitive complexity today: 139 (the SPA drawer module).
cognitive_max = 150.0
# Highest hotspot score today: 2.97 (crates/codelore-cli/src/main.rs).
hotspot_score_max = 4.0
# Exactly one import cycle exists (the nine-file codelore-rca cluster).
# Introducing a second cycle fails the gate.
max_dependency_cycles = 1
# Propagation cost today: 0.043 — a change reaches ~4% of the system.
max_propagation_cost = 0.10
# Share of recent churn landing in red-band files today: 9.6%.
max_red_effort_pct = 15.0

[diff]
# A pull request may not introduce a new import cycle.
no_new_cycles = true
```
  (Do NOT set `disallow_clone_type_1` — Type-1 families exist today and would fail. Do NOT set code_familiarity_min/max_findings_in_hot_files/corpus_percentile_max — not calibrated in this cycle.)
- [ ] **Step 2: Blocking CI job.** In `.github/workflows/ci.yml`, add a new job `self-gate` (NOT inside dogfood — dogfood has continue-on-error): ubuntu-latest, timeout 20, NO continue-on-error; steps: checkout@v7 fetch-depth 0 → dtolnay/rust-toolchain@1.96.0 → Swatinem/rust-cache@v2 with `shared-key: release-dogfood` (reuses dogfood's cache) → `cargo build --release -p codelore-cli` (no --features spa; check doesn't need it — but NOTE: sharing the cache key with a --features spa build is still a valid cache reuse, cargo just adds the non-spa artifacts) → `./target/release/codelore check --repo .` (text format; GHA annotations + $GITHUB_OUTPUT come free). Match the workflow's existing step style/indentation exactly.
- [ ] **Step 3: README claim** — one sentence in the CI/workflow-adjacent area (near the `codelore check` mention at ~line 180 or in "What makes CodeLore different"): CodeLore's own repository is gated by `codelore check` in CI against its committed `.codelore-thresholds.toml` — the gates in this repo are the product's own output. Match surrounding voice.
- [ ] **Step 4: CHANGELOG** `[Unreleased]` — new `### Added` subsection (before Changed, per Added/Changed/Fixed order) with one entry.
- [ ] **Step 5: Gates.** Local end-to-end: `/Users/emrec/.cache/cargo-target/release/codelore check --repo .` → must print PASS with all configured gates evaluated (NOT the vacuous message). YAML sanity-parse ci.yml. Commit `feat(ci): gate this repository with its own quality gates`.

### Task 2: "vs GitHub Code Quality" comparison — README + landing section

**Files:** Modify `README.md`; Create `docs/landing/compare-section.html` (staging file for the gh-pages edit, applied post-merge by the controller); Modify `CHANGELOG.md` (no entry needed if judged non-behavioral — README-only content got no entry in past cycles; SKIP changelog for this task).

- [ ] **Step 1: README comparison block** appended to "What makes CodeLore different" (after the differentiator bullets, ~line 67): a short intro sentence + compact table comparing CodeLore with GitHub Code Quality on: analysis signal (point-in-time static findings vs git-history behavioral: hotspots, coupling, ownership, defect-calibrated risk); where it runs (hosted CI on GitHub's runners vs a single local binary, offline); cost model (per-committer subscription plus metered CI compute and AI credits vs free and open source); agent surface (none vs MCP server with local tools); data residency (code analyzed in GitHub's cloud vs nothing leaves the machine). Frame with the established voice: open by conceding what GitHub Code Quality does well (native PR integration, zero setup for GitHub-hosted repos, CodeQL rule depth) then the deltas. End with the existing disambiguation: CodeLore *feeds* GitHub Code Scanning via SARIF — the comparison is with the Code Quality product, not the Scanning integration. NO prices, NO dates, NO version numbers.
- [ ] **Step 2: Landing section staging file** `docs/landing/compare-section.html`: a `<section>` matching gh-pages index.html's existing markup conventions (the implementer reads `git show origin/gh-pages:index.html` for classes/structure) titled "How it compares", carrying the same comparison in landing-page tone, to be spliced after the "In your workflow" section. Include an HTML comment at the top of the file stating it is the staged source for the gh-pages landing section (this file lives under docs/landing/ on main as the canonical source).
- [ ] **Step 3: Gates.** Markdown renders sanely (visual check of table syntax); docs guard `git grep -nE "F[0-9]{3}|v0\.[0-9]+" README.md docs/landing/` → no new hits. Commit `docs(readme): honest comparison with GitHub Code Quality + staged landing section`.

---

# Verification / rollout

- PR → merge on green. After merge: controller splices the staged section into `gh-pages:index.html` (direct commit to gh-pages, mirroring merge-approved content) and verifies the live page renders.
- The `self-gate` job must be green on the PR itself (its first run IS the proof).
- No release this cycle unless the user asks (positioning content is live via README/landing immediately).
