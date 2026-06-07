# Plan 8 — v1.x Release Readiness (pre-tag hardening + differentiators)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the gap between "Plans 1–7 shipped" and "v1.0 is tag-ready and visibly best-of-class." Five phases:

1. **Pre-tag hardening** — fix all 5 small drift items from `docs/validation-report-2026-06-07.md` so the tag-time state is internally consistent.
2. **Spec-gap closures** — implement the small in-scope items that got carved out (`--analysis authors`, `--group-file` flag, `--exclude` path-filter, clones JSON/Markdown/SARIF emitters).
3. **Persistent fact-store cache** — wrap `FactsDb::ingest` with an XDG-style cache keyed by `(repo, HEAD, options, version)`. Foundation for §7.
4. **FactsDb integration for clones** — populate the `clones` table during ingest so SQL JOINs become possible. Foundation for §6.
5. **Parallel complexity extraction** — Rayon `map_init` over the working-tree walk. 3-5× wall-time win.
6. **Clone-coupling intersection** — *the strategic differentiator*. JOIN clones × Fisher-significant coupling. New `clone-coupling` analysis + `CODELORE-LIVE-CLONE` SARIF rule.
7. **`codelore diff <base>..<head>` subcommand** — PR-mode delta analysis. The form users actually deploy in CI.

**Architecture choices (locked from research; see briefs at end):**

- **Cache**: `$XDG_CACHE_HOME/codelore/<repo_hash_8>/<cache_key_16>.duckdb`, atomic `.tmp` → rename, LRU 5-per-repo + 2 GB global. DuckDB read-only mode for hits.
- **Parallel extraction**: `rayon::par_iter().map_init(|| (), ...)` — tree-sitter `Parser` is both `Send + Sync` in 0.25.x. No `unsafe`, no thread-local pool needed (per-parse `Parser::new()` is ~3 µs vs ~0.3-2 ms parse cost). Results collected into `Vec` first, then DuckDB Appender drains them serially on the connection-owning thread (Appender is `!Send + !Sync`).
- **Clone-coupling algorithm**: any-pair intersection (CodeScene X-Ray pattern). For each clone family, JOIN against `coupling` WHERE `p_value < fisher_significance`. False-positive filters: min 6 lines / 50 tokens, exclude generated/vendored paths, min `shared_revs ≥ 3`, similarity floor ≥ 0.70, optional `--skip-same-dir`.
- **`diff` strategy**: dual full analysis (base + head) with result-set diff. `--base-cache PATH` flag for the cache-across-PRs optimisation. Three-dot merge-base notation. `fetch-depth: 0` required in GHA.

**Tech Stack (deltas over Plans 1–7):**
- `rayon` 1.x for parallel walk
- `dirs` 5.x or `etcetera` 0.10 for XDG cache path resolution (no new heavy dep)
- `serde_json` (already in tree) for `--base-cache` serialization
- `globset` 0.4 for `--exclude PATTERN` glob matching

**Test discipline:** every task ships either a unit test or end-to-end smoke that exercises the new behaviour. Two analyses (clone-coupling + diff) get golden-output tests against `differential_repo`.

---

## §0 — Cold-start audit

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test --workspace --all-features 2>&1 | tail -5
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo clippy --workspace --all-targets --all-features -- -D warnings
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo fmt --all --check
df -h /                                # disk pressure — DuckDB bundled build is heavy
git log --oneline -10
```

Expected baseline: **322 tests** passing, 3 ignored, clippy/fmt clean, latest commit on `main` is `f9d1ec5` (validation report).

---

## §1 — Pre-tag hardening (Phase 8.A)

Five small fixes from the validation report. Each is independent; can land in any order.

### Task 1: Fix README "11/12 analyses" inconsistency (Finding S2)

**Files:** `README.md`

`README.md` claims "12 analyses" in the lead and "11 analyses" twice in the body. Reconcile to "12 user-facing + 1 reserved (Authors, lands in §2 Task 6)".

- [ ] **Step 1: Edit `README.md`** — replace both "11 analyses" lines with "12 analyses" matching the lead. Add a footnote that `Authors` is reserved and bails until Plan 8 Task 6.
- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(readme): fix '11/12 analyses' inconsistency (validation S2)"
```

---

### Task 2: Refresh `docs/perf-evidence-v1.md` codescene-workspace timing (Finding S1)

**Files:** `docs/perf-evidence-v1.md`

The doc says 0.24s / 87 MB; cold re-measurement is 1.06s / 89 MB (4× drift after Plan 7 added tree-sitter grammar deps). The gitoxide row is correct within variance.

- [ ] **Step 1: Re-run the measurement**

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo build --release -p codelore-cli 2>&1 | tail -2
# Run 5 times for variance
for i in 1 2 3 4 5; do
  /usr/bin/time -l ./target/release/codelore analyze --analysis hotspots --repo . --min-revs 1 --format parquet --output /tmp/c.parquet 2>&1 | grep -E "real|peak memory" | head -2 | tr '\n' ' '
  echo
