# Plan 6 — Differential Testing + Perf Benchmarks + Release Infrastructure (v1.0)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the v1 release gap. Build the differential-test harness against C git + code-maat goldens, wire criterion perf benchmarks against the Linux kernel target, and stand up release infrastructure (`cargo-dist`, SLSA L3, distroless container, PGO scaffolding) so v1.0 ships from `main` without manual cherry-picking.

**Architecture:** Three parallel work streams that converge on a tagged release. (A) Correctness — `GitCliRepo` shell-out oracle implements the existing `Repo` trait; property tests cross-check every method against `GixRepo`. Code-maat goldens captured into `fixtures/golden/code-maat/` and checked into git. (B) Performance — `benches/` directory with `criterion` benches against tiny / medium / Linux-kernel-snapshot fixtures. Weekly CI job tracks regression vs baseline. (C) Release — `cargo-dist` for multi-platform binaries, SLSA L3 provenance via `slsa-framework/slsa-github-generator`, distroless image, PGO scaffolding (campaign deferred to v1.1 per spec §6.5).

**Tech Stack:**
- `cargo-dist` >= 0.24
- `criterion` 0.7 (benchmarks)
- `proptest` 1.x (property tests against differential oracle)
- `slsa-framework/slsa-github-generator` >= v2.1.0 (binary provenance)
- `cargo-pgo` (scaffolding only — campaign deferred)
- `goblin` distroless base (`gcr.io/distroless/cc-debian12:nonroot`)
- Plan 1–5 stack carried forward (gix 0.84, duckdb 1.10503.1, arrow 58.3.0, bca-rca, tree-sitter 0.25.3)

---

## §0 — Cold-start audit

Plan 6 lands on `main` after Plan 5. Before any task begins, the implementer audits the current state:

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test --workspace --all-features 2>&1 | tail -5
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1 | tail -10
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo fmt --all --check
df -h /                                # disk pressure — Plan 4 hit this twice
git log --oneline -10
```

Expected baseline: **292 tests** passing, clippy/fmt clean, latest commit is the Plan 5 docs commit.

---

## §1 — Differential testing harness (Phase 6.A)

### Task 1: `GitCliRepo` — shell-out implementation of the `Repo` trait

**Files:**
- Create: `crates/bca-lib/src/repo/git_cli_repo.rs`
- Modify: `crates/bca-lib/src/repo/mod.rs`
- Test: `crates/bca-lib/tests/git_cli_repo_test.rs`

Implement the same `Repo` trait that `GixRepo` implements, but back every method with a `std::process::Command::new("git")` invocation. This is the differential oracle — slower than gix, but treats C git as ground truth.

The trait surface (verified from `crates/bca-lib/src/repo/mod.rs`):
- `walk_commits<'a>(&'a self, opts: &'a Options) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>>`
- `changed_files(&self, rev: &str) -> Result<Vec<FileChange>>`
- `diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>>`
- `resolve_alias(&self, email: &str) -> String` (note: returns `String`, **not** `Result<String>` — alias resolution returns the original email on no match, never errors)
- `commit_metadata(&self, rev: &str) -> Result<CommitMetadata>`

Note: `Repo: Send + Sync`, so `GitCliRepo` must be both. `std::process::Command` itself is `Send + Sync` so this is automatic.

Implementation notes:
- `walk_commits` calls `git log --pretty=format:%H%x1f%P%x1f%ae%x1f%aI%x1f%s --name-status` (US-separator inside the prettyformat, blank line between commits). Stream stdout via `BufReader::lines()`; parse the line stream into `CommitEvent`s. The merge filter is driven by `opts.include_merges` — match `GixRepo` behavior exactly.
- `changed_files` calls `git show --name-status --pretty=format: <rev>` and parses status letters (A/M/D/R/C → `ChangeType`). Renames (`R100\told\tnew`) carry both old + new paths.
- `diff_hunks` calls `git diff <rev>^..<rev> -- <path> --unified=0` and parses `@@ -a,b +c,d @@` headers into `Hunk { old_start, old_lines, new_start, new_lines }`. For root commits (no parent), use `git show --format= -p --unified=0 <rev> -- <path>`.
- `resolve_alias` calls `git check-mailmap "<email>"` and parses the output (`"Name <canonical@x>"` → `canonical@x`). Returns the input email unchanged if check-mailmap exits nonzero.
- `commit_metadata` calls `git show --no-patch --format='%G?%n%(trailers:key=Signed-off-by,valueonly)' <rev>` and parses the result into `CommitMetadata`. Read `crates/bca-lib/src/repo/types.rs` to confirm `CommitMetadata` field names + types before implementing.

