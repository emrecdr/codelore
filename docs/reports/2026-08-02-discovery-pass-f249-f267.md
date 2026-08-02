# CodeLore — Discovery Pass 2026-08-02 (F249–F267)

Read-only research pass. Findings are candidate F-IDs for the next audit cycle
(`deep_analysis_report.md` re-opened at F249). **Nothing here is implemented.**
Each finding is marked with its verification status:

- **✅ verified** — the controller re-checked the claim against `main` source directly.
- **researcher-validated** — a research agent traced it; confirm at implementation time.

## Method

Six independent research agents (Sonnet) each swept one dimension against the full tracked
baseline (roadmap Tier 1–5 + the "Deliberately out of scope" set + `deep_analysis_report.md`
F1–F248 + hardening cycles 2/3/4), instructed to surface only genuinely-new opportunities or
fresh re-scopes of known-open levers, each grounded in `file:line`:

1. degenerate / adversarial repo-input robustness
2. statistical / methodological rigor & small-sample honesty
3. performance & scalability
4. feature-set deepening via already-ingested latent data
5. error handling & the newest (least-audited) cycle-2/3/4 code
6. testing & verification gaps

The controller (Opus) then verified the load-bearing items against source, resolved one
inter-agent conflict, and refuted one agent claim by direct inspection (all noted below).

**Headline:** the engine is genuinely well-hardened — four dimensions each reported their
surface is *mostly closed* with extensive clean rule-outs. The leverage concentrates in one
convergent correctness cluster, one perf re-scope, and a handful of latent-data feature wins.

---

## The convergent finding: "no-data silently passes green"

Five independent signals (rigor, test, failure, robustness agents + hardening cycles 2/3/4)
converge on one cluster. This is the single highest-leverage hardening opportunity of the pass.

### F249 — `ensure_ingest_witnessed` guards only 2 of ~13 ingest entry points · HIGH · ✅ verified

*   **Location**: guard defined `facts/mod.rs:582` (`ensure_ingest_witnessed`) + `evaluators.rs:495`
    (`head_has_scorable_source`), both `pub`. Called in production at exactly **2** sites:
    `check.rs:108` and `mcp.rs:987` (the `check_gates` tool). **Unguarded** ingest entry points
    (verified by grepping every `open_or_ingest_with_cache_root`/`new_in_memory` call site):
    `analyze.rs:224,230` (the flagship command), `gate.rs:82` (`codelore gate`), `explain.rs:478`,
    `mcp.rs:1403` (`gate_changes`), and 8 MCP report tools — `repo_overview` (:621), `hotspots`
    (:662), `code_health` (:704), `refactoring_targets` (:855), `function_xray` (:920),
    `finding_hotspot_overlap` (:1158), `explain_file` (:1235), `change_context` (:1349).
*   **Gap**: `ensure_ingest_witnessed` was built by cycle-3's G2 fix (#178) for the shallow /
    blind-ingest signature — HEAD names a real commit but the walk ingested 0 commits (an
    `actions/checkout@v4` default `fetch-depth: 1` on a PR ref, whose merge tip gix's merge
    filter discards). At the unguarded sites, ingest silently sees zero commits, every analysis
    returns empty rows, and the SPA / report / MCP response renders as a complete, confident
    "no hotspots, no findings" artifact **indistinguishable from a genuinely clean repo** —
    silently wrong in the trust-destroying direction. `analyze` is the first command a new user
    runs and the one that gets screenshotted / CI-wrapped; `change_context` / `explain_file` are
    the flagship agent-facing tools an LLM uses to judge "is this file risky."
*   **Conflict resolved**: the robustness agent judged `gate_changes` "fine" (shares the #191
    `report.changes.is_empty()` skip). The failure agent showed — and the controller confirmed —
    that skip is *dead for its stated purpose*: both `gate`/`gate_changes` callers early-return on
    an empty working tree (`gate.rs:88-90`, `mcp.rs:1399-1401`), so the predicate never fires; the
    real hazard is a shallow checkout **with** uncommitted edits (`changes` non-empty → no early
    return → blind ingest → confident `✅ PASS (N changed files evaluated)`). `gate_changes` is
    genuinely unguarded.
*   **Direction**: thread `ensure_ingest_witnessed(&head_sha)` into the unguarded sites, mirroring
    the `check.rs` / `check_gates` call shape exactly. **Implementation gotcha**: `analyze` (unlike
    `check`/`mcp`) exposes `--after`/`--before` walk filters — an all-excluding date range on a
    healthy full clone also trips `commit_count() == 0`, so the error message must branch on
    `opts.after`/`opts.before` before blaming `fetch-depth`, or use a distinct message for `analyze`.