done
```

- [ ] **Step 2: Edit the table** — replace the codescene-workspace row with mean + variance from the 5 runs. Add a note: "row reflects timing after Plan 7 tree-sitter grammar deps landed (`tree-sitter-rust 0.23.2` + 4 sibling grammars + `walkdir 2`); pre-Plan-7 timing was 0.24s/87 MB; the larger working-tree footprint at HEAD (now ~155 files vs. ~147) accounts for ~50% of the drift."
- [ ] **Step 3: Commit**

```bash
git add docs/perf-evidence-v1.md
git commit -m "docs(perf): refresh codescene-workspace timing post-Plan-7 (validation S1)"
```

---

### Task 3: Better unknown-analysis CLI error (Finding S8)

**Files:** `crates/codelore-lib/src/analysis.rs`, `crates/codelore-cli/src/main.rs`, `crates/codelore-cli/tests/cli_test.rs`

Replace `unknown analysis: bogus` with an enumerated list of valid options.

- [ ] **Step 1: Read `crates/codelore-lib/src/analysis.rs`** to confirm the `Display for UnknownAnalysisError` impl and `AnalysisName::all()` shape.

- [ ] **Step 2: Update `UnknownAnalysisError` `Display`** to list `AnalysisName::all()` (omit `Authors` if it still bails after Task 6):

```rust
impl fmt::Display for UnknownAnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = AnalysisName::all()
            .iter()
            .map(|a| a.as_str())
            .collect();
        write!(f, "unknown analysis {:?}. Supported: {}", self.0, names.join(", "))
    }
}
```

- [ ] **Step 3: Add a CLI test**

```rust
#[test]
fn unknown_analysis_lists_supported_names() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore").unwrap()
        .args(["analyze", "--analysis", "definitelybogus", "--repo", tiny.dir.path().to_str().unwrap()])
        .assert().failure()
        .stderr(predicate::str::contains("unknown analysis"))
        .stderr(predicate::str::contains("hotspots"))
        .stderr(predicate::str::contains("clones"));
}
```

- [ ] **Step 4: Run, commit**

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test -p codelore-cli --test cli_test --all-features unknown_analysis 2>&1 | tail -5
git add crates/codelore-lib/src/analysis.rs crates/codelore-cli/
git commit -m "feat(cli): enumerate supported analyses in unknown-analysis error (validation S8)"
```

---

### Task 4: Add `write_clones_csv` snapshot test (Finding S4)

**Files:** `crates/codelore-lib/tests/output_csv_test.rs` (or new `output_clones_csv_test.rs`)

Lock the CSV column shape so silent header drift breaks the build.

- [ ] **Step 1: Add a test** that constructs 2 `ClonesRow` values and asserts byte-equal CSV output.

```rust
#[test]
fn write_clones_csv_locks_column_order() {
    let rows = vec![
        codelore_lib::analyses::clones::ClonesRow {
            clone_group_id: 1,
            fingerprint: "deadbeef".repeat(8),
            entity: "src/a.rs".into(),
            function: "add".into(),
            start_line: 10, end_line: 20,
            node_count: 42, similarity: 1.0, family_size: 2,
        },
        codelore_lib::analyses::clones::ClonesRow {
            clone_group_id: 1,
            fingerprint: "deadbeef".repeat(8),
            entity: "src/b.rs".into(),
            function: "mul".into(),
            start_line: 5, end_line: 15,
            node_count: 42, similarity: 1.0, family_size: 2,
        },
    ];
    let mut buf = Vec::new();
    codelore_lib::output::csv::write_clones_csv(&rows, &mut buf).unwrap();
    let s = String::from_utf8(buf).unwrap();
    let expected = "\
clone-group,fingerprint,entity,function,start-line,end-line,node-count,similarity,family-size
1,deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef,src/a.rs,add,10,20,42,1.0000,2
1,deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef,src/b.rs,mul,5,15,42,1.0000,2
";
    assert_eq!(s, expected);
}
```

- [ ] **Step 2: Run, commit**

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test -p codelore-lib --test output_csv_test --all-features write_clones_csv 2>&1 | tail -5
git add crates/codelore-lib/tests/
git commit -m "test(lib): lock clones CSV column shape (validation S4)"
```

---

### Task 5: Update spec §8 — clones is partial-shipped (Finding S7)

**Files:** `docs/superpowers/specs/2026-06-06-codelore-design.md`

Spec line 727 lists "Clone detection × co-change" as deferred. Plan 7 shipped the clone-detection half; the × coupling half is Plan 8 §6. Add a status note.

- [ ] **Step 1: Edit the spec** — change line 727 to:

```markdown
| **Clone detection × co-change** (only flag clones that also change together) | **PARTIAL — clone detection ships in Plan 7; intersection lands in Plan 8 §6.** Kills dead-clone noise; requires both clone detection (new) and coupling (existing) | CodeScene X-Ray docs |
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-06-06-codelore-design.md
git commit -m "docs(spec): mark clone-coupling row PARTIAL — clones in Plan 7, intersection Plan 8 (validation S7)"
```

---

## §2 — Spec-gap closures (Phase 8.B)

Small in-scope items that got carved out of earlier plans.

### Task 6: Implement `--analysis authors` (closes spec §1.1 gap)

**Files:** `crates/codelore-lib/src/analyses/authors.rs` (new), `crates/codelore-lib/src/analyses/mod.rs`, `crates/codelore-cli/src/main.rs`, `crates/codelore-lib/src/output/{csv,json,markdown}.rs`, `crates/codelore-lib/tests/authors_test.rs` (new)

code-maat parity: emit one row per canonical author with their total commit count, sorted desc.

```rust
// crates/codelore-lib/src/analyses/authors.rs
pub struct AuthorsRow {
    pub author: String,
    pub commits: u32,
}

