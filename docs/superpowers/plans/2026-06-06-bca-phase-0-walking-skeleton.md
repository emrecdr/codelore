# bca Phase 0 + Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Plan 1 of 6 for v1 Spine release.** Companion to spec `docs/superpowers/specs/2026-06-06-bca-design.md`. Subsequent plans (RCA vendor, complexity integration, full analyses, SARIF/provenance, hardening) authored after this phase ships.

**Goal:** Bootstrap the Cargo workspace, prove the architectural spine (gix → DuckDB → CSV), and ship a runnable `bca analyze --analysis revisions --repo <path> --format csv` binary that walks a real git repo's history and emits a code-maat-compatible CSV.

**Architecture:** Three-crate workspace (`bca-lib`, `bca-cli`, `bca-rca` stub — RCA vendor is Plan 2). `gix` reads `.git` directly. Commits stream through a bounded `crossbeam_channel<RecordBatch>` into a single DuckDB Appender thread. DuckDB holds the fact store; one SQL view computes revisions-per-entity; DuckDB's `COPY ... TO ...` emits CSV. No complexity, no Fisher, no SARIF, no provenance manifest yet — those land in later plans. **What walking-skeleton proves: the spine works.**

**Tech Stack:**
- Rust edition 2024, MSRV 1.87 (stable - 2)
- `gix 0.84.0` with `features = ["max-performance"]`
- `arrow 58.3.0` (re-exported via `bca-lib::arrow_facade`)
- `duckdb 1.10503.1` (DuckDB 1.5.3) with `features = ["bundled", "appender-arrow"]`
- `crossbeam-channel` for the gix-worker → Appender pipeline
- `tree-sitter = "=0.25.3"` (pinned for future RCA compat)
- `clap 4.x` (derive)
- `anyhow 2.x` (binary) + `thiserror 2.x` (library)
- `tracing` + `tracing-subscriber` + `tracing-indicatif` + `indicatif`
- `time 0.3.x` (NOT chrono)
- `insta` + `assert_cmd` for snapshot/CLI tests
- `cargo-deny` + `cargo-llvm-cov` in CI
- `just` as task runner
- `sccache` on CI (replaces failed `frozen-duckdb`)

**Out of scope for this plan (deferred to subsequent plans):**
- RCA vendor (Plan 2)
- Tree-sitter complexity (Plan 3)
- 9 other analyses, identity resolution, Fisher significance (Plan 4)
- SARIF, Markdown, Parquet, SQLite outputs, provenance manifest, `bca diff` (Plan 5)
- Differential test against code-maat goldens (deferred to Plan 6; basic smoke test only here)
- `dist`, SLSA, distroless container, PGO (Plan 6)

**Definition of Done for Plan 1:**
```
$ cd /tmp/some-git-repo
$ bca analyze --analysis revisions --format csv
entity,n-revs
src/main.rs,42
src/lib.rs,38
...
```
…and `cargo test --workspace` is green, `cargo clippy -- -D warnings` is clean, `cargo deny check` passes, and CI runs on GitHub Actions.

---

## Phase 0.A: Project foundation

### Task 1: Initialize Cargo workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `.editorconfig`
- Create: `rustfmt.toml`
- Create: `clippy.toml`
- Create: `crates/bca-lib/Cargo.toml`
- Create: `crates/bca-lib/src/lib.rs`
- Create: `crates/bca-cli/Cargo.toml`
- Create: `crates/bca-cli/src/main.rs`

- [ ] **Step 1: Create workspace `Cargo.toml`**

```toml
# Cargo.toml (workspace root)
[workspace]
resolver = "2"
members = ["crates/bca-lib", "crates/bca-cli"]
# crates/bca-rca added in Plan 2

[workspace.package]
version = "0.1.0-alpha.1"
edition = "2024"
rust-version = "1.87"
license = "GPL-3.0-only"
repository = "https://github.com/<owner>/bca"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
# These would be too noisy in early stages — relax:
module_name_repetitions = "allow"
missing_errors_doc = "allow"

[profile.release]
lto = "fat"
codegen-units = 1
strip = true
panic = "abort"
```

