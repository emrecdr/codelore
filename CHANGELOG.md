# Changelog

Conventional Commits format. All notable changes documented here.

## [Unreleased]

## [0.1.3] - 2026-06-09

### Fixed — additional correctness findings surfaced post-v0.1.2

- **`Repo::resolve_alias` now accepts `(name, email)` so `.mailmap` name+email
  rules resolve in both walkers (NEW-A).** `.mailmap` supports two rule
  shapes: 3-token `Canonical Name <canonical@email> <old@email>` (email-only
  match) and 4-token `Canonical Name <canonical@email> Old Name <old@email>`
  (name+email match). Earlier, the trait method took only `email` — both
  `GixRepo` (passed `name: b""` to gix's SignatureRef) and `GitCliRepo`
  (passed `<email>` to `git check-mailmap`) silently missed all 4-token
  rules. The bug was invisible because `GixRepo::walk_commits` had its OWN
  inline mailmap resolution that already passed the actual author name —
  so `GixRepo::walk_commits` and `GitCliRepo::walk_commits` diverged on any
  real `.mailmap` using the 4-token form. Differential test fixtures didn't
  exercise 4-token rules, so the divergence shipped quietly. Trait signature
  is now `fn resolve_alias(&self, name: &str, email: &str) -> String`;
  callers pass `event.author_name` alongside the email. `GitCliRepo`'s
  per-event mailmap cache key extended to `(name, email)` to avoid silently
  sharing one resolution across distinct identities that happen to share
  an email. New test `mailmap_name_plus_email_rule_resolves` exercises both
  forms; the existing `differential_repo_test::resolve_alias_matches` now
  iterates over `(name, email)` pairs and asserts parity for both
  empty-name and paired-name probes.

- **Clone-extractor recursion descends into function bodies so nested
  helpers get their own fingerprints (NEW-B).** The tree-sitter walker in
  `clones/extractor.rs::visit` used to `return;` immediately after
  emitting a function's fingerprint — a comment claimed "nested functions
  become separate entries via the outer-loop walk that follows", but no
  such outer walk existed. The early return silently dropped every nested
  function/closure/IIFE from clone detection. For Python (`def` inside
  `def`), JavaScript (closures everywhere), and Rust (`fn outer() { fn
  helper() { ... } }`), this missed real clone families. The fix removes
  the `return;` so recursion descends into the function body and extracts
  nested fingerprints additively — the outer function's own fingerprint
  is unchanged (still computed via the same `walk_preorder_internal`
  pre-traversal that captures nested structure as part of its sequence),
  so existing clone-pair detection at the outer level is not regressed.

### Fixed — correctness + performance (closes the v0.1.2 deferred findings)

- **Cache hits on dirty working trees now emit a warning (F3).** The persistent
  cache key is hashed from `(canonical_repo_path, head_sha, opts, version,
  schema)` — explicitly NOT from worktree state. That's correct for the
  majority of analyses (`revisions`, `coupling`, `ownership`, `churn`,
  `messages`, etc. all read only committed history), but `hotspots`-style
  HEAD-time metrics computed by `ingest_complexity_at_head` and
  `populate_clones_at_head` read files from disk at ingest time and can
  silently mismatch the current worktree on a cache hit. Now: a
  `tracing::warn!` fires whenever a cache hit lands on a working tree with
  uncommitted modifications or untracked Tier-1 source files, telling the
  user to pass `--no-cache` if the dirty state matters. Detection uses gix's
  `Repository::status` API for `GixRepo` and `git status --porcelain` for
  `GitCliRepo`. New `Repo::is_worktree_dirty(&self) -> bool` trait method
  with a `false` default so future backends can opt in without breaking the
  existing API contract. Errors during detection are deliberately swallowed
  (return `false`) — a missed warning is strictly preferable to a hard
  analyze failure on an edge case like an unusual submodule layout.

- **`--after` date-range walk uses gix's `ByCommitTimeCutoff` for graph
  pruning (NEW-3 perf).** Previously the walker eagerly traversed the
  entire reachable commit graph via `repo.rev_walk([head]).all()`, then
  dropped commits in-memory via `.filter_map`. On large histories
  (linux-kernel size, 100k+ commits) this turned a "last 7 days" analysis
  into the same work as a full-history walk. Now: when `opts.after` is
  set, the walk runs with `Sorting::ByCommitTimeCutoff { seconds, order:
  NewestFirst }` so gix stops traversing the moment it crosses below the
  cutoff. The cutoff uses committer time (gix's primitive); the in-memory
  filter still enforces author-time semantics. Unusual rebases that move
  commit time far earlier than author time can now be dropped by the
  cutoff — but git's own `--after` flag has identical behaviour, so this
  CONVERGES `GixRepo` and `GitCliRepo` rather than diverging them. The 10
  differential cross-walker tests (which assert event-stream parity) all
  still pass on the existing fixtures. `--before` falls through to the
  existing in-memory filter unchanged — gix has no symmetric primitive
  for "walk forward until newer than" and `--before`-only is the rarer
  workflow.

### Added — internal

- **`scripts/cut-release.sh`** — codifies the release-cut procedure into a single idempotent script. Validates preconditions (clean tree, on main, in sync with origin, no existing tag, non-empty `[Unreleased]`), bumps the workspace version, flips the CHANGELOG section, pushes the release commit, waits for CI green, runs the `disable-ruleset → tag → push → restore` dance (auto-restoring the ruleset via `trap EXIT` so the repo is never left unprotected on interrupt or failure), and prints the release-monitor links. Captures the v0.1.2 lesson that ruleset `required_status_checks` does not reliably consume Check Runs even with `integration_id`. Supports `--dry-run` for safe preview and `--skip-ci-wait` for re-attempts after the commit has already gone green.

- **Dependabot ignore for `dtolnay/rust-toolchain`** in `.github/dependabot.yml`. The action's `@<version>` tag names the Rust toolchain to install, not the action's own release version — Dependabot was offering bumps to versions that don't yet ship as stable (closed PR #1 was 1.96.0 → 1.100.0 while 1.96.0 was still current). Coordinated Rust bumps happen deliberately at release-cut time via `scripts/cut-release.sh`, not weekly via Dependabot.

## [0.1.2] - 2026-06-09

### Fixed — correctness

- **Pre-flight banner failures now exit with the correct typed-error code.**
  The new `analyze` pre-flight gate (path-missing / not-a-repo / empty-repo /
  output-not-writable) was emitting `anyhow::bail!` without a wrapping
  `CodeLoreError`, so the chain-walk in `main()` fell through to the default
  exit code `1` instead of the spec §6.6 codes (`3` for repo errors, `5` for
  output errors). Replaced each `bail!` with `Err(CodeLoreError::Repo(...))`
  / `Err(CodeLoreError::Output(...))`. Closes the regression in
  `invalid_repo_exits_with_code_3` that I introduced in the banner commit
  earlier in this release.

- **Rename-lineage CTE now respects chronological ordering.**
  `materialize_path_lineage` walked the rename graph by joining
  `c.rename_from = l.current` recursively, with no chronological constraint.
  If a filename was renamed away and a DIFFERENT file later took the same
  name, the CTE would walk through the old rename and produce a spurious
  lineage edge — e.g. `A → B` in commit 1 plus `C → A` in commit 10 produced
  a fictitious `C → A → B` trace that merged two unrelated files' history.
  Fixed: the CTE now joins `commits.date` in both the seed and recursive
  step, and the recursive step adds `WHERE co.date > l.current_date` so
  only chronologically forward renames extend a chain.

- **`coupling` analysis no longer silently truncates significant pairs.**
  The SQL query had `LIMIT ?` as its final clause, applied BEFORE the Rust-side
  Fisher exact significance filter — so when `--rows N` was set, DuckDB
  returned the top-N pairs by degree, then Fisher dropped the non-significant
  ones, leaving the user with fewer than N rows even when more significant
  pairs sat just below the truncation point. Fix: remove `LIMIT` from the
  SQL builder, collect ALL candidates, apply the Fisher filter, then
  `Vec::truncate(rows_limit)` in Rust. The composite call sites
  (`code_health::materialize_centrality`, `clone_coupling::run_clone_coupling`)
  were already passing `Options::with_no_row_limit()`, so they're unaffected
  by the change. Memory cost: the in-memory `Vec` grows from `O(rows_limit)`
  to `O(passing-pairs)` before truncation — bounded by the SQL `WHERE`
  clauses (`min_revs`, `min_shared_revs`, `min_coupling_pct`,
  `max_coupling_pct`) so it stays modest in practice.

- **Hotspot score divisor `/10.0` → `/4.0` closes the documented-range gap.**
  The module-level docstring promised `hotspot_score ∈ [0, 10]` but the math
  capped output at `4.0`: `code_health` is computed inline as
  `100 × (1 − 0.40 × normalize(cognitive))`, bounded `[60, 100]` because the
  `0.40` weight limits the deduction; that makes `(100 − code_health) ∈ [0, 40]`;
  multiplied by two percent ranks (each `[0, 1]`) the unscaled max is `40`;
  dividing by 10 capped scores at `4.0`. Dividing by 4 lands them in the
  documented `[0, 10]` band, matching the CodeScene convention that
  `≈10 ⇒ "on fire"`. **Behavioural impact:** existing reports with a top
  hotspot of `3.78` will now report `≈9.45` for the same file — the RANKING
  is unchanged (only the absolute scale shifts), but anyone with thresholds
  (e.g. SARIF rule cutoffs, code-review alerts) calibrated against the old
  `[0, 4]` range needs to rescale them by ×2.5.

- **`clones` analysis short-circuit now covers all 4 supported formats.**
  Previously the `format == "csv"` guard on the early-exit branch meant
  `--analysis clones --format json|markdown|sarif` fell through to the
  full git-ingest path even though clones extraction is a HEAD-only
  filesystem walk — making non-CSV clones runs 10–100× slower and
  breaking them outright in non-git directories. Expanded the guard to
  `matches!(format, "csv" | "json" | "markdown" | "sarif")` and added
  per-format dispatch inside the short-circuit. Parquet/SQLite for
  clones remain unsupported (no clones emitter for those formats).

- **`communication` analysis is now rename-aware.** Conway's-law shared-
  work output was the last path-aggregating analysis missing canonical
  lineage rewrite: two authors who co-edited the same logical file across
  a rename were counted as if they edited two different files, under-
  counting team coupling on every history with renames. Added the
  standard `materialize_if_needed` + `lineage::rewrite` pair, mirroring
  the pattern in `entity_effort`, `messages`, `ownership`, etc.

### Added — distribution

- **Homebrew tap actually publishes a working formula.** The `emrecdr/homebrew-codelore` tap was created during v0.1.0 setup but stayed empty through v0.1.1 because the release workflow never produced a `.rb` formula — `[workspace.metadata.dist]` in `Cargo.toml` declared `installers = [..., "homebrew"]` and `publish-jobs = ["homebrew"]` but `.github/workflows/release.yml` is hand-rolled and never invokes `cargo dist`. Now wired end-to-end:
  - New `homebrew-publish` job in `release.yml` (`needs: [plan, build, release]`) downloads the build artifacts from the workflow's `upload-artifact` storage (bit-identical to the eventually-published Release assets, no CDN propagation race), computes SHA256 of the four macOS/Linux archives, renders `Formula/codelore.rb` from a heredoc template, checks out `emrecdr/homebrew-codelore` via SSH using the `HOMEBREW_TAP_DEPLOY_KEY` deploy-key secret, and pushes the regenerated formula. Idempotent (`git diff --cached --quiet && exit 0` skips no-op pushes).
  - Formula uses the modern nested `on_macos`/`on_linux` × `on_arm`/`on_intel` pattern (per current Homebrew Formula DSL docs); pinned 4-platform: macOS aarch64+x86_64, Linux aarch64+x86_64-gnu. Single `bin.install "codelore"` since the release archive has the binary at the top level. `test do` asserts `codelore --version` matches the formula version.
  - Cross-repo auth via SSH deploy key (no expiry, min-privilege, single-repo scoped) instead of fine-grained PAT — eliminates the yearly-renewal silent-failure mode.
  - v0.1.1 formula was backfilled manually so `brew install emrecdr/codelore/codelore` works against the existing release; v0.1.2+ get the formula auto-regenerated by the workflow.
- **README install section now leads with Homebrew** alongside `cargo binstall` and `cargo install --git`, replacing the "once a published release lands" placeholder copy from the v0.1.0 README.

### Removed — dead configuration

- **`[workspace.metadata.dist]` block removed from `Cargo.toml`.** The 35-line block had been inert since the release workflow was rewritten (`890537e: fix(release): replace broken SLSA generator with attest-build-provenance`) — `cargo dist` is not invoked anywhere, so the metadata block was misleading any reader who assumed cargo-dist was driving the release. Replaced with a short comment explaining the historical context and pointing at the actual `release.yml` workflow.

### Changed — documentation

- **`docs/RELEASING.md` rewritten to describe the actual release pipeline.** Previously claimed cargo-dist runs the build and handles pre-release detection; now documents the real `release.yml` job graph (`plan` → `build × 5` → `release` → `homebrew-publish`), the `protect-release-tags` ruleset gating tag creation on green CI, the `cargo binstall` auto-scan path, and the manual step needed for pre-release tag handling (since `softprops/action-gh-release` doesn't auto-flag pre-releases).
- **`docs/codebase_analysis.md` §8 release-pipeline line** now lists the actual workflow components (hand-rolled cargo build matrix, `attest-build-provenance`, deploy-key Homebrew publish) instead of "cargo-dist (6 target binaries)".
- **`docs/advanced-usage.md` workspace tree comment** updated: `release.yml` is "cargo-build matrix + SLSA L3 + Homebrew (on tag push)" instead of "cargo-dist + SLSA L3".
- **`docs/roadmap-v1.x-and-beyond.md` musl entry** rewritten: was framed around cargo-dist's `github-custom-runners` field; now describes the two real routes for adding a musl target to the current hand-rolled `release.yml` (musl-cross Docker image or musl-cross-make toolchain install).
- **README "Status" line** drops the `cargo-dist` mention; now lists the actual pipeline outputs (SLSA L3 binaries, distroless container, Homebrew tap, `cargo binstall` manifest).

## [0.1.1] - 2026-06-09

### Fixed — correctness defects discovered post-v0.1.0

- **`.codelorebots` extension hook wired into production (previously dead code).** `BotPatterns::from_repo()` existed and was unit-tested, but the production ingest pipeline called the free `identity::is_bot()` / `identity::ai_attribution()` functions that only consulted `DEFAULT_BOT_PATTERNS`. User-configured patterns from `<repo>/.codelorebots` were silently ignored — internal/custom bots were classified as `human` instead of `ai-authored`, polluting author counts and contribution metrics. `ingest()` now loads `BotPatterns::from_repo(&opts.repo_path)` once and passes it into `ingest_loop`, which routes both the bot flag (used by `author_aliases`) AND ai_attribution through the user-extensible patterns. Added `ai_attribution_with(patterns, …)` as the variant that honors `BotPatterns`.

- **Kamei history features (`ndev`, `nuc`, `age`, `sexp`) routed through canonical lineage.** `enrich_history` and `enrich_experience` joined `changes pchg` against `changes cchg` on literal `path`; a renamed file lost all pre-rename history under both the old and new path. `enrich(db, use_lineage)` now materializes `changes_lineage` first and routes the path joins through it when canonical lineage is on, so rename ancestry merges into the canonical post-rename name. Kamei JIT-SDP predictions for renamed files reflect their full history.

- **GitCliRepo now resolves `.mailmap` at walk time.** Previously the differential-test oracle set `canonical_author: None` with a stale comment claiming "matches GixRepo behaviour" — but GixRepo had been resolving via `gix-mailmap` at walk time since v0.1.0. Two walkers, two `canonical_author` columns, breaking the cross-walker parity invariant the differential tests assert. `walk_commits` now post-processes events with a per-unique-email cache that calls the existing `resolve_alias` helper, so a `git check-mailmap` subprocess runs once per author (not once per commit) and `canonical_author` matches gix's output bit-for-bit.

- **Parquet emitters route through canonical lineage.** `write_hotspots_parquet` and `write_revisions_parquet` had `FROM changes` hardcoded, so Parquet output split renamed files even when `--use-canonical-lineage` was on (default). Now both writers materialize `changes_lineage` and pull from the lineage-resolved source via the shared `analyses::lineage` helper, matching the CSV/JSON/Markdown emitters.

- **Canonical lineage wired into the remaining 9 path-aggregating analyses.** B.1 shipped lineage for `revisions`, `hotspots`, `coupling` as the canonical pattern. New shared `crates/codelore-lib/src/analyses/lineage.rs` module exposes `materialize_if_needed(db, opts)`, `source_table(opts)`, and `rewrite(sql, opts)`. The rewriter regex-handles both SQL conventions (`FROM changes` with qualified `changes.col` refs → `FROM changes_lineage AS changes`; `FROM changes c` with per-query alias `c.col` → `FROM changes_lineage c`). Applied to `churn` (3 functions), `code_age`, `entity_ownership`, `main_dev` (3 metrics), `ownership`, `entity_effort`, `messages`, and `soc`. All 12 path-aggregating analyses now honor `--use-canonical-lineage`. `revisions` + `hotspots`'s prior per-file copies of this pattern were collapsed onto the shared helper.

- **`.mailmap` and `.codeloreignore` edits now invalidate the cache.** The mutable-config content-hashing pass covered `--team-map-file`, `--group-file`, `.codelorebots`, and (after the first simplify pass) auto-discovered `.codelore-teams`. But two other repo-root files were missed: `.mailmap` (consumed by gix at walk time to canonicalize author identities; edits change every author-bearing analysis silently) and `.codeloreignore` (parsed by `build_clones_exclude_set` to filter clone-extraction targets; edits change clone families silently). Both are now hashed into the cache key via `canonical_json()`. Two regression tests in `options::tests`: edit-in-place mailmap and edit-in-place codeloreignore both invalidate.

- **`changes_lineage` temp table now has `path` and `rev` indexes** matching the base `changes` table's covering indexes. Without them, downstream `GROUP BY path` and `JOIN changes ... ON rev` aggregations against `changes_lineage` fell to full table scans, regressing query performance whenever canonical lineage was on (which is the default).

- **`gix_repo::count_loc` skipped for bit-identical Rewrite blobs.** When gix's rewrite tracker emits `diff: None` for a perfect 100% rename (gix's documented contract: source_id == id), the code was still reading both blobs and running a histogram diff — wasted I/O whose result was always `(0, 0)`. Now returns `(100u8, 0, 0)` directly.

- **Rename-aware aggregation via a canonical-lineage CTE (silent file-history split).** Every analysis that aggregated on `changes.path` was treating a renamed file's pre- and post-rename history as two separate entities. The `rename_from` column was captured at ingest by both `GixRepo` (after A.4 enabled rewrite tracking) and `GitCliRepo` — but ZERO analyses queried it. A file renamed once showed split revision counts; coupling pairs across the rename boundary went missing; hotspots ranked under-counted refactored files. New `facts::ingest::materialize_path_lineage` builds a `(old_path, canonical_path)` lookup via a recursive `WITH RECURSIVE` CTE that walks `rename_from` chains (depth cap = 50, deterministic latest-name-wins resolution); `materialize_changes_lineage` then projects `changes` with `path` replaced by the canonical lineage path. Wired into `revisions`, `hotspots`, and `coupling` as the canonical examples — the other 9 path-aggregating analyses gain `changes_lineage` support in `v0.1.2` via the same `build_sql(src)` pattern. New `Options::use_canonical_lineage: bool` (default `true`) gates the behavior; `--no-canonical-lineage` opts out, `--code-maat-compat` also flips it off (code-maat uses `--no-renames` and has never canonicalized). Cache key auto-tracks via `canonical_json()`. Empirical regression test against the `differential_repo` fixture's `old_name.rs → new_name.rs` chain: with lineage ON, `revisions` reports ONE entity with merged counts; with lineage OFF, the historical split returns for code-maat parity.

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

### Internal hygiene

- **`--explain` flag prints the DuckDB optimizer plan** for the analysis's underlying SQL to stderr before running the query. Useful for debugging performance ("which join was the dominator?") and for verifying index use. New `FactsDb::explain_sql(sql, params) -> Result<String>` helper runs `EXPLAIN <sql>` against the connection and returns the plan rows joined by newlines. Initially wired into `run_hotspots` only; deep-analysis follow-up surfaced the gap (20 analyses silently ignored the flag), now closed: shared `analyses::query::explain_if_requested(db, sql, params, label, opts)` helper added to the existing query module, wired across all 21 analyses with a single call before each `prepare(sql)` / `query_map_collect(...)` site. Single-source-of-truth helper means future analyses get `--explain` by adding one line. Doc note in the CLI flag's help text + regression `explain_sql_returns_non_empty_plan` test asserts the plan carries DuckDB operator markers (PROJECTION / JOIN / ORDER).

- **`Options::validate()` rejects pathological flag combinations at the CLI boundary.** Four cross-field invariants used to silently produce empty results: `min_coupling_pct > max_coupling_pct`, `clone_similarity_floor` outside `[0, 1]`, `fisher_significance` outside `[0, 1]`, and `after > before`. `Options::validate(&self) -> Result<()>` now checks all four with descriptive error messages; `codelore-cli::main` calls it immediately after constructing `Options`. The simpler approach was chosen over a full `OptionsBuilder` pattern (which would have forced every callsite to migrate to a new construction path) — one method, zero callsite churn, same coverage. 4 unit tests in `options::tests`.

- **`gix_repo::walk_commits` hoists `.mailmap` parsing out of the per-commit closure.** The mapped iterator step was calling `repo_local.open_mailmap()` on every commit event (re-reading and re-parsing `.mailmap` from disk per commit) AND calling `inner_clone.to_thread_local()` twice per step (once for the diff walk, once redundantly for the mailmap-open). `gix_mailmap::Snapshot` is owned bytes (`Send + Sync`), so it now lives outside the closure and moves cleanly into the mapped iterator; the redundant `to_thread_local()` was removed. On a 10k-commit walk this drops `.mailmap` disk I/O from ~10k reads to 1.

- **`populate_clones_at_head` parallelizes the per-file fingerprint pass via rayon.** HEAD-time complexity extraction was already parallel (`ingest_complexity_at_head` uses `into_par_iter().map_init`), but fingerprint extraction for clone detection stayed sequential — every Tier-1 file was read and fed through tree-sitter on the calling thread. Split into a serial walk phase (cheap `WalkDir` + exclude-globset filter, captures `(absolute path, POSIX rel, lang)`) feeding a `rayon::into_par_iter()` phase that reads + fingerprints in parallel. Fail-fast error semantics preserved via `collect::<Result<Vec<_>>>` (first extract failure short-circuits the pass); unreadable files silent-skip as before. Mirrors the established complexity-pass pattern.

- **`output/csv.rs::quote_if_needed` now triggers on `\r` for RFC 4180 §2.5 compliance.** The hand-rolled quoting helper triggered on `,`, `"`, and `\n` but missed bare carriage return — an author name or commit metadata field carrying `\r` would split a CSV row in two. Added `\r` to the trigger predicate; three unit tests cover the CR / comma+quote-doubling / plain-string paths. Migration to the `csv` crate was considered and explicitly rejected: would regenerate 14+ golden snapshots for zero correctness gain (CSV injection — `=CMD|...` style — is a downstream-Excel concern that neither approach addresses; defense belongs in consumer tooling, not the emitter).

- **Dependency-version drift test** (`crates/codelore-lib/tests/dep_versions_drift_test.rs`) asserts that the hardcoded provenance strings (`gix_version`, `duckdb_version`, `ARROW_RUNTIME_VERSION`) match the corresponding `[[package]]` entries in `Cargo.lock`. `include_str!("../../../Cargo.lock")` + simple string matching — 3 tests, no new deps. Future `cargo update` runs that don't sync the provenance strings now fail in CI rather than shipping desync'd provenance.

- **Per-stage timing via `tracing` spans** — `RUST_LOG=codelore::bench=info` now prints elapsed time for the three load-bearing stages of `analyze` (repo open, cache lookup / ingest, analysis + emit). No new `--bench` flag — the existing `tracing` + `tracing-subscriber` infrastructure (already wired in `init_logging`) carries the timing, which composes with `RUST_LOG=codelore=debug` for per-analysis sub-timings. Default WARN-level filtering suppresses the bench spans entirely (zero overhead for normal runs). Documented in `advanced-usage.md` §11.5.

- **Three narrow typed-error variants** added to `CodeLoreError` at the highest-leverage sites: `MalformedTeamMap { path, line, reason }`, `UnknownAnalysisName { name, supported }`, `BlobNotFound { oid }`. The audit found that only ONE call site (`codelore-cli::main::exit_code`) pattern-matches `CodeLoreError` today, so a wholesale "every failure mode gets a typed variant" migration would be ceremony without payoff. These three are the failure modes where downstream consumers can meaningfully act: surface a "fix `<path>` line N" hint, list accepted analysis names, signal repo corruption distinctly from generic repo errors. `UnknownAnalysisError` (analysis.rs) now `From`-converts to the new variant. Existing `Provenance(String)` / `Repo(String)` / `Analysis(String)` / `Output(String)` / `Io(io::Error)` stay as-is because no consumer differentiates among them today.

- **Default-value constants extracted to `codelore_lib::constants`** as the single source of truth. The pre-extraction state had `min_revs = 5` (and 9 other knobs) literal-encoded in both `Options::default()` (`options.rs:178+`) and `clap` `default_value_t` annotations in `args.rs`; same number in two places, zero compile-time check that they stayed in sync. The post-v0.1.0 audit confirmed they did match, but the silent-drift hazard is real. New module exports 10 `pub const`s — `DEFAULT_MIN_REVS`, `DEFAULT_MIN_SHARED_REVS`, `DEFAULT_MIN_COUPLING_PCT`, `DEFAULT_MAX_COUPLING_PCT`, `DEFAULT_MAX_CHANGESET_SIZE`, `DEFAULT_FISHER_SIGNIFICANCE`, `DEFAULT_MIN_CLONE_NODE_COUNT`, `DEFAULT_MIN_CLONE_SHARED_REVS`, `DEFAULT_CLONE_SIMILARITY_FLOOR`, `DEFAULT_CLONE_SKIP_SAME_DIR` — each with docstring explaining the semantics and the rationale for the chosen value. `Options::default()` and every matching `#[arg(default_value_t = …)]` annotation in `codelore-cli` now reference the constants. Regression test `defaults_use_the_constants_so_drift_is_caught_at_compile_time` asserts the round-trip; future drift compile-errors.

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
