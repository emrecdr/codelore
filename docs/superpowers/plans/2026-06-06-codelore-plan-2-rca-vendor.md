# CodeLore Plan 2: RCA Vendor + Plan 1 Carry-Over

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Plan 2 of 6 for v1 Spine release.** Builds on Plan 1 (`docs/superpowers/plans/2026-06-06-codelore-phase-0-walking-skeleton.md`).

**Goal:** Add the third workspace crate `codelore-rca/` (vendored fork of [mozilla/rust-code-analysis](https://github.com/mozilla/rust-code-analysis)), validate it produces working metrics for Tier-1 languages (Rust, TypeScript/JavaScript, Python, Java), gate the buggy JS/TS Halstead+MI metrics behind a `metrics-experimental` feature flag, and clean up the small carry-over items from Plan 1's final review.

**Architecture:** `codelore-rca` is a self-contained vendored crate under dual SPDX license `MPL-2.0 AND GPL-3.0-only` (original RCA files retain MPL-2.0; new files are GPL-3.0). It is NOT integrated with `codelore-lib` in Plan 2 — that's Plan 3 (complexity integration). Plan 2 ships the raw metric library; Plan 3 wires it into hotspots and Code Health.

**Tech Stack:**
- All Plan 1 stack (Rust edition 2024, gix 0.84, duckdb =1.10503.1, etc.)
- Vendored: `mozilla/rust-code-analysis` (master, ~24,500 LOC)
- `tree-sitter = "=0.25.3"` (already pinned in workspace) + per-language tree-sitter grammars (tree-sitter-rust, tree-sitter-typescript, tree-sitter-javascript, tree-sitter-python, tree-sitter-java)

**Out of scope for this plan (deferred to subsequent plans):**
- codelore-lib integration of complexity metrics (Plan 3)
- Hotspot ranking, Code Health composite (Plan 3)
- Go support (deferred to v1.5 per spec §4.2 — uses whitespace fallback in v1)
- Kotlin metric impls (spec §8.1 v1.5)
- C# language support (spec §8.1 v1.5)
- Fixing RCA upstream bugs #528 #1183 (JS/TS Halstead — Plan 2 quarantines them)

**Definition of Done for Plan 2:**
- `crates/codelore-rca/` exists in the workspace with vendored RCA code
- Cargo.toml SPDX: `MPL-2.0 AND GPL-3.0-only`
- `cargo test -p codelore-rca` is green (RCA's own snapshot tests preserved for the 4 Tier-1 languages)
- A new integration test in `codelore-rca/tests/` proves each Tier-1 language produces non-zero Cyclomatic + Cognitive metrics for a representative sample
- `metrics-experimental` feature flag gates JS/TS Halstead + MI
- All Plan 1 carry-over items from §1 below resolved
- `cargo test --workspace --all-features` green; clippy clean; fmt clean; cargo-deny clean
- CHANGELOG updated

---

## §1 — Plan 1 carry-over fixes (Phase 2.A)

These are the items the Plan 1 final reviewer flagged as "fix early in Plan 2." Apply first — they're small and unblock everything else.

### Task 1: Wire `CodeLoreError::exit_code()` into `main()`

**Files:**
- Modify: `crates/codelore-cli/src/main.rs`

- [ ] **Step 1: Update main() to map CodeLoreError to its exit code**

The current `fn main()` prints the error and exits with `1` always:

```rust
fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
```

Replace with:

```rust
fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        // Map CodeLoreError to its spec §6.6 exit code if present in the chain.
        // Falls back to 1 for non-CodeLoreError errors (e.g. clap parse errors).
        let code = e
            .chain()
            .find_map(|cause| cause.downcast_ref::<codelore_lib::CodeLoreError>())
            .map_or(1, codelore_lib::CodeLoreError::exit_code);
        std::process::exit(code);
    }
}
```

- [ ] **Step 2: Add a test that verifies exit codes**

Append to `crates/codelore-cli/tests/cli_test.rs`:

```rust
#[test]
fn invalid_repo_exits_with_code_3() {
    let status = Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze",
            "--analysis", "revisions",
            "--repo", "/tmp/definitely-does-not-exist-codelore-test",
        ])
        .status()
        .unwrap();
    // CodeLoreError::Repo → exit 3 per spec §6.6
    assert_eq!(status.code(), Some(3));
}
```

- [ ] **Step 3: Verify**

Run: `cargo test -p codelore-cli`

Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-cli/
git commit -m "fix(cli): wire CodeLoreError::exit_code into main per spec §6.6"
```

---

### Task 2: Restrict `query_one_value` to `pub(crate)` and clean up Plan 11 comment

**Files:**
- Modify: `crates/codelore-lib/src/facts/mod.rs`
- Modify: `crates/codelore-lib/src/repo/gix_repo.rs`
- Modify: `crates/codelore-lib/tests/facts_test.rs` (move under test-support gating)
- Modify: `crates/codelore-lib/tests/ingest_test.rs` (move under test-support gating)
- Modify: `crates/codelore-lib/tests/revisions_test.rs` (move under test-support gating)

- [ ] **Step 1: Change `query_one_value` visibility**

In `crates/codelore-lib/src/facts/mod.rs`, change:

```rust
pub fn query_one_value(&self, sql: &str) -> Result<String> {
```

to:

```rust
#[cfg(any(test, feature = "test-support"))]
pub fn query_one_value(&self, sql: &str) -> Result<String> {
```

(Method is now gated to test-support builds only. This is more restrictive than `pub(crate)` since it gates compile-time too, preventing accidental production use.)

- [ ] **Step 2: Update `gix_repo.rs` Plan 11 comment**

Around `crates/codelore-lib/src/repo/gix_repo.rs:35` the `NOTE(Plan 11)` comment is a typo. Update to:

```rust
// NOTE(Plan 4): full traversal happens here before any consumer sees commits.
// When the channel pipeline lands (Plan 4), consider a lazy walk with OIDs
// collected into a bounded channel instead. Current design is correct for Plan 1.
```

- [ ] **Step 3: Verify tests still use query_one_value via test-support feature**

The tests that use `query_one_value` (`facts_test.rs`, `ingest_test.rs`, `revisions_test.rs`) already require the `test-support` feature for `tiny_repo`. Confirm they still pass:

Run: `cargo test -p codelore-lib --all-features`

Expected: 17 lib tests pass (unchanged).

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/
git commit -m "refactor(lib): gate query_one_value behind test-support + fix Plan 11 comment"
```

---

### Task 3: Add test for file-backed `FactsDb::open()`

**Files:**
- Modify: `crates/codelore-lib/tests/facts_test.rs`

- [ ] **Step 1: Add roundtrip test**

Append to `crates/codelore-lib/tests/facts_test.rs`:

```rust
#[test]
fn file_backed_db_persists_and_reopens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.duckdb");

    // First open: create + write
    {
        let db = FactsDb::open(&path).expect("create file-backed");
        let tables = db.list_tables().expect("list");
        assert!(tables.iter().any(|n| n == "commits"));
    }

    // Second open: same path should re-open without re-creating
    {
        let db = FactsDb::open(&path).expect("reopen file-backed");
        let schema_version: String = db
            .query_one_value("SELECT value FROM provenance WHERE key = 'schema_version'")
            .expect("query");
        assert_eq!(schema_version, "1");
    }
}
```

(`facts_test.rs` already runs with `--all-features` which activates `test-support` → makes `tempfile` available.)

- [ ] **Step 2: Verify**

Run: `cargo test -p codelore-lib --all-features`

Expected: 18 lib tests pass (17 previous + 1 new).

- [ ] **Step 3: Commit**

```bash
git add crates/codelore-lib/
git commit -m "test(lib): add FactsDb::open() file-backed roundtrip test"
```

---

## §2 — Vendor RCA (Phase 2.B)

### Task 4: Clone and vendor `mozilla/rust-code-analysis` as `crates/codelore-rca/`

**Files:**
- Create: `crates/codelore-rca/` (entire vendored crate)
- Modify: `Cargo.toml` (workspace) — add `crates/codelore-rca` to `members`

**Reference:**
- Upstream: https://github.com/mozilla/rust-code-analysis (Mozilla 2026-01-20 commit baseline; last published release was v0.0.25 2023-01-13)
- Validation Stream 2 in spec §4.1 documented per-language metric availability against `src/` of upstream.

- [ ] **Step 1: Clone upstream into a temp dir**

```bash
cd /tmp
rm -rf rust-code-analysis-upstream
git clone --depth 1 https://github.com/mozilla/rust-code-analysis.git rust-code-analysis-upstream
cd rust-code-analysis-upstream
git log -1 --format='%H %ad %s' --date=short
```

Note the commit SHA and date — record it in `crates/codelore-rca/UPSTREAM.md` (created in Step 5).

- [ ] **Step 2: Copy the `src/` tree into `crates/codelore-rca/`**

```bash
cd /Users/emrec/Projects/playground/codescene
mkdir -p crates/codelore-rca
cp -r /tmp/rust-code-analysis-upstream/src crates/codelore-rca/src
cp /tmp/rust-code-analysis-upstream/build.rs crates/codelore-rca/build.rs 2>/dev/null || true
cp /tmp/rust-code-analysis-upstream/LICENSE crates/codelore-rca/LICENSE-MPL
```

- [ ] **Step 3: DROP unused subdirectories and grammars**

Per spec §4.1 vendoring procedure, drop:

```bash
# Drop the web crate (actix-web; unused)
rm -rf crates/codelore-rca/src/web

# Drop Mozilla-specific tree-sitter grammar forks (~30 MB)
# These are mozcpp and mozjs — we'll use upstream tree-sitter-cpp and tree-sitter-javascript instead
rm -rf crates/codelore-rca/src/languages/language_mozcpp.rs
rm -rf crates/codelore-rca/src/languages/language_mozjs.rs

# Drop unused metric impls (ABC, WMC, NPA, NPM — Java-only specializations not in our spec)
rm -f crates/codelore-rca/src/metrics/abc.rs
rm -f crates/codelore-rca/src/metrics/wmc.rs
rm -f crates/codelore-rca/src/metrics/npa.rs
rm -f crates/codelore-rca/src/metrics/npm.rs
```

(If any of the paths above don't exist in the upstream you cloned, that's fine — note it in your report. The upstream layout has shifted over time.)

- [ ] **Step 4: Update mod declarations to remove dropped files**

In `crates/codelore-rca/src/lib.rs` (or `src/metrics/mod.rs` and `src/languages/mod.rs`), find and remove the `pub mod abc;` / `pub mod wmc;` / `pub mod npa;` / `pub mod npm;` / `pub mod language_mozcpp;` / `pub mod language_mozjs;` lines.

Similarly, remove any references to the deleted types in `src/lib.rs`'s public API surface.

- [ ] **Step 5: Create `crates/codelore-rca/UPSTREAM.md`**

```markdown
# codelore-rca: Vendored Mozilla rust-code-analysis

This crate is a maintained fork of [mozilla/rust-code-analysis](https://github.com/mozilla/rust-code-analysis).

## Upstream baseline

- **Source commit:** `<SHA from Step 1>`
- **Source date:** `<date from Step 1>`
- **Vendored on:** 2026-06-06

## Modifications from upstream

- `src/web/` — REMOVED (actix-web; unused in CodeLore)
- `src/languages/language_mozcpp.rs` — REMOVED (Mozilla-specific tree-sitter-cpp fork)
- `src/languages/language_mozjs.rs` — REMOVED (Mozilla-specific tree-sitter-js fork)
- `src/metrics/abc.rs`, `wmc.rs`, `npa.rs`, `npm.rs` — REMOVED (Java-only specializations, not in our spec)
- `src/lib.rs` — `pub mod` declarations updated to match removals

## License

Original files retain their MPL-2.0 license headers (preserved per Mozilla's
[MPL combining guide](https://www.mozilla.org/en-US/MPL/2.0/combining-mpl-and-gpl/)).
New files added by CodeLore contributors carry GPL-3.0-only headers.

SPDX: `MPL-2.0 AND GPL-3.0-only`

## Sync procedure

To pull upstream fixes:
1. Fetch upstream commits since the SHA above.
2. Cherry-pick correctness fixes and grammar bumps (avoid Mozilla-specific features).
3. Update this file with the new SHA + date.
4. Run `cargo test -p codelore-rca` to verify.

Year-1 maintenance budget: ~8 days (see spec §4.1).
```

- [ ] **Step 6: Add `crates/codelore-rca/` to workspace members**

In `/Users/emrec/Projects/playground/codescene/Cargo.toml`, update the `members` line from:

```toml
members = ["crates/codelore-lib", "crates/codelore-cli"]
# crates/codelore-rca added in Plan 2
```

to:

```toml
members = ["crates/codelore-lib", "crates/codelore-cli", "crates/codelore-rca"]
```

- [ ] **Step 7: Commit (no build yet — Cargo.toml comes in Task 5)**

```bash
git add crates/codelore-rca/ Cargo.toml
git commit -m "feat(codelore-rca): vendor mozilla/rust-code-analysis (drop -web, mozcpp, mozjs, ABC/WMC/NPA/NPM)"
```

---

### Task 5: Configure `crates/codelore-rca/Cargo.toml`

**Files:**
- Create: `crates/codelore-rca/Cargo.toml`

- [ ] **Step 1: Create `Cargo.toml` for the vendored crate**

```toml
[package]
name = "codelore-rca"
version.workspace = true
edition = "2021"  # RCA upstream is on edition 2021; do not bump to 2024 without auditing
rust-version.workspace = true
# SPDX: original files are MPL-2.0; new files (Step 5 of Task 4 onwards) are GPL-3.0-only.
# Both terms apply simultaneously to this crate.
license = "MPL-2.0 AND GPL-3.0-only"
repository.workspace = true
description = "Vendored fork of Mozilla rust-code-analysis for CodeLore"

[lints]
# Don't apply workspace lints to vendored MPL files — keeps upstream-merge friction low.
# Re-enable selectively if we add many new GPL-3.0 files.

[dependencies]
tree-sitter = "=0.25.3"  # locked workspace-wide per spec §2.2

# Per-language tree-sitter grammars (Tier-1)
tree-sitter-rust = "0.21"
tree-sitter-python = "0.21"
tree-sitter-java = "0.21"
tree-sitter-typescript = "0.21"  # provides both TS and TSX
tree-sitter-javascript = "0.21"

# RCA's other dependencies (port from the upstream Cargo.toml; trim what's not needed)
serde = { version = "1", features = ["derive"] }
serde_json = "1"
crossbeam = "0.8"
walkdir = "2"
regex = "1"

[features]
default = []

# Gates buggy JS/TS Halstead and MI metrics (upstream issues #528 #1183).
# When enabled, the metrics will be computed and returned but their accuracy
# is not guaranteed. SARIF output (Plan 5) excludes these by default.
metrics-experimental = []

[dev-dependencies]
insta = "1"

[[test]]
name = "tier1_languages_smoke"
required-features = []
```

**Versions may need adjustment** — the per-language tree-sitter grammar versions move independently of `tree-sitter` core. Use whatever versions compile against `tree-sitter = "=0.25.3"`. If 0.21 doesn't compile, try `cargo add tree-sitter-rust@latest` and let cargo resolve.

- [ ] **Step 2: Build and confirm clean compile**

Run: `cargo build -p codelore-rca`

This may surface compile errors from the upstream code (renamed APIs, missing modules, etc.). Adapt as needed. Common fixes:
- Remove or stub references to deleted `mozcpp`/`mozjs`/ABC/WMC/NPA/NPM
- Update tree-sitter grammar version mismatches
- Disable upstream features that pulled in removed dependencies

Document any non-trivial adaptations in `UPSTREAM.md`.

If the cold compile takes >15 minutes, that's expected — tree-sitter grammars are large C compilations.

- [ ] **Step 3: Commit**

```bash
git add crates/codelore-rca/Cargo.toml crates/codelore-rca/UPSTREAM.md
git commit -m "feat(codelore-rca): Cargo.toml with MPL-2.0 AND GPL-3.0-only SPDX + Tier-1 grammars"
```

---

## §3 — Verify metrics work (Phase 2.C)

### Task 6: Tier-1 language smoke tests

**Files:**
- Create: `crates/codelore-rca/tests/tier1_languages_smoke.rs`

- [ ] **Step 1: Write smoke test**

```rust
//! Smoke tests proving each Tier-1 language produces non-zero metrics
//! for a representative sample. Catches RCA upstream API regressions.

// NOTE: API depends on RCA's actual surface. The function names below may need
// adjustment based on what RCA exposes. Pattern: load grammar, parse, compute
// metric on root node.

#[test]
fn rust_cyclomatic_and_cognitive() {
    let src = b"
fn complex(x: i32) -> i32 {
    if x > 0 {
        for i in 0..x { println!(\"{i}\"); }
    } else if x < 0 {
        match x { -1 => return -1, _ => return -2 }
    }
    0
}
";
    // Adapt to RCA's actual API:
    let metrics = codelore_rca::compute_metrics_for_language(src, codelore_rca::Language::Rust)
        .expect("compute metrics");
    assert!(metrics.cyclomatic > 1, "Rust cyclomatic should be > 1 for branching code");
    assert!(metrics.cognitive > 0, "Rust cognitive should be > 0");
}

#[test]
fn python_cyclomatic_and_cognitive() {
    let src = b"
def complex(x):
    if x > 0:
        for i in range(x):
            print(i)
    elif x < 0:
        if x == -1:
            return -1
        return -2
    return 0
";
    let metrics = codelore_rca::compute_metrics_for_language(src, codelore_rca::Language::Python)
        .expect("compute metrics");
    assert!(metrics.cyclomatic > 1);
    assert!(metrics.cognitive > 0);
}

#[test]
fn typescript_cyclomatic_and_cognitive() {
    let src = b"
function complex(x: number): number {
    if (x > 0) {
        for (let i = 0; i < x; i++) console.log(i);
    } else if (x < 0) {
        switch (x) { case -1: return -1; default: return -2; }
    }
    return 0;
}
";
    let metrics = codelore_rca::compute_metrics_for_language(src, codelore_rca::Language::TypeScript)
        .expect("compute metrics");
    assert!(metrics.cyclomatic > 1);
    assert!(metrics.cognitive > 0);
}

#[test]
fn java_cyclomatic_and_cognitive() {
    let src = b"
class C {
    int complex(int x) {
        if (x > 0) {
            for (int i = 0; i < x; i++) System.out.println(i);
        } else if (x < 0) {
            switch (x) { case -1: return -1; default: return -2; }
        }
        return 0;
    }
}
";
    let metrics = codelore_rca::compute_metrics_for_language(src, codelore_rca::Language::Java)
        .expect("compute metrics");
    assert!(metrics.cyclomatic > 1);
    assert!(metrics.cognitive > 0);
}

#[test]
fn javascript_cyclomatic_and_cognitive() {
    let src = b"
function complex(x) {
    if (x > 0) {
        for (let i = 0; i < x; i++) console.log(i);
    } else if (x < 0) {
        switch (x) { case -1: return -1; default: return -2; }
    }
    return 0;
}
";
    let metrics = codelore_rca::compute_metrics_for_language(src, codelore_rca::Language::JavaScript)
        .expect("compute metrics");
    assert!(metrics.cyclomatic > 1);
    assert!(metrics.cognitive > 0);
}

#[cfg(feature = "metrics-experimental")]
#[test]
fn jsts_halstead_with_experimental_flag() {
    // Halstead+MI for JS/TS only exposed under metrics-experimental.
    // Verifies the gating works; doesn't assert correctness of the bug-affected metric.
    let src = b"function f(x) { return x + 1; }";
    let metrics = codelore_rca::compute_metrics_for_language(src, codelore_rca::Language::JavaScript)
        .expect("compute metrics");
    let _ = metrics.halstead;  // just confirms the field is accessible under the feature
}
```

**Adapt the API surface** (`codelore_rca::compute_metrics_for_language`, `codelore_rca::Language`, the `metrics` struct shape) based on what RCA actually exposes after the vendor + cleanup. RCA's actual public API is in `src/lib.rs` — read it and write the test to match.

- [ ] **Step 2: Run tests**

```bash
cargo test -p codelore-rca --test tier1_languages_smoke
```

Expected: 5 tests pass.

```bash
cargo test -p codelore-rca --test tier1_languages_smoke --features metrics-experimental
```

Expected: 6 tests pass (the 6th gated by the feature flag).

- [ ] **Step 3: Run RCA's own preserved tests (if any survived the cleanup)**

```bash
cargo test -p codelore-rca
```

If RCA's inline snapshot tests are still in place, they should pass too. If any fail because of our deletions, mark them `#[ignore]` with a comment pointing at the removed feature, OR delete them.

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-rca/tests/
git commit -m "test(codelore-rca): tier-1 language smoke tests (Cyclomatic + Cognitive)"
```

---

## §4 — Workspace integration (Phase 2.D)

### Task 7: Full workspace CI green + cargo-deny accepts MPL-2.0

**Files:**
- Verify (no edits expected): `deny.toml`
- Verify CI: `.github/workflows/ci.yml`

- [ ] **Step 1: Verify cargo-deny allows MPL-2.0**

Run: `cargo deny check licenses`

Should pass: `MPL-2.0` is already in the `deny.toml` allow list. If for some reason it fails, document the failure and adjust deny.toml.

- [ ] **Step 2: Run full workspace check**

Run:
```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo deny check
```

All should be clean. If clippy complains about RCA's upstream code, the vendor crate's `[lints]` section in Cargo.toml is empty (no workspace lints applied) — that's intentional. Clippy on `codelore-lib` and `codelore-cli` should still be strict.

Expected test counts:
- codelore-lib: 18 (previous 17 + new file-backed FactsDb test)
- codelore-cli: 4 (previous 3 + new exit code test)
- codelore-rca: 5 base + 1 conditional = 6 with --all-features
- **Total: 28**

- [ ] **Step 3: Commit (if anything was changed)**

```bash
git add -A
git diff --cached  # confirm what's in
git commit -m "ci: ensure full workspace green after codelore-rca addition" || true
```

(`|| true` because there may be nothing to commit if everything was already aligned.)

---

## §5 — Docs + Plan 2 Done (Phase 2.E)

### Task 8: Update CHANGELOG and README

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `README.md`

- [ ] **Step 1: Update `CHANGELOG.md`**

Insert a new Plan 2 entry above the Plan 1 section:

```markdown
## [Unreleased]

### Added (Plan 2: RCA Vendor)
- `crates/codelore-rca/` — vendored fork of mozilla/rust-code-analysis
  - SPDX: `MPL-2.0 AND GPL-3.0-only`
  - Dropped `-web`, mozcpp/mozjs grammars, ABC/WMC/NPA/NPM impls
  - Per-language tree-sitter grammars for Rust, TypeScript/JavaScript, Python, Java
  - `metrics-experimental` feature flag for JS/TS Halstead+MI (RCA bugs #528 #1183)
- Tier-1 language smoke tests verify Cyclomatic + Cognitive metrics

### Fixed (Plan 1 carry-over)
- `CodeLoreError::exit_code()` now wired into `codelore` CLI per spec §6.6
- `FactsDb::query_one_value` restricted to test-support builds only
- `gix_repo.rs` "Plan 11" comment typo → "Plan 4"
- Added file-backed `FactsDb::open()` roundtrip test

### Added (Plan 1: Phase 0 + Walking Skeleton)
...
```

- [ ] **Step 2: Update `README.md`**

In the "What works today" section, append:

```markdown
- Per-language complexity metrics (Cyclomatic, Cognitive) for Rust, TypeScript/JavaScript, Python, Java via vendored `codelore-rca/`
```

In the Roadmap section, change "**Plan 2** — vendor Mozilla's `rust-code-analysis`..." to:

```markdown
- **Plan 2** ✅ — vendored Mozilla's `rust-code-analysis` as `codelore-rca/`, Tier-1 metric smoke tests
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md README.md
git commit -m "docs: CHANGELOG + README for Plan 2 RCA vendor"
```

---

## Plan 2 Definition of Done

- [ ] `crates/codelore-rca/` exists with vendored RCA source
- [ ] `Cargo.toml` has `license = "MPL-2.0 AND GPL-3.0-only"`
- [ ] `crates/codelore-rca/UPSTREAM.md` documents the source SHA + modifications
- [ ] Tier-1 smoke tests pass (5 base + 1 conditional)
- [ ] `metrics-experimental` feature flag works (verified by the conditional test)
- [ ] All Plan 1 carry-over items resolved (Task 1, 2, 3)
- [ ] `cargo test --workspace --all-features` reports 28 tests passing
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- [ ] `cargo fmt --all --check` clean
- [ ] `cargo deny check` clean (with MPL-2.0 in allow list)
- [ ] CHANGELOG and README updated

After Plan 2 ships: author **Plan 3** (complexity integration into codelore-lib + hotspot ranking + Code Health composite).

---

## Self-Review

### Spec coverage check

| Spec section | Plan 2 coverage |
|---|---|
| §1.1 v1 in scope — `crates/codelore-rca` slot | ✓ Tasks 4–7 |
| §4.1 RCA fork procedure | ✓ Task 4 (clone, copy, drop -web/mozcpp/mozjs, drop unused metric impls) |
| §4.1.1 License precision (MPL-2.0 AND GPL-3.0-only) | ✓ Task 5 |
| §4.2 Tier-1 languages (Rust, TS/JS, Python, Java) | ✓ Task 6 (smoke tests) |
| §4.2 Tier-2 languages (C/C++/Ruby) | Deferred — not in Plan 2's scope. The RCA implementations for those exist in the vendored code but no smoke test. Plan 3 adds them if needed. |
| §4.2.1 `metrics-experimental` gate for JS/TS Halstead/MI | ✓ Task 5 (feature flag) + Task 6 (conditional test) |
| §1.1 CodeLoreError exit code wiring | ✓ Task 1 (carry-over) |

### Placeholder scan

Searched for "TBD", "TODO", "similar to": none in steps. References forward ("Plan 3 will wire complexity into hotspots") are intentional cross-plan dependencies.

### Type consistency check

- `codelore_lib::CodeLoreError::exit_code` — usage in Task 1 matches the signature from Plan 1.
- `FactsDb::query_one_value` — gated by feature flag in Task 2; tests in `tests/` directories run with `--all-features` so they continue to work.
- RCA public API in Task 6 — the `compute_metrics_for_language` and `Language` symbols are placeholders for whatever RCA actually exposes. The implementer must read RCA's `src/lib.rs` to find the real names.

### Known soft spots

- **RCA upstream's exact public API** is not pinned in this plan because it depends on the SHA the implementer vendors. Task 6 Step 1's test code is a template — names need adaptation.
- **Per-language tree-sitter grammar versions** — `tree-sitter-rust = "0.21"` etc. are guesses. The implementer needs to find versions compatible with `tree-sitter = "=0.25.3"`. Cargo should resolve this; if not, the implementer documents what they picked.
- **JS/TS Halstead under `metrics-experimental`**: the gate is at the FEATURE level. The actual code path within RCA needs to be wrapped in `#[cfg(feature = "metrics-experimental")]` for the JS/TS Halstead computation. The implementer needs to locate that code in the vendored source and add the cfg gate.

---

*End of Plan 2.*