pub fn run_authors(db: &FactsDb, opts: &Options) -> Result<Vec<AuthorsRow>> {
    let sql = "SELECT canonical_author, COUNT(*) FROM commits GROUP BY canonical_author ORDER BY 2 DESC, 1 ASC";
    // Standard FactsDb::query pattern; see analyses::summary::run_summary
    // Apply opts.rows_limit
    todo!()
}
```

CSV header (code-maat parity): `name,n-commits`. Tests assert against `tiny_repo` and `differential_repo` (3 authors + 1 bot = 4 rows).

- [ ] **Step 1: Read `analyses/summary.rs` for the SQL pattern**
- [ ] **Step 2: Write the failing test in `tests/authors_test.rs`**
- [ ] **Step 3: Implement `run_authors` + the 3 emitters (csv/json/markdown)**
- [ ] **Step 4: Wire into CLI's `analyze()` match arms for csv/json/markdown × `AnalysisName::Authors`** — remove the existing bail
- [ ] **Step 5: Add the parity test in `tests/code_maat_parity_test.rs`**
- [ ] **Step 6: Run, commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat(lib+cli): authors analysis (code-maat parity; closes spec §1.1)"
```

---

### Task 7: Expose `--group-file` clap flag (closes spec §1.1 gap)

**Files:** `crates/codelore-cli/src/args.rs`, `crates/codelore-cli/src/main.rs`

`Options::group_file: Option<PathBuf>` exists but isn't exposed. Add the flag; wire into `opts` construction. Document that the actual `-g` aggregation logic lands in Plan 9 — for now, the flag is parsed-but-warned-only:

```rust
#[arg(short = 'g', long)]
pub group_file: Option<PathBuf>,
```

In `analyze()`:
```rust
if args.group_file.is_some() {
    eprintln!("warning: --group-file is recognized but aggregation lands in Plan 9; flag has no effect yet");
}
```

- [ ] **Step 1: Add the clap arg + warning**
- [ ] **Step 2: Add a CLI test** that asserts the flag parses without error
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-cli/
git commit -m "feat(cli): expose --group-file flag (parsing only; aggregation lands in Plan 9)"
```

---

### Task 8: `--exclude PATTERN` + `.codeloreignore` (validation Finding S9)

**Files:** `crates/codelore-cli/src/args.rs`, `crates/codelore-lib/src/options.rs`, `crates/codelore-lib/src/analyses/clones.rs`, `crates/codelore-cli/src/main.rs`, `crates/codelore-lib/tests/clones_exclude_test.rs` (new)

Add `globset 0.4` for glob matching. Surface CLI as repeatable `--exclude GLOB`. Read `.codeloreignore` from `opts.repo_path` if present (one glob per line, `#` comments).

```rust
// Options addition
pub exclude_patterns: Vec<String>,
```

In `analyses::clones::run_clones`, after `from_path` lang dispatch and before `extract_functions`, apply the exclude glob set:

```rust
if exclude_globs.is_match(&rel) {
    continue;
}
```