`std::process::Command` IO must be buffered to avoid pipe-fill deadlocks on large repos — pipe stdout into a `BufReader`, iterate lines, never `.wait_with_output()` on streams >4 MB.

- [ ] **Step 1: Read existing `Repo` trait + `GixRepo` impl to confirm trait surface**

```bash
cat crates/bca-lib/src/repo/mod.rs
cat crates/bca-lib/src/repo/types.rs    # CommitMetadata fields
ls crates/bca-lib/src/repo/
```

Confirm: trait methods are `walk_commits`, `changed_files`, `diff_hunks`, `resolve_alias`, `commit_metadata` — and `resolve_alias` returns `String` (not `Result<String>`). Read `tiny_repo::build()` to confirm what mailmap mapping it writes (if any).

- [ ] **Step 2: Write the failing test**

```rust
// crates/bca-lib/tests/git_cli_repo_test.rs
use bca_lib::repo::{GitCliRepo, Repo};
use bca_lib::Options;

#[test]
fn git_cli_repo_walks_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options { repo_path: tiny.dir.path().to_path_buf(), ..Options::default() };
    let events: Vec<_> = repo.walk_commits(&opts).expect("walk").collect();
    assert!(events.len() >= 5, "tiny_repo has 5 commits, got {}", events.len());
}

#[test]
fn git_cli_repo_resolves_mailmap_alias() {
    // tiny_repo writes a .mailmap mapping — check actual content via Step 1.
    // If tiny_repo doesn't ship a mapping, use the differential_repo built in Task 2 instead.
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    // resolve_alias returns String (no Result); on no match it returns input unchanged.
    let resolved = repo.resolve_alias("unmapped@example.com");
    assert_eq!(resolved, "unmapped@example.com");
}

#[test]
fn git_cli_repo_lists_changed_files_for_a_commit() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GitCliRepo::open(tiny.dir.path()).expect("open");
    let opts = Options { repo_path: tiny.dir.path().to_path_buf(), ..Options::default() };
    let first_commit = repo.walk_commits(&opts).unwrap().next().unwrap().unwrap();
    let files = repo.changed_files(&first_commit.rev).expect("changed_files");
    assert!(!files.is_empty(), "first commit should change at least one file");
}
```

Run: `PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test -p bca-lib --test git_cli_repo_test --all-features`
Expected: FAIL with "no `GitCliRepo` in repo" or similar.

- [ ] **Step 3: Implement `GitCliRepo`**

```rust
// crates/bca-lib/src/repo/git_cli_repo.rs
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::repo::{Repo, /* CommitEvent etc */};
use crate::{BcaError, Options, Result};

pub struct GitCliRepo {
    root: PathBuf,
}

impl GitCliRepo {
    pub fn open(root: &Path) -> Result<Self> {
        // Verify it's a git repo
        let output = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(root)
            .output()
            .map_err(|e| BcaError::Repo(format!("git rev-parse: {e}")))?;
        if !output.status.success() {
            return Err(BcaError::Repo(format!("not a git repo: {}", root.display())));
        }
        Ok(Self { root: root.to_path_buf() })
    }
}

impl Repo for GitCliRepo {
    fn walk_commits<'a>(
        &'a self,
        _opts: &Options,
    ) -> Result<Box<dyn Iterator<Item = Result<CommitEvent>> + Send + 'a>> {
        // ... implement via git log + parse
        todo!()
    }

    fn resolve_alias(&self, email: &str) -> String {
        let Ok(output) = Command::new("git")
            .args(["check-mailmap", &format!("<{email}>")])
            .current_dir(&self.root)
            .output()
        else {
            return email.to_string();
        };
        if !output.status.success() {
            return email.to_string();
        }
        let s = String::from_utf8_lossy(&output.stdout);
        parse_email_from_mailmap_line(s.trim()).unwrap_or_else(|| email.to_string())
    }

    fn changed_files(&self, rev: &str) -> Result<Vec<FileChange>> { /* git show --name-status ... */ todo!() }
    fn diff_hunks(&self, rev: &str, path: &str) -> Result<Vec<Hunk>> { /* git diff <rev>^..<rev> -- <path> --unified=0 */ todo!() }
    fn commit_metadata(&self, rev: &str) -> Result<CommitMetadata> { /* git show --format=... */ todo!() }
}

fn parse_email_from_mailmap_line(line: &str) -> Option<String> {
    // "Canonical Name <canonical@example.com>" → "canonical@example.com"
    let (_, after) = line.rsplit_once('<')?;
    let (email, _) = after.rsplit_once('>')?;
    Some(email.to_string())
}
```

