//! Test fixtures. Public so CLI integration tests can reuse.
//!
//! Every fixture extracts its repo from a checked-in git bundle via a single
//! atomic `git clone` into a fresh tempdir. Capturing the fixture once as a
//! bundle (rather than rebuilding it per test via a chain of `git` shell-outs)
//! removes the multi-process construction race that intermittently corrupted
//! the object store on loaded CI runners, and collapses hundreds of `git`
//! invocations per build down to one `clone`.
//!
//! `differential_repo::build()` extracts a 50-commit repo; used to assert
//! `GixRepo` ≡ `GitCliRepo`.

/// Coupling-permissive `Options` so a small fixture's co-change pairs
/// survive into a non-empty graph: every p-value allowed
/// (`fisher_significance = 1.0`), no degree floor/ceiling, `min_revs = 1`.
/// Shared by the centrality + communities end-to-end tests, which both
/// need the differential fixture's pairs to reach the graph builders.
#[cfg(feature = "test-support")]
#[must_use]
pub fn permissive_coupling_opts(repo_path: std::path::PathBuf) -> crate::Options {
    crate::Options {
        repo_path,
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        fisher_significance: 1.0,
        ..crate::Options::default()
    }
}

/// Write `BUNDLE` to an OS-temp file outside `target`, `git clone` it into
/// `target`, and return the clone's HEAD SHA. Shared by the bundle-backed
/// fixtures below.
///
/// The bundle bytes land in a `NamedTempFile` in the OS temp dir (not inside
/// `target`) so `git clone` sees an empty destination; that file auto-deletes
/// when it drops at the end of this function — the clone no longer needs it.
///
/// # Panics
///
/// Panics if the scratch tempfile cannot be created/written or if `git clone`
/// from the bundle fails (either indicates a broken local git install, not a
/// fixture issue).
#[cfg(feature = "test-support")]
fn clone_bundle(bundle: &[u8], target: &std::path::Path) -> String {
    let bundle_file = tempfile::NamedTempFile::new().expect("bundle scratch tempfile");
    std::fs::write(bundle_file.path(), bundle).expect("write bundle bytes");

    let status = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(bundle_file.path())
        .arg(target)
        .status()
        .expect("spawn git clone");
    assert!(status.success(), "git clone from embedded bundle failed");
    drop(bundle_file);

    String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(target)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("spawn git rev-parse")
            .stdout,
    )
    .expect("utf8")
    .trim()
    .to_string()
}

#[cfg(feature = "test-support")]
pub mod tiny_repo {
    //! A tiny 5-commit repo with distinct per-commit timestamps.
    //!
    //! Timestamps matter: Kamei `enrich_history` / `enrich_experience` / SEXP
    //! all use **strict** `prev.date < c.date` semantics, so same-second peers
    //! don't count as priors. The fixture's commits land on
    //! 2026-06-01..2026-06-05 (one per day) so every Kamei history / experience
    //! metric reads non-zero.
    //!
    //! ## Regenerating the bundle
    //!
    //! The fixture is captured once into a checked-in git bundle
    //! (`src/test_support/data/tiny-repo.bundle`; HEAD
    //! `38e219d9d448be9da489d96cc59dcb0bb1d37824`, deterministic across
    //! regenerations because every author / date / file content is fixed). To
    //! regenerate (e.g. when the commit shape must change), revive the
    //! pre-bundle programmatic builder from git history at commit `b412e6d`
    //! (it shells out to `git` for each commit and lived in this module), run
    //! it to produce a fresh repo, then capture:
    //!
    //! ```text
    //! git -C <fresh-repo> bundle create \
    //!     crates/codelore-lib/src/test_support/data/tiny-repo.bundle --all
    //! ```
    //!
    //! Commit the updated bundle.

    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    static BUNDLE: &[u8] = include_bytes!("data/tiny-repo.bundle");

    pub struct TinyRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Extract the tiny 5-commit fixture from the embedded bundle into a fresh
    /// tempdir. Single atomic `git clone` — no multi-process race surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the bundle
    /// fails (either case indicates a broken local git install, not a fixture
    /// issue).
    #[must_use]
    pub fn build() -> TinyRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let head_sha = super::clone_bundle(BUNDLE, dir.path());
        TinyRepo { dir, head_sha }
    }
}

