# Changelog

Conventional Commits format. All notable changes documented here.

## [Unreleased]

### Fixed — correctness defects discovered post-v0.1.0

- **Real `loc_added` / `loc_deleted` in both repo walkers (SHOWSTOPPER).** Both `GixRepo` and `GitCliRepo` were emitting `loc_added: 0, loc_deleted: 0` on every change event — a `// Plan 1 stub` left from the walking-skeleton phase that was never wired in. Every churn-driven analysis in v0.1.0 silently returned zero: `abs-churn`, `author-churn`, `entity-churn`, `main-dev`-by-added-lines (every author tied at 0, so ranking was arbitrary), `entity-ownership`'s added/deleted columns, the `code-health` churn term (20% of the composite score), and Kamei `la`/`ld`/diffusion features. `GitCliRepo` now drives `git log` with `--raw --numstat` (replacing `--name-status`) and pairs raw lines with numstat lines by index; `GixRepo` uses `gix::diff::blob::diff_with_slider_heuristics` (Git's default histogram algorithm with slider postprocessing) to count added/removed lines between blob OIDs, and reuses the `DiffLineStats.insertions`/`removals` the rewrite tracker already computed for `Rewrite` events. New differential `tests/differential_repo_test.rs::line_counts_are_non_zero_and_match_across_walkers` asserts non-zero aggregate churn across the 50-commit fixture AND that the two walkers' totals match within 5% (small drift expected from root-commit edge cases).

- **`GixRepo` now detects renames at parity with `GitCliRepo`.** The pure-Rust walker had `diff_opts.track_rewrites(None)` set, so every rename surfaced as a `Deleted` event on the old path plus an `Added` event on the new path. `GitCliRepo` (via `git log --name-status`'s default `-M` detection) correctly emitted a single `Renamed { from, .. }` entry, so the two backends produced divergent change-type streams on the SAME commit history — silent file-history splits in every downstream analysis depending on which walker was selected. Now passes `gix::diff::Rewrites::default()` (50% similarity, copies off, 1000-file fuzzy limit — matches Git's `-M` defaults). New differential invariant `tests/differential_repo_test.rs::rename_commit_change_type_matches_across_walkers` asserts both walkers emit `Renamed` for the same commit and that no orphan `Deleted` event remains. Copies stay off because Git's default `git log` doesn't pass `-C`; preserves walker bit-equivalence.

- **`max_coupling_pct` was silently ignored.** Surfaced by the post-v0.1.0 deep-analysis pass. The field was wired through `Options` (default `100`) but `build_coupling_sql` only bound the lower threshold; pairs with degree above `max_coupling_pct` were returned regardless. Users who set `--max-coupling 80` to filter file-split / copy-rename artifacts got the unfiltered output. SQL gains an `AND degree <= ?` clause; bind list grows from 5 to 6. Regression test in `tests/coupling_test.rs::coupling_respects_max_coupling_pct` builds against `differential_repo`, observes the max coupling degree, then asserts that capping below half drops the top pair.

- **`rows_limit` propagation into composite analyses poisoned scores AND the cache.** `code_health::materialize_centrality` and `clone_coupling::run_clone_coupling` both invoke `coupling::run_coupling` to materialize the full coupling graph as a sub-computation. They were passing the parent `opts` straight through, so `--rows N` flowed into the SQL `LIMIT ?` and the inner result truncated to N pairs — centrality scores were then computed over an arbitrary 10-pair sliver, and clone-coupling silently dropped any clone whose partner sat outside the top N. Worse: `Options::canonical_json()` deliberately drops `rows_limit` (cosmetic output truncation; cache should hit regardless), so the corrupted result was *cached* under the no-row-limit key and poisoned subsequent runs that didn't pass `--rows`. New `Options::with_no_row_limit()` helper used at both nested call sites. Regression invariant in `tests/code_health_test.rs::code_health_score_invariant_under_rows_limit`: paths that survive truncation must carry the same score as in the unlimited baseline.

### Added — code-maat parity completion

- **`--team-map-file PATH` flag aliases author identities to team names** at ingest time. Mirrors code-maat's `-p / --team-map-file`. Two-column CSV (`author,team` with a required header row), applied as the LAST identity projection after mailmap normalization and bot filtering — the team-map sees the already-canonical author email and replaces it with the team name in `commits.canonical_author`. Every author-bearing analysis (`authors`, `author-churn`, `entity-ownership`, `main-dev`, `communication`, etc.) downstream sees the team name. Unmatched authors pass through unchanged (matches code-maat's `(get team-lookup author author)` fallback).
- **`.codelore-teams` auto-discovery.** If `--team-map-file` is omitted, CodeLore checks `<repo>/.codelore-teams` and loads it transparently. Mirrors the `.codelorebots` repo-root convention so a team-map can be checked into the repo without every contributor needing to set the flag.
- **New `crates/codelore-lib/src/identity/team_map.rs` module** with `load(path)`, `apply(map, author)`, and `discover(repo_root)` helpers. Strict parser: empty header, missing comma, blank fields, and duplicate authors all raise structured errors with the offending line number. Round-trip tested.
- **Cache key now hashes mutable-config FILE CONTENT, not just paths.** The team-map / group-file / `.codelorebots` paths are stripped from `Options::canonical_json()` and replaced with their sha-256 digests. Without this, a user editing the team-map CSV in place (same path) would silently see stale cached results because the cache key remained byte-equal across the edit — a known footgun in code-maat that bit users for years. Two regression tests in `options::tests`: (a) editing the team-map content invalidates the cache key, (b) the path is stripped so two machines with identical content hit the same cache entry.

- **`fragmentation` and `code-ownership` aliases for the `ownership` analysis.** code-maat exposes the Herfindahl-Hirschman fractal value under the analysis name `fragmentation`; CodeLore already computed the same value (`analyses/ownership.rs:64`) and emitted it alongside the main-author column, but `--analysis fragmentation` returned `UnknownAnalysisError`. Resolved via the same `FromStr` alias pattern as `refactoring-main-dev → main-dev-by-deletions`. Surfaced during validation: the canonical analysis name is `ownership`, but the user-facing docs uniformly used `code-ownership` (4 references across `advanced-usage.md` and `codebase_analysis.md`), so users typing what the docs said also failed; `code-ownership` is now an alias too. Both share a single regression test in `tests/ownership_test.rs`.

### Docs