- [ ] **Step 1: Add `globset = "0.4"` to `crates/codelore-lib/Cargo.toml`**
- [ ] **Step 2: Build the global glob set from `opts.exclude_patterns ++ read_codeloreignore(opts.repo_path)`**
- [ ] **Step 3: Wire into `analyses::clones::run_clones`** (and any other analysis that iterates files — for now, only clones does)
- [ ] **Step 4: Test exclude works on a fixture with a generated/* path**
- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat: --exclude PATTERN + .codeloreignore for path-filter (validation S9)"
```

---

### Task 9: Clones JSON / Markdown emitters

**Files:** `crates/codelore-lib/src/output/{json,markdown}.rs`, `crates/codelore-cli/src/main.rs`, `crates/codelore-lib/tests/output_{json,markdown}_test.rs`

Follow the pattern of the 11 existing analyses. CLI dispatch unbails `(json, Clones)` and `(markdown, Clones)`.

- [ ] **Step 1: `write_clones_json`** (uses `serde_json::to_writer_pretty` on `&[ClonesRow]`)
- [ ] **Step 2: `write_clones_markdown`** (header `# CodeLore clones`, GFM table with same 9 columns as CSV)
- [ ] **Step 3: CLI dispatch arms** — replace the current bail for non-csv clones
- [ ] **Step 4: Tests for both**
- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat: clones JSON + Markdown emitters"
```

---

### Task 10: `CODELORE-CLONE` SARIF rule

**Files:** `crates/codelore-lib/src/output/sarif.rs`, `crates/codelore-cli/src/main.rs`, `crates/codelore-lib/tests/output_sarif_test.rs`

New SARIF rule for plain clone detection (the live-clone variant comes in §6). Each clone family is one `result` with `locations[]` = all members + `partialFingerprints.cloneGroupFingerprint/v1` = the AST digest. Security severity proxy: 5.0 by default (raises with family size: `min(10, 3 + family_size)`).

- [ ] **Step 1: Add `write_clones_sarif`** following the existing `write_hotspots_sarif` shape
- [ ] **Step 2: CLI dispatch arm `(sarif, Clones)`**
- [ ] **Step 3: Validate with a SARIF linter** (`pysarif` or `npx @microsoft/sarif-multitool validate`)
- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat(lib): CODELORE-CLONE SARIF 2.1.0 rule for clone families"
```

---

## §3 — Persistent fact-store cache (Phase 8.C)

XDG-style content-addressed cache that wraps `FactsDb::ingest`. Independent of all other §s — can be developed in parallel.

### Task 11: Cache key + path derivation

**Files:** `crates/codelore-lib/src/cache.rs` (new), `crates/codelore-lib/src/lib.rs`

```rust
pub fn cache_key(repo_path: &Path, head_sha: &str, opts: &Options) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let canonical = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(b"\x00");
    hasher.update(head_sha.as_bytes());
    hasher.update(b"\x00");
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(b"\x00");
    hasher.update(opts_hash(opts).as_bytes());
    hasher.update(b"\x00");
    hasher.update(b"schema_v1");
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    out
}

pub fn cache_path(key: &[u8; 32], repo_path: &Path) -> PathBuf {
    let xdg = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let mut repo_hash = Sha256::new();
    repo_hash.update(repo_path.to_string_lossy().as_bytes());
    let repo_short = hex::encode(&repo_hash.finalize()[..4]);  // 8 hex chars
    let key_short = hex::encode(&key[..8]);                    // 16 hex chars
    xdg.join("codelore").join(repo_short).join(format!("{key_short}.duckdb"))
}
```

`opts_hash` serializes the threshold/filter knobs (after, before, include_merges, min_revs, etc.) into a stable string. Exclude `rows_limit` and `repo_path` (already in the key) and any cosmetic flags.

- [ ] **Step 1: Add `dirs = "5"` to `codelore-lib/Cargo.toml`**
- [ ] **Step 2: Implement `cache_key` + `cache_path`**
- [ ] **Step 3: Unit test that the key is stable across runs and changes on Options changes**
- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): persistent cache key + path derivation (Plan 8 §3)"
```

---

### Task 12: Hit/miss paths (read-only open vs ingest-and-write)

**Files:** `crates/codelore-lib/src/cache.rs`, `crates/codelore-lib/src/facts/mod.rs`

New `FactsDb::open_or_ingest(opts, repo)` constructor that:
1. Resolves HEAD via `repo.head_oid()` (gix call — verify the existing GixRepo surface; add the method if missing)
2. Computes `cache_key`
3. Looks for `cache_path(key, repo_path)`. If present, opens DuckDB with `access_mode='READ_ONLY'`. Returns the `FactsDb`.
4. If not present: creates a new file at `cache_path(...).with_extension("duckdb.tmp")`, runs the existing ingest into it, atomic-renames to `.duckdb`, returns the `FactsDb`.

```rust
pub fn open_or_ingest(opts: &Options, repo: &impl Repo) -> Result<Self> {
    let head_sha = repo.head_sha()?;  // new method on Repo trait — needs vendor patch
    let key = cache::cache_key(&opts.repo_path, &head_sha, opts);
    let cache_p = cache::cache_path(&key, &opts.repo_path);

    if cache_p.exists() {
        tracing::info!("cache hit: {}", cache_p.display());
        return Self::open_read_only(&cache_p);
    }

    tracing::info!("cache miss: ingesting to {}", cache_p.display());
    std::fs::create_dir_all(cache_p.parent().unwrap())?;
    let tmp = cache_p.with_extension("duckdb.tmp");
    let _ = std::fs::remove_file(&tmp);  // clean any prior aborted tmp
    let db = Self::open_file(&tmp)?;
    db.ingest(repo, opts)?;
    db.flush()?;            // ensure DuckDB writes are durable on disk
    drop(db);               // release DuckDB lock before rename
    let f = std::fs::File::open(&tmp)?;
    f.sync_all()?;          // mac APFS gotcha per research brief
    std::fs::rename(&tmp, &cache_p)?;
    Self::open_read_only(&cache_p)
}
```

- [ ] **Step 1: Add `Repo::head_sha(&self) -> Result<String>`** — implement on `GixRepo` and `GitCliRepo`
- [ ] **Step 2: Add `FactsDb::open_read_only(path)` + `FactsDb::open_file(path)`** — DuckDB Config supports `access_mode`
- [ ] **Step 3: Implement `open_or_ingest`**
- [ ] **Step 4: Integration test** — ingest twice on `tiny_repo`, assert second ingest is a hit (instrument via a `cache_hit` field on a new `IngestStats` struct)
- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): FactsDb::open_or_ingest with persistent cache hit/miss paths"
```

---

### Task 13: LRU eviction

**Files:** `crates/codelore-lib/src/cache.rs`

After a successful miss-and-write, count entries in `cache_path.parent()`. If > 5 (per-repo cap), delete the oldest by `mtime`. Then check the global cap: walk `$XDG_CACHE_HOME/codelore` recursively, sum sizes, if > 2 GB delete oldest leaves until under.

- [ ] **Step 1: `prune_repo_cache(repo_dir: &Path, max_entries: usize) -> Result<()>`**
- [ ] **Step 2: `prune_global_cache(root: &Path, max_bytes: u64) -> Result<()>`**
- [ ] **Step 3: Call both from `open_or_ingest` after the rename**
- [ ] **Step 4: Test** — write 7 cache entries via repeated runs with mutated `opts_hash` inputs, assert only 5 survive
- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): LRU eviction for persistent cache (5/repo + 2GB global)"
```

---

### Task 14: `--no-cache` + `--cache-dir` CLI overrides

**Files:** `crates/codelore-cli/src/args.rs`, `crates/codelore-cli/src/main.rs`

Two flags:
- `--no-cache`: skip cache entirely, always run fresh ingest
- `--cache-dir PATH`: override XDG default (useful for CI workflows that want per-job cache)

- [ ] **Step 1: Add flags**
- [ ] **Step 2: Wire into `analyze()` — `--no-cache` falls back to `FactsDb::new_in_memory()`; `--cache-dir` overrides the dirs::cache_dir() resolution**
- [ ] **Step 3: Add CLI tests**
- [ ] **Step 4: Commit**

```bash
git add crates/codelore-cli/
git commit -m "feat(cli): --no-cache + --cache-dir flags for persistent fact-store cache"
```

---

## §4 — FactsDb integration for clones (Phase 8.D)

Closes validation Finding S3. Foundation for §6.

### Task 15: `extract_clones_at_head` integrated into `FactsDb::ingest`

**Files:** `crates/codelore-lib/src/facts/ingest.rs`, `crates/codelore-lib/src/clones/grouper.rs` (new module that wraps `extractor.rs` for the FactsDb path)

After the existing `ingest_complexity_at_head` pass, walk the same working-tree set and populate the `clones` table via `FactsDb::Appender("clones")`. Honor `opts.min_clone_node_count` and the `--exclude` patterns from Task 8.

- [ ] **Step 1: Read `crates/codelore-lib/src/facts/ingest.rs` + `clones/extractor.rs`** to confirm the integration point
- [ ] **Step 2: New `pub fn populate_clones_at_head(&self, repo: &impl Repo, opts: &Options) -> Result<usize>` on `FactsDb`** — returns the row count inserted
- [ ] **Step 3: Call from `FactsDb::ingest` after `ingest_complexity_at_head`**
- [ ] **Step 4: Integration test** — `tiny_repo` should produce 0 rows (no cloned functions); `differential_repo` should produce > 0 rows
- [ ] **Step 5: Add an `IngestStats { commits, changes, entities, clones }` for observability**
- [ ] **Step 6: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): populate clones table during FactsDb::ingest (closes validation S3)"
```

---

### Task 16: Migrate `analyses::clones::run_clones` to use the table

**Files:** `crates/codelore-lib/src/analyses/clones.rs`

Replace the ad-hoc filesystem walk with a SQL SELECT from the now-populated `clones` table. This keeps the CLI flow unchanged (still works on shallow clones via the cache short-circuit) but enables §6 to JOIN against `coupling`.

- [ ] **Step 1: Refactor `run_clones` to query `clones` table** when the FactsDb has been ingested; fall back to the in-memory extractor for the "HEAD-only" short-circuit path in the CLI
- [ ] **Step 2: Existing test still passes**
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-lib/
git commit -m "refactor(lib): clones analysis reads from FactsDb table when available"
```

---

## §5 — Parallel complexity extraction (Phase 8.E)

Per the research brief: `rayon::par_iter().map_init(|| (), ...)` over `path_rows`, collect into `Vec` first, drain into DuckDB Appenders serially on the connection-owning thread.

### Task 17: Add Rayon dep + `par_iter` over the working-tree walk

**Files:** `crates/codelore-lib/Cargo.toml`, `crates/codelore-lib/src/facts/ingest.rs`

- [ ] **Step 1: `rayon = "1"`** to `[dependencies]`
- [ ] **Step 2: Rewrite `ingest_complexity_at_head`** per the research brief snippet — `par_iter().map_init(|| (), |_, (path, head_rev)| { ... }).collect::<Vec<_>>()` then serial drain into Appenders
- [ ] **Step 3: Tune `RAYON_NUM_THREADS` default** — leave at Rayon's `available_parallelism()` default
- [ ] **Step 4: Errors propagated as `Vec<Result<...>>`** — log per-file failures but don't abort the parallel scan
- [ ] **Step 5: Run existing `complexity_test.rs`** — same inputs, same outputs regardless of thread ordering (DuckDB's PRIMARY KEY would surface any duplicates as errors)
- [ ] **Step 6: Run with `--test-threads=1` AND `--test-threads=8`** — diff outputs, should be none
- [ ] **Step 7: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): parallel complexity extraction via rayon::map_init"
```

---

### Task 18: Bench parallel vs serial extraction

**Files:** `crates/codelore-lib/benches/end_to_end.rs`

Add two bench targets: `ingest_medium_serial` (with `RAYON_NUM_THREADS=1`) and `ingest_medium_parallel` (default). Compare on the existing 500-commit `medium_repo` fixture.

- [ ] **Step 1: Add the bench targets** — use `rayon::ThreadPoolBuilder` per bench iteration if needed to force serial
- [ ] **Step 2: Run, capture numbers, add to `docs/perf-evidence-v1.md`**
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-lib/benches/ docs/
git commit -m "bench(lib): serial vs parallel complexity extraction"
```

---

## §6 — Clone-coupling intersection (Phase 8.F) — THE DIFFERENTIATOR

Per the research brief: any-pair intersection. JOIN `clones × coupling` WHERE `p_value < opts.fisher_significance`, filtered by the 5 false-positive mitigations.

### Task 19: `clone_coupling` analysis — per-pair query

**Files:** `crates/codelore-lib/src/analyses/clone_coupling.rs` (new), `crates/codelore-lib/src/analyses/mod.rs`

```rust
pub struct CloneCouplingRow {
    pub clone_group_id: u32,
    pub fingerprint: String,
    pub file_a: String,
    pub file_b: String,
    pub entity_a: String,
    pub entity_b: String,
    pub start_line_a: u32, pub end_line_a: u32,
    pub start_line_b: u32, pub end_line_b: u32,
    pub node_count: u32,
    pub similarity: f64,
    pub shared_revs: u32,
    pub support_a: u32,
    pub support_b: u32,
    pub degree_pct: f64,
    pub p_value: f64,
    pub combined_score: f64,    // similarity * degree_pct * (1 - p_value)
}

pub fn run_clone_coupling(db: &FactsDb, opts: &Options) -> Result<Vec<CloneCouplingRow>> {
    let sql = "
        SELECT c1.clone_group_id, hex(c1.fingerprint) AS fingerprint,
               c1.path AS file_a, c2.path AS file_b,
               c1.function AS entity_a, c2.function AS entity_b,
               c1.start_line AS start_line_a, c1.end_line AS end_line_a,
               c2.start_line AS start_line_b, c2.end_line AS end_line_b,
               c1.node_count, c1.similarity,
               cp.shared_revs, cp.support_a, cp.support_b, cp.degree_pct, cp.p_value,
               c1.similarity * cp.degree_pct * (1.0 - cp.p_value) AS combined_score
        FROM clones c1
        JOIN clones c2
          ON c1.clone_group_id = c2.clone_group_id
         AND c1.path < c2.path
        JOIN coupling_view cp
          ON cp.entity_a = c1.path AND cp.entity_b = c2.path
        WHERE cp.p_value < ?              -- opts.fisher_significance
          AND cp.shared_revs >= ?         -- min_shared_revs (default 3)
          AND c1.similarity >= ?          -- similarity_floor (default 0.70)
          AND c1.node_count >= ?          -- min_clone_node_count
        ORDER BY combined_score DESC
    ";
    // ... bind params, execute, collect rows
}
```

`coupling_view` is a DuckDB VIEW we define once on top of the existing `coupling` analysis output (or we materialize it in a CTE inside the query).

- [ ] **Step 1: Read `analyses::coupling.rs`** to confirm how the coupling pairs are computed and which intermediate is queryable
- [ ] **Step 2: Implement `run_clone_coupling`** with the SQL above
- [ ] **Step 3: Add Options fields**: `min_shared_revs` (default 3), `similarity_floor` (default 0.70), `skip_same_dir` (default true)
- [ ] **Step 4: Test** — construct a fixture with 2 clone families: one whose members co-change frequently, one whose members never co-change. Assert only the co-changing family appears in `clone-coupling` output.
- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): clone-coupling intersection analysis (the differentiator)"
```

---

### Task 20: CSV / JSON / Markdown emitters + `--analysis clone-coupling` CLI

**Files:** `crates/codelore-lib/src/output/{csv,json,markdown}.rs`, `crates/codelore-lib/src/analysis.rs`, `crates/codelore-cli/src/main.rs`

Add `AnalysisName::CloneCoupling`. CSV header: 18 columns matching the row struct. JSON via serde. Markdown table with the columns rank-ordered for human consumption.

- [ ] **Step 1: Three emitters**
- [ ] **Step 2: `AnalysisName` enum variant + `all()` + `as_str()`**
- [ ] **Step 3: CLI dispatch (3 format arms)**
- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat(cli): wire clone-coupling analysis across csv/json/markdown outputs"
```

---

### Task 21: `CODELORE-LIVE-CLONE` SARIF rule

**Files:** `crates/codelore-lib/src/output/sarif.rs`, `crates/codelore-cli/src/main.rs`

Per the research brief:
- One SARIF result per clone pair
- `locations[0]` = higher-`support_a` (more-frequently-changed) file; `locations[1]` = lower-support partner
- `partialFingerprints.cloneGroupFingerprint/v1` = AST digest from `fingerprint` col
- `partialFingerprints.filePairHash/v1` = `sha256(sorted(file_a, file_b))`
- `correlationGuid` = UUIDv5 derived from `clone_group_id`
- `properties.precision = "medium"`
- `properties.security-severity = combined_score * 10` (high-severity for high combined score)
- Rule `CODELORE-LIVE-CLONE` registered alongside existing `CODELORE-HOTSPOT` and (from Task 10) `CODELORE-CLONE`

- [ ] **Step 1: Add UUIDv5 helper** (or implement inline with sha2; spec defines the namespace-based derivation)
- [ ] **Step 2: `write_clone_coupling_sarif`**
- [ ] **Step 3: CLI dispatch arm `(sarif, CloneCoupling)`**
- [ ] **Step 4: Validate with SARIF linter**
- [ ] **Step 5: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat(lib): CODELORE-LIVE-CLONE SARIF rule for clone-coupling findings"
```

---

### Task 22: False-positive mitigation tests

**Files:** `crates/codelore-lib/tests/clone_coupling_fp_test.rs` (new)

Build 5 small fixtures, one per mitigation:

1. **min-fragment-size**: 2 trivial clones (< 6 lines / < 50 tokens). Assert NOT in output.
2. **path-exclusion**: clones in `generated/` paths. Assert NOT in output when `--exclude 'generated/**'`.
3. **min shared_revs**: clones that co-change only twice (< 3). Assert NOT in output.
4. **similarity floor**: clones with similarity 0.65 (< 0.70 default). Assert NOT in output.
5. **skip-same-dir**: clones in the same directory. Assert NOT in output by default, IN output with `--no-skip-same-dir`.

- [ ] **Step 1: Fixture builders**
- [ ] **Step 2: 5 tests, each ~30 LOC**
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-lib/tests/
git commit -m "test(lib): 5 false-positive mitigation tests for clone-coupling"
```

---

## §7 — `codelore diff <base>..<head>` (Phase 8.G)

Per the research brief: Strategy A (dual full analysis + result-set diff), with `--base-cache` flag for the dominant pattern of reuse-across-PRs. Three-dot merge-base notation.

### Task 23: `diff` subcommand + CLI surface

**Files:** `crates/codelore-cli/src/args.rs`, `crates/codelore-cli/src/main.rs`

```
codelore diff <base>..<head> [OPTIONS]
  --analysis KIND          [default: hotspots]; values: hotspots, coupling, clones, all
  --top-n N                [default: 10]
  --score-threshold F      [default: 0.05]
  --base-cache PATH        [optional]
  --output FORMAT          [default: text]; values: text, json, sarif, markdown
  --fail-on CONDITION      [default: none]; values: none, rank-entrant, score-increase, any
  --history-days N         [default: 365]
  --exclude GLOB           (repeatable)
```

`diff` becomes a top-level subcommand sibling of `analyze`. Reuses `--exclude` from Task 8.

- [ ] **Step 1: Add `Command::Diff(DiffArgs)` enum variant**
- [ ] **Step 2: Parse `<base>..<head>` and `<base>...<head>` (three-dot)** — extract base and head SHAs via `git merge-base` shell-out (for three-dot) or trivial split (for two-dot)
- [ ] **Step 3: Smoke test the CLI parses correctly**

---

### Task 24: Dual-analysis runner with `--base-cache`

**Files:** `crates/codelore-cli/src/diff.rs` (new)

```rust
pub fn run_diff(args: &DiffArgs) -> Result<DiffOutput> {
    let (base_sha, head_sha) = resolve_revs(args)?;

    // Run analysis at HEAD first (cheaper if HEAD is cached)
    let head_results = analyze_at_rev(&head_sha, args, /*use_cache=*/ true)?;

    // Run analysis at BASE — either from --base-cache or fresh
    let base_results = match &args.base_cache {
        Some(path) => load_base_cache(path)?,
        None => analyze_at_rev(&base_sha, args, /*use_cache=*/ true)?,
    };

    // Compute deltas per analysis kind
    Ok(DiffOutput {
        hotspots: diff_hotspots(&base_results.hotspots, &head_results.hotspots, args),
        coupling: diff_coupling(&base_results.coupling, &head_results.coupling, args),
        clones:   diff_clones(&base_results.clones, &head_results.clones, args),
    })
}
```

`analyze_at_rev` is a thin wrapper that:
1. Stashes the working tree (if dirty) — error out if dirty without `--allow-dirty`
2. `git checkout <sha>` (or uses `git worktree add` to a tempdir for non-destructive analysis)
3. Runs full analysis via `FactsDb::open_or_ingest` (cache hit if `<sha>` already analysed)
4. Returns the result-set

**Critical**: §3 cache is what makes this tractable. Without it, every `codelore diff` does 2× the work of one `analyze`.

- [ ] **Step 1: `git worktree` strategy** — non-destructive checkout into a temp worktree, run analysis there, drop the worktree
- [ ] **Step 2: `analyze_at_rev` helper**
- [ ] **Step 3: `--base-cache` JSON load/save format** — round-trip the 3 result-set vecs
- [ ] **Step 4: Smoke test on a tiny fixture** — build a 2-commit fixture, diff `HEAD^..HEAD`, assert sensible output

---

### Task 25: Hotspot delta logic

**Files:** `crates/codelore-cli/src/diff.rs`

```rust
pub struct HotspotDelta {
    pub rank_entrants: Vec<HotspotRow>,         // files NEW in top-N at head
    pub score_increased: Vec<ScoreDelta>,       // files in top-N at both, score up by ≥ threshold
    pub pr_touched_existing: Vec<HotspotRow>,   // files PR touched that were already in top-N at base
}
```

Computation:
- `rank_entrants` = (head top-N) − (base top-N)
- `score_increased` = files in (head top-N ∩ base top-N) where `head.score - base.score ≥ args.score_threshold`
- `pr_touched_existing` = (base top-N ∩ files-modified-in-PR-range)

"Files modified in PR range" = `git log --name-only <base>..<head> | sort -u`.

- [ ] **Step 1: Implement the three delta computations**
- [ ] **Step 2: Test** — 3-commit fixture where commit 3 introduces a new hotspot
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-cli/
git commit -m "feat(cli): codelore diff — hotspot delta (rank-entrants + score-increased + pr-touched)"
```