#[cfg(feature = "test-support")]
pub mod biomarker_repo {
    //! A repo with a deliberate complexity gradient plus a duplicated function
    //! pair and co-changed files, so the `code-health` biomarker layer produces
    //! a SPREAD of `structural_risk` and fires several distinct smells
    //! (complex-method, large-method, dry, and — via co-change — shotgun).
    //! Small fixtures like `tiny_repo` are too homogeneous to exercise the
    //! metric's distribution; this one is purpose-built for that.
    //!
    //! Six commits (2026-06-01..2026-06-06): a seed writing the whole
    //! complexity gradient, then co-changing pairs (complex+moderate, then the
    //! duplicated `dup_a`/`dup_b` pair) to create temporal coupling.
    //!
    //! ## Regenerating the bundle
    //!
    //! The fixture is captured once into a checked-in git bundle
    //! (`src/test_support/data/biomarker-repo.bundle`; HEAD
    //! `fc3edfb3435c690a87750c8fe0050a2497d75b60`, deterministic across
    //! regenerations because every author / date / file content is fixed). To
    //! regenerate, revive the pre-bundle programmatic builder from git history
    //! at commit `b412e6d` (it shells out to `git` for each commit and lived in
    //! this module), run it to produce a fresh repo, then capture:
    //!
    //! ```text
    //! git -C <fresh-repo> bundle create \
    //!     crates/codelore-lib/src/test_support/data/biomarker-repo.bundle --all
    //! ```
    //!
    //! Commit the updated bundle.

    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    static BUNDLE: &[u8] = include_bytes!("data/biomarker-repo.bundle");

    pub struct BiomarkerRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Extract the biomarker fixture from the embedded bundle into a fresh
    /// tempdir. Single atomic `git clone` — no multi-process race surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the bundle
    /// fails (either case indicates a broken local git install, not a fixture
    /// issue).
    #[must_use]
    pub fn build() -> BiomarkerRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let head_sha = super::clone_bundle(BUNDLE, dir.path());
        BiomarkerRepo { dir, head_sha }
    }
}

/// Fixture for `function-xray` and `function-coupling` tests.
///
/// Contains a single Rust file `src/target.rs` with three functions:
/// - `hot` — 11-line function touched in every non-seed commit
/// - `cold` — touched only in the three coupled commits
/// - `meta` — touched only in `meta-tweak-1..10`; provides `neither = 10` in
///   the Fisher table for `(hot, cold)`, driving p < 0.1
///
/// Commit history:
///   seed              — writes all three functions
///   tweak-1           — single-hunk edit of `hot` (line 2)
///   tweak-2           — single-hunk edit of `hot` (line 2)
///   tweak-3           — single-hunk edit of `hot` (line 2)
///   tweak-mh          — two-hunk edit of `hot` (lines 2 and 10) in ONE commit
///   coupled-1         — changes BOTH `hot` (line 10) AND `cold`
///   coupled-2         — changes BOTH `hot` (line 10) AND `cold`
///   coupled-3         — changes BOTH `hot` (line 10) AND `cold`
///   meta-tweak-1..10  — each changes ONLY `meta`; hot and cold untouched
///
/// `tweak-mh` verifies that a multi-hunk commit counts as exactly one
/// revision for `change_freq`, not one per hunk.
///
/// `coupled-1/2/3` verify that `function-coupling` detects the `(hot, cold)`
/// pair with `co_changes = 3` and `confidence = 1.0`.
///
/// `meta-tweak-1..10` give the Fisher `(hot, cold)` table `neither = 10`,
/// making the table non-degenerate and driving `p ≈ 0.051 < 0.1`.
///
/// `hot` is an 11-line function so that edits on line 2 and line 10 are far
/// enough apart to guarantee two separate hunks in a single commit. The gap
/// (lines 3–9 = 7 lines) exceeds 2× git's default context window (2×3 = 6),
/// so the diff engine cannot bridge the two edit regions into one hunk. The
/// `n = 17` accounting (seed is an Add → no hunks; tweak-1/2/3 + tweak-mh give
/// `a_only = 4`; coupled-1/2/3 give `co = 3`; meta-tweak-1..10 give
/// `neither = 10`) yields Fisher table `[3, 4, 0, 10]` with `p ≈ 0.051 < 0.1`.
///
/// ## Regenerating the bundle
///
/// The fixture is captured once into a checked-in git bundle
/// (`src/test_support/data/function-xray-repo.bundle`; HEAD
/// `c84e622ba6f7687ac051003fa3fc4ebffeb0a7ee`, deterministic across
/// regenerations because every author / date / file content is fixed). To
/// regenerate (e.g. when the commit shape or the Fisher accounting must
/// change), revive the pre-bundle programmatic builder from git history at
/// commit `b412e6d` (it shells out to `git` for each commit, encodes the
/// `HOT_*` / `COLD_*` / `META_*` source versions, and lived in this
/// module), run it to produce a fresh repo, then capture:
///
/// ```text
/// git -C <fresh-repo> bundle create \
///     crates/codelore-lib/src/test_support/data/function-xray-repo.bundle --all
/// ```
///
/// Commit the updated bundle.
#[cfg(feature = "test-support")]
pub mod function_xray_repo {
    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    static BUNDLE: &[u8] = include_bytes!("data/function-xray-repo.bundle");

