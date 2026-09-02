# CodeLore Deep Analysis — Second Wave

**Date:** 2026-09-02 · **Baseline:** main `515e144` (after PRs #321–#333, ledger through F336)
**Method:** 12 specialist agents (8 codebase auditors + 4 researchers) plus their fork children, each seeded with the F-ledger and the 2026-09-01 report and instructed to find *new* material. Every load-bearing claim below was then re-verified against source by the coordinating session before inclusion: **18 headline claims checked directly — 18 confirmed, 0 refuted.** Items the coordinator verified personally are marked ✅; items resting on an agent's own computed/methodical evidence are marked ◆; genuinely unverified aspects are flagged inline.

Prior wave: `docs/reports/2026-09-01-deep-analysis-and-improvement-report.md` (its backlog remains valid; this report supersedes its priority ordering).

---

## Part 1 — Verified defects (the implementation queue)

### Wave 1 · Engine correctness

**1. The lineage rename map is applied with no time bound — recycled filenames merge unrelated files. Default-on, cached.** ✅ *(found independently by two auditors; join re-read by coordinator)*
`materialize_path_lineage`'s recursive CTE carefully date-guards chain *construction* (`facts/ingest/lineage.rs:26-46`), but `materialize_changes_lineage` *applies* the map with `LEFT JOIN path_lineage pl ON pl.old_path = c.path` (`lineage.rs:118`) — string key only. Rename `a.rs → b.rs`, later create a new unrelated `a.rs`: every commit to the new file canonicalizes onto `b.rs`. The new file vanishes from hotspots/ownership/bus-factor; `b.rs` inherits a stranger's churn and authors. `use_canonical_lineage` defaults **true** (`options.rs:680`), and the corruption is *persisted*: `kamei::enrich` runs during ingest (`facts/ingest/mod.rs:167`), so wrong attribution is baked into the cache file. No test covers recycling.
**Fix:** carry the rename's `(date, rowid)` into `path_lineage`; canonicalize a row only when its commit predates the rename that retired the name. Bump `CACHE_EPOCH`. Add the recycle fixture. Three follow-ons ride along: (a) the lineage seed also ingests `Copied` rows (`consumer.rs:281` writes `rename_from` for copies; seed filters only `IS NOT NULL`) — one-word fix `change_type = 'renamed'`, latent today but reachable via repo-local `diff.renames = copies` on the git-CLI backend; (b) add `PRIMARY KEY (rev, path)` to the temp table so any residual conflation is a hard error — nine analyses document a `(rev, path)` PK invariant that `changes_lineage` currently doesn't hold; (c) `summary`, `architecture_trend::live_paths_at`, and `health_trend::top_hotspot_paths` don't opt into lineage at all, so their counts/rankings disagree with lineage-aware views on the same page (the health-trend docstring justifies the columns but not the *ranking*).

**2. `--time-bucket` + `--format spa` silently empties half the dashboard.** ✅
The gate at `analyze.rs:141` checks `supports_time_bucket()` for the *named* analysis only. `--analysis hotspots --time-bucket month --format spa` passes, then `build_spa_dashboard` fans out to ~30 analyses whose `lineage::source_table` returns `changes_bucketed` — whose `rev` is a date-truncated *string* (`grouping.rs:54`), so every `JOIN commits USING (rev)` matches zero rows. The degradation wrappers catch `Err`, not empty. Worse: `knowledge_shares` caches behind its built-flag, so one bucketed build poisons the rest of the run.
**Fix:** move the rule into `Options::validate()` ("time-bucket requires every analysis this invocation runs to support it"); reject the flag in the spa/step-summary branches. Related: `changes_bucketed` has no build-once guard (rebuilt per call — mirror the F184 guard) and collapses `change_type` with lexicographic `MAX` beside a chronological `arg_max` one line away (`grouping.rs:56-57`).

**3. `SUM(sloc)` over `complexity_metrics` double/triple-counts files.** ◆
The table holds the whole-file `unit` row *plus* per-impl and per-function rows, all overlapping. `effort_exposure.rs:110` (with a comment asserting the opposite), `code_familiarity.rs:81`, and `knowledge/shares.rs:377` (the DOE size term) all `SUM`; `code_health.rs:673` correctly uses `MAX`. Inflation is ~1–3× per file and does **not** cancel out of shares — it tilts every ratio toward function-dense files.
**Fix:** three `MAX`s + comment correction, behind a byte-identical-baseline run (output moves by design, so the baseline documents the intended delta).

**4. The gix walker zeroes line counts for text files over 1 MiB; git's threshold is ~512× larger.** ◆ *(reproduced by the auditor against real git)*
`MAX_DIFF_BLOB_BYTES = 1 MiB` (`gix_repo/history.rs:221`) with a comment claiming it "matches git's `core.bigFileThreshold` default" — git's default is 512 MiB. A 1.44 MiB text file: git reports `80000/3`, GixRepo reports `0/0`. Every large generated/vendored file enters `changes` with zero churn — on exactly the files most likely to be hot. The differential gate can't see it: line counts are compared as an *aggregate* with 5% tolerance, not per-file (CLAUDE.md's "identical event streams / single byte fails" claim is materially overstated for `loc_*` fields).
**Fix:** raise the cap (or read the config), keep the NUL sniff for binary detection, add per-file `loc` equality + a >1 MiB text file to the differential fixture. If a cap is wanted as cost control, disclose via `ScanOutcome` — never emit a silent zero.

**5. Chronology tiebreaks are inverted or arbitrary at six sites.** ◆
Convention (documented at `facts/ingest/mod.rs:255-264`): smaller `rowid` = newer. Violations: `coordination_needs.rs:182` (LAG order — corrupts the author-interleave count at same-second ties), `cycle_origins.rs:72`, `architecture_trend.rs:73`, `dashboard.rs:320` (picks the *older* of a same-second pair), plus two "latest commit" queries tiebreaking on SHA lex order (`effort_exposure.rs:443` — anchors the `new_code` gate window — and `evidence.rs:59`), which the ingest comments explicitly call meaningless. Deterministic, so nothing flakes; just wrong at ties. *(The rowid-direction premise rests on the ingest comment, not an empirical gix check — verify with the same-second fixture the fix adds.)*

**6. Renamed-away paths count as live.** ◆
A rename writes one row keyed on the *new* path; the old path's last event stays `modified` and passes every `!= 'deleted'` liveness rule. `query_live_paths` just wastes blob lookups (correctly bucketed NotCounted), but `knowledge_islands::count_live_files` inflates the denominator of the knowledge-prevalence tile against a lineage-canonical numerator — unlike populations in one ratio.

**7. `--group-file` silently zeroes two analyses.** ◆
Grouping rewrites `changes` (and builds a complexity rollup with a long comment explaining why) but leaves `clones`/`imports` raw — so `clone_coupling` (`:221`) and `crossing` (`:72`) join grouped keys against raw keys and return zero rows, reading as a clean bill. **Fix:** reject the combination the way `FunctionHotspots` already does.

**8. `knowledge_shares` claims "deleted paths excluded" but filters rows, not paths.** ◆
Dead files keep pre-deletion contributions; `coordination_needs` emits an output row per dead file (`health_band: unknown`); `code_familiarity.total_authors` counts authors who only ever touched deleted files. Fix once in the materializer with the existing liveness CTE.

**9. Bot filtering is inconsistent across author-aggregating analyses.** ✅ *(ownership vs knowledge-islands checked directly)*
Three regimes coexist: `HUMAN_ALIASES_CTE` (authors, bus-factor, knowledge-islands, …), `BotPatterns::from_repo` re-read (pair-programming, documented), and **none** (`ownership`, `main_dev`, `entity_ownership`, `entity_effort`, `churn::run_author_churn`, `lead_time`, `coordination_needs`, and — sharpest — the fragmentation term inside `code_health`'s composite score). A dependabot-heavy lockfile reads "well-owned" in `ownership` and "an island" in `knowledge-islands`. Also: `summary` under `--code-maat-compat` skips the bot filter, so the `authors` metric changes meaning with a *format* flag. Needs a deliberate policy pass (which analyses *should* see bots?) rather than blanket filtering — but the health-score term is hard to defend as-is.

**10. Float-division falsehoods (verified against the vendored DuckDB C++).** ◆
`/` is float division in the pinned engine (`//` is integer). `coupling.rs:299`'s comment claiming integer-floor is false — the `ORDER BY` sorts on 3.5 while the column displays 3 (duckdb-rs `FromSql<u32>` truncates), so sort key and displayed value disagree (`communication.rs:48` same). `team_composition.rs:154`'s `DOUBLE→BIGINT` cast **rounds** (`nearbyint`), so a 10.6-day tenure reports 11. Fix: `FLOOR`/`//` + `date_diff('day', …)`.

### Wave 2 · Trust & supply chain

**11. SARIF hotspot severity can never reach its own `error` band.** ✅
`sarif.rs:182` computes `(100 − cognitive_health)/10` while `hotspots.rs:299` bounds health to [60, 100] (README:98 documents the floor). Result range: [0.0, 4.0] — the `≥7 → error` branch is dead code, `warning` fires only at exactly 4.0, and a healthy file emits `0.0`, which GitHub maps to "no severity". The in-file comment "(range 0.0–10.0)" is internally wrong. **Fix:** rescale to use the real range — and while in the emitter, adopt the consumer-research must-fixes: add `properties.problem.severity` (the correct non-security channel; `security-severity` is semantically a vulnerability score), emit `automationDetails.id` (`codelore/check` vs `codelore/diff` currently collide on multi-upload), prefer `help.markdown`, add `properties.precision` (Fisher-gated coupling honestly claims `high`), and self-truncate above the 5,000-displayed cap with disclosure. Already right and worth keeping: the three versioned `partialFingerprints` keys shared between check and diff.

**12. Attestations never reach release assets — Scorecard's Signed-Releases stays 0 forever.** ◆
The wiring is complete and *correct* (matrix attest jobs, release depends on them, L3 permission split enforced) — but bundles go only to the GitHub attestations API, and Scorecard inspects release *assets* (`*.sigstore.json` etc.). **Fix:** upload the `actions/attest` `bundle-path` output as a `…sigstore.json` release asset in `attest-artifact.yml` — no new permissions; the SHA256SUMS globs don't match the new suffix.

**13. The published GitHub Action's checksum verification fails open.** ✅
`action.yml:192-211`: if `SHA256SUMS` can't be *fetched* (5xx, proxy, adversary dropping the manifest), it warns and runs the binary anyway; only a *mismatch* hard-fails. The repo itself applies the correct status-branching pattern at `release.yml:452-474`. **Fix:** branch 404-vs-else; better, verify via `gh attestation verify --signer-workflow` (gh is preinstalled on runners) with checksum as the pre-attestation fallback.

**14. An ambient `ANTHROPIC_API_KEY` silently redirects the LLM layer to the hosted endpoint.** ✅
`client.rs:308-322`: with no explicit provider, key presence alone selects the Anthropic dialect — while `docs/advanced-usage.md` promises "out of the box nothing leaves the machine; a hosted provider requires an explicit environment change" four lines before documenting the inference. Developers export that key for unrelated tools. For a local-first product this is the claim most worth making true. **Fix:** require explicit `CODELORE_LLM_PROVIDER=anthropic`; fix the doc. Related hardening: `CODELORE_LLM_BASE_URL` accepts non-loopback `http://` while sending a bearer token — restrict plain HTTP to loopback. (Credential handling is otherwise verified clean: hand-redacted Debug, hardcoded Anthropic base URL, nothing persisted.)

**15. Repo-controlled text reaches the LLM prompt unfenced.** ◆
Author names, paths, and function names are rendered raw into the fact sheet (`prompt.rs:76-82`, `fact_sheet.rs:51-64`); paths may contain newlines. The grounded stamp is *unforgeable* (numeric ground truth is collected from typed sections pre-render — a property worth pinning with a regression test, since nothing states it as a contract), but the narrative text is steerable, and its sinks are an agent's context (`explain_file` via MCP) and future PR comments (`diff --format markdown`) — second-order prompt injection from hostile repo content. **Fix:** fence the sheet, add the data-not-instructions system line, escape control characters in `render_canonical`, bump `PROMPT_VERSION` (caches orphan naturally), add the anti-forgery test.

**16. Arrow version drift makes every provenance stamp wrong, behind a guard that can't see it.** ✅
`Cargo.toml` declares direct `arrow = "59.1"` (→59.2.0) while duckdb pins arrow 58.3.0 — both in the lockfile. `arrow_facade.rs:21` stamps `"58.3.0"` into every provenance sidecar and the fact-store provenance table, but the facade's re-exports resolve to 59.2.0. The drift guard (`dep_versions_drift_test.rs`) does `lock.find(…)` — first match wins, which is 58.3.0, so it passes. The ledger even records PR #69 being *closed* to avoid exactly this desync; it landed later anyway. The facade's type re-exports have zero consumers (parquet goes through DuckDB `COPY`). **Fix:** drop the direct arrow dep + the `appender-arrow` feature (sweep `Appender` usage first), re-point the facade at duckdb's re-export, make `locked_version` fail on duplicate package names, refresh `deny.toml`'s stale duplicate list.

**17. Smaller trust items.** MCP `resolve_rev` lacks `--` before the caller-supplied rev and doesn't assert the result is a 40-hex SHA before `git worktree add` (two lines; low exploitability). `release.yml`'s Windows staging swallows 7z failures (`2>/dev/null || true`) and no downstream guard requires the zip — a release can publish with the Windows asset silently missing. `cut-release.sh` and `build.rs` pin verification were audited and are **clean/fail-closed** — no action.

### Wave 3 · CLI, MCP, and dashboard UX

**18. `--output -` writes a file literally named `-`.** ✅
No stdout handling exists; `File::create(path)` is unconditional. The README's flagship CI recipe (`diff … --output - >> "$GITHUB_STEP_SUMMARY"`, README:445) silently produces an *empty* step summary plus a junk untracked file the next `codelore gate` sees. Fix: one stdout match arm in each of two functions — and route `diff` through `emit_to_output_or_stdout`, which also fixes `diff --output` not being atomic (raw `File::create` truncates the previous good report on failure; `analyze` uses `atomic_publish`).

**19. `codelore diff` exits 1 for everything.** ◆
Zero typed errors in `diff.rs` — bad rev range, missing git, full disk, and a genuine gate violation are indistinguishable to CI, directly contradicting the documented exit-code design (`advanced-usage:821`). `check`/`gate` carry in-source comments describing the discipline `diff` never adopted. Also: format×analysis validation happens *after* the 5–30 s ingest and returns four different exit codes for the same class of typo (`supported_formats()` is pure — call it before preflight and delete the mirrored SARIF special case); unknown-name errors exit 4 from `schema`/`explain` where the documented table says 2.

**20. `check`/`gate` split the verdict across stdout/stderr against their own docs.** ✅
Text-mode PASS/WARNING go to stdout, FAIL to stderr; SARIF mode prints no PASS line at all. `codelore check > log.txt` captures PASS but loses FAIL. Move text-mode verdicts to stderr (matches docs, JSON mode, and the `--quiet` promise).

**21. MCP server startup half-validates.** ◆
It fail-fast-validates both calibration artifacts but never opens `--repo` — a typo'd path in `claude_desktop_config.json` yields a healthy-looking server that fails per-tool. Also missing: `--cache-dir`/`--temp-dir` (containerized servers fall back to `/tmp/codelore-fallback-*`). And the calibration asymmetry runs the other way from F323: `--calibration` is missing on `gate`/`explain`/`diff` while their MCP twins have it — `codelore explain` and `explain_file` print **different corpus percentiles for the same file**, and that number grounds the citation check. F323's parity-test pattern is directly reusable.

**22. Dashboard: one dead lens, one failing palette, and a cluster of computed a11y defects.** ◆ *(color math computed, not estimated; one-line item spot-verified ✅)*
- **The AI-attribution lens is dead** ✅: `buildFsHierarchy` copies only 4 fields into `metrics`; the renderer reads `m.ai_pct` → always null → entire map renders "no data" grey while the table two panels down shows real percentages. One-line fix; no browser test covers the mode. (`mi`/`mi_rank` dropped the same way.)
- **The bivariate palette fails both ways**: fixed hexes vs the theme'd card background put the *danger* cells at 1.70:1/2.67:1 on the default dark theme (the healthy cells pop instead); light theme inverts it. Separately, three cross-band pairs sit at ~1.1:1 for *normal* vision because both axes ride lightness (the stated monotonicity invariant was verified true — and insufficient). Fix: theme-split the palette like `--heatmap-*` + a two-hue Stevens construction, or drop the activity axis from color (area already encodes it). External research corroborates: no certified CVD-safe bivariate set exists; redundant encoding + a focusable legend is the standard answer.
- **Escape doesn't close the drawer** ✅ — the dialog uses non-modal `.show()`, the only Escape handler is gated behind `if (window.Alpine) return`, and `@keydown.escape.window` exists in comments only (four of them claim it works).
- **Boot loop is the one unguarded fan-out**: a throw in widget 13 leaves eight later panels empty, reading as "no data" — every other fan-out is individually try/caught.
- The rest, all verified in source: focus-ring color swaps at 1.07:1 across 41 tooltip triggers; the tooltip's 6 px hover gap makes the citation link mouse-unreachable; localStorage keys not namespaced per repo (state bleeds between dashboards on the same GitHub Pages origin); the share-link hash drops `archGraphLayout`/`archMatrixMode`; mouse clicks desync the roving tabindex (arrow keys then silently change the lens); unconditional wheel-`preventDefault` scroll-traps the two hero widgets; the off-boarding picker declares a listbox with zero options; the first off-board toggle silently switches lenses and double-renders (sharpens F218); the KPI panel's instructions describe an interaction that doesn't exist; 25 injected buttons share 3 accessible names; `buildFsHierarchy` is quadratic in directory width.

**23. Cold-cache runs give no signal.** ◆ Banner prints "✓ ready", then 5–30 s of silence; cache hit/miss logs at `info` under a `warn` default. Promote to a banner `Cache:` row or a one-line stderr notice. (Ecosystem research: pair with a `--no-progress`/`CODELORE_NO_PROGRESS` escape hatch per the uv convention. `NO_COLOR`/`CLICOLOR_FORCE` handling already exists and was verified — the gap is progress, not color.)

### Wave 4 · Guards and test hardening

**24. The differential gate enforces far less than CLAUDE.md claims — and the fix exists in-tree.** ◆
All 13 `Repo` methods are touched, but: 4 of 8 commit-event scalars are never compared (`author_name`, `committer_email`, `message` all have live consumers), `changed_files` compares only path sets over 8 commits, line counts use the 5% aggregate band, no test ever *ingests* through GitCliRepo, and gix-vs-cli `message` trailing-newline shape divergence would flip `$`-anchored `--expression-to-match` patterns. `cache_test.rs:460` already defines `fact_store_digest` with its own anti-vacuity control — point it at both backends and the gate becomes what the docs claim. Also: `git_cli_repo` sets `canonical_author` only-if-different under a comment claiming it mirrors gix (gix always sets it) — harmless today via consumer fallback, but the comment is backwards.

**25. The browser suite reports green when Chrome fails to launch.** ◆
Launcher errors map to `println!` + clean return (3 sites, ~20 downstream skips); no CI env check anywhere in the file. This is the only place JS executes in CI, and a fixture's defining invariant lives exclusively inside it. Fix: `CODELORE_REQUIRE_BROWSER=1` in the CI step; panic on the skip path when set. Three more silently-inert steps of the closed-F242 shape ride along.

**26. `check.rs` — the authoritative gate surface — has no `Gates` exhaustiveness anchor; the advisory MCP surface does.** ◆
`mcp.rs:582` holds the only exhaustive `let Gates {…}` in the workspace ("adding a field fails to compile until classified"). `check.rs` uses unanchored independent branches — a gate added to the TOML can silently enforce nothing in CI. The mechanism exists 800 lines away; closing this closes the structural half of F335.

**27. Instrumentation gaps the coverage sentinel was built for.** ◆
PR #331's oversize disclosure landed on the clone engine the product *doesn't* run: `analyses/clones.rs::run_clones` (what `analyze`/SARIF/`gate` use) drops over-cap files with a `debug!`; the instrumented HEAD pass feeds only `check`. And the two historical at-rev scans (`architecture_trend`, `at_rev.rs`) have no `ScanOutcome` at all — a rev with unreadable blobs renders as *architectural improvement* on the trend chart and feeds defect-calibration weight tuning. Route all three through the existing `ScanCoverage`; for trends, warn on relative coverage drops between sample points.

**28. Assorted verified hardening items.** `quality_gates/ledger.rs:249` byte-slices an untrusted `head_sha` at 12 bytes — the F298 class verbatim, reachable via a hand-edited/corrupted `gate_runs.jsonl` (5 sibling sites are safe only by provenance; normalize to one boundary-safe helper) ✅. 15 sort sites use `partial_cmp().unwrap_or(Equal)` (Rust 1.81+ sorts *panic* on detected total-order violations; `total_cmp` is a drop-in and already the house pattern in 5 files). `CalibrationArtifact::validate` accepts `languages: []` — a silent no-op lens indistinguishable from "not in corpus". Cardinality-floor gaps in 8 ordinary tests (headed by the negative-age *regression* guard, which passes on zero rows). Two named guards lack matcher self-tests (`doc_analysis_count`'s 49-line predicate; `enrichment_isolation`). The enrichment guard covers 7 of ~20 scoring roots, and the obvious widening false-positives on a doc comment — widen roots + comment-strip the matcher in the same change. `prune_global_cache` can delete the entry the run just wrote (keep-path param). Code-maat parity tests are both `#[ignore]`d with goldens that were never created and a parser that can't discriminate (`unwrap_or(-1)` both sides). Emitters: 57 markdown / 3 tested — the F91/F109 escaping class already recurred once; one table-driven `AnalysisName::all()` render sweep closes it by construction. Exit codes 4 and 5 are each asserted exactly once across 117 CLI tests; 13 bare `.failure()`s pass on panics.

### Wave 5 · Documentation (35 verified findings, top slice)

- **`docs/RELEASING.md`'s manual fallback cannot be executed as written**: it omits the `protect-main` ruleset half of the dance, prescribes the exact `gh run watch --exit-status` check the script documents as unsafe, and cites a PR template that doesn't exist. This is the emergency path — read only when the script is already unavailable.
- **`docs/codebase_analysis.md` is systemically stale**: `Repo` trait shows 7 of 14 methods; the registry omits 15 shipped analyses while contradicting its own diagram (42 vs 57); 7 emitters listed vs 11; `unsafe_code` attributed to the wrong file; and its cache-key paragraph now states the *opposite* of the shipped F319 ingest/analysis split.
- **`SECURITY.md` is missing while the README displays a Scorecard badge** — the cheapest high-value file in the audit (+ CONTRIBUTING, PR template, `CITATION.cff` for the research audience).
- CHANGELOG: zero link-reference definitions (3,363 lines of dead `[X.Y.Z]` refs; `cut-release.sh` never appends compare links), duplicated `###` headers in `[0.27.1]` (*not* `[Unreleased]` — correcting this wave's own brief), inconsistent section order with RELEASING prescribing a third one, and a guard test that checks presence only.
- README: the Quick-start CSV sample is missing a column the binary emits (`hotspot-score-anchored`) — the first output a new user diffs against; `function-hotspots` absent from all tables; `just ci` description omits `zizmor`; stale Rust patch version; two wrong static line counts.
- `advanced-usage`: `.codeloreignore` scope claim is false (it's ingest-wide; same stale line in `options.rs:94`); `--time-bucket` help names a rejected analysis and omits two accepted ones; `change-coupling` isn't a valid analysis name (×3 sites); §11.9's MCP reference has four verified drifts (missing `--calibration` entirely, wrong default limit, two missing params); `--departed-threshold-days` documented nowhere canonical; three formats (`ndjson`/`gha`/`html`) undocumented outside a pasted help block.
- `codelore docs` promises formulas and citations it doesn't emit; `codelore schema`'s stub disclosure exists at runtime but not in the three doc surfaces advertising Spectral/OpenAPI integration; `codelore notes` doesn't exist (a local CLAUDE.md error). The demo's "regenerated on every push" claim sits on a `continue-on-error: true` job; the landing page has no publish automation at all.

### Wave 6 · Structure (decisions attached)

- **Four of six module cycles are single misplaced helpers** (e.g. `facts` calls *up* into `analyses::query::wall_clock_utc_literal`, a 3-line wrapper that calls straight back *down* into `facts::ingest`). `memo.rs` states the no-cycle rule the codebase violates one file over. Move four helpers + add a layering guard test (comment-stripping matcher from day one). The other two cycles are structurally justified — leave them.
- **`codelore-lib` is published to crates.io, so every `pub` item is a semver contract** — and nine top-level modules are referenced by nothing external; ~20 arrow types are re-exported for zero consumers; two functions are provably dead (`centrality::from_coupling_pairs`, `arch_rules_path` ✅); `AnalysisName` is reachable via four public paths (all four in use). The `cli_api` facade's "only via these re-exports" claim is bypassed at ~40 sites. **Recommendation:** decide whether the lib API is supported surface, then do the narrowing + helper moves + facade enforcement in *one* breaking slice.
- `SpaDashboard`'s only real assembly (with display policy) lives in the CLI (~470 lines); every lib test hand-populates with `..default()`. Moving assembly lib-side is what unlocks the known `CodeHealthMemo` perf item cleanly.
- Eleven hand-built `Options` sites in `mcp.rs` (all verified correct today — one auditor nearly filed the false positive and killed it) + `validate()` on only 4 of 8 entry points → `base_options()` seam.
- SZZ's `is_candidate_cosmetic` re-reads and re-decodes the whole blob once per candidate *line* (a 200-line fix = 200 tree walks); hoist one warm reader per fix + memoize per `(rev, path)`.
- `mailmap.file`/`mailmap.blob` config sources affect ingest but aren't in the cache key.
- Ledger correction found during audit: F319's evidence list names a nonexistent `opts.track_rewrites` and omits `team_map_file` (classification unaffected).

---

## Part 2 — Research-driven improvements

### Competitive position (all primary-source, fetched 2026-09-02)

The category's currency became **published benchmarks**: Repowise ships a pre-registered, sealed-split benchmark suite and a head-to-head defect-prediction win over CodeScene. Meanwhile their own docs concede a decisive structural limit: **history analysis covers 500 commits by default, 5,000 max** — every behavioral signal they produce is a recency window, and their `.mailmap` handling folds email-only (the exact 4-token bug our differential gate exists to catch). CodeScene's real boundary is **private repos** (€18/author/mo; public repos are free). Sonar shipped architecture management GA at no added cost + an embedded MCP server — still zero git history. code-maat is 14 months dormant; the migration tail is unclaimed. The MCP registry is cheap, uncontested territory (Repowise is in it; nobody else serious is), and outside Repowise the behavioral-git-history MCP niche is empty.

**Actions (adaptation, not copy-paste):**
1. **Ship a window-sensitivity analysis** — rank deltas between a 500-commit window and full history from the same fact store ("9 of your top-25 hotspots are invisible to a windowed tool"). One `WHERE` clause; windowed competitors *cannot compute the comparison arm*. Make "every other tool truncates your history or charges for it" the headline claim.
2. **Elevate incremental ingest out of Tier-4.** Fast-cold is only felt once; the daily agent loop re-walks on every HEAD move. Design: append-only delta over `cached-HEAD..HEAD`, *rematerialize* (never patch) lineage, gated by byte-identity vs a cold walk — the gate is exactly what Repowise lacks and why they keep shipping staleness bugs.
3. **Enter the benchmark conversation**, indexing-time arm first (their docs concede "slowest indexer, 22× CodeGraph"), then code-health-vs-defects on their public corpus — with the vendor-benchmark caveats stated.
4. **Register in the MCP registry** (`mcp-publisher`, `io.github.emrecdr/` namespace, metadata-only) and publish the code-maat migration table + captured goldens (which also fixes the inert parity tests).

### Research-grade features (literature-verified; one fabricated citation caught and dropped by the researcher)

Ranked by evidence × effort:
1. **`defect-validation` becomes a defensible study protocol** (S–M): add a size-only baseline ranking, effort-aware measures (Popt/PofB20), and a permutation p-value + n. The Code Red evidence base has zero independent replications — publishing honest local validation is an open lane.
2. **Revert-anchored labels in `calibrate-defects`** (S–M): `This reverts commit <sha>` is a second high-precision label stream (SZZ-style fix-keyword labels now measured at ~50% precision); report per-source AUC and a calibration line (ECE) — all three published JIT-DP techniques are miscalibrated.
3. **MCP surface consolidation, eval first** (M): fold the four ranked-table dump tools into one parameterised tool with `detail: concise|full`. The evidence is strong (tool-count costs context; competing tools suppress each other; CLI-vs-MCP cost studies), but it's breaking — cheapest pre-1.0, and step one is measuring *this* server. Free today: a README recipe positioning `codelore gate` piped from a shell as a first-class agent interface.
4. **Three-way grounding taxonomy** (S–M): split `⚠ uncited` into verified-cited / uncited-but-supported / unsupported (the literature's converged framing); the labelled corpus makes this a re-label + one field, and nothing comparable is published.
5. **Recency-weighted co-change column** (S, moderate evidence — half-life unparameterised in the literature; byte-identical gate applies).
6. **AI-attribution-adjusted bus factor** (S–M, speculative — ship as a disclosed instrument; we hold the measurement instrument the "substrate collapse" papers say is missing).

### Ecosystem currency

Dependencies are already current (duckdb-rs, gix 0.87.1, clap — all newest; the "1.4.5 LTS" duckdb line is a trap, not an upgrade). Real items: **Rust 1.96.1 → 1.98** (policy compliance; 1.97's `CARGO_BUILD_WARNINGS` also un-splits the CI/local cache key that `RUSTFLAGS=-Dwarnings` causes — disk-treadmill relevant); **`[package.metadata.binstall]`** (6 lines — the flat tarball layout likely breaks binstall's directory-prefixed defaults; a 30-second live test settles it); **cargo-auditable** (blocked on one check: does `strip = true` eat the `.dep-v0` section); **musl targets** (prereq for Alpine/ubi/mise; needs a real bundled-DuckDB build attempt); **rmcp 3.1.4 → 3.2.0** on its own PR. MCP protocol currency: two researchers returned different "current" revisions (2025-11-25 vs 2026-07-28, the latter stateless with the handshake removed) — resolve via the rmcp SDK's supported revision rather than hand-tracking; near-term wins regardless: `readOnlyHint: true` + `title` on all 11 tools (one line each; affects client auto-approval), `outputSchema` reusing `codelore schema`'s JSON Schema output, and a `schema_version` field in `--format json` payloads (the cargo/rustc stable-contract convention).

---

## Part 3 — Recommended execution order

| Wave | Contents | Gate |
|---|---|---|
| **A. Engine correctness** | Lineage time-bound (+copies, +PK, +opt-in stragglers, CACHE_EPOCH bump) · time-bucket×SPA validation · SUM(sloc) trio · diff-cap raise + per-file differential · tiebreak sextet | recycle/same-second fixtures; byte-identical baselines where output moves by design (documented deltas) |
| **B. Trust** | SARIF emitter batch · attestation assets · action.yml fail-closed · LLM explicit-provider + doc fix · prompt fencing + anti-forgery test · arrow de-drift + guard fix | failed-guard probes; `gh attestation verify` on a real artifact |
| **C. UX** | `--output -` + diff atomicity + diff exit codes · verdict channels · MCP startup validation + `--cache-dir` · calibration symmetry (+F323-pattern parity test) · SPA batch (AI lens first — one line) | CLI tests with `.code(N)`; browser tests for the fixed modes |
| **D. Guards** | fact-store differential digest · `CODELORE_REQUIRE_BROWSER` · Gates exhaustiveness in check.rs · scan instrumentation ×3 · floors/self-tests batch · emitter sweep · total_cmp class · ledger boundary helper | each guard lands with its own anti-vacuity probe (house rule) |
| **E. Docs** | RELEASING fallback · codebase_analysis refresh · SECURITY.md + CONTRIBUTING + CITATION.cff · CHANGELOG links + guard extension · README/advanced-usage batch (35 findings) | doc claims re-checked against source at PR time |
| **F. Features/competitive** | window-sensitivity analysis · defect-validation columns · revert labels · binstall metadata · registry listing · MCP hints/schema · incremental-ingest design doc | new analyses: fixtures + explain topics + docs move together |
| **G. Structure (one breaking slice)** | pub narrowing + dead code + helper moves + layering guard + facade decision | semver-conscious; needs the Part-4 lib-API decision first |

Waves A–E are independent of the disk constraint only for CI-side validation; local full-gate work still needs headroom (~6 GiB build / ~25 GiB gate).

## Part 4 — Decisions that are yours (not implemented without a call)

1. **Is `codelore-lib` a supported public API?** Determines Wave G's scope (narrowing is breaking).
2. **MCP consolidation & namespacing timing** — breaking for configured clients; evidence says pre-1.0 is the moment.
3. **CodeQL/SAST**: only recognized option for the Scorecard SAST check, but Rust-pack value over clippy-pedantic is unverified and it costs a bundled-DuckDB build per PR. Honest alternative: document the reasoning in SECURITY.md.
4. **Fuzzing investment**: ranked target is the tree-sitter C grammars (the one attacker-bytes × C-parser surface); note Scorecard credits cargo-fuzz **nothing** — do it for bugs, not the badge.
5. **Trusted Publishing** (standing): now reinforced by finding 16's cousin — the crates.io token is exposed to every dependency's build.rs during publish, the exact isolation the OIDC signing token already gets.
6. **Bot-filter policy** (finding 9): which author-aggregating analyses *should* see bots?
7. Standing items: F315 gate enforcement, F332 hotspots semantics, F334 diff_hunks, Renovate app install (config already in-tree since June), CodSpeed, Marketplace listing.

---

*Verified-healthy list (don't re-audit): build.rs pin verification fail-closed; cut-release.sh; MCP stdio hygiene + memo-key discipline + deliberate un-memoized trio; `should_color` NO_COLOR/CLICOLOR_FORCE; atomic_publish cleanup; SQL injection sweep (2 guarded exceptions documented); prepared-statement hoisting; read-only INSERT rule; producer/consumer no-deadlock; cache-write atomicity; thiserror/anyhow boundary; named convention guards (zero detached-matcher recurrences); F327 fully closed; empty-state coverage in the SPA (24 messages); enrichment narrative cache (verdict recomputed on warm reads).*