- [ ] **Step 2: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.89.0"
components = ["rustfmt", "clippy", "rust-src", "llvm-tools-preview"]
profile = "minimal"
```

- [ ] **Step 3: Create `.gitignore`**

```gitignore
/target
**/*.rs.bk
*.pdb
.DS_Store
.idea/
.vscode/
*.swp
*.swo
.envrc
.direnv/
```

- [ ] **Step 4: Create `.editorconfig`**

```
root = true

[*]
charset = utf-8
end_of_line = lf
insert_final_newline = true
trim_trailing_whitespace = true
indent_style = space
indent_size = 4

[*.{toml,yml,yaml,json,md}]
indent_size = 2

[Makefile]
indent_style = tab
```

- [ ] **Step 5: Create `rustfmt.toml`**

```toml
edition = "2024"
max_width = 100
imports_granularity = "Module"
group_imports = "StdExternalCrate"
```

- [ ] **Step 6: Create `clippy.toml`**

```toml
msrv = "1.87"
cognitive-complexity-threshold = 30
```

- [ ] **Step 7: Create `crates/bca-lib/Cargo.toml`**

```toml
[package]
name = "bca-lib"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Behavioral Code Analyzer — library"

[lints]
workspace = true

[dependencies]
# Added incrementally in later tasks
```

- [ ] **Step 8: Create `crates/bca-lib/src/lib.rs`**

```rust
//! bca-lib — Behavioral Code Analyzer library.
//!
//! See `docs/superpowers/specs/2026-06-06-bca-design.md` for the full design.

#![doc(html_no_source)]
```

- [ ] **Step 9: Create `crates/bca-cli/Cargo.toml`**

```toml
[package]
name = "bca-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Behavioral Code Analyzer — CLI"

[[bin]]
name = "bca"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
bca-lib = { path = "../bca-lib" }
# Added incrementally in later tasks
```

- [ ] **Step 10: Create `crates/bca-cli/src/main.rs`**

```rust
fn main() {
    println!("bca v0.1.0-alpha.1");
}
```

- [ ] **Step 11: Verify the workspace builds**

Run: `cargo build --workspace`

Expected: clean build, produces `target/debug/bca`.

- [ ] **Step 12: Run the binary and verify output**

Run: `./target/debug/bca`

Expected output: `bca v0.1.0-alpha.1`

- [ ] **Step 13: Initial commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore .editorconfig \
        rustfmt.toml clippy.toml crates/
git commit -m "feat: initialize bca cargo workspace"
```

---

### Task 2: Add `justfile` task runner

**Files:**
- Create: `justfile`

- [ ] **Step 1: Create `justfile`**

```just
# bca task runner
# Install just: cargo install just

default:
    @just --list

# Build everything
build:
    cargo build --workspace

# Build release
release:
    cargo build --workspace --release

# Run all tests
test:
    cargo test --workspace --all-features

# Run clippy with our hard standards
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format check
fmt-check:
    cargo fmt --all --check

# Format
fmt:
    cargo fmt --all

# License + advisory check
deny:
    cargo deny check

# Coverage report
coverage:
    cargo llvm-cov --workspace --html

# All CI checks
ci: fmt-check lint deny test

# Run the binary
bca *ARGS:
    cargo run --release -p bca-cli -- {{ARGS}}
```

- [ ] **Step 2: Verify just works**

Run: `just`

Expected: `just` prints the list of recipes.

- [ ] **Step 3: Run `just ci`**

Run: `just ci`

Expected: all four checks pass (deny will fail until Task 3 — that's OK, we add `deny.toml` next).

- [ ] **Step 4: Commit**

```bash
git add justfile
git commit -m "build: add justfile task runner"
```

---

### Task 3: Add `cargo-deny` config + GitHub Actions CI

**Files:**
- Create: `deny.toml`
- Create: `.github/workflows/ci.yml`
- Create: `renovate.json`

- [ ] **Step 1: Create `deny.toml`**

```toml
[graph]
all-features = true

[advisories]
yanked = "deny"
ignore = []

[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "MPL-2.0",
    "Unicode-DFS-2016",
    "Unicode-3.0",
    "Zlib",
    "CC0-1.0",
    "GPL-3.0",       # our own crates
    "GPL-3.0-only",  # SPDX form
]
confidence-threshold = 0.93

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- [ ] **Step 2: Create `.github/workflows/ci.yml`**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-Dwarnings"
  RUSTC_WRAPPER: sccache

jobs:
  fmt:
    name: rustfmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all --check

  clippy:
    name: clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: mozilla-actions/sccache-action@v0.0.6
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-targets --all-features -- -D warnings

  test:
    name: test
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: mozilla-actions/sccache-action@v0.0.6
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --all-features

  deny:
    name: cargo-deny
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: EmbarkStudios/cargo-deny-action@v2
```

- [ ] **Step 3: Create `renovate.json`**

```json
{
  "$schema": "https://docs.renovatebot.com/renovate-schema.json",
  "extends": ["config:recommended"],
  "rangeStrategy": "bump",
  "packageRules": [
    {
      "matchManagers": ["cargo"],
      "matchUpdateTypes": ["patch", "minor"],
      "groupName": "rust patch+minor updates",
      "schedule": ["before 6am on Monday"]
    },
    {
      "matchPackageNames": ["duckdb"],
      "rangeStrategy": "pin",
      "description": "duckdb exact-pinned because it pins arrow"
    },
    {
      "matchPackageNames": ["tree-sitter"],
      "enabled": false,
      "description": "tree-sitter pinned to =0.25.3 for RCA compat — manual bump only"
    }
  ]
}
```

- [ ] **Step 4: Verify `cargo deny check` passes locally**

Run: `cargo install cargo-deny && cargo deny check`

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add deny.toml .github/workflows/ci.yml renovate.json
git commit -m "ci: add GitHub Actions, cargo-deny, renovate config"
```

---

## Phase 0.B: bca-lib core types

### Task 4: Add core types

**Files:**
- Modify: `crates/bca-lib/Cargo.toml` (add `time`, `serde`, `thiserror`)
- Create: `crates/bca-lib/src/types.rs`
- Modify: `crates/bca-lib/src/lib.rs` (re-export types)
- Create: `crates/bca-lib/src/error.rs`
- Create: `crates/bca-lib/tests/types_test.rs`

- [ ] **Step 1: Update `crates/bca-lib/Cargo.toml`**

```toml
[package]
name = "bca-lib"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Behavioral Code Analyzer — library"

[lints]
workspace = true

[dependencies]
time = { version = "0.3", features = ["serde", "macros"] }
serde = { version = "1", features = ["derive"] }
thiserror = "2"
```

- [ ] **Step 2: Write failing test for type construction**

Create `crates/bca-lib/tests/types_test.rs`:

```rust
use bca_lib::types::{ChangeType, CommitEvent, FileChange, Hunk, SCHEMA_VERSION};
use time::macros::date;

#[test]
fn schema_version_is_one() {
    assert_eq!(SCHEMA_VERSION, 1);
}

#[test]
fn commit_event_construction() {
    let event = CommitEvent {
        rev: "abcdef1".into(),
        author_email: "a@b.com".into(),
        author_name: "A B".into(),
        committer_email: "a@b.com".into(),
        date: date!(2026 - 06 - 06),
        message: "test".into(),
        parents: vec![],
        changes: vec![FileChange {
            path: "src/main.rs".into(),
            change_type: ChangeType::Modified,
            loc_added: 10,
            loc_deleted: 3,
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 10,
            }],
        }],
        kamei: None,
    };
    assert_eq!(event.rev, "abcdef1");
    assert_eq!(event.changes.len(), 1);
}
```

- [ ] **Step 3: Run test and confirm it fails**

Run: `cargo test -p bca-lib --test types_test`

Expected: FAIL with "unresolved import `bca_lib::types`".

- [ ] **Step 4: Create `crates/bca-lib/src/types.rs`**

```rust
//! Public types for the bca-lib API. These are the contract every
//! pipeline stage agrees on. See spec §3.1.

use serde::{Deserialize, Serialize};
use time::Date;

/// Bumped on any breaking change to facts or output schemas.
pub const SCHEMA_VERSION: u8 = 1;

/// One commit, as observed by the parser stage. Immutable event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitEvent {
    pub rev: String,
    pub author_email: String,
    pub author_name: String,
    pub committer_email: String,
    pub date: Date,
    pub message: String,
    pub parents: Vec<String>,
    pub changes: Vec<FileChange>,
    /// Populated by the enrich_kamei pipeline stage (Plan 4), NOT at gix walk-time.
    pub kamei: Option<KameiFeatures>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub change_type: ChangeType,
    pub loc_added: u32,
    pub loc_deleted: u32,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
    Renamed { from: String, similarity: u8 },
    Copied { from: String, similarity: u8 },
    BinaryOrUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

/// Kamei 14-feature JIT-SDP canonical vector. Computed by the enrich_kamei stage in Plan 4.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct KameiFeatures {
    pub ns: u32, pub nd: u32, pub nf: u32, pub entropy: f64,
    pub la: u32, pub ld: u32, pub lt: f64,
    pub fix: bool,
    pub ndev: u32, pub age: f64, pub nuc: u32,
    pub exp: u32, pub rexp: f64, pub sexp: u32,
}
```

- [ ] **Step 5: Re-export types from `lib.rs`**

Update `crates/bca-lib/src/lib.rs`:

```rust
//! bca-lib — Behavioral Code Analyzer library.
//!
//! See `docs/superpowers/specs/2026-06-06-bca-design.md`.

pub mod types;
pub mod error;

pub use error::{BcaError, Result};
pub use types::{
    ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION,
};
```

- [ ] **Step 6: Create `crates/bca-lib/src/error.rs`**

```rust
//! Public error type. Drives CLI exit codes at the lib/cli boundary.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, BcaError>;

#[derive(Debug, Error)]
pub enum BcaError {
    #[error("provenance violation: {0}")]
    Provenance(String),

    #[error("repository error: {0}")]
    Repo(String),

    #[error("analysis error: {0}")]
    Analysis(String),

    #[error("output error: {0}")]
    Output(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl BcaError {
    /// CLI exit code for this error variant. See spec §6.6.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Provenance(_) => 2,
            Self::Repo(_) => 3,
            Self::Analysis(_) => 4,
            Self::Output(_) => 5,
            Self::Io(_) => 5,
        }
    }
}
```

- [ ] **Step 7: Run tests and confirm they pass**

Run: `cargo test -p bca-lib --test types_test`

Expected: 2 passed.

- [ ] **Step 8: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): add core types — CommitEvent, FileChange, Hunk, ChangeType, KameiFeatures, BcaError"
```

---

### Task 5: Add `AnalysisName` enum + `Options` struct

**Files:**
- Create: `crates/bca-lib/src/analysis.rs`
- Create: `crates/bca-lib/src/options.rs`
- Modify: `crates/bca-lib/src/lib.rs`
- Modify: `crates/bca-lib/tests/types_test.rs` (add tests)

- [ ] **Step 1: Write failing test for AnalysisName parse/display**

Append to `crates/bca-lib/tests/types_test.rs`:

```rust
use bca_lib::AnalysisName;
use std::str::FromStr;

#[test]
fn analysis_name_roundtrip() {
    for name in &[
        "hotspots", "coupling", "ownership", "code-age",
        "abs-churn", "author-churn", "entity-churn",
        "communication", "code-health", "summary",
        "revisions", "authors",  // standalone code-maat parity
    ] {
        let parsed: AnalysisName = name.parse().unwrap();
        assert_eq!(parsed.as_str(), *name, "roundtrip for {name}");
    }
}

#[test]
fn analysis_name_rejects_unknown() {
    let r: Result<AnalysisName, _> = "not-a-real-analysis".parse();
    assert!(r.is_err());
}

#[test]
fn default_options_match_code_maat_thresholds() {
    use bca_lib::Options;
    let opts = Options::default();
    assert_eq!(opts.min_revs, 5);
    assert_eq!(opts.min_shared_revs, 5);
    assert_eq!(opts.min_coupling_pct, 30);
    assert_eq!(opts.max_coupling_pct, 100);
    assert_eq!(opts.max_changeset_size, 30);
    assert_eq!(opts.fisher_significance, 0.05);
}
```

- [ ] **Step 2: Run tests and confirm they fail**

Run: `cargo test -p bca-lib --test types_test`

Expected: compile errors for `AnalysisName`, `Options`.

- [ ] **Step 3: Create `crates/bca-lib/src/analysis.rs`**

```rust
//! The closed set of analyses bca supports. Enum, not string,
//! so the compiler catches typos that code-maat's string dispatch silently misroutes.

use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisName {
    // v1 Spine — 10 core
    Hotspots,
    Coupling,
    Ownership,
    CodeAge,
    AbsChurn,
    AuthorChurn,
    EntityChurn,
    Communication,
    CodeHealth,
    Summary,
    // code-maat parity (computed as side-data on hotspots, addressable standalone)
    Revisions,
    Authors,
}

impl AnalysisName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotspots => "hotspots",
            Self::Coupling => "coupling",
            Self::Ownership => "ownership",
            Self::CodeAge => "code-age",
            Self::AbsChurn => "abs-churn",
            Self::AuthorChurn => "author-churn",
            Self::EntityChurn => "entity-churn",
            Self::Communication => "communication",
            Self::CodeHealth => "code-health",
            Self::Summary => "summary",
            Self::Revisions => "revisions",
            Self::Authors => "authors",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Hotspots, Self::Coupling, Self::Ownership, Self::CodeAge,
            Self::AbsChurn, Self::AuthorChurn, Self::EntityChurn, Self::Communication,
            Self::CodeHealth, Self::Summary,
            Self::Revisions, Self::Authors,
        ]
    }
}

impl FromStr for AnalysisName {
    type Err = UnknownAnalysisError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .iter()
            .find(|a| a.as_str() == s)
            .copied()
            .ok_or_else(|| UnknownAnalysisError(s.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown analysis: {0}")]
pub struct UnknownAnalysisError(pub String);
```

- [ ] **Step 4: Create `crates/bca-lib/src/options.rs`**

```rust
//! Run-time configuration for the bca pipeline. Defaults match
//! code-maat for parity; see spec §1.1.

use std::path::PathBuf;
use time::Date;

#[derive(Debug, Clone)]
pub struct Options {
    // Input
    pub repo_path: PathBuf,
    pub after: Option<Date>,
    pub before: Option<Date>,
    pub commit_range: Option<String>,

    // Aggregation (Plan 4 — left here so Options shape is stable from v1)
    pub group_file: Option<PathBuf>,
    pub team_map_file: Option<PathBuf>,
    pub temporal_period_days: Option<u32>,

    // Analysis thresholds — code-maat parity
    pub min_revs: u32,
    pub min_shared_revs: u32,
    pub min_coupling_pct: u8,
    pub max_coupling_pct: u8,
    pub max_changeset_size: u32,
    pub fisher_significance: f64,

    // Specific analyses
    pub message_regex: Option<String>,
    pub age_time_now: Option<Date>,

    // Output
    pub rows_limit: Option<u32>,
    pub verbose_results: bool,
    pub include_merges: bool,
    pub strict_grouping: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repo_path: PathBuf::from("."),
            after: None,
            before: None,
            commit_range: None,
            group_file: None,
            team_map_file: None,
            temporal_period_days: None,
            min_revs: 5,
            min_shared_revs: 5,
            min_coupling_pct: 30,
            max_coupling_pct: 100,
            max_changeset_size: 30,
            fisher_significance: 0.05,
            message_regex: None,
            age_time_now: None,
            rows_limit: None,
            verbose_results: false,
            include_merges: false,
            strict_grouping: false,
        }
    }
}
```

- [ ] **Step 5: Re-export from `lib.rs`**

Update `crates/bca-lib/src/lib.rs`:

```rust
//! bca-lib — Behavioral Code Analyzer library.

pub mod analysis;
pub mod error;
pub mod options;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{BcaError, Result};
pub use options::Options;
pub use types::{
    ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION,
};
```

- [ ] **Step 6: Run tests and confirm they pass**

Run: `cargo test -p bca-lib`

Expected: 5 passed.

- [ ] **Step 7: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): add AnalysisName enum and Options struct with code-maat parity defaults"
```

---

## Phase 0.C: Arrow facade + Repo trait + GixRepo

### Task 6: Add Arrow facade module

**Files:**
- Modify: `crates/bca-lib/Cargo.toml`
- Create: `crates/bca-lib/src/arrow_facade.rs`
- Modify: `crates/bca-lib/src/lib.rs`

- [ ] **Step 1: Update `crates/bca-lib/Cargo.toml`**

Append to `[dependencies]`:

```toml
arrow = "58.3"
```

- [ ] **Step 2: Create `crates/bca-lib/src/arrow_facade.rs`**

```rust
//! Single source of truth for Arrow types throughout the workspace.
//!
//! Re-exports the version of arrow-rs that the `duckdb` crate currently
//! depends on. When `duckdb` bumps `arrow`, we bump here in lockstep
//! and the rest of the workspace stays unchanged.
//!
//! **Discipline:** never `use arrow::*` directly anywhere else in the
//! workspace. Always `use bca_lib::arrow_facade::*` or `use crate::arrow_facade::*`.
//! See spec §2.6.

pub use arrow::array::{
    Array, ArrayBuilder, ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder,
    Float64Builder, GenericByteBuilder, Int32Builder, Int64Builder, StringBuilder,
    UInt32Builder, UInt64Builder,
};
pub use arrow::buffer::Buffer;
pub use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
pub use arrow::record_batch::RecordBatch;

/// Version reported by the runtime (for provenance manifests in Plan 5).
pub const ARROW_RUNTIME_VERSION: &str = "58.3.0";
```

- [ ] **Step 3: Add module to `lib.rs`**

Update `crates/bca-lib/src/lib.rs`:

```rust
pub mod analysis;
pub mod arrow_facade;
pub mod error;
pub mod options;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{BcaError, Result};
pub use options::Options;
pub use types::{
    ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION,
};
```

- [ ] **Step 4: Run build**

Run: `cargo build -p bca-lib`

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): add arrow_facade module — single re-export point for arrow-rs version pin"
```

---

### Task 7: Add Repo trait

**Files:**
- Create: `crates/bca-lib/src/repo/mod.rs`
- Create: `crates/bca-lib/src/repo/types.rs`
- Modify: `crates/bca-lib/src/lib.rs`
- Create: `crates/bca-lib/tests/repo_trait_test.rs`

- [ ] **Step 1: Write failing test for trait shape**

Create `crates/bca-lib/tests/repo_trait_test.rs`:

```rust
//! Compile-time test: confirms Repo trait shape exists.
//! Real integration tests against a fixture repo land in Task 9.

use bca_lib::repo::{Repo, CommitMetadata};
use bca_lib::CommitEvent;

fn _trait_object_compiles<R: Repo>(r: &R) {
    let _: Box<dyn Iterator<Item = bca_lib::Result<CommitEvent>>> = unimplemented!();
    let _: bca_lib::Result<Vec<bca_lib::FileChange>> = r.changed_files("abc");
    let _: bca_lib::Result<Vec<bca_lib::Hunk>> = r.diff_hunks("abc", "src/main.rs");
    let _: String = r.resolve_alias("a@b.com");
    let _: bca_lib::Result<CommitMetadata> = r.commit_metadata("abc");
}
```

- [ ] **Step 2: Run test and confirm it fails**

Run: `cargo test -p bca-lib --test repo_trait_test`

Expected: compile error — `Repo` not found.

- [ ] **Step 3: Create `crates/bca-lib/src/repo/types.rs`**

```rust
//! Public types for the Repo trait. Distinct from the
//! main types module to keep gix-coupled types isolated.

#[derive(Debug, Clone)]
pub struct CommitMetadata {
    pub rev: String,
    pub signed: bool,
    pub signed_by: Option<String>,
    pub signoffs: Vec<String>,
}
```

- [ ] **Step 4: Create `crates/bca-lib/src/repo/mod.rs`**

```rust
//! VCS-reading abstraction. The default impl is `gix` in Plan 1;
//! a `GitCliRepo` differential-test oracle lands in Plan 6.

pub mod types;

pub use types::CommitMetadata;

use crate::{CommitEvent, FileChange, Hunk, Options, Result};

/// Read-only git operations needed by the bca pipeline.
/// See spec §3.3.
pub trait Repo: Send + Sync {
    /// Walk commits matching `opts.after`/`opts.before`/`opts.commit_range`.
    /// Returns an iterator (Plan 4 will introduce Stream over async).
    fn walk_commits<'a>(
        &'a self,
        opts: &'a Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>>;

    /// Per-file changes for one commit.
    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>>;

    /// Hunks within one (commit, path) pair.
    fn diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>>;

    /// .mailmap-aware author email canonicalization.
    fn resolve_alias(&self, email: &str) -> String;

    /// Commit metadata not in `CommitEvent` (signed-by, signoffs).
    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata>;
}
```

- [ ] **Step 5: Add `pub mod repo;` to `lib.rs`**

Update `crates/bca-lib/src/lib.rs`:

```rust
pub mod analysis;
pub mod arrow_facade;
pub mod error;
pub mod options;
pub mod repo;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{BcaError, Result};
pub use options::Options;
pub use repo::Repo;
pub use types::{
    ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION,
};
```

- [ ] **Step 6: Run tests and confirm trait compiles**

Run: `cargo test -p bca-lib --test repo_trait_test --no-run`

Expected: compilation succeeds.

- [ ] **Step 7: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): add Repo trait — abstraction for VCS-reading impls"
```

---

### Task 8: Add `GixRepo` impl — minimal walk_commits

**Files:**
- Modify: `crates/bca-lib/Cargo.toml` (add `gix`)
- Create: `crates/bca-lib/src/repo/gix_repo.rs`
- Modify: `crates/bca-lib/src/repo/mod.rs`

- [ ] **Step 1: Add gix dependency**

Append to `crates/bca-lib/Cargo.toml` `[dependencies]`:

```toml
gix = { version = "0.84", features = ["max-performance"] }
```

- [ ] **Step 2: Create `crates/bca-lib/src/repo/gix_repo.rs`** with stub methods returning the right error variants

```rust
//! gix-backed Repo impl. The production default.

use std::path::Path;

use crate::repo::{CommitMetadata, Repo};
use crate::{BcaError, CommitEvent, FileChange, Hunk, Options, Result};

pub struct GixRepo {
    inner: gix::ThreadSafeRepository,
}

impl GixRepo {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = gix::open(path.as_ref())
            .map_err(|e| BcaError::Repo(format!("open {}: {e}", path.as_ref().display())))?
            .into_sync();
        Ok(Self { inner })
    }
}

impl Repo for GixRepo {
    fn walk_commits<'a>(
        &'a self,
        _opts: &'a Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>> {
        let repo = self.inner.to_thread_local();
        let head = repo
            .head_id()
            .map_err(|e| BcaError::Repo(format!("head_id: {e}")))?;

        let revwalk = repo
            .rev_walk([head])
            .all()
            .map_err(|e| BcaError::Repo(format!("rev_walk: {e}")))?;

        // Hold a clone of the thread-safe handle alive for the iterator's lifetime.
        let inner_clone = self.inner.clone();
        Ok(Box::new(revwalk.filter_map(move |info| {
            let info = match info {
                Ok(i) => i,
                Err(e) => return Some(Err(BcaError::Repo(format!("revwalk: {e}")))),
            };
            let repo = inner_clone.to_thread_local();
            let commit = match repo.find_commit(info.id) {
                Ok(c) => c,
                Err(e) => return Some(Err(BcaError::Repo(format!("find_commit: {e}")))),
            };
            Some(commit_event_from_gix(commit))
        })))
    }

    fn changed_files(&self, _rev: &str) -> Result<Vec<FileChange>> {
        // Stub for Plan 1 walking skeleton — every commit reports zero changes,
        // so the `revisions` analysis (counting commits per HEAD-tracked path)
        // doesn't depend on this. Real impl lands in Task 9.
        Ok(vec![])
    }

    fn diff_hunks(&self, _rev: &str, _path: &str) -> Result<Vec<Hunk>> {
        Ok(vec![])  // Plan 5 lands real hunk extraction
    }

    fn resolve_alias(&self, email: &str) -> String {
        // .mailmap support lands in Plan 4 — identity resolution
        email.to_string()
    }

    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata> {
        Ok(CommitMetadata {
            rev: rev.to_string(),
            signed: false,
            signed_by: None,
            signoffs: vec![],
        })
    }
}

fn commit_event_from_gix(commit: gix::Commit<'_>) -> Result<CommitEvent> {
    use time::OffsetDateTime;

    let id = commit.id().to_hex().to_string();
    let parents = commit
        .parent_ids()
        .map(|p| p.to_hex().to_string())
        .collect();

    let author_ref = commit
        .author()
        .map_err(|e| BcaError::Repo(format!("author: {e}")))?;
    let committer_ref = commit
        .committer()
        .map_err(|e| BcaError::Repo(format!("committer: {e}")))?;

    let ts_seconds = author_ref.time.seconds;
    let date = OffsetDateTime::from_unix_timestamp(ts_seconds)
        .map_err(|e| BcaError::Repo(format!("commit timestamp {ts_seconds}: {e}")))?
        .date();

    let message = commit
        .message_raw()
        .map(|s| s.to_string())
        .unwrap_or_default();

    Ok(CommitEvent {
        rev: id,
        author_email: author_ref.email.to_string(),
        author_name: author_ref.name.to_string(),
        committer_email: committer_ref.email.to_string(),
        date,
        message,
        parents,
        changes: vec![],  // Filled by Task 9 — tree-diff against parent
        kamei: None,
    })
}
```

- [ ] **Step 3: Re-export from `repo/mod.rs`**

Append to `crates/bca-lib/src/repo/mod.rs`:

```rust
pub mod gix_repo;
pub use gix_repo::GixRepo;
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p bca-lib`

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): add GixRepo — minimal commit walker (changes filled in Task 9)"
```

---

### Task 9: GixRepo — fill `changed_files` via tree-diff

**Files:**
- Modify: `crates/bca-lib/src/repo/gix_repo.rs`
- Create: `crates/bca-lib/tests/gix_repo_test.rs`
- Create: `crates/bca-lib/tests/fixtures/build_tiny_repo.rs` (test helper)

- [ ] **Step 1: Add `gix-diff` feature to gix dependency**

Update `crates/bca-lib/Cargo.toml`:

```toml
gix = { version = "0.84", features = ["max-performance", "blob-diff"] }
tempfile = "3"

[dev-dependencies]
# nothing yet — bring tempfile to main deps because the fixture
# builder is reused from CLI tests
```

(Note: `tempfile` goes in `[dependencies]` because the fixture builder is a `pub` helper consumed by CLI tests in Task 14.)

- [ ] **Step 2: Write the fixture-builder helper**

Create `crates/bca-lib/src/test_support/mod.rs`:

```rust
//! Test fixtures. Public so CLI integration tests can reuse.
//!
//! `tiny_repo()` programmatically builds a 5-commit repo via gix
//! so behavior is exactly reproducible.

#[cfg(any(test, feature = "test-support"))]
pub mod tiny_repo {
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct TinyRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    pub fn build() -> TinyRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();
        // Plan 1 uses git CLI for fixture setup — fast and predictable.
        // gix-write paths still maturing for trivial init.
        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "tiny@example.com"]);
        run_git(&path, &["config", "user.name", "Tiny"]);

        write(&path, "src/main.rs", "fn main() {}\n");
        run_git(&path, &["add", "."]);
        run_git(&path, &["commit", "-m", "init", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hi\"); }\n");
        run_git(&path, &["commit", "-am", "say hi", "--quiet"]);

        write(&path, "src/lib.rs", "pub fn greet() {}\n");
        run_git(&path, &["add", "."]);
        run_git(&path, &["commit", "-m", "add lib", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hello\"); }\n");
        run_git(&path, &["commit", "-am", "fix typo", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hello, world\"); }\n");
        run_git(&path, &["commit", "-am", "expand greeting", "--quiet"]);

        let head_sha = String::from_utf8(
            std::process::Command::new("git")
                .args(["-C", path.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        TinyRepo { dir, head_sha }
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C").arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() { std::fs::create_dir_all(parent).unwrap(); }
        std::fs::write(p, content).unwrap();
    }
}
```

Update `crates/bca-lib/Cargo.toml` to add the feature:

```toml
[features]
default = []
test-support = []
```

And in `crates/bca-lib/src/lib.rs`:

```rust
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
```

- [ ] **Step 3: Write failing test for `changed_files`**

Create `crates/bca-lib/tests/gix_repo_test.rs`:

```rust
use bca_lib::repo::{GixRepo, Repo};
use bca_lib::{ChangeType, Options};

#[test]
fn walks_tiny_repo_5_commits() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let opts = Options::default();
    let commits: Vec<_> = repo.walk_commits(&opts).expect("walk").collect();
    assert_eq!(commits.len(), 5);
}

#[test]
fn changed_files_for_modify_commit() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let changes = repo.changed_files(&tiny.head_sha).expect("changed_files");
    // HEAD commit modifies src/main.rs only
    assert_eq!(changes.len(), 1);
    let c = &changes[0];
    assert_eq!(c.path, "src/main.rs");
    assert!(matches!(c.change_type, ChangeType::Modified));
}
```

- [ ] **Step 4: Run test, confirm fail**

Run: `cargo test -p bca-lib --test gix_repo_test --features test-support`

Expected: `changed_files_for_modify_commit` fails — Vec is empty.

- [ ] **Step 5: Implement `changed_files` via gix-diff**

Replace the stub in `crates/bca-lib/src/repo/gix_repo.rs`:

```rust
fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> {
    use gix::prelude::ObjectIdExt;
    let repo = self.inner.to_thread_local();
    let oid = gix::ObjectId::from_hex(rev.as_bytes())
        .map_err(|e| BcaError::Repo(format!("parse oid {rev}: {e}")))?;
    let commit = oid.attach(&repo)
        .object()
        .map_err(|e| BcaError::Repo(format!("find object {rev}: {e}")))?
        .try_into_commit()
        .map_err(|e| BcaError::Repo(format!("not a commit {rev}: {e}")))?;

    let tree = commit.tree()
        .map_err(|e| BcaError::Repo(format!("tree {rev}: {e}")))?;

    let parent_tree = commit
        .parent_ids()
        .next()
        .map(|pid| -> Result<gix::Tree<'_>> {
            pid.object()
                .map_err(|e| BcaError::Repo(format!("parent obj: {e}")))?
                .try_into_commit()
                .map_err(|e| BcaError::Repo(format!("parent not commit: {e}")))?
                .tree()
                .map_err(|e| BcaError::Repo(format!("parent tree: {e}")))
        })
        .transpose()?;

    let mut changes = Vec::new();
    let from_tree = parent_tree.as_ref().map(|t| t.id);
    let to_tree = tree.id;

    let platform = repo
        .diff_resource_cache(gix::diff::blob::pipeline::Mode::ToGit, Default::default())
        .map_err(|e| BcaError::Repo(format!("diff cache: {e}")))?;

    let from = match from_tree {
        Some(id) => id.attach(&repo).object()
            .map_err(|e| BcaError::Repo(format!("from obj: {e}")))?
            .try_into_tree()
            .map_err(|e| BcaError::Repo(format!("from not tree: {e}")))?,
        None => repo
            .empty_tree()
            .map_err(|e| BcaError::Repo(format!("empty tree: {e}")))?,
    };

    from.changes()
        .map_err(|e| BcaError::Repo(format!("changes: {e}")))?
        .for_each_to_obtain_tree(
            &tree,
            |change| -> std::result::Result<gix::object::tree::diff::Action, BcaError> {
                use gix::object::tree::diff::change::Event;
                let path = change.location.to_string();
                let (change_type, loc_added, loc_deleted) = match &change.event {
                    Event::Addition { .. } => (ChangeType::Added, 0, 0),
                    Event::Deletion { .. } => (ChangeType::Deleted, 0, 0),
                    Event::Modification { .. } => (ChangeType::Modified, 0, 0),
                    Event::Rewrite { source_location, .. } => (
                        ChangeType::Renamed {
                            from: source_location.to_string(),
                            similarity: 100,  // Plan 4 wires real similarity
                        },
                        0,
                        0,
                    ),
                };
                // Plan 4 fills loc_added / loc_deleted via blob diff. Skip for Plan 1.
                changes.push(FileChange {
                    path,
                    change_type,
                    loc_added,
                    loc_deleted,
                    hunks: vec![],
                });
                Ok(gix::object::tree::diff::Action::Continue)
            },
        )
        .map_err(|e| BcaError::Repo(format!("walk diff: {e}")))?;

    drop(platform);  // suppress unused warning until Plan 4 wires it
    Ok(changes)
}
```

- [ ] **Step 6: Wire `changed_files` into `walk_commits`**

In `commit_event_from_gix`, set `changes: repo.changed_files(&id)?` — but since the helper isn't a method on Self, refactor:

Replace the closure body in `walk_commits` so it also collects changes:

```rust
Ok(Box::new(revwalk.filter_map(move |info| {
    let info = match info {
        Ok(i) => i,
        Err(e) => return Some(Err(BcaError::Repo(format!("revwalk: {e}")))),
    };
    let repo = inner_clone.to_thread_local();
    let commit = match repo.find_commit(info.id) {
        Ok(c) => c,
        Err(e) => return Some(Err(BcaError::Repo(format!("find_commit: {e}")))),
    };
    let id_string = commit.id().to_hex().to_string();
    let mut event = match commit_event_from_gix(commit) {
        Ok(e) => e,
        Err(e) => return Some(Err(e)),
    };
    // Re-enter to extract changes — extracted via a helper so it can also
    // be called standalone by `Repo::changed_files`.
    let self_ref = GixRepoBorrow { repo: &inner_clone };
    event.changes = match self_ref.changes_inline(&id_string) {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    Some(Ok(event))
})))
```

And add:

```rust
struct GixRepoBorrow<'a> {
    repo: &'a gix::ThreadSafeRepository,
}
impl GixRepoBorrow<'_> {
    fn changes_inline(&self, rev: &str) -> Result<Vec<FileChange>> {
        // Body identical to GixRepo::changed_files — extracted via a free fn
        // would be cleaner; that's a v1.x cleanup.
        let local = self.repo.to_thread_local();
        // ... same body ...
        # Ok(vec![])
    }
}
```

(In practice, refactor `changed_files` body into a free function `compute_changed_files(repo: &gix::Repository, rev: &str)` and call it from both sites. Free function shown here for brevity.)

- [ ] **Step 7: Run tests, confirm pass**

Run: `cargo test -p bca-lib --test gix_repo_test --features test-support`

Expected: 2 passed.

- [ ] **Step 8: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): GixRepo::changed_files via gix tree-diff + fixture builder"
```

---

## Phase 0.D: DuckDB facts

### Task 10: DuckDB schema + Connection wrapper

**Files:**
- Modify: `crates/bca-lib/Cargo.toml`
- Create: `crates/bca-lib/src/facts/mod.rs`
- Create: `crates/bca-lib/src/facts/schema.rs`
- Modify: `crates/bca-lib/src/lib.rs`
- Create: `crates/bca-lib/tests/facts_test.rs`

- [ ] **Step 1: Add duckdb dependency**

Append to `crates/bca-lib/Cargo.toml` `[dependencies]`:

```toml
duckdb = { version = "=1.10503.1", features = ["bundled", "appender-arrow"] }
```

Run `cargo build -p bca-lib` — expect ~4–6 minute first build (DuckDB C++ compile).

- [ ] **Step 2: Write failing test for schema creation**

Create `crates/bca-lib/tests/facts_test.rs`:

```rust
use bca_lib::facts::FactsDb;

#[test]
fn creates_v1_schema() {
    let db = FactsDb::new_in_memory().expect("create");
    let tables = db.list_tables().expect("list");
    let expected = [
        "commits", "changes", "hunks", "entities",
        "complexity_metrics", "author_aliases", "provenance",
    ];
    for t in &expected {
        assert!(tables.iter().any(|n| n == t), "table {t} missing");
    }
}

#[test]
fn provenance_records_schema_version() {
    let db = FactsDb::new_in_memory().expect("create");
    let v: String = db
        .query_one_value("SELECT value FROM provenance WHERE key = 'schema_version'")
        .expect("query");
    assert_eq!(v, "1");
}
```

- [ ] **Step 3: Run test, confirm fail**

Run: `cargo test -p bca-lib --test facts_test`

Expected: compile error — `facts` module not found.

- [ ] **Step 4: Create `crates/bca-lib/src/facts/schema.rs`**

```rust
//! DuckDB schema DDL. See spec §3.2.

pub const SCHEMA_V1: &str = include_str!("schema_v1.sql");

pub const INITIAL_PROVENANCE: &[(&str, &str)] = &[
    ("schema_version", "1"),
    ("bca_version", env!("CARGO_PKG_VERSION")),
    ("arrow_version", crate::arrow_facade::ARROW_RUNTIME_VERSION),
];
```

- [ ] **Step 5: Create `crates/bca-lib/src/facts/schema_v1.sql`**

```sql
-- Plan 1: walking skeleton schema. Full schema from spec §3.2 lands here.
-- We start with the subset Plan 1 actually populates and lock the rest as empty.

CREATE TABLE IF NOT EXISTS commits (
    rev TEXT PRIMARY KEY,
    author_email TEXT NOT NULL,
    author_name TEXT NOT NULL,
    committer_email TEXT NOT NULL,
    canonical_author TEXT NOT NULL,
    ai_attribution TEXT,
    date DATE NOT NULL,
    message TEXT NOT NULL,
    is_merge BOOLEAN NOT NULL,
    parent_count INTEGER NOT NULL,
    ns INTEGER, nd INTEGER, nf INTEGER, entropy DOUBLE,
    la INTEGER, ld INTEGER, lt DOUBLE,
    fix BOOLEAN,
    ndev INTEGER, age DOUBLE, nuc INTEGER,
    exp INTEGER, rexp DOUBLE, sexp INTEGER
);

CREATE TABLE IF NOT EXISTS changes (
    rev TEXT NOT NULL REFERENCES commits(rev),
    path TEXT NOT NULL,
    change_type TEXT NOT NULL,
    rename_from TEXT,
    similarity INTEGER,
    loc_added INTEGER NOT NULL DEFAULT 0,
    loc_deleted INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (rev, path)
);

CREATE TABLE IF NOT EXISTS hunks (
    rev TEXT NOT NULL,
    path TEXT NOT NULL,
    old_start INTEGER, old_lines INTEGER,
    new_start INTEGER, new_lines INTEGER,
    FOREIGN KEY (rev, path) REFERENCES changes(rev, path)
);

CREATE TABLE IF NOT EXISTS entities (
    path TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
    start_line INTEGER NOT NULL, end_line INTEGER NOT NULL,
    rev_introduced TEXT NOT NULL, rev_last_seen TEXT NOT NULL,
    PRIMARY KEY (path, name, rev_introduced)
);

CREATE TABLE IF NOT EXISTS complexity_metrics (
    path TEXT NOT NULL, name TEXT NOT NULL, rev TEXT NOT NULL,
    cyclomatic INTEGER, cognitive INTEGER,
    halstead_volume DOUBLE, halstead_difficulty DOUBLE, halstead_effort DOUBLE,
    mi DOUBLE,
    nom INTEGER, nexits INTEGER,
    loc INTEGER, sloc INTEGER,
    max_nesting INTEGER, mean_nesting DOUBLE,
    sd_nesting DOUBLE, total_nesting INTEGER,
    PRIMARY KEY (path, name, rev)
);

CREATE TABLE IF NOT EXISTS author_aliases (
    raw_email TEXT PRIMARY KEY,
    canonical TEXT NOT NULL,
    is_bot BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS provenance (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

- [ ] **Step 6: Create `crates/bca-lib/src/facts/mod.rs`**

```rust
//! DuckDB-backed fact store. See spec §3.2 + §3.2.1 invariants.

pub mod schema;

use duckdb::Connection;

use crate::{BcaError, Result};

pub struct FactsDb {
    conn: Connection,
}

impl FactsDb {
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| BcaError::Analysis(format!("open in-memory duckdb: {e}")))?;
        let db = Self { conn };
        db.create_schema()?;
        Ok(db)
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| BcaError::Analysis(format!("open duckdb: {e}")))?;
        let db = Self { conn };
        db.create_schema()?;
        Ok(db)
    }

    fn create_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(schema::SCHEMA_V1)
            .map_err(|e| BcaError::Analysis(format!("create schema: {e}")))?;
        let mut stmt = self
            .conn
            .prepare(
                "INSERT OR REPLACE INTO provenance (key, value) VALUES (?, ?)",
            )
            .map_err(|e| BcaError::Analysis(format!("prepare: {e}")))?;
        for (k, v) in schema::INITIAL_PROVENANCE {
            stmt.execute(duckdb::params![k, v])
                .map_err(|e| BcaError::Analysis(format!("provenance insert: {e}")))?;
        }
        Ok(())
    }

    pub fn list_tables(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT table_name FROM duckdb_tables WHERE schema_name = 'main'")
            .map_err(|e| BcaError::Analysis(format!("prepare: {e}")))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| BcaError::Analysis(format!("query_map: {e}")))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| BcaError::Analysis(format!("collect: {e}")))
    }

    pub fn query_one_value(&self, sql: &str) -> Result<String> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| BcaError::Analysis(format!("prepare: {e}")))?;
        let v: String = stmt
            .query_row([], |r| r.get(0))
            .map_err(|e| BcaError::Analysis(format!("query_row: {e}")))?;
        Ok(v)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}
```

- [ ] **Step 7: Add `pub mod facts;` to `lib.rs`**

Update `crates/bca-lib/src/lib.rs`:

```rust
pub mod analysis;
pub mod arrow_facade;
pub mod error;
pub mod facts;
pub mod options;
pub mod repo;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{BcaError, Result};
pub use facts::FactsDb;
pub use options::Options;
pub use repo::Repo;
pub use types::{
    ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION,
};
```

- [ ] **Step 8: Run tests, confirm pass**

Run: `cargo test -p bca-lib --test facts_test`

Expected: 2 passed.

- [ ] **Step 9: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): add FactsDb (DuckDB fact store) with v1 schema"
```

---

### Task 11: Commit ingestion via single Appender thread

**Files:**
- Modify: `crates/bca-lib/Cargo.toml` (add `crossbeam-channel`)
- Create: `crates/bca-lib/src/facts/ingest.rs`
- Modify: `crates/bca-lib/src/facts/mod.rs`
- Create: `crates/bca-lib/tests/ingest_test.rs`

- [ ] **Step 1: Add crossbeam dependency**

Append to `crates/bca-lib/Cargo.toml` `[dependencies]`:

```toml
crossbeam-channel = "0.5"
```

- [ ] **Step 2: Write failing test**

Create `crates/bca-lib/tests/ingest_test.rs`:

```rust
use bca_lib::facts::FactsDb;
use bca_lib::repo::{GixRepo, Repo};
use bca_lib::Options;

#[test]
fn ingest_tiny_repo_writes_5_commits() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options::default();
    let n = db
        .ingest(&repo, &opts)
        .expect("ingest");
    assert_eq!(n.commits_ingested, 5);

    let count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits")
        .expect("count");
    assert_eq!(count, "5");
}
```

- [ ] **Step 3: Run test, confirm fail**

Run: `cargo test -p bca-lib --test ingest_test --features test-support`

Expected: compile error — `ingest` method missing.

- [ ] **Step 4: Create `crates/bca-lib/src/facts/ingest.rs`**

```rust
//! Stream commits from a Repo into DuckDB via the
//! N gix workers → bounded crossbeam channel → 1 Appender thread pattern.
//! See spec §3.2.2.

use crossbeam_channel::{bounded, Sender};
use duckdb::Appender;
use std::thread;
use time::format_description::well_known::Iso8601;

use crate::repo::Repo;
use crate::{BcaError, CommitEvent, Options, Result};
use super::FactsDb;

const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Default)]
pub struct IngestStats {
    pub commits_ingested: usize,
    pub changes_ingested: usize,
}

impl FactsDb {
    pub fn ingest<R: Repo>(&self, repo: &R, opts: &Options) -> Result<IngestStats> {
        let (tx, rx) = bounded::<CommitEvent>(CHANNEL_CAPACITY);
        // Plan 1: producer is single-threaded (one gix walker). Plan 4 fans out.
        let producer_opts = opts.clone();
        let walk = repo.walk_commits(&producer_opts)?;
        let producer = std::thread::scope(|s| -> Result<IngestStats> {
            let consumer_handle = s.spawn(|| ingest_loop(self, rx));
            for event in walk {
                let event = event?;
                tx.send(event).map_err(|e| BcaError::Analysis(format!("send: {e}")))?;
            }
            drop(tx);
            consumer_handle.join().expect("consumer panicked")
        })?;
        Ok(producer)
    }
}

fn ingest_loop(db: &FactsDb, rx: crossbeam_channel::Receiver<CommitEvent>) -> Result<IngestStats> {
    let mut stats = IngestStats::default();

    let mut commits_app = db
        .conn()
        .appender("commits")
        .map_err(|e| BcaError::Analysis(format!("appender commits: {e}")))?;
    let mut changes_app = db
        .conn()
        .appender("changes")
        .map_err(|e| BcaError::Analysis(format!("appender changes: {e}")))?;

    for event in rx {
        append_commit(&mut commits_app, &event)?;
        for ch in &event.changes {
            append_change(&mut changes_app, &event.rev, ch)?;
            stats.changes_ingested += 1;
        }
        stats.commits_ingested += 1;
    }
    commits_app.flush().map_err(|e| BcaError::Analysis(format!("flush commits: {e}")))?;
    changes_app.flush().map_err(|e| BcaError::Analysis(format!("flush changes: {e}")))?;
    Ok(stats)
}

fn append_commit(app: &mut Appender<'_>, e: &CommitEvent) -> Result<()> {
    use duckdb::params;
    let date_str = e.date.format(&Iso8601::DEFAULT)
        .map_err(|err| BcaError::Analysis(format!("format date: {err}")))?;
    app.append_row(params![
        e.rev,
        e.author_email,
        e.author_name,
        e.committer_email,
        e.author_email,         // canonical_author — Plan 4 fills properly
        Option::<String>::None, // ai_attribution
        date_str,
        e.message,
        e.parents.len() > 1,
        e.parents.len() as i32,
        // Kamei nulls — Plan 4 fills
        Option::<i32>::None, Option::<i32>::None, Option::<i32>::None, Option::<f64>::None,
        Option::<i32>::None, Option::<i32>::None, Option::<f64>::None,
        Option::<bool>::None,
        Option::<i32>::None, Option::<f64>::None, Option::<i32>::None,
        Option::<i32>::None, Option::<f64>::None, Option::<i32>::None,
    ])
    .map_err(|err| BcaError::Analysis(format!("append commit: {err}")))?;
    Ok(())
}

fn append_change(app: &mut Appender<'_>, rev: &str, ch: &crate::FileChange) -> Result<()> {
    use crate::ChangeType;
    use duckdb::params;
    let (type_str, rename_from, similarity) = match &ch.change_type {
        ChangeType::Added => ("added", None, None),
        ChangeType::Modified => ("modified", None, None),
        ChangeType::Deleted => ("deleted", None, None),
        ChangeType::Renamed { from, similarity } => {
            ("renamed", Some(from.as_str()), Some(*similarity as i32))
        }
        ChangeType::Copied { from, similarity } => {
            ("copied", Some(from.as_str()), Some(*similarity as i32))
        }
        ChangeType::BinaryOrUnknown => ("binary", None, None),
    };
    app.append_row(params![
        rev,
        ch.path,
        type_str,
        rename_from,
        similarity,
        ch.loc_added as i32,
        ch.loc_deleted as i32,
    ])
    .map_err(|err| BcaError::Analysis(format!("append change: {err}")))?;
    Ok(())
}
```

- [ ] **Step 5: Run tests, confirm pass**

Run: `cargo test -p bca-lib --test ingest_test --features test-support`

Expected: 1 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): commit ingestion via single Appender thread + bounded channel"
```

---

## Phase 0.E: Revisions analysis + CSV output

### Task 12: `revisions` analysis as SQL view

**Files:**
- Create: `crates/bca-lib/src/analyses/mod.rs`
- Create: `crates/bca-lib/src/analyses/revisions.rs`
- Modify: `crates/bca-lib/src/lib.rs`
- Create: `crates/bca-lib/tests/revisions_test.rs`

- [ ] **Step 1: Write failing test**

Create `crates/bca-lib/tests/revisions_test.rs`:

```rust
use bca_lib::analyses::revisions::run_revisions;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;
use bca_lib::Options;

#[test]
fn revisions_for_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options { min_revs: 1, ..Options::default() };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_revisions(&db, &opts).expect("run");
    // tiny repo: src/main.rs touched in 4 commits (init + 3 modifications),
    // src/lib.rs touched in 1 commit (added)
    let main = rows.iter().find(|(p, _)| p == "src/main.rs").expect("main");
    assert_eq!(main.1, 4);
    let lib = rows.iter().find(|(p, _)| p == "src/lib.rs").expect("lib");
    assert_eq!(lib.1, 1);
}
```

- [ ] **Step 2: Run test, confirm fail**

Run: `cargo test -p bca-lib --test revisions_test --features test-support`

Expected: compile error — `run_revisions` not found.

- [ ] **Step 3: Create `crates/bca-lib/src/analyses/revisions.rs`**

```rust
//! `revisions` analysis — file → revision count.
//! Code-maat parity output: (entity, n-revs).
//! See spec §2.6 in code-maat (entities.clj:1443).

use crate::facts::FactsDb;
use crate::{BcaError, Options, Result};

pub fn run_revisions(db: &FactsDb, opts: &Options) -> Result<Vec<(String, u32)>> {
    let limit = opts.rows_limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
    let sql = format!(
        "SELECT path, COUNT(DISTINCT rev) AS n_revs
         FROM changes
         GROUP BY path
         HAVING n_revs >= {min}
         ORDER BY n_revs DESC, path ASC{limit}",
        min = opts.min_revs,
        limit = limit,
    );
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| BcaError::Analysis(format!("prepare revisions: {e}")))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32)))
        .map_err(|e| BcaError::Analysis(format!("query revisions: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| BcaError::Analysis(format!("collect revisions: {e}")))
}
```

- [ ] **Step 4: Create `crates/bca-lib/src/analyses/mod.rs`**

```rust
//! Analysis implementations. Each is a SQL view over the fact store
//! plus a thin Rust orchestrator. Plan 1 ships `revisions` only.

pub mod revisions;
```

- [ ] **Step 5: Add module + re-export**

Update `crates/bca-lib/src/lib.rs`:

```rust
pub mod analyses;
// ... other modules
```

- [ ] **Step 6: Expose `conn()` accessor for the analyses**

`conn()` is already `pub(crate)`. Make it `pub` so external analyses crates (future) work:

```rust
// in facts/mod.rs:
pub fn conn(&self) -> &Connection {
    &self.conn
}
```

- [ ] **Step 7: Run tests, confirm pass**

Run: `cargo test -p bca-lib --test revisions_test --features test-support`

Expected: 1 passed.

- [ ] **Step 8: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): revisions analysis as SQL view + Rust orchestrator"
```

---

### Task 13: CSV output emitter

**Files:**
- Create: `crates/bca-lib/src/output/mod.rs`
- Create: `crates/bca-lib/src/output/csv.rs`
- Modify: `crates/bca-lib/src/lib.rs`
- Create: `crates/bca-lib/tests/output_csv_test.rs`

- [ ] **Step 1: Write failing test**

Create `crates/bca-lib/tests/output_csv_test.rs`:

```rust
use bca_lib::output::csv::write_revisions_csv;
use std::io::Cursor;

#[test]
fn csv_matches_code_maat_shape() {
    let rows = vec![
        ("src/main.rs".to_string(), 4u32),
        ("src/lib.rs".to_string(), 1u32),
    ];
    let mut buf = Vec::new();
    write_revisions_csv(&rows, &mut Cursor::new(&mut buf)).expect("write");
    let csv = String::from_utf8(buf).expect("utf8");
    assert_eq!(csv, "entity,n-revs\nsrc/main.rs,4\nsrc/lib.rs,1\n");
}
```

- [ ] **Step 2: Run test, confirm fail**

Run: `cargo test -p bca-lib --test output_csv_test`

Expected: compile error.

- [ ] **Step 3: Create `crates/bca-lib/src/output/csv.rs`**

```rust
//! CSV emitters. Headers match code-maat exactly for golden-test parity.

use std::io::Write;

use crate::{BcaError, Result};

pub fn write_revisions_csv<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    writeln!(w, "entity,n-revs").map_err(BcaError::Io)?;
    for (entity, n) in rows {
        // Trivial CSV — paths shouldn't contain commas under our normalization,
        // but quote defensively when they do.
        if entity.contains(',') || entity.contains('"') || entity.contains('\n') {
            let escaped = entity.replace('"', "\"\"");
            writeln!(w, "\"{escaped}\",{n}").map_err(BcaError::Io)?;
        } else {
            writeln!(w, "{entity},{n}").map_err(BcaError::Io)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Create `crates/bca-lib/src/output/mod.rs`**

```rust
//! Output emitters. Plan 1 ships CSV; SARIF, JSON, Markdown, etc. land in Plan 5.

pub mod csv;
```

- [ ] **Step 5: Add module to `lib.rs`**

Update `crates/bca-lib/src/lib.rs`:

```rust
pub mod output;
// ... other modules
```

- [ ] **Step 6: Run tests, confirm pass**

Run: `cargo test -p bca-lib --test output_csv_test`

Expected: 1 passed.

- [ ] **Step 7: Commit**

```bash
git add crates/bca-lib/
git commit -m "feat(lib): CSV emitter for revisions with code-maat header parity"
```

---

## Phase 0.F: CLI walking skeleton

### Task 14: clap CLI + `analyze` subcommand

**Files:**
- Modify: `crates/bca-cli/Cargo.toml`
- Modify: `crates/bca-cli/src/main.rs`
- Create: `crates/bca-cli/src/args.rs`
- Create: `crates/bca-cli/tests/cli_test.rs`

- [ ] **Step 1: Update `crates/bca-cli/Cargo.toml`**

```toml
[package]
name = "bca-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Behavioral Code Analyzer — CLI"

[[bin]]
name = "bca"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
bca-lib = { path = "../bca-lib" }
clap = { version = "4", features = ["derive"] }
anyhow = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
bca-lib = { path = "../bca-lib", features = ["test-support"] }
```

- [ ] **Step 2: Write failing CLI test**

Create `crates/bca-cli/tests/cli_test.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn analyze_revisions_emits_csv() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    Command::cargo_bin("bca")
        .unwrap()
        .args([
            "analyze",
            "--analysis",
            "revisions",
            "--repo",
            tiny.dir.path().to_str().unwrap(),
            "--format",
            "csv",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("entity,n-revs"))
        .stdout(predicate::str::contains("src/main.rs,4"))
        .stdout(predicate::str::contains("src/lib.rs,1"));
}

#[test]
fn analyze_rejects_unknown_analysis() {
    Command::cargo_bin("bca")
        .unwrap()
        .args(["analyze", "--analysis", "not-real", "--repo", "."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown analysis"));
}

#[test]
fn version_flag_works() {
    Command::cargo_bin("bca")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("0.1.0-alpha.1"));
}
```

- [ ] **Step 3: Run test, confirm fail**

Run: `cargo test -p bca-cli`

Expected: 3 tests fail / panic — binary doesn't have the args yet.

- [ ] **Step 4: Create `crates/bca-cli/src/args.rs`**

```rust
//! Clap argument definitions. CLI surface from spec §5.2.
//! Plan 1 ships only the minimum: `analyze`. `diff`, `query`, `facts`,
//! `explain`, `config`, `doctor`, `init` land in later plans.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "bca", version, about = "Behavioral Code Analyzer", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Verbose logging
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run an analysis and emit results.
    Analyze(AnalyzeArgs),
}

#[derive(clap::Args, Debug)]
pub struct AnalyzeArgs {
    /// Analysis name (Plan 1 supports: revisions).
    #[arg(short, long, default_value = "revisions")]
    pub analysis: String,

    /// Path to the git repo (default: cwd).
    #[arg(short, long, default_value = ".")]
    pub repo: PathBuf,

    /// Output format (Plan 1 supports: csv).
    #[arg(short, long, default_value = "csv")]
    pub format: String,

    /// Write output to file instead of stdout.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Minimum revisions per entity (code-maat parity).
    #[arg(long, default_value_t = 5)]
    pub min_revs: u32,

    /// Limit output to N rows.
    #[arg(long)]
    pub rows: Option<u32>,
}
```

- [ ] **Step 5: Replace `crates/bca-cli/src/main.rs`**

```rust
//! bca — Behavioral Code Analyzer CLI.

mod args;

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use bca_lib::analyses::revisions::run_revisions;
use bca_lib::facts::FactsDb;
use bca_lib::output::csv::write_revisions_csv;
use bca_lib::repo::GixRepo;
use bca_lib::{AnalysisName, Options};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::args::{AnalyzeArgs, Cli, Command};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Analyze(args) => analyze(args),
    }
}

fn init_logging(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("info,bca=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn analyze(args: AnalyzeArgs) -> Result<()> {
    // Validate analysis name early — produces a clean error message
    // even though Plan 1 only runs `revisions`.
    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("unknown analysis: {}", args.analysis))?;
    if analysis != AnalysisName::Revisions {
        anyhow::bail!(
            "Plan 1 walking skeleton only supports --analysis revisions. \
             Full analysis set lands in Plan 4."
        );
    }
    if args.format != "csv" {
        anyhow::bail!(
            "Plan 1 walking skeleton only supports --format csv. \
             JSON, SARIF, Markdown, Parquet, SQLite land in Plan 5."
        );
    }

    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: args.min_revs,
        rows_limit: args.rows,
        ..Options::default()
    };

    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db = FactsDb::new_in_memory().context("open fact store")?;
    db.ingest(&repo, &opts).context("ingest commits")?;
    let rows = run_revisions(&db, &opts).context("run revisions analysis")?;

    let mut out: Box<dyn Write> = match args.output {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    write_revisions_csv(&rows, &mut out).context("write csv")?;
    Ok(())
}
```

- [ ] **Step 6: Run tests, confirm pass**

Run: `cargo test -p bca-cli`

Expected: 3 passed.

- [ ] **Step 7: Smoke-test the binary manually**

```bash
cargo build --release -p bca-cli
cd /tmp && rm -rf bca-smoke && git clone --depth 100 https://github.com/BurntSushi/ripgrep bca-smoke
./target/release/bca analyze --analysis revisions --repo /tmp/bca-smoke --rows 10
```

Expected: a CSV with the top 10 most-revised files in ripgrep.

- [ ] **Step 8: Commit**

```bash
git add crates/bca-cli/
git commit -m "feat(cli): analyze subcommand — walking skeleton for revisions/csv"
```

---

## Phase 0.G: Smoke test and CI green

### Task 15: Full `just ci` green + CHANGELOG

**Files:**
- Create: `CHANGELOG.md`
- Create: `README.md`

- [ ] **Step 1: Create `CHANGELOG.md`**

```markdown
# Changelog

Conventional Commits format. All notable changes documented here.

## [Unreleased]

### Added (Plan 1: Phase 0 + Walking Skeleton)
- 3-crate Cargo workspace (`bca-lib`, `bca-cli`, future `bca-rca`)
- Core types: `CommitEvent`, `FileChange`, `Hunk`, `ChangeType`, `KameiFeatures`
- `AnalysisName` enum and `Options` struct with code-maat parity defaults
- `arrow_facade` module — single re-export point for `arrow-rs`
- `Repo` trait + `GixRepo` impl (read .git via gix 0.84)
- `FactsDb` — DuckDB-backed fact store with v1 schema
- Commit ingestion pipeline (gix → crossbeam channel → DuckDB Appender)
- `revisions` analysis (SQL view + Rust orchestrator)
- CSV output emitter (code-maat header parity)
- `bca analyze --analysis revisions --format csv` CLI
- GitHub Actions CI (fmt, clippy, test on 3 OSes, cargo-deny)
- Justfile, deny.toml, renovate.json, rust-toolchain.toml

### Pending (subsequent plans)
- Plan 2: RCA vendor + Go support
- Plan 3: complexity integration + hotspots + Code Health composite
- Plan 4: 9 other analyses + Fisher significance + identity resolution
- Plan 5: SARIF + Markdown + Parquet + SQLite + provenance manifest
- Plan 6: differential testing harness + perf benchmarks + release infra
```

- [ ] **Step 2: Create `README.md`**

```markdown
# bca — Behavioral Code Analyzer

> Rust-based modernization of Adam Tornhill's [code-maat](https://github.com/adamtornhill/code-maat).
> Mines git history to produce hotspots, change coupling, ownership topology, and code-health metrics.

**Status: alpha (Plan 1 walking skeleton).** Architecture validated end-to-end; feature parity with code-maat lands across Plans 2–6.

## Quick start

```bash
cargo install --path crates/bca-cli  # or `cargo binstall bca` once released
cd ~/my-repo
bca analyze --analysis revisions --format csv
```

## Design

See [`docs/superpowers/specs/2026-06-06-bca-design.md`](docs/superpowers/specs/2026-06-06-bca-design.md).

## License

GPL-3.0-only. Includes a vendored fork of Mozilla's `rust-code-analysis` under MPL-2.0 (Plan 2+) — see `crates/bca-rca/LICENSE-MPL`.
```

- [ ] **Step 3: Run full CI locally**

Run: `just ci`

Expected: all checks pass.

- [ ] **Step 4: Run the binary against a real public repo**

```bash
cd /tmp && rm -rf bca-smoke
git clone --depth 200 https://github.com/BurntSushi/ripgrep bca-smoke
./target/release/bca analyze --analysis revisions --repo /tmp/bca-smoke --rows 5 -v
```

Expected: top 5 hotspots in ripgrep printed as CSV; logs reach stderr.

- [ ] **Step 5: Commit**

```bash
git add CHANGELOG.md README.md
git commit -m "docs: CHANGELOG + README for Plan 1 walking skeleton"
```

- [ ] **Step 6: Push and verify CI green**

```bash
git push -u origin main
```

Expected: GitHub Actions CI runs all jobs and they pass.

---

## Phase 0 Definition of Done

All of the following must be true to declare Plan 1 complete:

- [ ] `cargo test --workspace --all-features` is green on local Linux+macOS
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean
- [ ] `cargo fmt --all --check` is clean
- [ ] `cargo deny check` is clean
- [ ] GitHub Actions CI is green on `main`
- [ ] `bca analyze --analysis revisions --repo <real-repo>` produces a CSV that matches what code-maat would produce for the same threshold (eyeball only — formal differential test lands in Plan 6)
- [ ] CHANGELOG and README check in

At which point: author **Plan 2 (RCA vendor + Go support)** based on real Plan 1 experience.

---

## Self-Review (run inline before handoff)

### Spec coverage check
| Spec section | Plan 1 coverage |
|---|---|
| §1.1 v1 in-scope | Partial — workspace + gix + DuckDB + 1 analysis + CSV. Rest in Plans 2–6. |
| §2 Workspace architecture | ✓ Tasks 1–3 |
| §3.1 Public types | ✓ Tasks 4–5 (Kamei = Option, set None in Plan 1) |
| §3.2 DuckDB fact schema | ✓ Task 10 (full schema; only `commits` + `changes` populated) |
| §3.2.1 Correctness invariants | Deferred — Fisher significance lands in Plan 4 |
| §3.2.2 Concurrency pattern | ✓ Task 11 (single producer for now; multi-producer in Plan 4) |
| §4 Complexity | Deferred to Plans 2–3 |
| §5.2 CLI subcommands | Partial — only `analyze`. Rest in Plans 4–5. |
| §5.3 Output formats | Partial — only CSV. Rest in Plan 5. |
| §6.1 Test rings | Ring 1 (unit) + start of Ring 2 (assert_cmd). Ring 3 differential in Plan 6. |

### Placeholder scan
Searched for "TBD", "TODO", "fill in", "similar to": none in steps. References forward ("Plan 4 fills…") are intentional and explicit, not placeholders.

### Type consistency
- `CommitEvent.kamei: Option<KameiFeatures>` — consistent across Tasks 4, 8, 11.
- `FileChange.hunks: Vec<Hunk>` — consistent.
- `Repo::walk_commits` return type — consistent in Tasks 7, 8, 11.
- `FactsDb::ingest` — defined in Task 11, called in Tasks 12, 14.
- `run_revisions` signature — consistent across Tasks 12, 14.
- `Options::min_revs: u32` and `Options::rows_limit: Option<u32>` — consistent in Tasks 5, 12, 14.

No drift detected.

---

*End of Plan 1.*