    pub struct FunctionXrayRepo {
        pub dir: TempDir,
    }

    /// Extract the function-xray fixture from the embedded bundle into a fresh
    /// tempdir. Single atomic `git clone` — no multi-process race surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the bundle
    /// fails (either case indicates a broken local git install, not a fixture
    /// issue).
    #[must_use]
    pub fn build() -> FunctionXrayRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let _head_sha = super::clone_bundle(BUNDLE, dir.path());
        FunctionXrayRepo { dir }
    }
}

#[cfg(feature = "test-support")]
pub mod differential_repo {
    //! 50-commit fixture exercising every `Repo`-trait method's edge cases:
    //! 3 authors + 1 bot, `.mailmap`, 1 rename, 1 merge, deterministic dates.
    //!
    //! ## Root-cause-fix history
    //!
    //! This fixture used to be rebuilt from scratch per test via ~100 sequential
    //! `git` shell-outs (`git init`, then 50 × `git add` + `git commit`).
    //! That pattern is timing-dependent: the kernel page cache and git's
    //! object store can briefly disagree between separate git processes,
    //! intermittently producing `error: invalid object … Error building
    //! trees` or `fatal: could not parse HEAD` on CI runners under
    //! filesystem-cache pressure. A pile of mitigations (`gc.auto = 0`,
    //! in-process mutex, cross-process file lock, `core.fsync`) each fixed
    //! some races but not all — because the actual root cause is the
    //! multi-process construction pattern itself.
    //!
    //! Current design: the fixture is captured **once** into a checked-in
    //! git bundle artifact (`src/test_support/data/differential-repo.bundle`,
    //! ~15 KB, deterministic SHA across regenerations because all author
    //! names / emails / dates / file contents are fixed). Each test does
    //! ONE `git clone` of the bundle into a fresh tempdir — a single
    //! atomic git invocation with no inter-process state to race over.
    //! No mutex, no file lock, no fsync knob, no gc disabling needed.
    //!
    //! To regenerate the bundle (e.g. when the fixture's commit shape
    //! needs to change), revive the pre-bundle programmatic builder from
    //! git history at commit `9df7a42` (it shells out to git for each
    //! commit), run it to produce a fresh repo, then capture as:
    //!
    //! ```text
    //! git -C <fresh-repo> bundle create \
    //!     crates/codelore-lib/src/test_support/data/differential-repo.bundle --all
    //! ```
    //!
    //! Commit the updated bundle. A proper non-shell-out regenerator
    //! (e.g. via `git fast-import` or `gix-object` write APIs) is a
    //! reasonable follow-up.

    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    /// 50 commits, deterministic SHAs (HEAD ≈ `64ef547f…` at last regen).
    static BUNDLE: &[u8] = include_bytes!("data/differential-repo.bundle");