(Full implementation of `walk_commits` is the bulk of the task — parse `git log --pretty=format:%H%x00%P%x00%ae%x00%aI%x00%s%x00 --name-status` into a stream of `CommitEvent`s.)

- [ ] **Step 4: Re-export from `repo/mod.rs`**

```rust
pub mod git_cli_repo;
pub use git_cli_repo::GitCliRepo;
```

- [ ] **Step 5: Run test to verify it passes**

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test -p bca-lib --test git_cli_repo_test --all-features
```

- [ ] **Step 6: Workspace check + commit**

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test --workspace --all-features 2>&1 | tail -3
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo clippy --workspace --all-targets --all-features -- -D warnings
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo fmt --all --check
git add crates/bca-lib/
git commit -m "feat(lib): GitCliRepo — shell-out impl of Repo trait for differential testing"
```

---

### Task 2: Differential property tests — GixRepo ≡ GitCliRepo

**Files:**
- Create: `crates/bca-lib/src/test_support/differential_repo.rs`
- Modify: `crates/bca-lib/src/test_support/mod.rs` (re-export)
- Create: `crates/bca-lib/tests/differential_repo_test.rs`
- Modify: `crates/bca-lib/Cargo.toml` — add `proptest = "1"` to dev-deps (only if writing proptest macros; otherwise skip — most differential tests are plain `#[test]` over a generated fixture)

For every public `Repo` method, assert `GixRepo::method(...) == GitCliRepo::method(...)` on a fixture.

`tiny_repo` is fast enough to drive property tests; for deeper signal, also add a deterministic generated fixture (`differential_repo` — built programmatically with `gix`, 50 commits, 10 files, 1 merge, 1 rename, .mailmap with 3 aliases).

```rust
use bca_lib::repo::{GitCliRepo, GixRepo, Repo};
use bca_lib::Options;

#[test]
fn walk_commits_matches_between_gix_and_git_cli() {
    let fixture = bca_lib::test_support::differential_repo::build();
    let opts = Options { repo_path: fixture.dir.path().to_path_buf(), ..Options::default() };

    let gix_events: Vec<_> = GixRepo::open(fixture.dir.path()).unwrap()
        .walk_commits(&opts).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();
    let cli_events: Vec<_> = GitCliRepo::open(fixture.dir.path()).unwrap()
        .walk_commits(&opts).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();

    let gix_set: std::collections::HashSet<_> = gix_events.iter().map(|e| (&e.rev, &e.author_email)).collect();
    let cli_set: std::collections::HashSet<_> = cli_events.iter().map(|e| (&e.rev, &e.author_email)).collect();
    assert_eq!(gix_set, cli_set, "commit identity should match");

    // Per-commit field equivalence (order may differ if walk order differs)
    let gix_by_rev: std::collections::HashMap<_, _> = gix_events.iter().map(|e| (&e.rev, e)).collect();
    for cli in &cli_events {
        let gix = gix_by_rev.get(&cli.rev).expect("missing rev");
        assert_eq!(gix.author_email, cli.author_email);
        assert_eq!(gix.parents, cli.parents);
        assert_eq!(gix.is_merge, cli.is_merge);
        assert_eq!(gix.subject, cli.subject);
        // CommitEvent.date may differ in tz handling — compare epoch seconds
    }
}

#[test]
fn resolve_alias_matches_between_gix_and_git_cli() {
    let fixture = bca_lib::test_support::differential_repo::build();
    let gix = GixRepo::open(fixture.dir.path()).unwrap();
    let cli = GitCliRepo::open(fixture.dir.path()).unwrap();
    for email in ["alice-old@example.com", "bob@example.com", "unmapped@example.com"] {
        assert_eq!(gix.resolve_alias(email), cli.resolve_alias(email),
                   "mismatch on {email}");
    }
}

#[test]
fn changed_files_matches() {
    let fixture = bca_lib::test_support::differential_repo::build();
    let opts = Options { repo_path: fixture.dir.path().to_path_buf(), ..Options::default() };
    let gix = GixRepo::open(fixture.dir.path()).unwrap();
    let cli = GitCliRepo::open(fixture.dir.path()).unwrap();
    let revs: Vec<String> = gix.walk_commits(&opts).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap()
        .into_iter().map(|e| e.rev).collect();
    for rev in &revs {
        let gix_files = gix.changed_files(rev).unwrap();
        let cli_files = cli.changed_files(rev).unwrap();
        let gix_paths: std::collections::HashSet<_> = gix_files.iter().map(|f| &f.path).collect();
        let cli_paths: std::collections::HashSet<_> = cli_files.iter().map(|f| &f.path).collect();
        assert_eq!(gix_paths, cli_paths, "changed_files mismatch at {rev}");
    }
}
```