---

### Task 26: Coupling absent-change-pattern detection

**Files:** `crates/codelore-cli/src/diff.rs`

The CodeScene signature signal: "You changed `auth/login.rs` but historically `auth/session.rs` always changes with it — did you miss it?"

```rust
pub struct CouplingAbsence {
    pub touched_file: String,           // file IN the PR
    pub expected_partner: String,       // file NOT in the PR
    pub historical_coupling: f64,       // degree_pct from base analysis
    pub p_value: f64,
}

pub fn detect_absences(
    base_coupling: &[CouplingRow],
    pr_files: &HashSet<String>,
    opts: &DiffArgs,
) -> Vec<CouplingAbsence> {
    base_coupling.iter()
        .filter(|c| c.p_value < 0.05 && c.shared_revs >= 5)  // require strong historical signal
        .filter_map(|c| {
            let a_in = pr_files.contains(&c.entity_a);
            let b_in = pr_files.contains(&c.entity_b);
            if a_in && !b_in {
                Some(CouplingAbsence {
                    touched_file: c.entity_a.clone(),
                    expected_partner: c.entity_b.clone(),
                    historical_coupling: c.degree_pct,
                    p_value: c.p_value,
                })
            } else if b_in && !a_in {
                Some(CouplingAbsence {
                    touched_file: c.entity_b.clone(),
                    expected_partner: c.entity_a.clone(),
                    historical_coupling: c.degree_pct,
                    p_value: c.p_value,
                })
            } else { None }
        })
        .collect()
}
```