- **`--format sqlite` documented as the code-maat `identity` equivalent.** code-maat's `identity` analysis dumps the raw parsed dataset for developer debugging; CodeLore's `--format sqlite` exports a strictly richer fact-store (all 8 tables — `commits`, `changes`, `complexity`, `clones`, `mailmap`, `provenance`, plus identity normalization metadata — vs code-maat's commits-and-changes-only view). Documented in `advanced-usage.md` line 73 so migrating users find the equivalent surface.

## [0.1.0] - 2026-06-08

### 2026-06-08 — Three-sprint deliverable: bugfix + modernization + code-maat parity

**Headline:** 14 → 21 analyses (every published code-maat analysis is now supported), 41 new regression tests (349 → 390), 29 atomic commits.

#### Added — Code-maat parity sprint (PAR-1 through PAR-7)

- **7 new analyses** closing the code-maat feature gap:
  - **`soc` (Sum of Coupling)** — per-entity total of `(commit-size − 1)` across every commit the entity appears in. New `--min-soc N` flag (replaces code-maat's overloaded-`--min-revs` semantic in this one analysis).
  - **`messages`** — commit-message regex matcher driven by `--expression-to-match REGEX` / `-e`. Validated eagerly via the `regex` crate; matching server-side via DuckDB `regexp_matches`.
  - **`main-dev`** — top author per file by lines added. Honest column headers `entity,main-dev,added,total-added,ownership`.
  - **`main-dev-by-revs`** — top author per file by revision count. Honest headers `entity,main-dev,revisions,total-revisions,ownership` (code-maat lied and used `added`/`total-added`).
  - **`main-dev-by-deletions`** (canonical name) / **`refactoring-main-dev`** (alias) — top author by lines removed. Same query, accepted under both names via `FromStr`.
  - **`entity-effort`** — per-(file, author) revision counts with file total alongside.
  - **`entity-ownership`** — per-(file, author) added/deleted churn breakdown.
- **Architectural grouping** via `--group-file FILE` (PAR-7). Plain-text and regex rules in code-maat's `<lhs> => <name>` format. **Full lookaround support** via the `fancy-regex` crate (code-maat's own test fixtures rely on it). First-match-wins, GROUP BY rev + new-path collapses multiple paths in one commit to the same group. Non-strict by default (CodeLore safety improvement: unmapped paths keep raw names; `--strict-grouping` opts back into code-maat's silent-drop behavior).
- **7 wired CLI flags** (PAR-6) — every Options field that had been pre-allocated finally gets a CLI surface: `--min-shared-revs`, `--min-coupling`, `--max-coupling`, `--max-changeset-size`, `--age-time-now YYYY-MM-DD`, `--expression-to-match REGEX` (alias `-e`), `--min-soc N`.
- **3 new Options fields** (PAR-0): `min_soc`, `time_bucket: Option<TimeBucket>`, `code_maat_compat: bool`. All participate automatically in `Options::canonical_json()` so they propagate to both the cache key and the provenance manifest.

#### Added — Modernization sprint (10 of 11; E.3 deferred)

- **Hot-path schema indexes** on `changes(path)`, `changes(rev)`, `commits(canonical_author)`, `commits(date)`.
- **SARIF `CODELORE-MISSING-COCHANGE` rule** — the CodeScene-signature "absent change pattern" signal now reaches Code Scanning (was computed correctly but silently dropped from SARIF).
- **`Options::canonical_json()`** — single helper that serializes the full struct via serde for cache-key + provenance manifest. Eliminates the drift bug where new fields silently disappeared from both surfaces.
- **`BotPatterns` extension hook** via project-level `.codelorebots` file. User additions are purely additive — defaults can never be turned off.
- **AI-attribution patterns extended** to cover 2024-2026 AI coders: Cursor, Aider, Cody, Continue, Codeium, Windsurf, Devin, Tabnine, Amazon Q. Case-insensitive throughout (fixes `Dependabot[Bot]` mixed-case detection bug).
- **Worktree prune on `codelore diff` startup** — SIGKILL / OOM no longer leaves orphan worktrees accumulating.
- **`code-health` centrality uses Fisher-filtered coupling** (eliminates spurious refactor-sweep noise).
- **Diff CLI typed enums** — `--analysis`, `--format`, `--fail-on` upgraded to `clap::ValueEnum`. Typos now caught at parse time.
- **`--absence-min-shared` + `--absence-fisher-p` flags** make coupling-absence thresholds tunable.
- **Hotspot SARIF `level` derives from security-severity bands** (matches the live-clone rule pattern).
- **`change_type` CHECK constraint** on the `changes` table.
- **11 analyses migrated to `params!` bind parameters** — SQL extracted to top-level `const` strings.

#### Fixed — Bugfix sprint (all 7 shipped)

- **`clone_coupling` p-value=0.0 hard-coded** — every live-clone row was shipping a fake p_value since launch. Now carries real Fisher exact p-value.
- **Empty `name` column** in hotspots + code-health output dropped.
- **`author_churn` + `abs_churn` non-deterministic ordering** — tertiary sorts added.
- **Cache key collision** — six clone-detection Options were silently omitted from the cache key (fixed via canonical_json).
- **SARIF coverage gap for coupling absences** — fixed via CODELORE-MISSING-COCHANGE.
- **Outdated AI-attribution patterns** — extended.
- **Worktree leak on SIGKILL** — fixed via startup prune.

#### Changed (BREAKING)

- **CSV header `main-dev` → `main-author`** in `ownership` output (matches struct field). Code-maat-compat surface restoring the legacy header is queued for the `--code-maat-compat` flag.
- **Hotspots + code-health output schema:** dropped the always-empty `name` column. Downstream consumers parsing the CSV verbatim see one fewer column.
- **Manifest JSON schema:** added nested `options: { ... }` field carrying the full canonical Options snapshot. 18 existing flat fields unchanged.
- **CI workflow:** `fetch-depth: 0` set on test job (the `gix_repo_walks_self_repo` test needs ancestry).

#### Docs

- New: `docs/RELEASING.md` (SemVer policy + full release procedure), `docs/github-topics.md` (18-topic recommended set + `gh repo edit` command).
- New plan files in `docs/superpowers/plans/`: `2026-06-08-codelore-bugfix-sprint.md`, `2026-06-08-codelore-modernization-sprint.md`, `2026-06-08-codelore-code-maat-parity.md`.
- README rewritten for clarity (5-minute pitch + 21-analysis table + `--group-file` example + migration story + acknowledgments).
- Cargo.toml: 5 keywords + 2 categories added. README badges expanded from 5 → 13.

#### Added — Code-maat parity completion (PAR-8 + PAR-9, post-wrap-up)

- **PAR-8 `--time-bucket DAY|WEEK|MONTH`** — modern replacement for code-maat's sliding-window `--temporal-period`. Materializes a `changes_bucketed` temp table via `date_trunc(<unit>, commit.date)` that the coupling-family analyses (coupling, clone-coupling indirectly, soc) query when active. Clean non-overlapping buckets — no commit-duplication artifact like code-maat's sliding window had. Implemented via a tiny `format!()` injection of the closed-enum-derived table-name identifier (`"changes"` vs `"changes_bucketed"`); MOD-A1's bind-parameterized threshold values are preserved.
- **PAR-9 `--code-maat-compat`** + **`--strict-grouping`** flags wired on the CLI. `--code-maat-compat` flips three internal defaults: `main-dev-by-revs` CSV emits lying `added`/`total-added` headers; `soc` falls back to `--min-revs` for the threshold; `--strict-grouping` is auto-implied (`strict_grouping: args.strict_grouping || args.code_maat_compat`).

#### Deferred (not deletions — queued for future work)

- **MOD-E3 table-driven dispatch** — refactor the 45-arm `match (format, analysis)` ladder. Current code works + is type-checked; refactor adds marginal value. Revisit if dispatch reaches ~80+ arms.
- **Legacy `--temporal-period N`** under `--code-maat-compat` — the literal sliding-window-with-duplication semantic code-maat shipped. The modern `--time-bucket` covers the actual user need; the `--temporal-period` flag itself rejects as unknown today. Future-work if migration users hit this.

---

### Added (Plan 8 §3 — persistent fact-store cache)
- **`FactsDb::open_or_ingest`** — content-addressed cache wrapper around `FactsDb::ingest`. Cache key is `SHA256(canonical_repo_path || head_sha || codelore_version || options_hash || schema_v1)`. Storage at `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`. Repeat invocations on the same `(repo, HEAD, options)` open the cached DuckDB read-only in ~10 ms — **100×+ speedup** on the dev inner loop.
- **Atomic writes** via `.tmp` → `fsync` → atomic rename (macOS APFS gotcha handled per the research brief).
- **LRU eviction**: 5 entries per repo + 2 GB global cap. Pruning runs after every successful miss-and-write.
- **`Repo::head_sha()` trait method** added; implemented on both `GixRepo` (via `gix::head_id`) and `GitCliRepo` (shell-out to `git rev-parse HEAD`).
- **CLI flags `--no-cache` + `--cache-dir PATH`** — skip the cache entirely (fresh in-memory ingest) or override the XDG cache root (useful in CI with per-job caches on shared runners).
- **Parquet + SQLite formats bypass the cache** by design — both require write access to the DuckDB connection. Documented as a deliberate carve-out.

### Added (Plan 8 §4 — FactsDb clones integration)
- **`clones` table populated during `FactsDb::ingest`** (closes validation report Finding S3). Walks the working tree at HEAD via `walkdir`, fingerprints every function in every Tier-1 file via `clones::extractor`, groups by AST structural digest, and INSERTs one row per family member via DuckDB Appender. Honors `opts.min_clone_node_count` (default 30) and `opts.exclude_patterns` (from `--exclude` + `.codeloreignore`).
- **`IngestStats.clones_ingested`** field added for observability.

### Added (Plan 8 §5 — parallel complexity extraction)
- **`ingest_complexity_at_head` runs via `rayon::par_iter().map_init`** over the working-tree file list. Each Rayon worker independently parses files via tree-sitter; results are collected into a `Vec` and drained serially into the DuckDB Appender on the connection-owning thread (Appender is `!Send + !Sync` per the research brief).
- `tree-sitter::Parser` is `Send + Sync` in 0.25.x — no thread-local pool needed.
- Per-file errors are logged via `tracing::warn!` but do not abort the parallel pass.
- **New bench targets** `complexity_extraction/parallel_default_threads` and `complexity_extraction/serial_1_thread` for measuring the speedup. The serial variant uses a 1-thread `rayon::ThreadPool` per-iteration via `pool.install(|| ...)` (you cannot reset the global pool mid-process).

### Added (Plan 8 §6 — clone-coupling intersection — THE differentiator)
- **`codelore analyze --analysis clone-coupling`** — surfaces "live clones": clone families whose members ALSO co-change at Fisher-significant rates. CodeScene calls this "X-Ray"; we ship the same analytical pattern with our published-formula transparency.
- **Algorithm**: any-pair intersection. JOIN the `clones` table (self-joined on `clone_group_id`, `path_a < path_b`) against `change-coupling` output (Fisher exact `p < 0.05`). Each surviving pair becomes one `CloneCouplingRow` with 18 fields including a `combined_score = similarity × degree_pct × (1 − p_value)` ranking.
- **5 false-positive mitigations** per the SourcererCC research brief, all `Options`-tunable:
  - `min_clone_node_count` (default 30 — drops trivial getters/setters)
  - `min_clone_shared_revs` (default 3 — below this Fisher's exact is unreliable)
  - `clone_similarity_floor` (default 0.70 — SourcererCC BCB benchmark optimum)
  - `--exclude` / `.codeloreignore` (already shipped in §2 Task 8)
  - `clone_skip_same_dir` (default true — drops intentional structural mirroring like `foo_test.rs` ↔ `foo.rs`)
- **Performance**: `O(n·k²)` where n=clone families, k=avg family size (typically ≤ 10). HashMap-based probe table built from coupling results.
- **All 4 output formats** wired (CSV, JSON, Markdown, SARIF).
- **`CODELORE-LIVE-CLONE` SARIF 2.1.0 rule** — one result per `(clone_group_id, file_a, file_b)` pair. `locations[0]` = higher-`support_a` partner (the primary for GitHub Code Scanning inline rendering); `locations[1]` = the lower partner. `partialFingerprints.cloneGroupFingerprint/v1` (AST digest) + `partialFingerprints.filePairHash/v1` (sorted sha256) for stable cross-run identity. `security-severity = combined_score * 10` clamped [0, 10]. Live clones get higher severity than the bare `CODELORE-CLONE` rule because the co-change signal proves real debt, not lookalike noise.

### Added (Plan 8 §7 — `codelore diff` PR-mode subcommand)
- **`codelore diff <base>..<head>`** (two-dot) **and `codelore diff <base>...<head>`** (three-dot, resolves via `git merge-base`). The form users actually deploy in CI.
- **Non-destructive `git worktree`** strategy: each rev checks out into a tempdir under `$XDG_CACHE_HOME/codelore/diff-worktrees/`, analysis runs there, the worktree auto-cleans on Drop via `git worktree remove --force`. The user's working tree is never touched.
- **`--base-cache PATH`** flag serializes the base rev's `RevAnalyses` as JSON. Next PR run with the same base SHA loads from the file instead of recomputing — cuts dual-analysis cost in half for the common case of many PRs against the same base.
- **Per-analysis delta semantics** (CodeScene Delta Analysis-style + research brief a18122f9ec6886ddf):
  - **Hotspots**: `rank_entrants` (new in top-N), `score_increased` (>= threshold), `pr_touched_existing` (info-only)
  - **Coupling**: `coupling_absences` — the CodeScene signature "you should have also changed X" signal. Fires when a historically-strong pair (`shared >= 5 AND fisher_p < 0.05`) has exactly one member in the PR's changed set.
  - **Clones**: `new_families` (introduced by the PR), `pr_touched_existing` (PR modified an existing family member)
- **Four output formats**: `text` (default, human-friendly terminal), `json` (full `DiffOutput` via serde), `markdown` (GFM tables for `$GITHUB_STEP_SUMMARY`), `sarif` (reuses `CODELORE-HOTSPOT` + `CODELORE-CLONE` rules with `properties.codelore/diff-classification` tagging).
- **`--fail-on` quality gate**: `none` (default), `rank-entrant`, `score-increase`, `any`. Exits 4 (analysis-failure per spec §6.6) when condition fires.
- **Example GitHub Actions workflow** at `examples/.github/workflows/codelore-pr.yml` shows the full deployment pattern with the critical gotchas documented (`fetch-depth: 0`, three-dot merge-base, SARIF upload permissions).

### Fixed (Plan 8 §2 follow-up + §6 architectural finding)
- **Mailmap canonicalization for Name+Email entries.** `GixRepo`'s inline mailmap lookup during ingest now passes `event.author_name` to `gix::SignatureRef`, so `.mailmap` entries of the form `Canonical Name <c@x> Original Name <o@x>` resolve correctly. Before: only the email-only form worked, which silently left Alice/Carol-style aliases un-canonicalized.

### Added (Plan 8 §1 — pre-tag hardening)
- **`--analysis` enumeration in error messages.** `UnknownAnalysisError::Display` now lists every supported analysis (12 today) so typos surface a complete menu instead of just "unknown analysis: bogus".
- **`write_clones_csv` snapshot test** locks the 9-column CSV shape against silent header drift.
- **Spec §8 status** updated: clone-coupling row marked `PARTIAL — clone detection ships in Plan 7; intersection lands in Plan 8 §6`.
- **README + perf-evidence-v1.md** corrected for accurate counts and warm/cold timing distinction (the previous 4× "drift" finding was a cold-cache first-run artifact, not a real regression).

### Added (Plan 8 §2 — spec-gap closures)
- **`--analysis authors` standalone.** Closes spec §1.1 gap. CSV header `name,n-commits` matches code-maat; sorted desc by commit count, tiebreak by name. Available across csv/json/markdown formats.
- **`-g` / `--group-file` flag** parsed and forwarded to `Options.group_file`. Aggregation logic deferred to Plan 9; today the flag warns "recognized but no effect yet".
- **`--exclude PATTERN` + `.codeloreignore`** for path-filter. Repeatable flag; the file uses `.gitignore`-style line conventions (blank + `#` comments ignored). Honored by `clones` today; other analyses join in Plan 9. Globset 0.4 dep added.
- **Clones JSON + Markdown emitters.** `--analysis clones --format json` and `--format markdown` now work (CSV was the only output in Plan 7).
- **CODELORE-CLONE SARIF 2.1.0 rule.** New rule for clone-family findings (live-clones in Plan 8 §6 get the higher-severity `CODELORE-LIVE-CLONE` variant). One SARIF result per family with multiple `locations[]` (one per member with line range); `partialFingerprints.cloneGroupFingerprint/v1` keys the family for stable cross-run identity.

### Fixed (Plan 8 §2 follow-up)
- **Mailmap canonicalization for Name+Email entries.** `GixRepo`'s inline mailmap lookup during ingest now passes `event.author_name` to `gix::SignatureRef`, so `.mailmap` entries of the form `Canonical Name <c@x> Original Name <o@x>` resolve correctly. Before: only the email-only form worked, which silently left Alice/Carol-style aliases un-canonicalized.

### Added (Plan 7: Clone Detection — Type 1 + Type 2)
- **`codelore analyze --analysis clones`** — surfaces clone families across the working tree at HEAD.
  - **Algorithm**: AST structural hashing on tree-sitter parses. Pre-order walk emits `(node_kind_id, child_count)` pairs while skipping identifier + literal nodes; SHA-256 over the byte stream is the 256-bit `Fingerprint::digest`. Identical digests = clone family.
  - **Catches**: Type 1 (exact, ignoring whitespace) and Type 2 (renamed/parameterized — names, types, literals normalized away).
  - **Languages**: Rust, Python, Java, JavaScript, TypeScript — same Tier-1 set as the complexity module.
  - **CSV output** (9 columns): `clone-group, fingerprint, entity, function, start-line, end-line, node-count, similarity, family-size`. Similarity is always `1.0000` for T1+T2; Type 3 near-miss (MinHash + LSH at Jaccard < 1.0) is deferred to v1.x.
  - **Validation**: ran against gitoxide @ 10k commits — surfaces ~760 real clone families (URL-parser test pairs, error-formatter duplications, etc.).
  - **HEAD-only**: no git ancestry needed; works on shallow clones, untracked trees, and non-git working dirs (the CLI short-circuits the ingest pipeline for this analysis).
  - **Tuning knob**: `Options::min_clone_node_count` (default 30 ≈ 5-8 statements) drops trivial getters/setters from results.
- **Modules added**: `codelore_lib::clones::{fingerprint, extractor, language}` + `codelore_lib::analyses::clones`. Direct deps added: `tree-sitter 0.25.3`, the 5 Tier-1 grammar crates (exact-pinned to match `codelore-rca`'s ABI), `walkdir 2`.
- **Schema**: `clones` table added to `FactsDb` v1 schema for future ingestion-time storage (clone-coupling intersection lands in v1.x).

### Deferred to v1.x
- **Type 3 near-miss clones** via MinHash + LSH (Plan 7 §2 Task 4) — design + algorithm captured in `docs/superpowers/plans/2026-06-07-codelore-plan-7-clone-detection.md`.
- **`clone-coupling` analysis** — the CodeScene X-Ray pattern: clones × Fisher-significant co-change. The differentiator. Pure SQL JOIN over the existing `clones` + `coupling` tables once T03b (FactsDb integration) lands.
- **SARIF rules `CODELORE-CLONE` + `CODELORE-LIVE-CLONE`** for the GitHub Code Scanning UI path.
- **JSON / Markdown output formats** for clones — currently CSV only.

### Changed (project rename)
- **Project rebranded `bca` → `CodeLore`.** Cargo crates `bca-lib` / `bca-cli` / `bca-rca` renamed to `codelore-lib` / `codelore-cli` / `codelore-rca`; binary `bca` → `codelore`; `BcaError` → `CodeLoreError`; SARIF rule `BCA-HOTSPOT` → `CODELORE-HOTSPOT`; SARIF tool name `bca` → `codelore`; SARIF property keys `bca/*` → `codelore/*`; provenance field `bca_version` → `codelore_version`; markdown headings `# bca <analysis>` → `# CodeLore <analysis>`; spec + plan filenames `2026-06-06-bca-*` → `2026-06-06-codelore-*`. Workspace dir kept as `codescene` to avoid breaking local path bookmarks. No semantic change — only naming.

### Added (Plan 6 in progress: Differential testing + Perf + Release infra)
- **`GitCliRepo`** — shell-out impl of the `Repo` trait, treats C git as ground truth. 11 integration tests cover open, walk, mailmap resolution, `changed_files`, hunk extraction, and `commit_metadata`.
- **`differential_repo` fixture** — 50-commit generated test repo with `.mailmap` (3 alias mappings), 3 authors + 1 bot, 1 rename (`src/old_name.rs` → `src/new_name.rs`), 1 `--no-ff` merge, deterministic per-hour-offset commit dates. **All 8 differential property tests pass**: walk identity (rev-set equality), per-commit field equivalence, `resolve_alias`, `changed_files`, `commit_metadata`, bot-commit visibility, rename visibility, merge-commit count.
- **`GitCliRepo` parser fix** — closed the bug surfaced by the differential test: `parse_git_log_stream` was dropping the commit immediately after a `--no-ff` merge with empty name-status. New `starts_with_pretty_block` helper detects chunks that begin directly with a pretty line (no preceding name-status block); `split_off_name_status_prefix` and `extract_name_status_prefix` fast-path that case. Reproducer added as a `#[cfg(test)]` unit test.
- **`criterion` bench harness** — `crates/codelore-lib/benches/end_to_end.rs` with 3 targets: `ingest_tiny` (5-commit sanity), `ingest/medium_500_commits` (500-commit fixture, CI baseline), `ingest_kernel/linux_kernel_snapshot` (only runs when `CODELORE_BENCH_LINUX_KERNEL_PATH` is set; validates spec §1.1 release blockers). `medium_repo` fixture in `test_support` (3 authors, 25 files, deterministic dates, `gc.auto=0` to avoid loose-blob races during 500-commit init).
- **Weekly CI bench job** at `.github/workflows/bench.yml` — Monday 06:00 UTC, cached cargo + Linux kernel snapshot, `benchmark-action/github-action-benchmark` regression tracking (>10% threshold).
- **`cargo-dist` config** — `[workspace.metadata.dist]` pins cargo-dist 0.28 with 6 release targets (Linux gnu/musl x86_64, Linux gnu aarch64, macOS aarch64+x86_64, Windows MSVC) and 4 installer kinds (shell, powershell, MSI, Homebrew via `emrecdr/homebrew-codelore` tap).
- **`.github/workflows/release.yml`** — multi-platform binary build + SLSA L3 build provenance via `slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml@v2.1.0`. Triggers on tag `v*` push. Per-target archive sha256s are forwarded as SLSA subjects. Aggregates artifacts + `.intoto.jsonl` provenance into the GitHub Release.
- **Distroless container image** — `Containerfile` (3-stage: chef → planner → builder → distroless/cc-debian12:nonroot, ~25-30 MB compressed) + `.github/workflows/container.yml` (linux/amd64 + linux/arm64 via QEMU, SBOM, attested build provenance, GHA layer cache). Image at `ghcr.io/emrecdr/codelore`.
- **PGO scaffolding** — `[profile.release-pgo]` + `scripts/pgo.sh` 3-stage `cargo-pgo` campaign script. v1.0 ships the standard release profile; the PGO campaign is deferred to v1.1 per spec §6.5.

### SLSA provenance rewrite (root-cause fix)

- **Replaced `slsa-framework/slsa-github-generator` reusable workflow with per-matrix-job `actions/attest-build-provenance@v4`.** The reusable workflow aggregates `outputs.hashes` across the build matrix into a single `base64-subjects` input — but GitHub Actions only preserves the LAST matrix job's outputs, silently dropping 4 of 5 subject hashes. (Also, our per-job hash computation emitted only `<hex>` while the reusable workflow expects `sha256sum`-format `<hex>  <filename>\n` lines.) v0.1.0's first publish attempt failed at `provenance / final` (exit code 27 — `SUCCESS=false` because the subject list didn't match the uploaded asset count) and skipped the `release` job that would have attached binaries to the release page.
- **New attestation flow:** each matrix job (`actions/attest-build-provenance@v4`) signs its OWN artifact with the runner's OIDC token, publishes to sigstore's Rekor transparency log, and anchors the attestation to the artifact's content hash. No matrix aggregation, no subjects-format coupling. SLSA v1.0 spec (the reusable workflow used SLSA v0.2). Consumers verify with `gh attestation verify <archive> --owner emrecdr`.
- **Removed the separate `provenance:` job** — attestation is now an inline step in `build` jobs, before the `upload-artifact` step. `release` job's `needs:` list shrinks from `[plan, build, provenance]` to `[plan, build]`.

### Runner version bumps

- **macOS release builds `macos-14` → `macos-15`** in `release.yml`. macos-15 (Sequoia) has been the GitHub-default macOS runner since mid-2025; macos-14 still works but is the prior stable. Both `aarch64-apple-darwin` and the cross-compiled `x86_64-apple-darwin` build on the same `macos-15` host via Apple's bundled cross-target SDK.

### CI action versions + Dependabot

- **Every GitHub Action in `.github/workflows/*.yml` bumped to latest major.** v4 → v6 (`actions/checkout`), v4 → v5 (`actions/cache`), v4 → v7 (`actions/upload-artifact`), v4 → v8 (`actions/download-artifact`), v2 → v4 (`actions/attest-build-provenance`), v3 → v4 (`docker/setup-buildx-action`, `docker/login-action`), v5 → v6 (`docker/metadata-action`), v6 → v7 (`docker/build-push-action`), v2 → v3 (`softprops/action-gh-release`), v0.0.6 → v0.0.10 (`mozilla-actions/sccache-action`). Resolves the "Node.js 20 actions are deprecated" warning we hit on the v0.1.0 release pipeline (Node 20 sunsetting Sep 2026; latest majors all use Node 24).
- **`timeout-minutes:` set on every job** in `release.yml` + `container.yml`. Without these, the GitHub default is 6 hours, which is how we ended up with a queued-for-2.5-hours macos-13 build sitting open during the v0.1.0 debug cycle.
- **`.github/dependabot.yml` added** with weekly grouped PRs for both `github-actions` and `cargo` ecosystems. Single batched PR per ecosystem per week keeps action versions current without per-bump review churn.

### CI hardening (Tier 2 roadmap items shipped)

- **`cargo-nextest`** replaces `cargo test` for unit/integration runs in `ci.yml`. Smarter scheduling, faster process spawning, better failure aggregation; ~20-30% wall-time win on the workspace's test phase. Doc tests still run via `cargo test --doc` since nextest doesn't support them.
- **Path filters** — docs-only pushes (`docs/**`, `**/*.md`, `LICENSE`, `.gitignore`) now skip the full CI matrix. Limited to `push` events so PRs always run CI (avoids the required-check stuck-pending pitfall).
- **`concurrency:` blocks** added to `ci.yml` and `bench.yml` (matching the `release.yml` + `container.yml` pattern). CI cancels superseded runs for the same ref; bench keeps superseded runs so the timeline data isn't dropped.
- **`timeout-minutes:` + `fail-fast: false`** on the test matrix — one OS failing no longer cancels the other two, and stuck jobs fail fast instead of consuming the full 6-hour GitHub default.
- **`sccache --show-stats`** diagnostic step on test + clippy jobs. Gives actual hit-rate numbers per OS per run; needed to design the next round of cache-key fixes for the 0%-hit-rate-on-Windows issue tracked in the roadmap.

### Pre-tag release-pipeline hardening

- **`x86_64-apple-darwin` cross-compiled from `macos-14`** instead of the deprecated `macos-13` runner. GitHub's macos-13 pool has been capacity-constrained since early-2026 (jobs queue for hours or never pick up); macos-14 (aarch64 Sonoma) carries the same Apple cross-SDK and builds the x86_64 target via the existing `--target` flag with no source changes. Intel-Mac binary distribution preserved.
- **Container build split per-arch + manifest merge.** Previously a single `ubuntu-latest` job built `linux/amd64,linux/arm64` together using `setup-qemu-action` to emulate arm64. The emulated build was pathologically slow on Rust workspaces (~2.5 hours when it didn't hang outright) and tripped rustup's toolchain-extract step with `EFAULT (Bad address)` errors. Rewrote `container.yml` as two parallel native-runner jobs (`ubuntu-latest` for amd64, `ubuntu-24.04-arm` for arm64), each pushing by digest only; a final `merge` job assembles the multi-arch manifest via `docker buildx imagetools create` and attests provenance on the manifest digest. No QEMU, ~7 min per arch.
- **`concurrency:` blocks added to both `release.yml` and `container.yml`** keyed on `${{ github.workflow }}-${{ github.ref }}` with `cancel-in-progress: true`. Without this, retags during debugging produced zombie queued runs that competed with the live run for scarce runner slots — observed firsthand during the v0.1.0 push cycle (five tag attempts left three queued release zombies behind that had to be cancelled manually).

### Pre-tag bug-free pass + dep refresh

- **6 verified correctness bugs fixed** before first stable tag:
  - **R1** — Hotspot score formula was producing `[-9, 1]` values because the SQL used `(10 − code_health) / 10` while `code_health` is on the `[0, 100]` scale. Corrected to `(100 − code_health) / 10` so the emitted score range matches the documented `[0, 10]` interpretation; ranking direction unchanged. Spec doc, README, and advanced-usage updated to the new formula.
  - **R12** — `write_hotspots_parquet` was emitting only `entity, revs, cognitive`, silently dropping the `code_health` and `score` columns CSV/JSON/Markdown/SARIF show. Parquet now emits the full 5-column schema using the same SQL as `run_hotspots`.
  - **R2** — `GixRepo::walk_commits` was ignoring `Options.after` / `Options.before` (parameter was prefixed with `_` and unused). Now resolves each commit's author-time date and applies the bounds during OID collection. GitCliRepo already honored these via `git log --after/--before`.
  - **R3** — `--after YYYY-MM-DD` / `--before YYYY-MM-DD` / `--include-merges` CLI flags added to `AnalyzeArgs`. Date flags reuse the `parse_date` helper from `--age-time-now`.
  - **R4** — `GixRepo` now honors `Options.include_merges` (filter at OID-collection time when `parent_count > 1` and the flag is off). Mirrors GitCliRepo's existing `--no-merges` behavior so both backends produce identical event streams. `authors_test::authors_against_differential_fixture` updated from asserting 5 authors (the old leaky GixRepo behavior) to 4 authors (spec-correct default-filtered); new sibling test `authors_against_differential_fixture_with_merges_included` covers the `include_merges = true` path.
  - **R6** — Kamei `enrich_history` and `enrich_experience` rewritten from per-commit correlated subqueries (O(N²) on commit count) to single hash-joined `UPDATE … FROM (subquery)` passes (plus leading zero-out to preserve `COALESCE(…, 0)` semantics for history-less commits). Orders of magnitude faster on large repos; output bit-identical on existing fixtures.

- **Flaky-fixture race fixed**: `differential_repo::build()` was emitting intermittent `error: invalid object … Error building trees` when run alongside other parallel test binaries. Added `git config gc.auto 0` (matches the proven `medium_repo` fix) plus a module-level `BUILD_MUTEX` to serialize concurrent fixture builds across test threads. Each test still gets its own tempdir; only the fixture-build's git-invocation storm is serialized. Stress-tested locally with 10× consecutive parallel runs (10/10 pass after fix; ~1/5 fail rate before).

- **Doc reorganization**: `docs/codebase_analysis_report.md` was mixing two audiences (descriptive architecture + forward-looking improvement backlog). Split into `docs/codebase_analysis.md` (architecture overview — workspace shape, pipeline data flow, threading model, 21-analysis taxonomy, identity layers, Kamei vector, quality posture) and merged the 4 open improvement items into `docs/roadmap-v1.x-and-beyond.md` Tier 3 / Tier 4. Also deleted three dev-process snapshot reports (`deep_analysis_report.md`, `validated_findings_report.md`, `validation-report-2026-06-07.md`) that served earlier sprints but have no role post-`v0.1.0`. Four user-facing references in README + advanced-usage updated.

- **Toolchain + dep refresh to latest stable**:
  - **Rust 1.89.0 → 1.96.0** in `rust-toolchain.toml` and `.github/workflows/ci.yml` (`dtolnay/rust-toolchain@1.96.0` × 3 jobs). MSRV in Cargo.toml workspace metadata bumped to match (was `1.87`).
  - **`sha2` 0.10 → 0.11**, **`fancy-regex` 0.14 → 0.18**, **`dirs` 5 → 6** — all direct deps, API-compatible at usage sites.
  - **`duckdb`** stays at `=1.10503.1` — cargo's resolver confirmed this IS the latest crates.io version (the duckdb-rs project uses an unusual minor-version scheme; `1.5.x`-style versions do not exist as crate releases). The exact pin remains intentional for cache-binary-format determinism.
  - **`tree-sitter` family** stays exact-pinned — deliberate ABI compatibility constraint with the vendored MPL-2.0 `codelore-rca` crate (parser ABI breaks if grammar versions drift).
  - **Two new Rust 1.96 clippy lints addressed**: `clippy::collapsible_match` in vendored `codelore-rca` (crate-level `#![allow]` so upstream code stays unmodified) and `clippy::map_unwrap_or` in `differential_repo_test.rs` (`.map(…).unwrap_or(false)` → `.is_ok_and(…)`).

### Pre-tag stabilization (post-version-bump, pre-`v0.1.0` push)

- **fix(facts): Windows `sync_all` Access-Denied** — `FactsDb`'s cache `.tmp` file was being opened with `std::fs::File::open` (read-only handle), then `sync_all()` was called. On Unix that's a no-op-on-mode `fsync`, but on Windows `sync_all` lowers to `FlushFileBuffers` which requires `GENERIC_WRITE` on the handle and returns `ERROR_ACCESS_DENIED` (os error 5). Surfaced as 17 of 28 Windows CI tests failing across the entire `--analysis` matrix. Fixed by opening the temp file with `OpenOptions::new().read(true).write(true)` so Windows accepts the handle.
- **ci: pin toolchain + drop nightly-only rustfmt options** — `rustfmt.toml` had `imports_granularity = "Module"` and `group_imports = "StdExternalCrate"` (both nightly-only). Stable rustfmt warned-and-ignored them, which let formatting drift accumulate across 17 source files. Dropped both options, ran `cargo fmt --all` to normalize the workspace, and pinned `.github/workflows/ci.yml` to `dtolnay/rust-toolchain@1.89.0` as defense-in-depth.
- **chore: resolve `<owner>` placeholders to `emrecdr`** — `Cargo.toml` `repository` field + `cargo-dist` Homebrew tap + `ghcr.io` image references in CHANGELOG, RELEASING.md, and the validation report.

### Deferred (out of scope for `v0.1.0`)
- **Code-maat golden parity tests** — requires Leiningen/Clojure runtime to invoke code-maat against the fixture repos. Owner runs the capture script and commits the resulting `fixtures/golden/code-maat/*.csv` files. Tracked for `0.1.x` patch.
- **`v1` performance evidence** — requires a Linux kernel snapshot to run the kernel bench and capture wall-clock + peak RSS into `docs/perf-evidence-v1.md` validating the spec §1.1 release blockers (<10 min, <4 GB). Tracked for `0.1.x` patch.
- **Legacy `--temporal-period N` under `--code-maat-compat`** — code-maat's sliding-window-with-duplication semantic. The modern `--time-bucket` covers the actual user need. Revisit only if migration users hit this.

### Added (Plan 5: Output formats + Provenance)
- **5 new output formats** alongside CSV (all 11 analyses unless noted):
  - `--format json` — structured JSON (pretty-printed, serde-derived)
  - `--format sarif` — **SARIF 2.1.0 with `CODELORE-HOTSPOT` rule** (hotspots only — the published Behavioral SARIF differentiator). Security severity proxy: `(100 − code_health) / 10`; stable `partialFingerprints` via `sha256(repo_root|path)`
  - `--format markdown` — GitHub-Flavored Markdown pipe tables, designed for `$GITHUB_STEP_SUMMARY`
  - `--format parquet --output FILE` — DuckDB-native `COPY ... TO ... (FORMAT PARQUET)`. Plan 5 ships hotspots, revisions, summary; binary format requires `--output`
  - `--format sqlite --output FILE` — full 7-table fact-store dump via `ATTACH '...' (TYPE SQLITE)`. Provenance table is included inside the DB; no sidecar needed. Requires explicit `INSTALL sqlite; LOAD sqlite;` (DuckDB bundled build doesn't auto-load)
- **Provenance manifest sidecar** (`codelore_lib::provenance::Manifest`) — 18-field reproducibility receipt:
  - `codelore_version`, `gix_version`, `arrow_version`, `duckdb_version`, `run_started_at` (UTC RFC3339)
  - `repo_path`, `analysis`, `after_date`, `before_date`, `age_time_now`, `merge_handling`, `include_merges`
  - All threshold knobs: `min_revs`, `min_shared_revs`, `min_coupling_pct`, `max_changeset_size`, `fisher_significance`, `complexity_sample`
  - Emitted as `{output}.provenance.json` next to every file output **except** `--format sqlite` (where the provenance table lives inside the DB) and stdout output (where no path exists). Addresses Spadoni 2025's 500% inter-tool disagreement problem
- CLI: `codelore analyze --format {csv | json | sarif | markdown | parquet | sqlite}` — 2-level (format × analysis) dispatch with validated constraints:
  - `parquet`/`sqlite` require `--output PATH` (binary; can't stream stdout)
  - `sarif` requires `--analysis hotspots` (Plan 5 scope; coupling SARIF lands in Plan 6)

### Added (Plan 4: Analyses + Identity + Kamei + Code Health completion)
- **Identity resolution**:
  - `.mailmap` lookup via gix mailmap API in `GixRepo::resolve_alias`
  - `bots.toml` default-deny bot list (dependabot, github-actions, copilot, claude-code, renovate, pre-commit-ci)
  - AI attribution stub (`ai-authored` / `ai-assisted` / `human`) on every commit
- **Kamei 14-feature change vector** (Kamei et al. JIT-SDP canonical) populated via SQL UPDATE pass after ingest:
  - Diffusion: NS, ND, NF, entropy
  - Size: LA, LD, LT (LT stubbed to 0 — Plan 5 may improve)
  - Purpose: FIX (regex on bug/fix/defect keywords)
  - History: NDEV, AGE, NUC
  - Experience: EXP, REXP, SEXP
- **8 new analyses**:
  - `code-age` — months since last modification per file (spec §1.1)
  - `abs-churn` — date-grouped lines added/deleted/commits
  - `author-churn` — canonical-author-grouped churn (uses .mailmap resolution)
  - `entity-churn` — file-grouped churn
  - `communication` — Conway's law author-pair shared-work + strength
  - `code-ownership` — Fractal Value (1−HHI complement) + main developer per file
  - `change-coupling` — per spec §3.2.1 correctness invariants (max-changeset-size pre-filter, mirrored pair dedup, Fisher exact significance at p<0.05 default)
  - `summary` — 4-row repo overview (commits/changes/entities/authors)
- **Code Health composite** now uses all 4 inputs from spec §4.6 (cognitive 0.40 + churn 0.25 + fragmentation 0.15 + coupling 0.20). Verified: src/main.rs (4 commits) now ranks lower than src/lib.rs (1 commit) in Code Health.
- CLI: `codelore analyze --analysis NAME --format csv` works for all 11 analyses.
- `--complexity-sample {head|adaptive|full}` flag (Plan 4 ships head only; adaptive/full land in Plan 5)

### Added (Plan 3: Complexity Integration + Hotspots + Code Health)
- `codelore-lib::complexity` module — wraps `codelore-rca` for Tier-1 languages (Rust, TS/JS, Python, Java)
- Path-based language dispatch (`Tier1Language::from_path`) maps file extensions to codelore-rca parsers
- Function-level entity extraction via `codelore-rca::FuncSpace` traversal (file + function + class scopes)
- `FactsDb::ingest()` now populates `entities` and `complexity_metrics` tables at HEAD by reading working-tree files
- `hotspots` analysis (`codelore_lib::analyses::hotspots::run_hotspots`) per spec §1.1 published formula:
  `percentile_rank(revisions) × percentile_rank(cognitive_complexity) × (10 − code_health) / 10`
- `code-health` composite analysis (`codelore_lib::analyses::code_health::run_code_health`) per spec §4.6
  - Plan 3 wires cognitive input only; churn/fragmentation/coupling inputs land in Plan 4
  - Reduced formula: `100 × (1 − 0.40 × normalize(cognitive))`
  - Range: [0, 100], higher = healthier
- CLI: `codelore analyze --analysis hotspots --format csv` and `codelore analyze --analysis code-health --format csv`
- New CSV emitters: `write_hotspots_csv`, `write_code_health_csv` with shared `quote_if_needed` helper

### Added (Plan 2: RCA Vendor)
- `crates/codelore-rca/` — vendored fork of mozilla/rust-code-analysis
  - SPDX: `MPL-2.0 AND GPL-3.0-only`
  - Dropped `-web`, mozjs grammar, ABC/WMC/NPA/NPM impls
  - Mozjs fully excised (Option B from UPSTREAM.md); standard `tree-sitter-javascript` covers everything we need
  - Mozcpp retained (language_cpp.rs is generated from mozcpp grammar; standard tree-sitter-cpp would silently break C++ metrics)
  - Per-language tree-sitter grammars exact-pinned for ABI compatibility with our generated `language_*.rs` enums
  - `metrics-experimental` feature flag for JS/TS Halstead+MI (RCA bugs #528 #1183)
  - 199 upstream RCA unit tests preserved and passing
  - 5 Tier-1 language smoke tests (Rust/Python/Java/TS/JS) + 1 conditional for metrics-experimental

### Fixed (Plan 1 carry-over)
- `CodeLoreError::exit_code()` now wired into `codelore` CLI per spec §6.6 (Plan 1 always exited 1)
- `FactsDb::query_one_value` gated behind `test-support` feature (no longer in production builds)
- `gix_repo.rs` "Plan 11" comment typo → "Plan 4"
- Added file-backed `FactsDb::open()` roundtrip test (was untested in Plan 1)

### Added (Plan 1: Phase 0 + Walking Skeleton)
- 3-crate Cargo workspace (`codelore-lib`, `codelore-cli`, future `codelore-rca`)
- Core types: `CommitEvent`, `FileChange`, `Hunk`, `ChangeType`, `KameiFeatures`
- `AnalysisName` enum and `Options` struct with code-maat parity defaults
- `arrow_facade` module — single re-export point for `arrow-rs`
- `Repo` trait + `GixRepo` impl (read .git via gix 0.84)
- Fixture builder (`test_support::tiny_repo`) for reproducible 5-commit test repos
- `FactsDb` — DuckDB-backed fact store with v1 schema (7 tables)
- Commit ingestion pipeline (gix → crossbeam channel → DuckDB Appender)
- `revisions` analysis (SQL view + Rust orchestrator)
- CSV output emitter (code-maat header parity)
- `codelore analyze --analysis revisions --format csv` CLI
- GitHub Actions CI (fmt, clippy, test on 3 OSes, cargo-deny)
- Justfile, deny.toml, renovate.json, rust-toolchain.toml

### Pending (subsequent plans)
- Plan 2: RCA vendor (Mozilla rust-code-analysis fork) + Go support ✅
- Plan 3: complexity integration + hotspots + Code Health composite ✅
- Plan 4: 8 new analyses + Kamei vector + identity resolution + full Code Health composite ✅
- Plan 5: SARIF + Markdown + Parquet + SQLite + provenance manifest ✅
- Plan 6: differential testing harness + perf benchmarks + release infra