    pub struct DifferentialRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Extract the 50-commit fixture from the embedded bundle into a fresh
    /// tempdir. Single atomic `git clone` — no multi-process race surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the
    /// bundle fails (either case indicates a broken local git install,
    /// not a fixture issue).
    #[must_use]
    pub fn build() -> DifferentialRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let head_sha = super::clone_bundle(BUNDLE, dir.path());
        DifferentialRepo { dir, head_sha }
    }
}

#[cfg(feature = "test-support")]
pub mod coupling_repo {
    //! A fixture with deliberate cross-module co-changes so the depth-2
    //! coupling sankey produces visible edges.
    //!
    //! Structure: 3 modules (`src/alpha`, `src/beta`, `src/gamma`), each with two
    //! files. `alpha/svc.rs` and `beta/svc.rs` co-change together in ≥5 commits,
    //! guaranteeing a `src/alpha`↔`src/beta` edge at depth 2 even under the most
    //! conservative coupling thresholds. Additional single-file commits ensure
    //! every file accrues enough churn to appear in the hotspot table. 19
    //! commits total, one per day 2026-06-01..2026-06-19.
    //!
    //! ## Regenerating the bundle
    //!
    //! The fixture is captured once into a checked-in git bundle
    //! (`src/test_support/data/coupling-repo.bundle`; HEAD
    //! `751dc3a192917070782946dabbc8317f196b9cbf`, deterministic across
    //! regenerations because every author / date / file content is fixed). To
    //! regenerate (e.g. when the co-change history the module-depth test relies
    //! on must change), revive the pre-bundle programmatic builder from git
    //! history at commit `b412e6d` (it shells out to `git` for each commit and
    //! lived in this module), run it to produce a fresh repo, then capture:
    //!
    //! ```text
    //! git -C <fresh-repo> bundle create \
    //!     crates/codelore-lib/src/test_support/data/coupling-repo.bundle --all
    //! ```
    //!
    //! Commit the updated bundle.

    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    static BUNDLE: &[u8] = include_bytes!("data/coupling-repo.bundle");

    pub struct CouplingRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Extract the cross-module co-change fixture from the embedded bundle into
    /// a fresh tempdir. Single atomic `git clone` — no multi-process race
    /// surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the bundle
    /// fails (either case indicates a broken local git install, not a fixture
    /// issue).
    #[must_use]
    pub fn build() -> CouplingRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let head_sha = super::clone_bundle(BUNDLE, dir.path());
        CouplingRepo { dir, head_sha }
    }
}

#[cfg(feature = "test-support")]
pub mod medium_repo {
    //! 500-commit fixture for criterion benchmarks. Heavier than `differential_repo`
    //! (which is 50 commits, optimized for differential-test edge-case coverage) but
    //! still small enough for a CI bench to iterate on quickly.
    //!
    //! Structure: 500 commits, 3 authors, 25 files, round-robin author×file, no
    //! merges, no renames, deterministic dates. Intended use: measure ingest+walk
    //! throughput without any of the edge cases `differential_repo` exercises.
    //!
    //! ## Regenerating the bundle
    //!
    //! The fixture is captured once into a checked-in git bundle
    //! (`src/test_support/data/medium-repo.bundle`, ~197 KB; HEAD
    //! `40c13e73bdb6c397a17dda5e2bbb9f1a332e0fd1`, deterministic across
    //! regenerations because every author / date / file content is fixed). To
    //! regenerate (e.g. when the commit / file count must change), revive the
    //! pre-bundle programmatic builder from git history at commit `b412e6d`
    //! (it shells out to `git` for each of the 500 commits and lived in this
    //! module), run it to produce a fresh repo, then capture:
    //!
    //! ```text
    //! git -C <fresh-repo> bundle create \
    //!     crates/codelore-lib/src/test_support/data/medium-repo.bundle --all
    //! ```
    //!
    //! Commit the updated bundle.

    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    /// 500 commits, deterministic SHAs (HEAD ≈ `40c13e73…` at last regen).
    static BUNDLE: &[u8] = include_bytes!("data/medium-repo.bundle");