- [ ] **Step 1: Implement** + minimum `shared_revs ≥ 5` guard (per research brief mitigation)
- [ ] **Step 2: Test** — fixture with strong A↔B historical coupling, PR that modifies only A
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-cli/
git commit -m "feat(cli): codelore diff — coupling absent-change-pattern detection"
```

---

### Task 27: Clones diff (new families introduced by PR)

**Files:** `crates/codelore-cli/src/diff.rs`

`new_clone_families = (head clone families) − (base clone families)`, keyed by fingerprint. A family is "new" if its fingerprint is absent from `base_results.clones`.

Secondary: `pr_touched_existing_clones` = clone families where the PR modified any member (intersection with `pr_files`).

- [ ] **Step 1: Implement both**
- [ ] **Step 2: Test** — 2-commit fixture where commit 2 copies a function
- [ ] **Step 3: Commit**

```bash
git add crates/codelore-cli/
git commit -m "feat(cli): codelore diff — new + touched clone families"
```

---

### Task 28: Text / JSON / SARIF / Markdown output for `diff`

**Files:** `crates/codelore-cli/src/diff_output.rs` (new), `crates/codelore-cli/src/main.rs`

Each output format takes the `DiffOutput` struct and produces format-specific rendering. SARIF emission uses the existing `CODELORE-HOTSPOT` + `CODELORE-CLONE` rules from §1 + §2; tag results with `properties.diff-classification` = `"rank-entrant"` / `"score-increase"` / etc.

Markdown is the most important output (`$GITHUB_STEP_SUMMARY` consumption). Text is for terminal use.

- [ ] **Step 1: All four formats**
- [ ] **Step 2: Test each renders without panic**
- [ ] **Step 3: `--fail-on rank-entrant` exit code** — non-zero when condition met
- [ ] **Step 4: Commit**

```bash
git add crates/codelore-cli/
git commit -m "feat(cli): codelore diff — text/json/sarif/markdown output + --fail-on gate"
```

---

### Task 29: GitHub Actions example workflow

**Files:** `examples/.github/workflows/codelore-pr.yml`

Per the research brief: `actions/checkout@v4` with `fetch-depth: 0`, three-dot merge-base notation, SARIF upload to GitHub Code Scanning, sticky PR comment on `--fail-on` trigger.

```yaml
name: CodeLore PR Analysis
on:
  pull_request:
    branches: [main]