*   **Value/effort**: HIGH / **S**. Both primitives exist and are correctly used at one site as the
    template. Byte-identical output on any healthy repo — turns a wrong-answer into a loud error;
    no schema / cache-key / hard-invariant touch.
*   **Pairs with**: F263 (`run_new_code_scope` test) + F264 (`is_shallow()` test) — the two tests
    that de-risk this exact fix.

---

## Tier A — do-now (high value, low risk, primitives exist)

### F250 — `codelore explain delivery-friction` 404s on a shipped, documented metric · LOW · ✅ verified
*   **Location**: `explain.rs` — 38 topics present incl. `delivery-metrics` (:211), `lead-time`
    (:299); `grep -c '"delivery-friction"'` = **0**.
*   **Gap**: `delivery_friction.rs`'s module doc already carries the full formula
    (`friction_score = pr(revs) × pr(lead_time) × pr(cog) × 100`); `codelore explain
    delivery-friction` errors on a fully-documented, already-shipped metric. Pure catalogue omission.
*   **Direction / effort**: add the one explain tuple (formula + citation). **XS**, zero engine work.

### F251 — `coordination-needs` / `knowledge-islands` classify tiers from thin samples with no denominator disclosed · MED · ✅ verified
*   **Location**: `CoordinationNeedsRow` (fields: path, authors, fragmentation, interleave,
    cochange_entropy, tier, health_band — **no commit count**); `KnowledgeIslandRow` (entity,
    main_author, ownership_pct, days_since_main_active, last_main_author_commit,
    n_substantial_others — **no total LOC**).
*   **Gap**: `interleave = switches/(n_commits−1)` is `1.0` at exactly 2 commits; combined with
    `fragmentation ≥ 0.50` (reachable off a 2-commit even LoC split), `classify_tier` can report
    `tier: "high"` off a file with 2 commits total — and the row exposes no `n` to judge it.
    `knowledge-islands`'s `ownership_pct` is a LoC share gated only by `--min-revs` (revision
    count, default 5), so 5 one-line commits → `ownership_pct: 100.0` off 5 LOC, with no `total_loc`
    to judge it. Inconsistent with `bus_factor` (exposes `total_commits`) and `ownership` (exposes
    `total_revs`), which already disclose their denominator — undercuts the "auditable formulas"
    brand exactly where `knowledge-islands` is a stated differentiator.
*   **Direction / effort**: add a `total_commits` / `total_loc`-shaped `u32` field to each row,
    populated from data already in the CTEs (`stats.n_commits`; `TOTALS_CTE.total_loc`). **S**,
    additive, no formula change; touches the CSV/JSON/markdown/SARIF emitters for the two row types.

### F252 — `write_github_output` silently swallows write failures · LOW · ✅ verified
*   **Location**: `main.rs:200-211` — `if let Ok(path)=env::var("GITHUB_OUTPUT") && let Ok(mut f)=…open() { … let _ = f.write_all(…); }`.
*   **Gap**: both the open and the write are dropped on `Err`. In a real CI run with `GITHUB_OUTPUT`
    set but momentarily unwritable, a downstream `if: steps.x.outputs.result == 'pass'` silently
    takes the else branch. The CLI exit code still reflects truth; only the GHA integration point
    goes dark, unflagged.
*   **Direction / effort**: `tracing::warn!` on either `Err` arm instead of `let _ =`. **S**.

---

## Tier B — high value, moderate effort or needs a gate/design

### F253 — HEAD-scan blob I/O: Phase-1 fix + broader scope (refines F173/F206) · HIGH · researcher-validated
*   **Location**: `repo/gix_repo/mod.rs:304-340` (`read_blob_at` — fresh `to_thread_local()` per
    call → cold ODB decode cache; `rev_parse_single → find_commit → commit.tree() →
    lookup_entry_by_path`, one tree decode per path segment); the 3 HEAD passes in
    `facts/ingest/{complexity,clones,imports}_head.rs`; **and** `architecture_trend.rs:211-253`
    (`resolve_imports_at_rev`) + `cycle_origins.rs` (binary-search × `MAX_CYCLES=10`).