    pub struct MediumRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Extract the 500-commit bench fixture from the embedded bundle into a
    /// fresh tempdir. Single atomic `git clone` — no multi-process race
    /// surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the bundle
    /// fails (either case indicates a broken local git install, not a fixture
    /// issue).
    #[must_use]
    pub fn build() -> MediumRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let head_sha = super::clone_bundle(BUNDLE, dir.path());
        MediumRepo { dir, head_sha }
    }
}

#[cfg(feature = "test-support")]
pub mod delivery_repo {
    //! A fixture covering delivery-metrics edge cases: 3 authors
    //! (Alice/Bob/Carol), annotated tags at ~day 10/50/110 plus one
    //! non-matching tag (`nightly-1`), a lightweight tag (`light-1`) sharing
    //! `v1.0.0`'s commit, two `--no-ff` merges with measurable branch
    //! durations, author-vs-committer date gaps on 2 commits, one rework
    //! signal (file modified then partially deleted within 5 days), and one
    //! file churned 62 days after first touch (outside the 21-day rework
    //! window).
    //!
    //! Merge commits are only ingested when `Options.include_merges` is true —
    //! the default drops them at the walker before they reach the `DuckDB`
    //! Appender, so `is_merge` rows will not appear with `Options::default()`.
    //!
    //! ## Root-cause-fix history
    //!
    //! This fixture used to be rebuilt from scratch per test via ~40 sequential
    //! `git` shell-outs (`git init`, then repeated `git add` + `git commit`).
    //! That multi-process construction is timing-dependent: on loaded runners
    //! separate git processes can briefly disagree about the object store,
    //! intermittently producing `git add` failures (`unable to create
    //! temporary file` / `failed to insert into database`) under filesystem
    //! pressure. The actual root cause is the multi-process pattern itself, so
    //! per-step mitigations (`gc.auto = 0`, `repack`) never removed it fully.
    //!
    //! Current design: the fixture is captured **once** into a checked-in git
    //! bundle artifact (`src/test_support/data/delivery-repo.bundle`,
    //! deterministic content SHAs across regenerations because every author
    //! name / email / date / file content is fixed). Each test does ONE
    //! `git clone` of the bundle into a fresh tempdir — a single atomic git
    //! invocation with no inter-process state to race over. The clone carries
    //! all 5 tags (git auto-follows tags pointing at fetched commits) and the
    //! full merge history, so consumers see identical refs and HEAD SHA.
    //!
    //! To regenerate the bundle (e.g. when the fixture's commit shape needs to
    //! change), revive the pre-bundle programmatic builder from git history at
    //! commit `8cfb373` (it shells out to git for each commit and lives in this
    //! module), run it to produce a fresh repo, then capture as:
    //!
    //! ```text
    //! git -C <fresh-repo> bundle create \
    //!     crates/codelore-lib/src/test_support/data/delivery-repo.bundle --all
    //! ```
    //!
    //! The `--all` flag captures every branch and tag the tests rely on. Commit
    //! the updated bundle.

    use tempfile::TempDir;

    /// The fixture's git bundle, captured once and embedded at compile time.
    /// 3 authors, 2 merges, 5 tags; deterministic SHAs (HEAD ≈ `3b23238e…`
    /// at last regen).
    static BUNDLE: &[u8] = include_bytes!("data/delivery-repo.bundle");

    pub struct DeliveryRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Extract the delivery-metrics fixture from the embedded bundle into a
    /// fresh tempdir. Single atomic `git clone` — no multi-process race
    /// surface.
    ///
    /// # Panics
    ///
    /// Panics if `tempfile::tempdir` fails or if `git clone` from the bundle
    /// fails (either case indicates a broken local git install, not a fixture
    /// issue).
    #[must_use]
    pub fn build() -> DeliveryRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let head_sha = super::clone_bundle(BUNDLE, dir.path());
        DeliveryRepo { dir, head_sha }
    }
}
