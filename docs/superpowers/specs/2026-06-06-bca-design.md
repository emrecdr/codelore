# bca — Behavioral Code Analyzer

**Design specification, v1 (Spine release)**
Status: draft for sign-off
Date: 2026-06-06
Author: emre + Claude

A modernized, Rust-based behavioral code analysis tool. Mines git history to produce hotspots, temporal coupling, ownership topology, code-age, code-health, and team-communication metrics. Inspired by Adam Tornhill's `code-maat` (the open-source seed of CodeScene). Rebuilt around 2026 industry-standard tooling and statistical methodology.

---

## 1. v1 scope (Spine release)

### 1.1 In scope

- **Rust** (edition 2024, MSRV = stable-2), Cargo workspace, **3 crates from day one**: `bca-lib` + `bca-cli` + `bca-rca` (vendored RCA fork). Split `bca-lib` further (into `bca-core` / `bca-repo` / `bca-facts` / `bca-complexity` / `bca-analyses`) when seams emerge — likely v1.5 when RefactoringMiner + co-change entropy land.
- **GPL-v3** license (matches code-maat heritage).
- **Git-only via `gix` (gitoxide)** behind a 5-method `Repo` trait. `GitCliRepo` as differential-test oracle + Windows fallback.
- **Embedded DuckDB** as the fact store + SQL query layer (Glean-style separation of facts/storage/query/access).
- **Event-sourced pipeline**: `Stream<CommitEvent>` + projections. Pure stages keyed by commit SHA so Salsa-style incremental recompute can retrofit in v2.
- **Function-level entity baseline** via tree-sitter; file-level rollups derive trivially.
- **Vendored fork of Mozilla's `rust-code-analysis`** in `crates/bca-rca/` for complexity (Cyclomatic, Cognitive, Halstead, MI, NOM, NEXITS, LOC variants) across supported languages.
- **Kamei 14-feature change vector** as the canonical change-record schema.
- **Tier-1 languages** (default-on, ship in the standard binary): Rust, TypeScript/JavaScript, Python, Java, **Go**. Note: Go is NOT in upstream `mozilla/rust-code-analysis`; we implement Go support ourselves in `crates/bca-rca/` as Phase 0 work (5–8 engineer-days budgeted). All other Tier-1 langs inherit RCA's existing implementations.
- **Tier-2 languages** (opt-in via `--features lang-*`, not in standard binary): C, C++, Ruby, **Kotlin (file detection only — every RCA Kotlin metric is currently a no-op stub; metrics return 0/empty until proper impl lands in v1.5)**. **C# dropped from v1 Tier-2** because RCA has no C# implementation at all (see Validation Stream 2); deferred to v1.5 alongside Kotlin metric impls.
- **~10 core analyses**: hotspots, change-coupling (Fisher exact, default p < 0.05), code-ownership (Fractal Value), code-age, abs-churn, author-churn, entity-churn, communication, code-health composite, summary.
- **Hotspot ranking — publicly documented formula** (the transparency wedge vs. CodeScene's ML-based opaque ranking):
  ```
  hotspot_score(entity) = percentile_rank(revisions)
                        × percentile_rank(cognitive_complexity)
                        × (10 − code_health) / 10
  ```
  Sorted descending. All inputs are also emitted alongside the score so users can verify.
- **Canonical default thresholds** (code-maat parity): `min_revs = 5`, `min_shared_revs = 5`, `min_coupling_pct = 30`, `max_coupling_pct = 100`, `max_changeset_size = 30`, `fisher_significance = 0.05`.
- **Default merge-commit behavior**: merge commits (`is_merge = true`, `parent_count > 1`) are **excluded** from coupling, churn, and ownership analyses by default; included in `summary` and `code-age`. Configurable via `--include-merges`. Choice recorded in provenance as `merge_handling`.
- **Default architectural-grouping behavior** (`-g`): files matching no group expression are **dropped silently** (code-maat parity), with a warning emitted to stderr listing how many files were dropped. Configurable to fail-fast via `--strict-grouping`.
- **Input encoding**: git-only via gix, which handles UTF-8 and falls back to byte-string for non-UTF-8 paths. No `--input-encoding` flag (code-maat's was needed for VCS log files; we read .git directly).
- **`authors` and `revisions`** are addressable standalone via `bca analyze --analysis authors` and `bca analyze --analysis revisions` (each is a SQL view over the fact store), in addition to being inputs to `hotspots`. Code-maat parity preserved.
- **Performance targets (v1)**: process the Linux kernel (~1.4M commits, ~70k files) in **under 10 minutes** on M3 / Ryzen 7-class hardware with **peak memory under 4 GB** (DuckDB spill enabled). Stretch: <5 minutes. Tracked as a release blocker; benchmarked weekly in CI against a cached Linux kernel snapshot.
- **Provenance manifest** alongside every analysis output — machine-readable record of every mining choice.
- **Identity resolution** via `.mailmap` + `bots.toml` (default-deny bot list) + canonical-author config (Faros-style).
- **Outputs**: CSV, JSON, **SARIF 2.1.0** (the differentiator), Markdown for `$GITHUB_STEP_SUMMARY`, Parquet, SQLite (via DuckDB ATTACH).
- **`dist`** for releases → `cargo binstall bca` works day one.
- **SLSA Level 3 provenance** on binary releases.

### 1.2 Explicitly out of scope (permanent)

- Multi-VCS support (svn/hg/p4/tfs).
- Polars streaming engine reliance.
- Web UI / hosted backend.
- AI refactoring agent (CodeScene ACE territory).
- FFI bindings (Python/JS); DuckDB SQLite/Parquet artifacts serve as language-neutral consumption layer.
- Async runtime (workload is CPU-bound batch).

### 1.3 Deferred (tracked in Feature Registry §8)

See §8 for the canonical list of every deferred feature with target version and rationale.

---

## 2. Workspace architecture

```
bca/
├── Cargo.toml                # workspace
├── crates/
│   ├── bca-lib/              # types, pipeline, analyses, complexity, outputs
│   ├── bca-cli/              # clap + glue → single binary
│   └── bca-rca/              # vendored fork of Mozilla rust-code-analysis
├── fixtures/                 # tiny test repos + golden CSVs
├── benches/                  # criterion benchmarks
├── docs/superpowers/specs/   # design docs (this one)
├── justfile                  # task runner
├── rust-toolchain.toml       # toolchain pin
├── deny.toml                 # cargo-deny config
├── renovate.json             # dependency updates
└── .devcontainer/            # reproducible dev env
```

### 2.1 Module layout inside `bca-lib`

```
bca-lib/src/
├── types/          # Commit, Change, FileChange, Hunk, ChangeType, KameiFeatures, schema_version
├── repo/           # Repo trait + GixRepo + GitCliRepo impls
├── pipeline/       # event-sourced stages: source → identity → group → temporal → team → ingest
├── facts/          # DuckDB schema, fact ingestion, provenance manifest emission
├── complexity/     # wraps bca-rca; tree-sitter visitors; nesting-depth fallback
├── analyses/       # the ~10 v1 analyses, each as SQL views + Rust orchestration
├── output/         # csv, json, sarif, markdown, parquet, sqlite emitters
├── stats/          # Fisher exact (via fishers_exact crate), averages, helpers
└── config/         # TOML config parsing
```

### 2.2 External dependencies (versions locked to mid-2026 reality)

Versions confirmed via Validation Stream 3 (gix + DuckDB + Arrow integration audit, 2026-06-06):

| Crate | Locked version | Pin policy | Purpose |
|---|---|---|---|
| `gix` (`features = ["max-performance"]`) | **0.84.0** | minor-pin | Read .git directly (zlib-ng, parallel pack caches) |
| `duckdb` (`features = ["bundled", "appender-arrow", "parquet", "json"]`) | **1.10503.1** (DuckDB 1.5.3) | exact-pin | Fact store, SQL, Arrow ingest |
| `arrow` (re-exported via `bca-lib::arrow_facade`) | **58.3.0** | **bumped in lockstep with `duckdb` releases** — see §2.6 | Columnar in-flight format |
| `tree-sitter` | **=0.25.3** (exact, pinned for RCA compatibility) | exact-pin | AST parsing core |
| per-language `tree-sitter-*` grammars | per-grammar minor pin | minor-pin | Per-language AST |
| `polars` (behind `query-backend` trait, bridges via Arrow IPC bytes, NOT direct) | 0.54.4 | minor-pin | Optional Polars query path |
| `rayon` | latest | latest | Parallel complexity scanning |
| `crossbeam-channel` | latest | latest | gix-worker → Appender pipeline |
| `fishers_exact` | latest | latest | Fisher exact significance |
| `time` (NOT chrono) | 0.3.x | minor-pin | Date arithmetic |
| `regex` | latest | latest | Boundary mapping, message regex |
| `serde` + `serde_json` + `serde_yaml` | latest | latest | Serialization |
| `clap` (derive) | latest | latest | CLI |
| `anyhow` 2.x | latest | latest | CLI errors |
| `thiserror` 2.x | latest | latest | Library errors |
| `tracing` + `tracing-subscriber` + `tracing-indicatif` + `indicatif` | latest | latest | Logging + progress |

**Critical pinning notes:**
- `tree-sitter = "=0.25.3"` is an **exact-version pin** matching RCA's working revision. Upstream RCA's attempted bump to 0.26.3 (PR #1207) was reverted within 24 hours (#1212). We hold at 0.25.3 until upstream lands a clean bump.
- `duckdb` is **exact-pinned** because it locks `arrow` to a specific minor (currently 58.x). If `arrow = "59"` ships and `duckdb` still pins 58, naive consumers get a hard build error. The `bca-lib::arrow_facade` module re-exports all Arrow types so we update in one place.
- **`frozen-duckdb` REMOVED from spec.** Validation found it has 3 stars, 17 commits, 0 published releases — personal hack, not viable. Replacement: `sccache` on CI for dependency caching; `cargo build --profile dev` for iteration; accept ~8–14 min cold release builds (DuckDB bundled C++ dominates).

### 2.6 Arrow facade pattern (CRITICAL — version-pin insulation)

```rust
// bca-lib/src/arrow_facade/mod.rs
//! Single source of truth for Arrow types throughout the workspace.
//! Re-exports the version of arrow-rs that the duckdb crate currently
//! depends on, so a duckdb release that bumps Arrow doesn't fragment
//! the workspace. Bump in lockstep with duckdb-rs major releases.

pub use arrow::array::*;
pub use arrow::record_batch::RecordBatch;
pub use arrow::datatypes::*;
// ... etc
```

All `use arrow::*` in workspace crates becomes `use bca_lib::arrow_facade::*`. CI lint rule (custom clippy) forbids direct `arrow::*` imports outside the facade module.

### 2.3 Feature flags

- `lang-{rust,typescript,python,java,go}` — default-on per-language tree-sitter grammars (Go support is bca-implemented; rest are RCA-vendored)
- `lang-{cpp,kotlin,ruby,c}` — opt-in Tier-2 (Kotlin is file-detection-only until v1.5 per §1.1)
- `sarif` — default on; small surface
- `mcp` — promoted from v2 to **v1.5** per Validation Stream 1; default off in v1, on in v1.5
- `metrics-experimental` — gates JS/TS Halstead and MI metrics (RCA bugs #528 and #1183 produce unreliable values; opt-in only for research / debugging until upstream fixes or we patch)
- `query-backend-polars` — opt-in Polars query path through Arrow IPC bridge (default off; DuckDB SQL is the v1 path)

### 2.4 Release infrastructure

- `dist` v0.24+ → multi-platform binaries + Homebrew + shell installer + dist-manifest.json
- `cargo-binstall` reads dist-manifest.json automatically
- `cargo-pgo` (PGO + BOLT) wired into release CI for v1.1 (not v1.0; needs benchmark suite first)
- **`sccache` on CI** for dependency caching (replaces deleted `frozen-duckdb`). On a cold CI runner, full build is 8–14 min (DuckDB C++ ~4–6 min); `sccache` recovers most of that on subsequent runs.
- Distroless container image (~35–55 MB — DuckDB bundled is ~20 MB of that; Polars (if enabled) adds ~10 MB). Optimize with `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true` to shave 15–25%.
- SLSA Level 3 provenance via `slsa-framework/slsa-github-generator`

### 2.5 Quality gates in CI

- `cargo test` (all crates, unit + integration)
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- `cargo deny check` (license + advisory DB)
- `cargo insta test` (snapshot regression)
- `cargo llvm-cov --workspace` (coverage threshold: 75% v1, 85% v1.5)
- Differential test vs C git (nightly + pre-release)
- Differential test vs code-maat goldens (every PR)
- CodeQL workflow (security)
- `dtolnay/rust-toolchain` for setup (NOT `actions-rs/*` which is deprecated)

---

## 3. Data model & event-sourced pipeline

### 3.1 Public types

```rust
pub const SCHEMA_VERSION: u8 = 1;

pub struct CommitEvent {
    pub rev: String,
    pub author_email: String,
    pub author_name: String,
    pub committer_email: String,
    pub date: time::Date,           // time crate, NOT chrono
    pub message: String,
    pub parents: Vec<String>,
    pub changes: Vec<FileChange>,
    /// Populated by the enrich_kamei pipeline stage, NOT at gix walk-time.
    /// Walk-stage emits None; enrichment stage folds across prior commits
    /// to populate ndev/age/nuc/exp/rexp/sexp before downstream stages see it.
    pub kamei: Option<KameiFeatures>,
}

pub struct FileChange {
    pub path: String,
    pub change_type: ChangeType,
    pub loc_added: u32,
    pub loc_deleted: u32,
    pub hunks: Vec<Hunk>,
}

pub enum ChangeType {
    Added,
    Modified,
    Deleted,
    Renamed { from: String, similarity: u8 },
    Copied  { from: String, similarity: u8 },
    BinaryOrUnknown,
}

pub struct Hunk { pub old_start: u32, pub old_lines: u32, pub new_start: u32, pub new_lines: u32 }

pub struct KameiFeatures {
    pub ns: u32, pub nd: u32, pub nf: u32, pub entropy: f64,
    pub la: u32, pub ld: u32, pub lt: f64,
    pub fix: bool,
    pub ndev: u32, pub age: f64, pub nuc: u32,
    pub exp: u32, pub rexp: f64, pub sexp: u32,
}
```

### 3.2 DuckDB fact schema

```sql
CREATE TABLE commits (
    rev TEXT PRIMARY KEY,
    author_email TEXT NOT NULL,
    author_name  TEXT NOT NULL,
    committer_email TEXT NOT NULL,
    canonical_author TEXT NOT NULL,
    -- ai_attribution populated when --track-ai-authorship is enabled (v1.5)
    -- distinguishes human-authored / AI-assisted / AI-authored commits via
    -- committer_email pattern + signed-by trailers (Copilot, Claude Code, Cursor patterns)
    ai_attribution TEXT,    -- 'human' | 'ai-assisted' | 'ai-authored' | NULL = unknown
    date DATE NOT NULL,
    message TEXT NOT NULL,
    is_merge BOOLEAN NOT NULL,
    parent_count INTEGER NOT NULL,
    -- Kamei vector inlined
    ns INTEGER, nd INTEGER, nf INTEGER, entropy DOUBLE,
    la INTEGER, ld INTEGER, lt DOUBLE,
    fix BOOLEAN,
    ndev INTEGER, age DOUBLE, nuc INTEGER,
    exp INTEGER, rexp DOUBLE, sexp INTEGER
);

CREATE TABLE changes (
    rev TEXT NOT NULL REFERENCES commits(rev),
    path TEXT NOT NULL,
    change_type TEXT NOT NULL,
    rename_from TEXT,
    similarity INTEGER,
    loc_added INTEGER NOT NULL,
    loc_deleted INTEGER NOT NULL,
    PRIMARY KEY (rev, path)
);

CREATE TABLE hunks (
    rev TEXT NOT NULL, path TEXT NOT NULL,
    old_start INTEGER, old_lines INTEGER,
    new_start INTEGER, new_lines INTEGER,
    FOREIGN KEY (rev, path) REFERENCES changes(rev, path)
);

CREATE TABLE entities (
    path TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
    start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
    rev_introduced TEXT NOT NULL, rev_last_seen TEXT NOT NULL,
    PRIMARY KEY (path, name, rev_introduced)
);

CREATE TABLE complexity_metrics (
    path TEXT NOT NULL, name TEXT NOT NULL, rev TEXT NOT NULL,
    cyclomatic INTEGER, cognitive INTEGER,
    halstead_volume DOUBLE, halstead_difficulty DOUBLE, halstead_effort DOUBLE,
    mi DOUBLE,
    nom INTEGER, nexits INTEGER,
    loc INTEGER, sloc INTEGER,
    -- nesting metrics (Tornhill whitespace-complexity equivalents + AST refinement)
    max_nesting INTEGER, mean_nesting DOUBLE, sd_nesting DOUBLE, total_nesting INTEGER,
    PRIMARY KEY (path, name, rev)
);

CREATE TABLE author_aliases (
    raw_email TEXT PRIMARY KEY,
    canonical TEXT NOT NULL,
    is_bot BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE provenance (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

Provenance populated with: `schema_version`, `bca_version`, `gix_version`, `duckdb_version`, `repo_head_sha`, `inputs_fingerprint` (sha256 of options + head_sha), `after_date`, `before_date`, `commit_range`, `merge_handling`, `rename_similarity_threshold`, `bot_filter_pattern`, `squash_unwrap_strategy`, `mailmap_source`, `temporal_period_days`, `group_file`, `team_map_file`, `min_revs`, `min_shared_revs`, `fisher_significance`, `max_changeset_size`, `complexity_sample_strategy`, `run_started_at`, `run_completed_at`.

### 3.2.1 Correctness invariants enforced in SQL views

The following invariants are baked into every analysis SQL view that touches `changes` or coupling. **These are not optional config — they are the algorithmic equivalent of code-maat's most important correctness moves**, hoisted from the Clojure pipeline (`coupling_algos.clj`) into the SQL layer:

1. **`max-changeset-size` pre-filter for coupling pairs.** Any commit touching more than `max_changeset_size` files (default 30) is **excluded before generating coupling pairs**. Without this, a single license-header sweep across N files produces O(N²) spurious pairs. SQL implementation: `WHERE rev IN (SELECT rev FROM changes GROUP BY rev HAVING COUNT(*) <= :max_changeset_size)`.

2. **Mirrored pair dedup.** Coupling self-joins use `e1 < e2` canonical ordering to drop `(B,A)` when `(A,B)` is present.

3. **Empty-changeset filter.** Commits with zero file changes are excluded from all analyses (matches code-maat `summary.clj:1648-1649`).

4. **Merge-commit exclusion** per §1.1 default (unless `--include-merges`).

5. **`min_revs` / `min_shared_revs` post-filters.** Applied AFTER computation but BEFORE Fisher significance testing.

6. **Fisher exact significance.** Applied to coupling pairs that pass thresholds 1–5. Pairs with `p ≥ fisher_significance` (default 0.05) are dropped from the output unless `--no-significance-filter` is set.

These six invariants together close the methodological gap that 2025 MSR research (Spadoni et al.) flagged as the root of the 500% inter-tool disagreement. Every applied filter is recorded in the provenance manifest.

### 3.2.2 Concurrency pattern (locked from Validation Stream 3)

```
┌──────────────┐
│  N gix       │
│  workers     │──┐
│  (rayon)     │  │
└──────────────┘  │
┌──────────────┐  │
│  N gix       │  │      bounded crossbeam_channel<RecordBatch>
│  workers     │──┼─────────────────────────────────────────────┐
│  (rayon)     │  │      capacity = 64 batches × ~8K rows = 512K│
└──────────────┘  │                                              │
       ...        │                                              ▼
                  │                                       ┌──────────────┐
                  │                                       │  1 DuckDB    │
                  │                                       │  Appender    │
                  │                                       │  thread      │
                  │                                       │ (append_     │
                  │                                       │  record_     │
                  │                                       │  batch)      │
                  │                                       └──────────────┘
```

Why N→1, not N→N: DuckDB's Appender serializes commits per table internally — N→N producers fighting one Appender mutex erases the parallelism gain. Single dedicated Appender thread + bounded backpressure channel is the validated production pattern.

`gix::Repository` is `Send` but not `Sync`. Per-worker handle pattern: `repo.into_sync().clone()` per worker; object database shared via `Arc`, pack files mmap'd zero-copy.

RecordBatch size: **8K–64K rows per batch**, matching DuckDB's internal vector size (2048) at an even multiple. Smaller batches starve the Appender; larger ones increase peak memory.

### 3.3 Pipeline as Rust function signatures

```rust
pub trait Repo {
    fn walk_commits(&self, opts: &Options) -> Result<impl Stream<Item = CommitEvent>>;
    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>>;
    fn diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>>;
    fn resolve_alias(&self, email: &str) -> String;
    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata>;
}

pub fn resolve_identities(s: impl Stream<Item = CommitEvent>, lookup: &AuthorLookup) -> impl Stream<Item = CommitEvent>;
/// Folds across prior commits to populate Kamei history features (ndev/age/nuc/exp/rexp/sexp).
/// MUST run after walk + identity, BEFORE ingest. Idempotent: re-running on already-enriched
/// events is a no-op.
pub fn enrich_kamei(s: impl Stream<Item = CommitEvent>) -> impl Stream<Item = CommitEvent>;
pub fn group_by_boundaries(s: impl Stream<Item = CommitEvent>, opts: &Options) -> Result<impl Stream<Item = CommitEvent>>;
pub fn group_by_temporal_period(s: impl Stream<Item = CommitEvent>, opts: &Options) -> Result<impl Stream<Item = CommitEvent>>;
pub fn map_teams(s: impl Stream<Item = CommitEvent>, opts: &Options) -> Result<impl Stream<Item = CommitEvent>>;
pub fn ingest_facts(s: impl Stream<Item = CommitEvent>, db: &Connection) -> Result<ProvenanceManifest>;
pub fn run_analysis(db: &Connection, name: AnalysisName, opts: &Options) -> Result<DataFrame>;
pub fn emit(df: &DataFrame, format: OutputFormat, dest: Dest, provenance: &ProvenanceManifest) -> Result<()>;
```

---

## 4. Complexity & language strategy

### 4.1 RCA fork (hard caveats from Validation Stream 2)

**State of upstream `mozilla/rust-code-analysis`** as of mid-2026:
- Last release: **v0.0.25 on 2023-01-13** (3+ years without a release).
- Last commit: 2026-01-20 (dependabot bump).
- 412 stars / 68 forks / 54 open issues / 15 open PRs.
- **Bug #528** (JS arrow function Halstead broken) open since March 2021 with zero triage.
- **`cargo install rust-code-analysis-cli` does not compile** on current toolchains. Vendoring from `master` is the only path.
- No community fork to align with.

**Vendoring procedure (Phase 0):**

1. Copy `master` into `crates/bca-rca/`. Last-good commit: 2026-01-20 `37e5d83` (or successor as of Phase 0).
2. **Drop from vendor**:
   - `rust-code-analysis-web/` (pulls actix-web; unused)
   - `rust-code-analysis-cli/` (we expose RCA through our own CLI)
   - Vendored `tree-sitter-mozcpp/` (~25 MB) — Mozilla's tree-sitter-cpp fork with custom macro handling; we don't need it
   - Vendored `tree-sitter-mozjs/` (~2.7 MB) — same rationale
   - `.gitmodules` and integration-test submodules (~GB total)
3. Replace integration-test fixtures with small reproducible repos.
4. **Strip unused per-language trait impls**: ABC, WMC, NPA, NPM (all Java-only specializations not in our spec).
5. **Add Go support ourselves** (5–8 engineer-days; see §4.2).
6. Pin `tree-sitter = "=0.25.3"` per §2.2.
7. Tag licenses per §4.1.1.

**Upstream merge cadence** (year-1 maintenance budget ~8 days):
- **Cherry-pick, do not full-rebase.** Mozilla's tempo is so slow (most 2026 commits are dependabot) that re-bases are cheap; the cost is reviewing what merged and deciding what to take.
- Pull only correctness fixes and grammar bumps. Ignore feature noise.
- Budget ~1 day/month for review + selective pick.

### 4.1.1 License precision

Per [Mozilla's MPL-2.0 FAQ Q14](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) and the [combining-MPL-and-GPL guide](https://www.mozilla.org/en-US/MPL/2.0/combining-mpl-and-gpl/), this is exactly the use case MPL-2.0 §3.3 was written for. Confirmed legal pattern.

- `crates/bca-rca/Cargo.toml`: `license = "MPL-2.0 AND GPL-3.0-only"` (NOT `OR` — we distribute under BOTH simultaneously, not as a downstream choice).
- All original RCA files retain MPL-2.0 headers untouched.
- Any modifications WE make to original RCA files remain MPL-2.0 (so we can push fixes upstream).
- NEW files (our Go impl, our test harness, our wrappers) carry GPL-3.0-only headers.
- `bca-lib/Cargo.toml` and `bca-cli/Cargo.toml`: `license = "GPL-3.0-only"`.
- Add `crates/bca-rca/LICENSE-MPL` (verbatim MPL-2.0 text).

**Watch-out:** if upstream patches we accept introduce GPL-only logic into MPL files, those files lose pushability. Discipline: never mix.

### 4.2 Language tiers (validated against RCA's actual per-language impl table)

Tree-sitter grammar quality + RCA metric coverage drives the tiering. Stream 2 verified every `impl X for YCode` in RCA's `src/`; the table below reflects **what's actually computed**, not what's listed in marketing:

| Tier | Language | Cyclomatic | Cognitive | Halstead | LOC | MI | NOM | NEXITS | Source |
|---|---|---|---|---|---|---|---|---|---|
| **v1 Tier-1** | Rust | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | RCA |
| **v1 Tier-1** | TypeScript/JavaScript | ✓ | ✓ | ⚠️ buggy | ✓ | ⚠️ buggy | ✓ | ✓ | RCA + `metrics-experimental` |
| **v1 Tier-1** | Python | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | RCA |
| **v1 Tier-1** | Java | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | RCA |
| **v1 Tier-1** | **Go** | bca-impl | bca-impl | bca-impl | bca-impl | bca-impl | bca-impl | bca-impl | **bca-rca additions (Phase 0)** |
| v1 Tier-2 | C | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | RCA |
| v1 Tier-2 | C++ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | RCA |
| v1 Tier-2 | Ruby | bca-impl | bca-impl | bca-impl | bca-impl | bca-impl | bca-impl | bca-impl | bca-rca additions (Phase 0 stretch) |
| v1 Tier-2 | **Kotlin (file detection only)** | stub | stub | stub | stub | stub | stub | stub | RCA stubs return 0/empty — v1.5 work |
| **DROPPED from v1** | **C#** | — | — | — | — | — | — | — | RCA has no C# impl. Deferred to v1.5 with Kotlin metric impls. |
| v1.5 | Swift, PHP, Scala | — | — | — | — | — | — | — | Tree-sitter grammar stabilization wait |

**Key consequences:**

1. **Go is Phase 0 work** (5–8 days): vendor `tree-sitter-go`, regenerate `src/languages/language_go.rs` via RCA's `enums/` build script, write `impl Cyclomatic for GoCode`, `impl Cognitive`, `impl Halstead`, `impl Loc`, `impl Exit`, `impl Nom` — ~50 LOC × 6 trait impls + snapshot tests. Pattern: copy `language_rust.rs` and adapt to Go AST node names.

2. **JS/TS Halstead and MI** are quarantined behind `--features metrics-experimental` until upstream fixes or we patch (issues #528 since 2021, #1183 since Aug 2025). SARIF default output excludes them; verified-by-tests metrics only.

3. **Kotlin in v1 is file detection only**. Release notes must say: *"Kotlin: file detection working; complexity metrics return 0/empty pending implementation. Tracked as v1.5 work."* Same caveat for C# but it's not even file-detection in v1.

### 4.2.1 Honest metric stance (Stream 2 recommendation)

- **Cyclomatic + Cognitive** are the headline metrics. Both are tree-sitter friendly and have decades of empirical study (Cyclomatic 1976; Cognitive Complexity, SonarSource 2017).
- **Halstead and MI** are exposed for compatibility with code-maat / industry tooling but **do not oversell their predictive validity**. Recent research (Muñoz Barón et al., JSS 2022) shows Cognitive Complexity is at best modestly better than Cyclomatic for defect prediction when controlling for size.
- **Don't claim numeric defect-correlation rates** in our docs unless we ship our own validation dataset.

### 4.2.2 Alternative reference monitoring

Validation Stream 2 identified **`StrangeDaysTech/arborist-metrics`** (Apache-2.0/MIT, active 2026-05, supports Go natively, Cognitive + Cyclomatic + SLOC only) as the most plausible v2 swap-in. Track it for 12 months. If it grows Halstead + MI and stays active, plan v2 evaluation — license is more permissive than ours.

### 4.3 Per-entity metrics (computed via tree-sitter)

Cyclomatic, Cognitive, Halstead (Volume, Difficulty, Effort), MI, NOM, NEXITS, LOC/SLOC/CLOC/BLANK, max-nesting, mean-nesting.

### 4.4 Computation modes

- `--complexity-sample head` (default): parse every file at HEAD only.
- `--complexity-sample adaptive`: every commit if file has <100 revs, every 5th for 100–1000, every 25th for 1000+ (folded from §4 improvement e).
- `--complexity-sample full`: every revision of every changed file.

Choice stored in provenance manifest.

### 4.5 Fallback for unsupported languages

`--complexity-fallback nesting-depth` (default): count consecutive same-character whitespace prefix per line (Tornhill's heuristic). Universal; coarse; documented.

### 4.6 Code-Health composite formula

```
codehealth(entity) ∈ [0, 100], higher = healthier
= 100 × (1
        - w_cx · normalize(cognitive_complexity)
        - w_cn · normalize(churn_rate)
        - w_au · normalize(author_fragmentation_FV)
        - w_cp · normalize(coupling_centrality_SoC)
        )

defaults: w_cx = 0.40, w_cn = 0.25, w_au = 0.15, w_cp = 0.20  (sum = 1.0)
```

`normalize` maps to `[0, 1]` via the repo's empirical 95th percentile of each metric. Weights configurable in `bca.toml`. Stored in provenance per run.

References: Campbell 2018 (cognitive), Bird 2011 (ownership), Tornhill 2013 (SoC).

---

## 5. Public API, CLI, outputs

### 5.1 Library API (5 public functions + types)

```rust
pub fn analyze(opts: Options) -> Result<Report>;
pub fn stream(opts: Options) -> Result<impl Stream<Item = ReportEvent>>;

pub struct Report {
    pub provenance: ProvenanceManifest,
    pub db: Connection,
}

impl Report {
    pub fn run_analysis(&self, name: AnalysisName, opts: &AnalysisOptions) -> Result<DataFrame>;
    pub fn sql(&self, query: &str) -> Result<DataFrame>;  // read-only connection
    pub fn write(&self, format: OutputFormat, dest: Dest) -> Result<()>;
    pub fn write_all_analyses(&self, format: OutputFormat, dir: &Path) -> Result<()>;
}
```

### 5.2 CLI subcommands

```
bca init                    # write a default bca.toml
bca analyze [opts]          # run analysis, emit outputs
bca diff <base> <head>      # PR-mode analysis wrapper
bca query <sql>             # SQL escape hatch against the fact store (read-only)
bca facts                   # inspect / dump the fact store
bca explain <analysis>      # print algorithm + provenance impact
bca config [show|validate]
bca doctor                  # diagnose common issues
bca version                 # human + --format json
```

Global flags: `-r/--repo`, `-c/--config`, `--after`, `--before`, `--commit-range`, `--format`, `-o/--output`, `--rows N` (limit output rows — code-maat parity), `--log-format`, `--no-progress`, `-v/--verbose`, `-q/--quiet`.

Analysis-specific flags: `--verbose-results` on `bca analyze --analysis coupling` emits per-entity revision counts and shared-revision counts alongside the degree (code-maat exact parity; named distinctly from the global `-v/--verbose` to avoid conflict). `--include-merges` overrides default merge-commit exclusion. `--strict-grouping` causes `-g` mismatch to error rather than warn. `--no-significance-filter` disables Fisher exact filtering on coupling (debugging / research only).

### 5.3 Output formats

| Format | Use | Implementation |
|---|---|---|
| CSV | Code-maat parity, spreadsheets | DuckDB `COPY ... (FORMAT 'csv')` |
| JSON | Pipe into other tools, CI parsing | DuckDB `COPY ... (FORMAT 'json')` |
| Parquet | Long-term storage, downstream analytics | DuckDB native |
| SQLite | Shareable, queryable artifact | DuckDB `ATTACH` |
| **SARIF 2.1.0** | **GitHub Code Scanning UI — the differentiator** | Custom emitter, ~200 LOC |
| Markdown | `$GITHUB_STEP_SUMMARY` in Actions | Custom emitter, ~150 LOC |

### 5.3.1 Naming hygiene (CodeScene trademark watch)

Per Validation Stream 1: CodeScene holds product marks on **CodeHealth™, X-Ray, ACE, System Mastery, and Code Biomarkers**. We must use neutral terms in CLI, docs, and outputs:

| Avoid | Use instead |
|---|---|
| Code Health™ / CodeHealth™ | `code-health` (lowercase, generic noun) |
| X-Ray | `function-analysis` |
| ACE | (we don't ship LLM refactoring; N/A) |
| System Mastery™ | `system-mastery-index` |
| Code Biomarkers | `code-smells` |

Headers, docs, and CLI subcommand names already follow this; review on every new doc page.

### 5.4 Behavioral SARIF rule taxonomy

- `BCA-HOTSPOT` — Hotspot (high revisions × complexity)
- `BCA-COUPLING` — Significant logical coupling pair
- `BCA-OWNERSHIP-RISK` — Ownership fragmentation high
- `BCA-CODE-HEALTH` — CodeHealth below threshold

Properties: `tags: ["behavioral", "<rule-tag>"]`, `security-severity: min(10, codehealth_below_threshold × 10)`, plus `bca/*` metric keys.

### 5.5 TOML configuration

`bca.toml` per industry convention. See §5 inline draft in the dialogue history; full schema documented in `docs/config.md` at implementation time.

---

## 6. Testing, observability, ops

### 6.1 Three concentric test rings

**Ring 1 — Unit tests** per module. Pure functions of types. `cargo test`. Always-on.

**Ring 2 — Property + snapshot tests.** `proptest` for pipeline invariants (v1). `insta` + `assert_cmd` for CLI behaviour (v1). `cargo-mutants` for mutation testing (v1.5, nightly).

**Ring 3 — Differential testing.**
- Against C git via `GitCliRepo` — every `GixRepo` op has a matching property test. Nightly + pre-release.
- Against code-maat goldens — every PR. Documents deltas where we've improved (e.g. Fisher-filtered coupling diverges from raw co-change).
- Against CodeScene Free Community Edition — manual nightly job; not in CI.

### 6.2 Fixture repos

```
fixtures/repos/
├── tiny/                # 10 commits, 3 files
├── medium/              # ~500 commits
├── refactoring-heavy/   # exercises future RefactoringMiner integration
├── tangled-commits/     # exercises future untangling
├── bot-noisy/           # exercises bot filtering
└── monorepo-style/      # exercises -g boundary mapping
fixtures/golden/
├── code-maat/           # outputs from code-maat
└── bca/                 # outputs from bca (current)
```

Repos built programmatically via `gix` for exact reproducibility.

### 6.3 Benchmarks

`criterion` benchmarks against the three reference repos. CI tracks regression; >5% requires explicit approval.

**Release-blocking performance targets** (per §1.1):
- Linux kernel (~1.4M commits, ~70k files): full hotspot + coupling analysis in **<10 minutes** on M3 / Ryzen 7-class hardware
- Peak memory: **<4 GB** with DuckDB spill enabled
- Stretch: <5 minutes for Linux kernel
- Weekly CI job runs against cached Linux kernel snapshot; PRs regressing >10% on this metric require explicit perf-approval label

### 6.4 Observability

- `tracing` 0.1.x structured logging. Every pipeline stage has a span.
- `--log-format json` for CI consumption; pretty by default.
- `tracing-indicatif` so progress bars don't tear log lines.
- `--log-stats` flag for final block: rows processed, time per stage, peak memory, DuckDB query timings.
- OTel deferred.

### 6.5 Distribution

- `dist` v0.24+ → multi-platform binaries + Homebrew + binstall manifest + shell installer + MSI installer
- SLSA Level 3 provenance attached to release artifacts
- Distroless container (~15 MB)
- `cargo-pgo` in release CI starting v1.1 (after benchmark suite stable)

### 6.6 Error handling

- `thiserror` 2.0 in `bca-lib`
- `anyhow` 2.0 in `bca-cli`
- `BcaError` enum at lib/cli boundary drives exit codes (provenance violation = 2, parser = 3, analysis = 4, IO = 5)

### 6.7 Fuzzing (v1.5)

`cargo-fuzz` targets on the parser stage. Specific corpora: Unicode author names, weird file paths, malformed mailmap entries. v1 ships with the parser instrumented for fuzz targets but does not run a fuzz campaign in CI; that lands in v1.5 (per §8.6).

### 6.8 Developer experience

- `just` task runner (`justfile`)
- `.devcontainer/devcontainer.json` for reproducible dev
- `rust-toolchain.toml` pinned
- `cargo-deny` config (`deny.toml`)
- `renovate.json` (NOT Dependabot)
- `.editorconfig`, `rustfmt.toml`, `clippy.toml`
- Pre-commit hook: `bca analyze --quick` on changed files (dogfooding)

### 6.9 Methodological honesty

Locked v1 commitments:
- Fisher exact on coupling pairs (default p < 0.05).
- Provenance manifest with every output.
- Identity resolution (mailmap + bots).
- Differential testing as a release blocker.

Phased commitments tracked in Feature Registry §8.

---

## 7. 2026 industry-standards audit summary

Choices revised to match 2026 norms:
- `time` crate, not `chrono`.
- `fishers_exact` crate for the Fisher test.
- `just` as task runner.
- `cargo-llvm-cov`, not `cargo-tarpaulin`.
- Renovate, not Dependabot.
- `dtolnay/rust-toolchain` action, not `actions-rs/*`.

Additions for parity with 2026 norms:
- `cargo-deny` config + CI gate.
- `cargo-mutants` mutation testing.
- SLSA Level 3 provenance on releases.
- Conventional Commits + `git-cliff` for CHANGELOG.
- `rust-toolchain.toml` pinned to stable, MSRV = stable-2.
- `devcontainer.json`.
- CodeQL workflow on the project itself.
- `.editorconfig`, `rustfmt.toml`, `clippy.toml`.

Outliers we keep (with rationale documented in §1.2 and dialogue history):
- No async runtime (CPU-bound).
- No LSP server in v1 (analytics shape, not code-intelligence).
- No web UI (CodeScene's space; SARIF integrates better with GitHub UI).
- No FFI bindings (DuckDB artifacts are the FFI).

---

## 8. Feature Registry — every deferred feature, tracked

Every feature mentioned in research, brainstorming, or improvement passes that did NOT make v1. This is the canonical "nothing forgotten" record.

### 8.1 v1.5 (target: ~3–4 weeks after v1)

| Feature | Why deferred | Source / citation |
|---|---|---|
| Remaining 13 analyses (main-dev, refactoring-main-dev, entity-effort, main-dev-by-revs, fragmentation, soc, messages, identity-dump, fn-coupling, fn-ownership, fn-hotspot, code-age-trend, hotspot-trend, complexity-trend) | Spine v1 ships 10; rest stack on stable architecture. | code-maat porting catalog (validation pass 2026-06-06) |
| `bca refactor-targets` ranked-list subcommand | Composes codehealth + churn + ownership into actionable refactor-ROI ranking | DDD Europe 2019 talk premise |
| Knowledge-map analysis (per-module → who knows it; deep-history replay through renames via `git log --follow`) | Distinct from knowledge-graph *output* in v2 | CodeScene how-it-works |
| **Knowledge Island detection** (single-author × hotspot × low-code-health three-way join) | Promoted from v2 per Validation Stream 1 — cheap SQL given fact store; high product value | CodeScene Knowledge Distribution |
| **MCP server mode** with 5–6 tools (`hotspots`, `coupling`, `code_health`, `ownership`, `refactor_targets`, `explain`) | **PROMOTED from v2 to v1.5.** CodeScene shipped `codescene-oss/codescene-mcp-server` in Jan 2026 with 11 tools; MCP is now table-stakes per Validation Stream 1. Cheap given DuckDB backend (5–6 thin tools wrapping SQL views). | CodeScene MCP (Jan 2026); Stream 1 |
| **Ticket-ID cross-repo coupling** (regex over commit messages for `JIRA-1234` / `#123` tokens + UNION across repos) | The trick CodeScene's X-Ray uses to span microservices. ~50 LOC in DuckDB. | CodeScene X-Ray docs |
| **Bumpy Road biomarker** (multiple sibling logic chunks in one function — Tornhill-coined; tree-sitter computable) | Formalized as v1.5 work — high diagnostic value, low cost given tree-sitter visitors already built | CodeScene Biomarkers |
| AI-authored code tracking (`ai_attribution` commit column + `--track-ai-authorship` flag) | New 2026 reality: significant code is AI-assisted. No public CodeScene equivalent — we're ahead. | Industry shift |
| RCA Kotlin metric impls (currently stubs in upstream) | Tier-2 language with no working metrics in v1 — implement properly | RCA validation (Stream 2) |
| RCA C# language support (currently absent from upstream) | Tier-2 language not in RCA at all — add ourselves | RCA validation (Stream 2) |
| Fix RCA JS/TS Halstead bug (#528, open since 2021) | Required to drop `metrics-experimental` gate on JS/TS Halstead | RCA validation (Stream 2) |
| Co-change graph entropy as first-class hotspot feature | Strong v1 candidate, deferred to keep v1 shippable | Ma et al. arXiv:2504.18511 |
| RefactoringMiner integration for refactoring-aware filtering | Correctness fix; correctness wins compound; ~1–2 weeks of work | Tsantalis TSE 2022 |
| Bootstrap CIs (≥100 iterations) on ranked outputs | Methodological table stakes per academic stream | Tantithamthavorn TSE 2017 |
| Scott-Knott ESD ranking on leaderboards | Same source | Tantithamthavorn TSE 2017 |
| `bca codehealth-calibrate` subcommand (gradient search for weights) | Turns CodeHealth from opinion into evidence-driven score | §4 improvement c |
| Swift, PHP, Scala language support | Tree-sitter grammar stabilization wait | §4 language scope |
| `cargo-pgo` (PGO + BOLT) in release CI | Needs stable benchmark suite first | §2.4 |

### 8.2 v2 (target: ~6 months after v1)

| Feature | Why deferred | Source / citation |
|---|---|---|
| **Knowledge-graph JSON output** (nodes/edges) — the *output format* | Distinct from knowledge-map *analysis* (v1.5). Emitter for graph-of-code consumers (Greptile, Graphify, GitNexus). | Competitive + architecture streams |
| Cost-of-change / refactoring-ROI regression (own model, NOT CodeScene's 15×/9× numbers) | Empirically calibrated linear/quantile regression of merge-time vs. our code-health composite + hotspot rank. Cite Tornhill/Borg "Code Red" arXiv:2203.04374 as prior art. | CodeScene whitepaper 2026 |
| **System Mastery–like index** (project/component-level aggregation of knowledge-loss + fragmentation) | Trivially derivable as `1 − (w_kl·knowledge_loss + w_frag·fragmentation)` once v1.5 Knowledge Island ships. Mark as "inspired by, not equivalent to" CodeScene's System Mastery™. | CodeScene Status Badges; Stream 1 |
| **Code smells full set (≥15 named)**: Brain Class, Low Cohesion (LCOM4), Developer Congestion, Complex Code by Former Contributors, Brain Method, DRY Violations (co-change-aware), Primitive Obsession, Large Method, Complex Conditional, Large Assertion Blocks, Duplicated Assertion Blocks, Knowledge Loss, Knowledge Island. Bumpy Road and Nested Complexity already in v1.5. | All tree-sitter computable. Defer because v1.5 ships Bumpy Road; rest is incremental. | CodeScene Biomarkers (concrete list validated Stream 1) |
| **Proximity-as-intermediate-functions metric** (distance = number of intermediate functions between coupled methods) | Novel CodeScene X-Ray metric; expose for compatibility | CodeScene X-Ray docs |
| **Clone detection × co-change** (only flag clones that also change together) | Kills dead-clone noise; requires both clone detection (new) and coupling (existing) | CodeScene X-Ray docs |
| **Code coverage analysis (LCOV input, hotspot-weighted)** — "low-coverage hotspots" rather than aggregate % | CodeScene shipped in 2025; agent-relevant. Requires LCOV parser. | CodeScene 2025 release |
| **Delivery analysis (DORA-adjacent flow metric)** | CodeScene v7.2.0 Oct 2025. Measures "how efficiently dev work becomes shippable code." | CodeScene 7.2.0 release notes |
| Pluggable SZZ for bug-link induction (start AG-SZZ; allow Neural-SZZ/LLM4SZZ later) | Defer LLM-based; ship pass-through interface | Tang et al. ASE 2023; arXiv:2504.01404 |
| Pluggable tangled-commit untangling (ship pass-through) | Embed interface; defer impl | Shen et al. FSE 2021 (SmartCommit); UTango FSE 2022 |
| Salsa-style incremental memoization | v1 stages designed to allow retrofit | Rust-analyzer architecture |
| LSP server mode | Optional per industry-standards stream; analytics shape | Architecture stream |
| LLM-based commit classification | Pluggable model interface; hold to ACE-style auditable/rejectable loop | CommitBERT/CC2Vec literature; CodeScene ACE pattern |
| CodeBERT/GraphCodeBERT semantic coupling | Optional analyzer | Wu et al. FSE 2024 |
| Track `arborist-metrics` for v2 RCA swap evaluation | Apache-2.0, supports Go natively, currently only 3 metrics — re-evaluate if it grows Halstead/MI | Stream 2 |
| Pluggable SZZ for bug-link induction (start AG-SZZ; allow Neural-SZZ/LLM4SZZ later) | Defer LLM-based; ship pass-through interface | Tang et al. ASE 2023; arXiv:2504.01404 |
| Pluggable tangled-commit untangling (ship pass-through) | Embed interface; defer impl | Shen et al. FSE 2021 (SmartCommit); UTango FSE 2022 |
| Salsa-style incremental memoization | v1 stages designed to allow retrofit | Rust-analyzer architecture |
| LSP server mode | Optional per industry-standards stream; analytics shape | Architecture stream |
| LLM-based commit classification | Pluggable model interface | CommitBERT/CC2Vec literature |
| CodeBERT/GraphCodeBERT semantic coupling | Optional analyzer | Wu et al. FSE 2024 |

### 8.3 v3+ (research / scale)

| Feature | Why deferred | Source / citation |
|---|---|---|
| Differential dataflow / live hotspot streaming | Genuine v3 work; small audience for v1/v2 | Naiad SOSP 2013; Frank McSherry's Timely Dataflow |
| DataFusion-backed scale-out (Chromium-scale repos) | Switch from DuckDB if we hit >10M files / >50M commits | Data-engine stream |
| Bayesian hierarchical hotspot models | Interesting; no practitioner demand | Academic stream |
| Causal inference layer | Literature not yet decisive | Couto et al. JSS 2014 |

### 8.4 Phased methodological honesty (per user's "make sure planned in phases" request)

| Practice | Version | Source |
|---|---|---|
| Fisher exact on coupling pairs (p<0.05 default) | **v1** | Hämäläinen arXiv:1405.1360 |
| Provenance manifest (machine-readable) | **v1** | Spadoni arXiv:2501.15114 |
| Kamei 14-feature change vector | **v1** | Kamei et al. canonical JIT-SDP |
| Differential testing vs C git | **v1** | Spadoni 2025 |
| Bootstrap CIs (≥100 iter) | v1.5 | Tantithamthavorn TSE 2017 |
| Scott-Knott ESD ranking | v1.5 | Same |
| RefactoringMiner filtering | v1.5 | Tsantalis TSE 2022 |
| Co-change graph entropy | v1.5 | Ma arXiv:2504.18511 |
| `codehealth-calibrate` (evidence-driven weights) | v1.5 | §4 improvement c |
| Pluggable SZZ | v2 | Tang ASE 2023; LLM4SZZ 2025 |
| Pluggable untangling | v2 | SmartCommit FSE 2021; UTango FSE 2022 |
| LLM-based commit classification | v2 | CommitBERT/CC2Vec literature |
| Semantic coupling (CC2Vec/CodeBERT) | v2 | Wu FSE 2024 |

### 8.5 Phased usability features (per user request)

| Feature | Version | Why phased |
|---|---|---|
| `bca diff <base> <head>` PR-mode command | **v1** | ~50 LOC wrapper; unlocks review-bot use cases day one |
| Pre-commit hook (`bca analyze --quick`) | **v1** | Dogfooding; free marketing if it works |
| `bca doctor` subcommand | **v1** | Diagnoses .mailmap issues, weird changesets, squash-merge workflows |
| `bca query` SQL escape hatch (read-only) | **v1** | Power-user feature, near-zero cost |
| Markdown output for `$GITHUB_STEP_SUMMARY` | **v1** | CI integration table stakes |
| Adaptive complexity sampling | **v1** | Folded in from §4 improvement e |
| `bca codehealth-calibrate` | v1.5 | Needs stable v1 first |
| `bca serve --mcp` MCP server mode | v2 | Tracked in §8.2 |
| `bca watch` (live hotspot updates as commits land) | v3 | Needs Differential Dataflow per §8.3 |

### 8.6 Phased safety features (per user request)

| Feature | Version | Why phased |
|---|---|---|
| Read-only DuckDB connection for `sql`/`query` commands | **v1** | Cheap fix per §5 improvement a |
| `cargo-deny` license/advisory gate in CI | **v1** | Standard 2026 hygiene |
| Differential test vs C git (release blocker) | **v1** | Spadoni 2025 requirement |
| Schema version per row + idempotent migrations | **v1** | Forward-compatibility insurance per §3 improvement e |
| `cargo-fuzz` targets on parser stage | v1.5 | Known footgun: Unicode names, weird paths, mailmap edges |
| `cargo-mutants` mutation testing (nightly) | v1.5 | Emerging 2025-26 standard |
| SLSA Level 3 provenance on releases | **v1** | Supply-chain expectation |
| CodeQL workflow on `bca` repo | **v1** | Security best practice; ~15 minutes to wire |
| Bot filter (`bots.toml` default-deny list) | **v1** | Folded from §1 improvement b |
| Inputs fingerprint in provenance manifest | **v1** | Sets up Salsa retrofit; folded from §3 improvement d |

---

## 9. Open items / decisions still ambiguous

| Item | Decision needed by | Default if not decided |
|---|---|---|
| Final project name (`bca` vs `codetide` vs `mnemo` vs `repomaat` vs other) | Before crates.io publish | `bca` |
| MSRV policy (stable-2 vs stable-4 vs Latest-only) | Phase 0 | stable-2 |
| CI provider (GitHub Actions vs others) | Phase 0 | GitHub Actions |
| Whether to maintain Mozilla RCA upstream-merge cadence or hard-fork | Phase 0 | Maintain merge cadence; revisit if Mozilla cadence breaks down |

---

## 10.5 Architecture references (Stream 3)

**Design analog: InfluxDB 3 / FDAP architecture** (Flight + DataFusion + Arrow + Parquet, [InfluxData FDAP post](https://www.influxdata.com/blog/flight-datafusion-arrow-parquet-fdap-architecture-influxdb/), [InfoQ writeup](https://www.infoq.com/articles/timeseries-db-rust/)). Production-validated (GA April 2025). Lessons: Arrow as in-memory format end-to-end; Parquet as cold storage between runs; modular execution.

**Documented alternative path** (if DuckDB's C++ toolchain becomes a CI burden in v1.x or v2): swap DuckDB for Apache DataFusion. Pure Rust (no C++ toolchain), arrow-rs native, used in production by InfluxDB 3 and Apple Comet. Trade-off: weaker end-user SQL ergonomics than DuckDB. We picked DuckDB for the SQL-surface-as-power-user-feature win, but the abstraction layer (Arrow IPC handoff) makes a future swap cheap if needed.

## 10. References

### Research streams (transcripts available in conversation history)

1. Competitive landscape (CodeScene, hercules, git2net, SonarQube, Greptile, Faros, DX, etc.)
2. Academic state of the art (MSR 2020–2026; Majumder EMSE 2022; Ma 2025; Tantithamthavorn 2017; Spadoni 2025; Tang ASE 2023; Wu FSE 2024)
3. Rust engineering best practices (Polars 1.x, gix vs git2, tree-sitter, dist, insta, tracing)
4. Data engine alternatives (Polars vs DuckDB vs DataFusion vs Arrow vs custom)
5. VCS reader alternatives (gix vs git2 vs git CLI)
6. Architecture patterns + output standards (CodeQL/Glean/SCIP; SARIF 2.1.0; MCP)

### Key academic citations

- Kamei et al. — canonical JIT-SDP 14-feature vector
- Majumder et al. EMSE 2022 — Revisiting process vs product metrics
- Ma et al. arXiv:2504.18511 (2025) — Co-Change Graph Entropy
- Tantithamthavorn et al. TSE 2017 — Model validation techniques
- Spadoni et al. arXiv:2501.15114 (2025) — Does the Tool Matter (provenance)
- Tsantalis et al. TSE 2022 — RefactoringMiner 3
- Tang et al. ASE 2023 — Neural SZZ; LLM4SZZ 2025
- Wu et al. FSE 2024 — CC2Vec semantic co-change
- Hämäläinen arXiv:1405.1360 — Statistical significance of association rules
- Tornhill 2013 — `code-maat` and "Your Code as a Crime Scene"

### Industry standards

- SARIF 2.1.0 (OASIS)
- SLSA Level 3 supply-chain provenance
- Conventional Commits
- DuckDB git-log analysis guide
- Glean (Meta) architecture for fact stores
- Rust edition 2024

---

*End of design specification.*