*   **New angle vs F173/F206**: the tracked deferral blocker ("divergent extractor error contracts")
    is *smaller* than described — the blob-**read** error handling is already identical across all
    three passes; the divergence is one level downstream in AST-parse handling (clones aborts via
    `collect::<Result>>?`; complexity/imports warn-and-skip), which is **orthogonal** to blob
    batching. Also: `architecture-trend`/`cycle-origins` are *worse* offenders than the ingest
    passes — never cached, re-paid in full on every `codelore analyze` (up to 12× a HEAD import
    scan, then multiplied by cycle-origins' binary search), where the ingest passes pay once per
    HEAD then cache-hit forever.
*   **Direction**: **Phase 1 (S, low risk)** — a new default-impl `Repo` trait method (e.g.
    `open_blob_cursor(rev)`) that GixRepo overrides to build one warm-ODB-cache `gix::Repository` +
    resolved root tree *per rayon worker*, reused across every file that worker processes; wire it
    through the 3 passes via the `.map_init` idiom already present at `complexity_head.rs:44`, and
    through `resolve_imports_at_rev`. Cuts per-file fresh-cache cost from O(F×depth) to
    ~O(unique_dirs×depth) per pass. GitCliRepo (differential-oracle-only) needs no change.
    **Phase 2 (M-L, deferred)** — full cross-pass blob dedup (ties into F161's memory tradeoff).
*   **Value/effort/gate**: large-repo `analyze`/`gate` wall-clock, most on cold cache and on
    `architecture-trend`/`cycle-origins`. Phase 1 = **S**. Byte-identical output (same
    `read_blob_at` bytes through a warm handle). Benchmark-gate via `benches/end_to_end.rs` on a
    *depth-representative* fixture (this repo itself: depth 6, 456 files).

### F254 — cache-hit path runs a full tracked-file dirty-scan on every invocation · MED · ✅ verified
*   **Location**: `facts/mod.rs` cache-hit branch (`if cache_p.exists()` → `if repo.is_worktree_dirty()`)
    → `gix_repo/mod.rs:260-278` (HEAD-tree-vs-index + index-vs-worktree walk over every tracked file).
*   **Gap**: the persistent cache exists to make repeat invocations near-O(1) (open a small read-only
    DuckDB file), but every cache hit unconditionally runs a full working-tree status walk purely to
    *maybe* emit a `tracing::warn!` — even for scripted / non-TTY / warn-suppressed invocations where
    nobody sees it. On a large monorepo in an agent loop / CI hook (exactly the cache's target usage)
    this turns near-O(1) into O(tracked-files) per call.
*   **Direction / effort**: the `Repo` trait already treats `is_worktree_dirty` as a best-effort hint
    ("a missed warning is better than a hard failure" — `repo/mod.rs:65-69`). Apply the same
    philosophy: cheap proxy (compare `.git/index` mtime against the cached ingest's recorded time),
    or gate the walk behind a TTY / flag. False negatives are consistent with the trait's contract.
    **S**, one call site; no ingest / cache-key / schema touch.

### F255 — `panic = "abort"` × long-lived `codelore mcp`: one panicking tool call kills the server for every client · HIGH · ✅ verified
*   **Location**: `Cargo.toml:50-54` (`[profile.release]` `panic = "abort"`, inherited by
    `release-pgo`) × the 11 `tokio::task::spawn_blocking` MCP tool handlers in `mcp.rs`.
*   **Gap**: under `panic = "abort"` (the shipped release profile) there is no unwind — a panic
    inside a `spawn_blocking` closure SIGABRTs the whole process instead of becoming a `JoinError`
    the caller could turn into a JSON-RPC error. This matters *only* for `codelore mcp` — every
    other subcommand is one-shot (abort there is a pure size/speed win). `mcp` is a long-lived
    process serving many tool calls from an MCP host; the tool params are LLM-constructed (not
    human-typed), transitively reaching the whole analysis engine + duckdb-rs FFI / gix /
    tree-sitter / serde. A rare panic three layers down takes down the transport loop, every
    in-flight call, and the in-process memo cache; the host sees the stdio pipe close, not an error.
    No panic hook / crash breadcrumb exists to identify the triggering tool.
*   **Direction / effort**: **do not** touch the workspace-wide `panic = "abort"`. Give the MCP
    dispatch path its own boundary — wrap each handler body (or a shared dispatch point) in
    `std::panic::catch_unwind(AssertUnwindSafe(...))` → `ErrorData::internal_error`, and survive to
    serve the next request. **M** — mechanically small, but **verify before landing** that a caught
    panic mid-query can't leave `FactsDb`'s connection observably torn (CLAUDE.md's read-only-post-
    ingest invariant suggests query-phase panics are safe to catch — confirm, don't assume; adjacent
    to the `!Send + !Sync` connection hard invariant).

### F256 — small per-language cohorts collapse biomarker intensities to near-binary → false red-bands · MED · researcher-validated (refines F236 residual)
*   **Location**: `code_health.rs:507-562` (`materialize_biomarkers`, per-language `PERCENT_RANK`
    with only a `files.len() <= 1` guard) + `:670-736` (`apply_corpus_lens`) + `:174-204`
    (`CodeHealthRow`).
*   **Gap**: biomarker intensities that drive `structural_risk` (→ the absolute 0.55/0.28 red/yellow
    thresholds) are `PERCENT_RANK` within each language's file cohort. For a 2-file cohort intensities
    are binary {0.0, 1.0}; for 3, {0, 0.5, 1.0}. A repo with 200 Rust files + 3 Python scripts always
    ranks its worst Python file at 1.0 on complex/large/god-class/dry regardless of absolute
    complexity, which can push it over the red threshold on cohort-size artifact alone.
*   **Controller note**: the agent claims F236's residual ("the cross-repo corpus percentile addresses
    this") is inaccurate — `apply_corpus_lens` is a pure additive side-channel (sets only
    `corpus_percentile`/CI fields, never the score/band/percentile) and requires an opt-in artifact.
    **Confirm this claim against `apply_corpus_lens` before acting** — it is the crux of whether F256
    is a live gap or already-mitigated.
*   **Direction / effort**: extend the guard to a real per-repo floor (e.g. 5–10 files); below it,
    fall back to the cross-language full-universe percentile (already computed in the same fn) with a
    provenance flag, or emit the biomarker as low-confidence; add a cohort-`n` field to `CodeHealthRow`
    (matching the `corpus_percentile_ci_*` disclosure precedent). **M**, changes small-cohort band
    output (cache-epoch bump); needs a thin-cohort fixture.

---

## Tier C — feature deepening (latent data, little/no new ingest)

### F257 — repo-wide function-level hotspots via `entities × hunks × commits` · HIGH · ✅ (columns) / researcher-validated (semantics)
*   **Location / data**: `entities` (HEAD spans: `path, name, kind, start_line, end_line`) ×
    `hunks` (`rev, path, new_start, new_lines`) × `commits.date` — all verified present in
    `schema_v1.sql`. `function_xray.rs` already implements the hunk↔span overlap attribution, but
    only for one `--target` file via an ad-hoc tree-sitter reparse.
*   **Gap / value**: no repo-wide function-granularity hotspot ranking exists — every hotspot-family
    analysis is file-granularity, so a 2000-line file with one hot function looks identical to one
    with uniform low-grade churn. A pure-SQL `COUNT(DISTINCT rev)` grouped by `(path, name)` with the
    overlap predicate needs *no* tree-sitter reparse (`entities` already holds the exact HEAD spans
    `function_xray` recomputes).
*   **Caveats to carry**: `hunks` is indexed `(rev, path)` — a path-keyed repo-wide scan may want an
    additive `hunks(path)` index (no schema-version bump). `entities.rev_introduced`/`rev_last_seen`
    are **degenerate** (always `head_rev`, single HEAD-only appender), so this answers "hot now," not
    "was hot historically" — fine for the use, worth documenting. **M**.

### F258 — `first_party_import_share` wildcard misclassification + a `wildcard_import_share` row · MED-HIGH · ✅ verified
*   **Location**: `architecture_metrics.rs:233` (`… FILTER (WHERE target_path IS NOT NULL OR kind =
    'relative')`) × `imports/extractor.rs:528` (`classify` — the wildcard test at :532
    `ends_with('*') || contains("::*")` runs **before** the relative test at :537).
*   **Gap**: a first-party glob (`use crate::foo::*;`, `from .foo import *`) is unambiguously in-repo,
    but `classify` tags it `Wildcard` (wins the branch order, verified), so it's silently excluded from
    `first_party_import_share`'s numerator and `resolution_rate_first_party`'s denominator — a second,
    undocumented instance of the same undercount the module's own "Definition caveat" section already
    discloses for a different case. `imports.kind` is CHECK-constrained and fully populated at ingest
    but queried in only this one rollup.
*   **Direction / effort**: (a) add a `wildcard_import_share` row (one more `COUNT(*) FILTER`, purely
    additive to the byte-identical `import_resolution_rate` contract); (b) extend the "Definition
    caveat" doc to name the relative-wildcard case. **S**, but touches an analysis with exact-output
    golden tests. Correctness + feature blend.

### F259 — dead `commits.committer_email` → a `landed_by_other_pct` gatekeeper metric · MED · ✅ verified (dead) / researcher-validated (metric)
*   **Location / data**: `committer_email` (`schema_v1.sql:25`) — ingested, but verified **never
    SELECTed** by any analysis (all 7 references in `analyses/` are test-fixture `INSERT`s).
*   **Signal**: git author≠committer divergence is a validated MSR proxy for "someone other than the
    author landed this" (peer-applied patch / rebase-merge / bot merge) — Rigby & German 2008 + later
    pull-based-development gatekeeper studies. Fits `delivery_metrics.rs`'s `(metric, p50/p75/p90, n,
    caveat)` row shape (precedent: `rework_pct` is already a single-aggregate row there).
*   **Honesty caveat (must ship with it)**: `commits` has `author_name` but **no `committer_name`**,
    so unlike `canonical_author` the committer side can't be mailmap-resolved — a person authoring and
    landing under two emails they both own registers as a false "gatekept" commit. Surface via the
    row's existing `caveat: String` field, never as ownership-grade signal. **S**.

### F260 — `hotspot-velocity` per-window floor + uncited/unoverridable constants · MED · researcher-validated
*   **Location**: `hotspot_velocity.rs:35-38` (`RECENT_DAYS=30`/`BASELINE_DAYS=90`, uncited private
    consts) + `:113` (`min_revs` floor applied to the **combined** window sum). Same shape:
    `refactoring_targets.rs:33` `EA_Z_FLOOR=25`.
*   **Gap**: `acceleration = recent_per_week − baseline_per_week` is a bare difference-of-small-counts
    with no uncertainty signal; the combined-window floor lets a single-window burst (`recent=5,
    baseline=0`) outrank steadier `(4,3)` activity despite strictly less total signal. The window
    consts have no empirical citation and no CLI override, bypassing the repo's own `constants.rs`
    single-source + `Options`-field + `#[arg]` convention.
*   **Direction / effort**: (a) split the floor into per-window minimums; (b) framing fix in
    docs/`explain` (the raw counts are already in the row); (c) migrate `RECENT_DAYS`/`BASELINE_DAYS`/
    `EA_Z_FLOOR` into `constants.rs` with CLI overrides. (c) is mechanical and closes a "no parallel
    pattern" gap. **S–M**; (a) changes ranking (byte-identical-gated for the unaffected population).

### F261 — dead `changes.similarity` (rename %) → an `avg_rename_similarity` signal · LOW · researcher-validated
*   **Location**: `schema_v1.sql:57` (`similarity INTEGER`, set on renamed/copied), carried through
    `changes_lineage` as a passthrough but never used (lineage resolves by date/rowid, not similarity);
    zero non-clone analysis consumers.
*   **Signal / direction**: low-similarity renames (rename + heavy rewrite in one commit) are a known
    hard-to-review anti-pattern. Lowest priority; if pursued, fold `avg_rename_similarity` / count of
    sub-70%-similarity renames into `churn.rs`. **S**.

---

## Tier D — strategic (design phase first)

### F262 — survival analysis on hotspots (Kaplan-Meier over hot-episodes) · HIGH strategic · researcher-validated (re-scope of roadmap Tier-1)
*   Bucket each file's `commits.date × changes.path` timeline into monthly windows, classify each
    "hot/cold" via `hotspot_velocity`'s existing revs-per-week threshold, derive hot *episodes*
    (start→end, currently-hot right-censored), and fit a Kaplan-Meier estimator (Kaplan & Meier,
    JASA 1958) → "P(hotspot still hot after N months)". No new ingest; genuinely new user value vs
    the two-point `hotspot-velocity`. **NOT** a one-query bolt-on (stateful episode extraction + KM
    math + a repo-level survival-curve output shape) — needs a design pass per the roadmap's
    "Hard difficulty deserves a design phase" rubric. No `research-foundations.md` entry yet (unclaimed).

### Correction (not a finding): Type-3 near-miss clones is **not** a latent-data quick win
The roadmap Tier-1 estimate ("~100 LOC on top of existing fingerprinting") is off. `clones/fingerprint.rs`
stores a **single SHA-256 digest** over the AST-kind sequence — an avalanche hash carrying *zero*
similarity signal between near-but-not-identical functions. MinHash+LSH needs a *set*/shingle
representation, i.e. a new fingerprint stored alongside the digest = **new ingest**, not latent-data
surfacing. Still worth doing; re-scope the estimate before scheduling.

---

## Tier E — test / verification hardening (pairs with A/B)

*   **F263 — `[new_code]` gate `run_new_code_scope` has zero test coverage anywhere** · HIGH · S.
    Only the pure evaluator (`evaluate_new_code_rows`) is tested; the SQL/window path that builds a
    `NewCodeScope` from a real fact store — born-vs-touched partition, `window_start_rev` lookup, the
    skip branch — is untested. This is a live CI gate whose shallow-skip no-op was flagged the cycle
    it shipped. Pairs directly with F249.
*   **F264 — `is_shallow()` has zero tests** · HIGH · XS. The exact primitive behind cycles 2/3/4's
    top finding (G2/N1/H5). `git clone --depth=1` a fixture → assert `GixRepo::is_shallow() == true`.
    De-risks whatever fix lands on F249. (GitCliRepo hardcodes `false` by design — unit, not differential.)
*   **F265 — `calibrate` total-failure (0-of-N) exit path untested** · MED · XS. The only test covers
    partial failure (1-of-2). Writing the 0-of-N test today would immediately red-flag cycle-2's G2
    bug (exits 0 + writes a data-free artifact) — land with the G2 fix.
*   **F266 — differential harness missing binary / non-ASCII / submodule probes** · HIGH · S-M ·
    touches the two-backend-parity hard invariant. The fixture documents its own boundary (no binary
    blob, no non-ASCII path, no gitlink); `GixRepo`-only binary/quotepath unit tests are never
    cross-checked against `GitCliRepo`. A `gix` bump is the likely trigger. Extend the fixture + add 3
    differential probes.
*   **F267 — MCP `hotspots` never invoked via `tools/call`; `entity-effort`/`entity-ownership` have
    zero behavioral coverage** · MED · XS–S. `hotspots` is the one MCP tool with no `tools/call` test
    (every sibling has one). The two `entity-*` analyses (SPA-embedded, rename-aware) are never run
    against a fixture.

---

## Pointers (not new F-IDs)

*   **`calibrate_defects` temporal train/validation positive-leakage** — already fully diagnosed in
    `2026-07-28-hardening-cycle-3.md` §A2-1 (reconfirmed HIGH in cycle-4); inflates the measured AUC
    delta from +0.0029 to +0.1699. Squarely a rigor defect; don't drop it when triaging.
*   **Known-open F-list** carried forward: F244 (dispatch fan-out / `enum Format`), F161 (streaming
    emitters — the perf agent confirms multi-GB is only realistic for O(F²) analyses, already
    `HAVING`-bounded; LOW rating stands), F177 (3 schema-version sentinels), F218 residual (per-widget
    SPA rerenderer routing), F246 (canvas-chart keyboard a11y).
*   **Stale doc figure**: the roadmap's "explain covers 15 metrics" is stale — 38 topics exist.

## Rule-outs (checked, found solid — logged so they aren't re-audited)

Robustness: submodule/symlink filtering, `core.quotepath=false` injection, dirty-worktree cache
warnings, `NULLIF` division guards, SZZ blame-failure handling, `diff`'s own blind-ingest disclosure,
`gate`'s empty-population skip. Rigor: `bus-factor`/`ownership` denominator disclosure, `lead-time`
raw rows, MI/hotspot-percentile caveating, coupling Fisher + knowledge decay citations, tie-breaking
determinism (F151/F152/F226/F227). Perf: `changes`/`commits` indexes (columnar engine — not the
lever), LIMIT-pushdown split (intentional), `changes_lineage` build-once guard, producer/consumer
channel sweep. Failure: `ratchet.rs`, `resolve_defect_calibration`, `calibrate` partial-failure,
MCP `map_lib_err`, `change_set.rs` calibration-digest swallow (unreachable). Test: mailmap 4-token
(already fixed #194), `cycle_health`/`delta_health` behavioral tests (real, not shape-only).

---

## Disposition

Next audit sweep re-opens at **F268**. Recommended sequencing (leverage × 1/risk): **F249 + F263 +
F264** (the convergent correctness win + its pairing tests) first; then the latent-data feature
cluster (**F257, F258, F250, F259**); **F253/F254** as a perf slice; **F255** as MCP hardening;
**F256/F260/F251** as a rigor/honesty slice. F262 needs a design pass before scheduling.