Also create `bca_lib::test_support::differential_repo` — pattern it after `tiny_repo::build()`. Read `crates/bca-lib/src/test_support/tiny_repo.rs` first to match the existing builder shape (typically returns a `Fixture { dir: TempDir }`).

Required fixture properties:
- 50 commits, 10 files
- 1 merge commit
- 1 file rename (`old_name.rs` → `new_name.rs`)
- `.mailmap` with 3 alias mappings (e.g. `Alice <canonical-alice@example.com> Alice <alice-old@example.com>`)
- 3 author identities (Alice, Bob, Carol)
- 1 bot commit (author: `dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>`)

Use `gix::init()` + `gix::ObjectDatabase` to build deterministically (see `tiny_repo::build()` for the existing pattern). Author dates must be fixed (no `Date.now()` or relative dates) to keep the fixture reproducible across runs and platforms.

Commit: `test(lib): differential property tests — GixRepo ≡ GitCliRepo + differential_repo fixture`.

---

### Task 3: Capture code-maat goldens for fixture repos

**Files:**
- Create: `fixtures/golden/code-maat/README.md`
- Create: `fixtures/golden/code-maat/{tiny,differential,bot-noisy}/{revisions,coupling,authors}.csv`
- Create: `scripts/capture-code-maat-goldens.sh` (one-shot script for reproducibility)
- Create: `crates/bca-lib/tests/code_maat_golden_test.rs`

Run code-maat against our fixture repos, capture the output as the golden CSVs. Plan 6 ships the goldens (committed to git) plus the regenerator script. Tests assert byte-equivalence (modulo header-only column reordering) between `bca` output and the goldens for the metrics that code-maat actually computes the same way.

**Methodology honesty caveat:** Some bca outputs *deliberately* differ from code-maat (Fisher exact filter on coupling, Code Health composite, hotspot ranking). For those, document the divergence in `fixtures/golden/code-maat/README.md` and skip the test. Match goldens on:
- `revisions` (must match)
- `authors` (must match)
- `abs-churn`, `author-churn`, `entity-churn` (must match)
- `summary` (must match)
- `coupling` (must match *before* Fisher filter — emit a `--no-fisher-filter` test mode to validate the pre-filter set)
- `code-age` (must match)