jobs:
  codelore-diff:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - run: git fetch origin ${{ github.base_ref }}
      - uses: cargo-bins/cargo-binstall@v1
      - run: cargo binstall codelore --no-confirm
      - run: |
          codelore diff origin/${{ github.base_ref }}...${{ github.sha }} \
            --analysis all --top-n 10 \
            --output markdown >> "$GITHUB_STEP_SUMMARY"
          codelore diff origin/${{ github.base_ref }}...${{ github.sha }} \
            --analysis hotspots --output sarif > codelore.sarif
      - uses: github/codeql-action/upload-sarif@v3
        with: { sarif_file: codelore.sarif }
```

- [ ] **Step 1: Write the example workflow**
- [ ] **Step 2: Add a short `examples/README.md`** explaining the integration patterns
- [ ] **Step 3: Commit**

```bash
git add examples/
git commit -m "docs: GitHub Actions example for codelore diff PR mode"
```

---

## §8 — Docs (Phase 8.H)

### Task 30: CHANGELOG + README + roadmap update

**Files:** `CHANGELOG.md`, `README.md`, `docs/roadmap-v1.x-and-beyond.md`

Add a Plan 8 section to CHANGELOG capturing all 29 tasks. Bump README to "Plans 1–8 complete; v1.0 ready to tag". Move Plan 8 items in the roadmap from "pending" to "shipped" (or "in-flight" if any defer).

- [ ] **Step 1: CHANGELOG entry** following the existing structure
- [ ] **Step 2: README status line + analysis count (now 13: 12 + clone-coupling)**
- [ ] **Step 3: Roadmap status update**
- [ ] **Step 4: Commit**

```bash
git add CHANGELOG.md README.md docs/
git commit -m "docs: Plan 8 complete — v1.x release readiness shipped"
```

---

## Plan 8 Definition of Done

- [ ] All 5 validation-report findings closed
- [ ] `--analysis authors` works, code-maat parity tested
- [ ] `--group-file` parsed (deferred aggregation noted in error)
- [ ] `--exclude` + `.codeloreignore` honored by clones
- [ ] Clones ship across CSV/JSON/Markdown/SARIF
- [ ] Persistent fact-store cache: hit/miss/LRU all tested; 2nd-run on same HEAD is < 50ms
- [ ] Parallel complexity extraction: 3-5× wall-time improvement measured on `medium_repo`
- [ ] `clones` table populated during ingest; `analyses::clones::run_clones` uses it
- [ ] `clone-coupling` analysis ships with 5 false-positive mitigations + `CODELORE-LIVE-CLONE` SARIF rule
- [ ] `codelore diff` ships for hotspots / coupling / clones with text/json/sarif/markdown output
- [ ] GitHub Actions example workflow committed
- [ ] All previous tests pass + Plan 8 tests pass (estimated +30 to +40 new tests)
- [ ] clippy/fmt/deny clean
- [ ] CHANGELOG + README + roadmap updated; v1.0 tag-ready

---

## Research briefs (full agent outputs)

The detailed research that informed this plan lives at:
- Clone-coupling algorithm + SARIF + false-positive mitigations: see "Research: clone-coupling intersection" agent output (Tier 1 sources: CodeScene X-Ray docs, SourcererCC papers, SARIF 2.1.0 OASIS spec, Roy & Cordy survey)
- PR-mode diff semantics + GitHub Actions integration: see "Research: diff-mode analysis" agent output (sources: CodeScene Delta Analysis docs, SonarQube PR analysis, GitHub SARIF docs, Codecov Delta docs)
- Parallel extraction + persistent cache design: see "Research: parallel + cached pipeline" agent output (sources: tree-sitter Send/Sync issue tracker, gix Repository docs, DuckDB concurrency docs, Nx/Cargo caching strategies)

Cite-worthy URLs are embedded throughout the task templates above.

---

*End of Plan 8. After Plan 8 closes: v1.0 is tag-ready with all spec §1.1 promises met, the clone-coupling differentiator shipped, and the diff subcommand makes the tool credible for CI deployment. Plan 9 (v1.1 scope) picks up PGO + Type 3 MinHash + the bus-factor/knowledge-island detectors.*
