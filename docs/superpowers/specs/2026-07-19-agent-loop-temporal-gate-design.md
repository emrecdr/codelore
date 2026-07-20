# Agent-Loop Temporal Quality Gate — Design

## 1. Purpose and positioning

AI coding agents consume quality signal inside their loop through MCP tools and CLI gates. The incumbents supply only static context there: Sonar Vortex injects AST/control-flow/dependency context and verifies output in real time but carries zero git-history signal and is enterprise-cloud-gated; CodeScene's MCP server has a pre-commit health safeguard but its behavioral tools require a paid account and token; GitHub Code Quality has no agent surface at all; repowise ships a change-risk MCP tool and contests the token-economy axis.

CodeLore's version is the **temporal, calibrated dimension in the loop, fully local, no account**: before an agent writes, it can ask what history says about the files it is about to touch (ownership, co-change blast radius, calibrated defect risk); after it writes, the uncommitted change-set is gated on projected health delta, newly introduced cycles, and violated historical couplings — signals no static analyzer can compute. This is not a health-score gate copy; it is the dimension the category is missing.

Two halves, one initiative, context-first:

- **Pre-write context** — `change_context` (MCP): a temporal briefing for files the agent intends to modify. Reads only the committed-history fact cache; no new engine.
- **Post-write verification** — `gate_changes` (MCP) and `codelore gate` (CLI): a verdict over the uncommitted change-set, powered by a new delta-scoped analysis engine.

## 2. The change-set definition