Skip (deliberately divergent):
- `hotspots` (bca uses published formula; code-maat doesn't have this)
- `code-health` (bca-original)
- `communication` (subtle date-grouping differences acceptable)

Capture script — code-maat is in `~/Projects/playground/codescene/repomix-dumps/code-maat-source/` per session context. If not present, the script clones it:

```bash
#!/usr/bin/env bash
# scripts/capture-code-maat-goldens.sh
set -euo pipefail

CODE_MAAT="${CODE_MAAT:-/tmp/code-maat}"
if [ ! -d "$CODE_MAAT" ]; then
  git clone --depth=1 https://github.com/adamtornhill/code-maat.git "$CODE_MAAT"
  (cd "$CODE_MAAT" && lein deps)
fi
CMD="lein run -- -l /tmp/logfile.log -c git2 -a"

# Build each fixture programmatically, dump its git log, run code-maat
for FIXTURE in tiny differential bot-noisy; do
  cargo run --quiet --bin bca-fixture-dump -- "$FIXTURE" /tmp/logfile.log
  for ANALYSIS in revisions authors abs-churn author-churn entity-churn summary code-age; do
    cd "$CODE_MAAT"
    lein run -- -l /tmp/logfile.log -c git2 -a "$ANALYSIS" \
      > "fixtures/golden/code-maat/$FIXTURE/$ANALYSIS.csv"
  done
done
```

(`bca-fixture-dump` is a tiny new binary that takes a fixture name + output path and writes a `git2` format log. Either ship it as a bin in `bca-cli` or as a `dev-bin` helper.)

The test:

```rust
// crates/bca-lib/tests/code_maat_golden_test.rs
use bca_lib::analyses::revisions::run_revisions;
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;
use bca_lib::Options;

fn read_golden(fixture: &str, analysis: &str) -> String {
    let path = format!("../../fixtures/golden/code-maat/{fixture}/{analysis}.csv");
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden: {path}"))
}

#[test]
fn revisions_matches_code_maat_on_tiny_repo() {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).unwrap();
    let db = FactsDb::new_in_memory().unwrap();
    let opts = Options { repo_path: tiny.dir.path().to_path_buf(), min_revs: 1, ..Options::default() };
    db.ingest(&repo, &opts).unwrap();

    let rows = run_revisions(&db, &opts).unwrap();
    let mut buf = Vec::new();
    bca_lib::output::csv::write_revisions_csv(&rows, &mut buf).unwrap();
    let bca_out = String::from_utf8(buf).unwrap();

    let golden = read_golden("tiny", "revisions");
    // Sort lines (excluding header) so CSV row order doesn't matter
    assert_eq!(normalize(&bca_out), normalize(&golden));
}

fn normalize(csv: &str) -> Vec<String> {
    let mut lines: Vec<String> = csv.lines().skip(1).map(|s| s.to_string()).collect();
    lines.sort();
    lines
}
```

If code-maat isn't installed in CI, gate the test behind `#[cfg_attr(not(feature = "golden-tests"), ignore)]` and add a `golden-tests` opt-in feature. Run it locally + in a nightly CI job.

Commit: `test(lib): code-maat golden parity tests for revisions/authors/churn/age/summary`.

---

## §2 — Performance benchmarks (Phase 6.B)

### Task 4: `benches/` directory + criterion harness

**Files:**
- Create: `crates/bca-lib/src/test_support/medium_repo.rs` (500-commit generated fixture)
- Modify: `crates/bca-lib/src/test_support/mod.rs` — re-export `medium_repo`
- Create: `crates/bca-lib/benches/end_to_end.rs` (or `benches/end_to_end.rs` if workspace-bench dir preferred — match the existing layout convention)
- Modify: `crates/bca-lib/Cargo.toml` — add `criterion = "0.7"` to dev-deps, add `[[bench]] name = "end_to_end" harness = false required-features = ["test-support"]`
- Create: `.github/workflows/bench.yml`

Three bench targets:
1. `ingest_tiny` — 5 commits, ingestion time. Sub-1ms target.
2. `ingest_medium` — 500 commits programmatically generated, target <100ms.
3. `ingest_linux_kernel_snapshot` — only runs when `BCA_BENCH_LINUX_KERNEL_PATH` env var is set; CI fetches a cached snapshot once a week.

```rust
// benches/end_to_end.rs
use bca_lib::facts::FactsDb;
use bca_lib::repo::GixRepo;
use bca_lib::Options;
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn ingest_tiny(c: &mut Criterion) {
    let tiny = bca_lib::test_support::tiny_repo::build();
    let opts = Options { repo_path: tiny.dir.path().to_path_buf(), ..Options::default() };
    c.bench_function("ingest_tiny", |b| {
        b.iter(|| {
            let repo = GixRepo::open(tiny.dir.path()).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
}

fn ingest_medium(c: &mut Criterion) {
    let medium = bca_lib::test_support::medium_repo::build(); // Build a 500-commit fixture
    let opts = Options { repo_path: medium.dir.path().to_path_buf(), ..Options::default() };
    let mut group = c.benchmark_group("ingest");
    group.sample_size(10); // medium repo takes ~100ms per iter
    group.bench_function("medium_500_commits", |b| {
        b.iter(|| {
            let repo = GixRepo::open(medium.dir.path()).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
    group.finish();
}

fn ingest_linux_kernel_snapshot(c: &mut Criterion) {
    let Some(path) = std::env::var_os("BCA_BENCH_LINUX_KERNEL_PATH") else {
        eprintln!("BCA_BENCH_LINUX_KERNEL_PATH not set — skipping linux kernel bench");
        return;
    };
    let opts = Options { repo_path: path.into(), ..Options::default() };
    let mut group = c.benchmark_group("ingest_kernel");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(120));
    group.bench_function("linux_kernel_1y_snapshot", |b| {
        b.iter(|| {
            let repo = GixRepo::open(&opts.repo_path).unwrap();
            let db = FactsDb::new_in_memory().unwrap();
            db.ingest(black_box(&repo), black_box(&opts)).unwrap();
        });
    });
    group.finish();
}

criterion_group!(benches, ingest_tiny, ingest_medium, ingest_linux_kernel_snapshot);
criterion_main!(benches);
```

GitHub Actions workflow (`bench.yml`):

```yaml
name: bench
on:
  schedule: [{ cron: '0 6 * * MON' }]   # weekly Monday 06:00 UTC
  workflow_dispatch:
jobs:
  bench:
    runs-on: ubuntu-latest-large
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Cache Linux kernel snapshot
        uses: actions/cache@v4
        with:
          path: /tmp/linux-kernel-snapshot
          key: linux-kernel-snapshot-2026
      - name: Fetch kernel snapshot
        run: |
          if [ ! -d /tmp/linux-kernel-snapshot ]; then
            git clone --depth=10000 --filter=blob:none \
              https://github.com/torvalds/linux.git /tmp/linux-kernel-snapshot
          fi
      - name: Run benches
        env:
          BCA_BENCH_LINUX_KERNEL_PATH: /tmp/linux-kernel-snapshot
        run: cargo bench --workspace -- --output-format bencher | tee bench-output.txt
      - name: Regression gate
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: 'cargo'
          output-file-path: bench-output.txt
          alert-threshold: '110%'   # fail if >10% regression
          fail-on-alert: true
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
          comment-on-alert: true
```

Commit: `bench: criterion harness with tiny/medium/kernel benches + weekly CI gate`.

---

### Task 5: Performance release-blocker verification

**Files:**
- Modify: `docs/superpowers/specs/2026-06-06-bca-design.md` — append §11 (release-blocker performance evidence)
- Create: `docs/perf-evidence-v1.md`

Run the kernel bench locally (or in the weekly CI job) and capture:
- Wall time
- Peak RSS
- DuckDB temp-file usage (via `pragma database_size`)

Spec §1.1 commits to:
- Linux kernel hotspot + coupling analysis in **<10 minutes** on M3-class hardware
- Peak memory **<4 GB**
- Stretch: <5 minutes

If either is missed, document the gap, plan the optimization in §11, and either re-scope the v1 target or land the optimization first.

```bash
# Local kernel run — example
git clone --depth=10000 https://github.com/torvalds/linux /tmp/linux-snapshot
/usr/bin/time -v ./target/release/bca analyze --analysis hotspots \
  --repo /tmp/linux-snapshot --min-revs 50 --format parquet \
  --output /tmp/linux-hotspots.parquet
```

Capture the `/usr/bin/time -v` output into `docs/perf-evidence-v1.md`. Commit.

Commit: `docs: v1 perf evidence — Linux kernel hotspot + coupling under release targets`.

---

## §3 — Release infrastructure (Phase 6.C)

### Task 6: `cargo-dist` configuration

**Files:**
- Create: `dist-workspace.toml` (or update `Cargo.toml` with `[workspace.metadata.dist]`)
- Create: `.github/workflows/release.yml` (cargo-dist regenerates this)
- Modify: `crates/bca-cli/Cargo.toml` — set `publish = false` (binary only, not on crates.io for v1.0)

Install `cargo-dist` v0.24+ locally, run `cargo dist init`, walk the interactive prompts:
- Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `x86_64-pc-windows-msvc`
- Installers: shell installer, MSI installer, Homebrew tap
- `cargo binstall` manifest enabled

The init step generates `.github/workflows/release.yml` with the standard cargo-dist flow:
1. On tag push (`v*`), build all targets in parallel
2. Generate SHA256 manifests + signed archives
3. Upload to GitHub release

```bash
cargo install cargo-dist --version "^0.24"
cargo dist init \
  --installer shell \
  --installer powershell \
  --installer msi \
  --installer homebrew \
  --tap your-org/homebrew-bca \
  --hosting github
```

Acceptance: `cargo dist plan` runs cleanly and lists all targets. Don't actually release — that's a v1.0 launch action.

Commit: `release: cargo-dist init for v1.0 multi-platform binaries`.

---

### Task 7: SLSA L3 provenance

**Files:**
- Modify: `.github/workflows/release.yml`

Wire `slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml` into the release workflow so every release artifact gets an attached `.intoto.jsonl` provenance file.

```yaml
# In release.yml, after cargo-dist's archive step:
provenance:
  needs: [build]
  permissions:
    actions: read
    id-token: write
    contents: write
  uses: slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml@v2.1.0
  with:
    base64-subjects: ${{ needs.build.outputs.hashes }}
    upload-assets: true
```

`needs.build.outputs.hashes` is set by a small shell step in the build job that hashes every archive and base64-encodes the result.

Acceptance: cargo-dist's `.github/workflows/release.yml` includes the SLSA generator. Smoke-test by running `actionlint .github/workflows/release.yml`.

Commit: `release: SLSA L3 provenance via slsa-github-generator`.

---

### Task 8: Distroless container image

**Files:**
- Create: `Containerfile` (or `Dockerfile` — Containerfile is the OCI-standard name; either works)
- Create: `.github/workflows/container.yml`

Two-stage build: a `rust:1.89-bookworm` builder + a `gcr.io/distroless/cc-debian12:nonroot` runtime. Static-link where possible (musl target) to minimize runtime requirements.

```Dockerfile
# Containerfile
FROM rust:1.89-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p bca-cli --target x86_64-unknown-linux-gnu

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/x86_64-unknown-linux-gnu/release/bca /usr/local/bin/bca
USER nonroot
ENTRYPOINT ["/usr/local/bin/bca"]
```

Container workflow:

```yaml
name: container
on:
  push: { tags: ['v*'] }
  workflow_dispatch:
jobs:
  build:
    runs-on: ubuntu-latest
    permissions: { contents: read, packages: write, id-token: write }
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/build-push-action@v6
        with:
          context: .
          file: Containerfile
          push: true
          tags: ghcr.io/${{ github.repository }}:${{ github.ref_name }}
          sbom: true
          provenance: true
```

Target image size: <30 MB compressed (DuckDB-bundled binary is the bulk).

Commit: `release: distroless container image with sbom + slsa provenance`.

---

### Task 9: PGO scaffolding (campaign deferred to v1.1)

**Files:**
- Modify: `Cargo.toml` (workspace) — add `[profile.release-pgo]` derived from release
- Create: `scripts/pgo.sh` — manual PGO campaign script

Don't run the PGO campaign in v1.0 release CI (spec §6.5: "campaign starts v1.1 after benchmark suite is stable"). Just leave the scaffolding so v1.1 can flip it on.

```toml
[profile.release-pgo]
inherits = "release"
lto = "fat"
codegen-units = 1
```

```bash
#!/usr/bin/env bash
# scripts/pgo.sh — manual PGO campaign (run before v1.1 tag)
set -euo pipefail
cargo install cargo-pgo
cargo pgo build -- --bin bca
# Run the training workload
./target/x86_64-unknown-linux-gnu/release-pgo/bca analyze \
  --analysis hotspots --repo /tmp/linux-snapshot --output /tmp/training.parquet
cargo pgo optimize build -- --bin bca
```

Commit: `release: PGO scaffolding (campaign deferred to v1.1 per spec §6.5)`.

---

## §4 — Final polish (Phase 6.D)

### Task 10: README, CHANGELOG, version bump to v1.0.0

**Files:**
- Modify: `Cargo.toml` (workspace) — bump `[workspace.package] version = "1.0.0"`
- Modify: `CHANGELOG.md`
- Modify: `README.md`
- Create: `docs/superpowers/specs/2026-06-06-bca-design.md` — flip §1 status from "Spine v1 (draft for sign-off)" to "Spine v1 (released)"

CHANGELOG entry for Plan 6:

```markdown
### Added (Plan 6: Differential Testing + Perf + Release Infra)
- **Differential testing harness**: `GitCliRepo` shell-out impl of the `Repo` trait,
  property tests asserting `GixRepo ≡ GitCliRepo` on a 50-commit generated fixture
  (commit walk, mailmap resolution, blob read, merge detection)
- **Code-maat golden parity**: revisions/authors/abs-churn/author-churn/entity-churn/
  code-age/summary outputs match code-maat byte-for-byte (modulo row order). Hotspots,
  Code Health, and post-Fisher coupling deliberately diverge — see fixtures/golden/code-maat/README.md
- **Criterion benchmarks** (`benches/end_to_end.rs`): tiny / medium-500 / linux-kernel-snapshot.
  Weekly CI job tracks regression; >10% PR regressions fail the bench gate.
- **Release-blocker perf evidence** (`docs/perf-evidence-v1.md`): Linux kernel
  hotspot + coupling analysis completes in <X minutes on M3-class hardware, peak RSS <Y GB.
- **`cargo-dist` release pipeline** with shell + PowerShell + MSI + Homebrew installers
- **SLSA L3 provenance** on every release artifact via slsa-github-generator
- **Distroless container image** (`ghcr.io/.../bca`) — <30 MB compressed, nonroot, signed
- **PGO scaffolding** (campaign deferred to v1.1 per spec §6.5)
```

README update — bump status, link to perf evidence, link to release page.

Commit: `release: v1.0.0 — Plan 6 done, docs + version bump`.

---

### Task 11: Tag v1.0.0

**Files:** none (only git operations)

```bash
git tag -s v1.0.0 -m "v1.0.0 — Spine release. See CHANGELOG.md for details."
git push origin main --tags
```

(`-s` requires a signed commit — if GPG isn't set up on the dev machine, use `git tag -a` instead; the release workflow signs the artifacts.)

This triggers `release.yml` (cargo-dist) and `container.yml`. Wait for both to succeed; verify artifacts on the GitHub release page; verify `cargo binstall bca` works.

Commit: tag only, no file changes.

---

## Plan 6 Definition of Done

- [ ] `GitCliRepo` lives at `crates/bca-lib/src/repo/git_cli_repo.rs` and implements every `Repo` method
- [ ] Differential property tests pass (`GixRepo ≡ GitCliRepo` on a 50-commit generated fixture)
- [ ] Code-maat goldens for tiny/differential/bot-noisy committed under `fixtures/golden/code-maat/`
- [ ] Code-maat golden parity tests pass for revisions/authors/abs-churn/author-churn/entity-churn/code-age/summary
- [ ] `benches/end_to_end.rs` has tiny/medium/kernel targets; `cargo bench` runs locally
- [ ] Weekly CI bench job at `.github/workflows/bench.yml`, with regression gate
- [ ] `docs/perf-evidence-v1.md` documents the Linux kernel run meeting <10 min / <4 GB targets
- [ ] `cargo dist init` complete; `.github/workflows/release.yml` generated and committed
- [ ] SLSA L3 provenance wired into the release workflow
- [ ] `Containerfile` + `.github/workflows/container.yml` build a distroless image <30 MB
- [ ] PGO scaffolding in place (`scripts/pgo.sh`, `[profile.release-pgo]`); no campaign in v1.0
- [ ] CHANGELOG + README updated; workspace version bumped to 1.0.0
- [ ] Spec §1 status flipped to "released"
- [ ] All previous tests + Plan 6 tests pass
- [ ] clippy/fmt/deny clean
- [ ] `v1.0.0` tag pushed; release workflow succeeds; artifacts verified

---

*End of Plan 6. After Plan 6: v1.0 has shipped. v1.1 picks up PGO campaign, mutation testing in CI, and the deferred features tracked in spec §8.*