One definition everywhere: **the tracked working tree versus HEAD** — every tracked file whose content differs from the HEAD commit (added, modified, deleted, renamed), staged or not. Untracked files are never part of the change-set (matching `Repo::is_worktree_dirty`'s tracked-only semantics). Agents do not manage the git index; for human pre-commit hooks this is stricter than index-only gating, which is the honest direction for a gate. `codelore diff` remains the committed-side complement (branch vs base), unchanged.

## 3. The `change_set` engine

New library module `crates/codelore-lib/src/change_set/`, consumed by both surfaces. Design principle: **cost scales with the edit, not the repo.**

1. **Discovery.** New `Repo` trait method `worktree_changes() -> Result<Vec<FileChange>>` enumerating the change-set (reusing the existing `FileChange` shape: path, change type, rename source). Implemented on both `GixRepo` and `GitCliRepo`, pinned by a differential test, excluding untracked files.
2. **Two-version parse, changed files only.** For each changed Tier-1 file: read the HEAD blob (`read_blob_at`) and the working-tree content; run both through the existing `rca` extractor for per-function structural metrics. Non-Tier-1 files are reported as unanalyzed, never silently skipped.
3. **Projected health delta.** The code-health composite is **linear** in structural risk and the two history factors (`score = 100·(1 − 0.50·structural_risk − 0.30·norm_churn − 0.20·author_frag)`, `code_health.rs`), so with the history factors held from the committed-HEAD cache the delta on the structural term is **exact**, not an approximation — the only caveat is saturation at the `0`/`100` clamp boundary, noted in output. **Critical carve-out:** `structural_risk` is itself a weighted sum over eight smells, and one of them — `shotgun-surgery` (weight `0.12`) — is Fisher co-change coupling centrality, a *history-derived* signal that cannot be recomputed from a two-version parse of the changed files. The engine therefore recomputes only the seven structurally-derived smell contributions from the new metrics (using calibrated weights when configured) and **holds the coupling-centrality contribution frozen from the HEAD cache**, exactly as it holds the two outer history factors. Recomputing or omitting it would bias projected structural risk by up to `0.12` of the weight mass. Every per-file health delta is labeled **projected**; "projected" denotes the frozen-history latency (the coupling and churn/ownership terms reflect HEAD, not the uncommitted state), not a modelling approximation.
4. **History facts from the committed cache, read-only.** Co-change partners, ownership, hotspot standing, and cycle membership come from the standard committed-history fact store (`open_or_ingest`). The engine never opens a fact store over dirty state — it reads the clean HEAD cache, which is exactly the state the cache-write guard protects. If no cache exists yet, it is built once and the first-call cost is disclosed in output.
5. **Cycle delta.** Re-extract the imports of changed files and splice them into the cached import graph by **replacing each changed node's entire out-edge set** with its new working-tree out-edges (dropping the stale HEAD edges for that node before adding the new ones), then SCC-check: report any cycle present in the spliced graph but not the HEAD graph. Edge *replacement* — not addition — is required because the import graph is an adjacency list keyed by source file (a file owns many edge rows); appending would leave a phantom edge for any import the edit removed or repointed, yielding false-positive cycles. Only added edges can create a new cycle, so the changed node's own edges are the only ones that must be re-derived. No full re-ingest.
6. **Temporal findings.**
   - **Coupling absence:** a changed file A whose historical partner B (per the thresholds `codelore diff` already uses for `CouplingAbsence`: same confidence and Fisher significance conventions) is not in the change-set produces a warning-level finding.
   - **Calibrated delta risk:** per-file structural-risk delta under the active weights; labeled `uncalibrated` when no artifact is configured.
7. **Report.** One `ChangeSetReport` struct: change-set files with per-file two-version deltas, new cycles, coupling absences, projected health deltas, calibrated risk deltas, unanalyzed files, and disclosure notes. Every finding carries a **stable `finding_id`** (content-derived hash of finding kind + subject) so agents can recognize a finding across loop iterations.

**Engine hardening (in scope):**
- **Content-hash memoization.** Per-file analysis results are memoized in-process (and in the sidecar cache dir, keyed on file content hash + HEAD sha) so repeated gate calls in an agent loop re-analyze only files edited since the previous call.
- **No engine-level change-set cap.** The engine analyzes every changed Tier-1 file — the cost is bounded above by a full ingest, which a cold cache already pays. Large change-sets are tamed at the *render* layer instead: the delta table caps at the ten largest-`|delta|` rows with a `(+n more files)` tail (§6) while the JSON report carries every row, so no gate ever degrades for change-set size.
- **Change-type handling.** Deleted files contribute their removed structural risk (a deletion of a red file is an improvement, reported as such); renames follow the rename source for history lookups; binary files are listed unanalyzed.

## 4. Surfaces

### 4.1 `change_context` (MCP tool)

Parameters: `paths` — 1 to 20 repo-relative paths the caller intends to modify (an empty list or more than 20 paths is a tool-argument error naming the limit). Reads only the committed-HEAD cache (no engine invocation). Per file:

- code-health band and score; hotspot standing (rank/score when in the hotspot set);
- top-3 historical co-change partners with confidence and significance — "editing this file historically means editing these" — with any further significant partners disclosed as ` (+n more)`;
- dominant-owner share, with a knowledge-concentration flag when high;
- calibrated defect risk when calibration is configured (labeled `uncalibrated` otherwise);
- a one-line recent-churn note.

Every line has an honest-absence form when its feed has no data for the path: `health: no code-health row` (the path has history but no scored code-health row), `not in the hotspot set`, `co-change: none significant`, `owner: inconclusive`, and `recent: quiet in last <n>d`. A path absent from *every* feed — no code-health row, not in the hotspot ranking, no significant co-change partners, no attributable ownership, and untouched in the recent-churn window (brand-new, untracked, or mistyped) — collapses to a two-line "no history at HEAD" block instead; never an error, because agents create files constantly. Output is compact fixed-order structured text (see §6).

### 4.2 `gate_changes` (MCP tool)

No parameters — the working tree is the argument. Runs the engine, evaluates thresholds, and returns: verdict (`pass` / `fail` / `pass (no uncommitted changes)`), the findings list (capped, each with `finding_id`), and the per-file delta table. Threshold failures list the same reason wording the CLI gate prints.

### 4.3 `codelore gate` (CLI subcommand)

Same engine and verdict. Text output for humans; `--format json` for scripts (the full `ChangeSetReport` serialized). Exit-code contract identical to `check`: 0 pass, 1 gate failure, typed error codes for repo/config failures. No thresholds configured → vacuous pass with the same honest one-line message `check` prints. The documented pre-commit hook becomes `codelore gate --repo .`.

### 4.4 Threshold semantics — reuse `[diff]`

No third config section. The `[diff]` keys express change-gates; each binds on the surfaces documented below, with equal-passes boundaries everywhere:

- `no_new_cycles` — in working-tree mode, a cyclic-node **membership** comparison: one violation per path that is cyclic in the projected import graph but was not cyclic at HEAD, naming the file. This deliberately diverges from `codelore diff`'s cycle-*count* comparison — membership names the offending files and still fires when two existing cycles merge into one bigger tangle (which *drops* the count).
- `delta_code_health_min` — a floor on the **whole-repo median** delta (projected − baseline), the same semantics the key carries on `codelore diff`: one key, one meaning on both surfaces. The median is computable in working-tree mode because both scoped scoring runs return full row sets. (The populations differ honestly: the gate scores with `min_revs = 1` over all scoreable files, while `diff` medians over its min-revs-filtered hotspot rows.)
- `delta_code_health_min_per_file` — the change-scoped sharp tool: a floor on each changed file's own projected − baseline score, one violation per offending file. A whole-repo median dilutes a single file's regression across every unchanged file; the per-file floor catches exactly that. Evaluated only by `codelore gate` / `gate_changes`; `codelore diff` does not evaluate this key.
- `new_hotspot_max`, `delta_health_min`, `deny_degrading_verdict` — **diff-only** (they need committed churn or a committed-range function delta to be meaningful); never evaluated in working-tree mode.

The docs table states, per `[diff]` key, which consumers it binds in.

### 4.5 `[calibration]` configuration

New optional section in `.codelore-thresholds.toml`:

```toml
[calibration]
defect_artifact = "relative/path/to/defects.calib.json"
```

Load-time validation identical to the flag path: format-version check, repo-identity guard, `--allow-foreign-calibration` escape unchanged. Precedence: explicit CLI flag > MCP server startup flag > `[calibration]` > uncalibrated. Absent everywhere → byte-identical uncalibrated behavior. The `deny_unknown_fields` posture extends to the new section.

### 4.6 MCP registration

Tools 10 and 11 on the existing server. Descriptions are written for agent task-selection and cross-reference each other and `check_gates` ("evaluates committed HEAD; for uncommitted changes use `gate_changes`"). Server startup is unchanged.

## 5. Error handling and degradation

Honest-absence convention throughout: every degradation is visible in output; none changes the exit-code contract except real gate failures.

- Non-Tier-1 changed files: listed unanalyzed.
- No calibration: risk labeled `uncalibrated`.
- No committed cache: built once; first-call cost disclosed in the report.
- Clean tree: `gate_changes`/`codelore gate` return an explicit "no uncommitted changes" pass.
- Large change-sets: every changed file is analyzed and gated; only the rendered delta table truncates, with a `(+n more files)` tail (§6).
- Repo/config errors (bad artifact, non-repo path): typed hard errors with the existing exit codes; MCP tool calls surface them as tool errors except artifact misconfiguration at server startup, which already fails fast.

## 6. Token economy (spec'd contract, not a vibe)

Both tools emit compact fixed-order structured text — never JSON blobs, never file contents. Hard caps with `(+n more)` disclosure: top-3 co-change partners per file (` (+n more)` for the rest), the 20-file `change_context` limit, and a **render-level** delta-table cap — `gate_changes` / `codelore gate` show the ten largest-`|delta|` rows with a `(+n more files)` tail while the JSON report carries every row. The cap is rendering only: the engine analyzes and gates every changed file, so no gate ever degrades for change-set size. Findings are not capped; they are budgeted per-finding. Budget targets, pinned by a token-counting test on a fixture (whitespace-split proxy measure, asserted with headroom):

- `change_context`: ≤ 150 tokens per requested file (compact mode).
- `gate_changes`: ≤ 80 tokens base + ≤ 40 per finding.

## 7. Contracts

1. **Additive-only.** Every existing command, tool, analysis, and output is byte-identical. New surfaces only. `[calibration]`/`[diff]` additions change nothing when absent.
2. **Determinism.** Same working-tree state + same committed cache → identical `ChangeSetReport` (and identical rendered outputs), test-pinned.
3. **Dual-backend parity.** `worktree_changes()` differential test on both git backends; untracked files excluded on both.
4. **Scoring isolation.** The engine's projected scores are never written into the fact store and never perturb any committed-history analysis. The engine module is read-only toward `FactsDb`.

## 8. Testing

- **Engine unit tests** on fixtures with scripted edits: modify a file → assert projected delta sign and magnitude class; introduce an import cycle → assert cycle-delta detection; edit half of a known coupled pair → assert the absence finding; delete a red-band file → assert improvement reporting; rename handling; memoization (second call re-analyzes only re-edited files, asserted via probe counters or cache inspection).
- **MCP integration tests**: temp clone, scripted edit, `gate_changes` verdict flips from pass to fail when a `[diff]` threshold is configured and violated; `change_context` returns the briefing shape incl. the no-history row.
- **CLI tests**: `codelore gate` exit codes (0 clean/pass, 1 violation), vacuous-pass message parity with `check`, `--format json` shape.
- **Differential test**: `worktree_changes()` on both backends across added/modified/deleted/renamed/untracked cases.
- **Token-budget test**: fixture-pinned budget assertions per §6.
- **Real-CLI smoke** on this repository: edit `crates/codelore-cli/src/main.rs`, run `codelore gate`, observe the projected delta and verdict.
- No live network anywhere; all tests offline.

## 9. Phasing (plan structure)

- **Phase 1 — context** (ships standalone value): `[calibration]` config + precedence chain; `change_context` tool; docs. No engine dependency.
- **Phase 2 — verification**: `worktree_changes()` on the Repo trait; the `change_set` engine; `gate_changes`; `codelore gate`; `[diff]` working-tree binding; hook docs.

## 10. Out of scope (v1)

- Function-level pre-write context (function-xray granularity in `change_context`) — file-level first.
- A `--explain <finding_id>` deep-dive path — the LLM enrichment layer already serves deep-dives via `explain <path>`.
- Dogfooding `codelore gate` in this repo's own CI — a post-release cycle, once the subcommand exists in a released binary.
- Index-vs-HEAD (staged-only) gating; branch gating (exists as `codelore diff`); watch/daemon mode; editor integrations.
- AI-authorship provenance and cross-repo benchmark harness — separate ranked initiatives, not this one.

## 11. Refinements from spec validation (2026-07-19)

Each was verified against source before adoption; two proposed refinements were adopted in **corrected** form because their stated premise did not hold.

1. **Projected-delta framing — corrected and folded into §3.3.** The composite is linear, not multiplicative, so the frozen-history delta is exact, not an approximation. The real nuance is the `shotgun-surgery` coupling-centrality carve-out (now in §3.3): that history-derived sub-term of `structural_risk` must be frozen from the HEAD cache rather than recomputed.
2. **Cycle-delta edge replacement — folded into §3.5.** Confirmed the import graph is adjacency-list keyed by source node, so edges must be replaced, not appended.
3. **Median blind spot — corrected and folded into §4.4.** `codelore diff` medians over the *whole repo*, not changed files; the spec previously mis-described this. The working-tree gate keeps the key's whole-repo-median semantics (one key, one meaning on both surfaces) and adds the optional `delta_code_health_min_per_file` floor for change-scoped sharpness.
4. **Deterministic delta-table ordering.** The report's delta rows sort by descending `|projected delta|` (rows with no delta last), tie-broken by path, **before** the render-level row cap, so the determinism contract (§7) holds. Matches the codebase's existing sort-before-emit discipline for HashSet/HashMap iteration order.
5. **Per-file delta-table token budget (§6).** `gate_changes` caps the delta table at the top-N changed files by `|projected delta|` with a `(+n more)` disclosure line, folded into the §6 token-budget test; on a large change-set the full table would otherwise blow the base budget.
6. **Determinism at the rendered level (§7).** The determinism contract is defined over the *rounded/bucketed rendered* `ChangeSetReport` (sign + magnitude class), not bit-identical raw `f64`, since projected deltas are floating-point.
7. **Memoization: HEAD-advances-mid-loop cost.** Per-file memoization is keyed on `file-content-hash + HEAD sha`, so a commit inside the agent loop moves HEAD and invalidates the *entire* per-file cache (even untouched files), forcing a full re-derive on the next call. The write→gate→write→commit-last loop is unaffected; a commit-then-continue agent pays repeated re-derives. Disclosed in the cost note.
8. **`change_context` honest-absence edges (§4.1).** A path present neither at HEAD nor in the working tree (a typo/hallucinated path) returns a "no history yet" row, not an error. During an in-progress merge or rebase (`MERGE_HEAD`/rebase state present), where "versus HEAD" is ambiguous, the tool returns a one-line honest-absence note rather than misreporting — no merge/rebase-state detection exists in the `Repo` trait today, so this is new (small) plumbing.
9. **`departed-owner` field in `change_context` (§4.1, adopted from §8.3).** Alongside dominant-owner share, surface a **departed-owner flag** — "the primary historical author of this file is no longer active" — with an `Inconclusive` honest-absence label when the identity layer can't determine it. The primitive exists (`knowledge_islands.rs`: `author_last_commit` / `days_since_main_active` / `departed_threshold_days`) but is inline SQL inside one monolithic query; Phase 1 extracts it into a shared helper the tool can call for an arbitrary path. This is the one uniquely-temporal "the expert is gone, tread carefully" signal no static in-loop tool can provide.
10. **`check` vs `gate` vs `diff` "when to use which" note.** The docs deliverable adds a short table — `check` = committed HEAD, `gate` = working tree, `diff` = branch vs base — mirroring the §4.6 MCP cross-reference for CLI users.
