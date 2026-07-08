//! Test fixtures. Public so CLI integration tests can reuse.
//!
//! `tiny_repo::build()` programmatically constructs a 5-commit repo via
//! shell-out to `git` so behavior is exactly reproducible. We use git CLI
//! (not gix-write) because gix-write is still maturing for trivial init in 0.84,
//! and shell-out is fast + predictable for tests.
//!
//! `differential_repo::build()` extracts a 50-commit repo from a checked-in
//! git bundle via a single atomic `git clone`. Used to assert
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

#[cfg(feature = "test-support")]
pub mod tiny_repo {
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct TinyRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Build a tiny 5-commit repository for testing.
    ///
    /// # Panics
    ///
    /// Panics if the OS cannot create a temporary directory or if any `git` command fails.
    #[must_use]
    pub fn build() -> TinyRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "tiny@example.com"]);
        run_git(&path, &["config", "user.name", "Tiny"]);

        // Distinct per-commit timestamps. Kamei `enrich_history` /
        // `enrich_experience` / SEXP all use **strict** `prev.date <
        // c.date` semantics; same-second peers don't count as priors.
        // Without explicit dates, all 5 commits land at the wall-clock
        // second of fixture construction and every Kamei history /
        // experience metric reads 0 — masking real-world behaviour.
        let dates = [
            "2026-06-01T10:00:00Z",
            "2026-06-02T10:00:00Z",
            "2026-06-03T10:00:00Z",
            "2026-06-04T10:00:00Z",
            "2026-06-05T10:00:00Z",
        ];

        write(&path, "src/main.rs", "fn main() {}\n");
        run_git(&path, &["add", "."]);
        run_git_at(&path, dates[0], &["commit", "-m", "init", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hi\"); }\n");
        run_git_at(&path, dates[1], &["commit", "-am", "say hi", "--quiet"]);

        write(&path, "src/lib.rs", "pub fn greet() {}\n");
        run_git(&path, &["add", "."]);
        run_git_at(&path, dates[2], &["commit", "-m", "add lib", "--quiet"]);

        write(&path, "src/main.rs", "fn main() { println!(\"hello\"); }\n");
        run_git_at(&path, dates[3], &["commit", "-am", "fix typo", "--quiet"]);

        write(
            &path,
            "src/main.rs",
            "fn main() { println!(\"hello, world\"); }\n",
        );
        run_git_at(
            &path,
            dates[4],
            &["commit", "-am", "expand greeting", "--quiet"],
        );

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
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    /// `run_git` variant that pins `GIT_AUTHOR_DATE` /
    /// `GIT_COMMITTER_DATE` so the commit lands at a deterministic
    /// timestamp. Required for Kamei history / experience metrics
    /// (which use strict `prev.date < c.date` semantics) to be
    /// non-zero on a manufactured fixture.
    fn run_git_at(path: &std::path::Path, iso_date: &str, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", iso_date)
            .env("GIT_COMMITTER_DATE", iso_date)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} (at {iso_date}) failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
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
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct BiomarkerRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    // A trivial function: cyclomatic 1, cognitive 0, few lines.
    const TRIVIAL: &str = "pub fn trivial() -> i32 {\n    1\n}\n";

    // A moderately-branchy function.
    const MODERATE: &str = "pub fn moderate(x: i32, y: i32) -> i32 {\n    let mut r = 0;\n    for i in 0..x {\n        if i % 2 == 0 {\n            r += i;\n        } else {\n            r -= i;\n        }\n    }\n    if y > 0 {\n        r += y;\n    }\n    r\n}\n";

    // A deeply-nested, high-cyclomatic function.
    const COMPLEX: &str = "pub fn complex(a: i32, b: i32, c: i32) -> i32 {\n    let mut total = 0;\n    for i in 0..a {\n        if i % 2 == 0 {\n            for j in 0..b {\n                if j > c {\n                    if j % 3 == 0 {\n                        total += j;\n                    } else if j % 5 == 0 {\n                        total -= j;\n                    } else {\n                        total += 1;\n                    }\n                } else if j < 0 {\n                    total -= 1;\n                }\n            }\n        } else {\n            match i % 4 {\n                0 => total += 1,\n                1 => total -= 1,\n                2 => total *= 2,\n                _ => total = 0,\n            }\n        }\n    }\n    total\n}\n";

    // A large but flat function (many statements, little branching): high LOC.
    fn big_source() -> String {
        let mut s = String::from("pub fn big() -> i64 {\n    let mut acc: i64 = 0;\n");
        for i in 0..60 {
            s.push_str("    acc += ");
            s.push_str(&i.to_string());
            s.push_str(";\n");
        }
        s.push_str("    acc\n}\n");
        s
    }

    // An identical non-trivial function, written into two files → clone (DRY).
    const DUPLICATED: &str = "pub fn shared_logic(items: &[i32]) -> i32 {\n    let mut sum = 0;\n    for it in items {\n        if *it > 0 {\n            sum += it;\n        } else {\n            sum -= it;\n        }\n    }\n    sum\n}\n";

    /// Build the biomarker fixture.
    ///
    /// # Panics
    ///
    /// Panics if the OS cannot create a temporary directory or any `git`
    /// command fails.
    #[must_use]
    pub fn build() -> BiomarkerRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "bio@example.com"]);
        run_git(&path, &["config", "user.name", "Bio"]);

        let dates = [
            "2026-06-01T10:00:00Z",
            "2026-06-02T10:00:00Z",
            "2026-06-03T10:00:00Z",
            "2026-06-04T10:00:00Z",
            "2026-06-05T10:00:00Z",
            "2026-06-06T10:00:00Z",
        ];

        // Seed the complexity gradient.
        write(&path, "src/trivial.rs", TRIVIAL);
        write(&path, "src/moderate.rs", MODERATE);
        write(&path, "src/complex.rs", COMPLEX);
        write(&path, "src/big.rs", &big_source());
        write(&path, "src/dup_a.rs", DUPLICATED);
        write(&path, "src/dup_b.rs", DUPLICATED);
        run_git(&path, &["add", "."]);
        run_git_at(&path, dates[0], &["commit", "-m", "seed", "--quiet"]);

        // Several edit commits, co-changing pairs to create temporal coupling
        // (complex+moderate change together; the dup pair changes together).
        write(&path, "src/complex.rs", &format!("{COMPLEX}// tweak 1\n"));
        write(&path, "src/moderate.rs", &format!("{MODERATE}// tweak 1\n"));
        run_git_at(
            &path,
            dates[1],
            &["commit", "-am", "co-change 1", "--quiet"],
        );

        write(&path, "src/complex.rs", &format!("{COMPLEX}// tweak 2\n"));
        write(&path, "src/moderate.rs", &format!("{MODERATE}// tweak 2\n"));
        run_git_at(
            &path,
            dates[2],
            &["commit", "-am", "co-change 2", "--quiet"],
        );

        write(&path, "src/dup_a.rs", &format!("{DUPLICATED}// da\n"));
        write(&path, "src/dup_b.rs", &format!("{DUPLICATED}// db\n"));
        run_git_at(
            &path,
            dates[3],
            &["commit", "-am", "dup co-change 1", "--quiet"],
        );

        write(&path, "src/dup_a.rs", &format!("{DUPLICATED}// da2\n"));
        write(&path, "src/dup_b.rs", &format!("{DUPLICATED}// db2\n"));
        run_git_at(
            &path,
            dates[4],
            &["commit", "-am", "dup co-change 2", "--quiet"],
        );

        write(&path, "src/complex.rs", &format!("{COMPLEX}// tweak 3\n"));
        run_git_at(
            &path,
            dates[5],
            &["commit", "-am", "touch complex", "--quiet"],
        );

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

        BiomarkerRepo { dir, head_sha }
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn run_git_at(path: &std::path::Path, iso_date: &str, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", iso_date)
            .env("GIT_COMMITTER_DATE", iso_date)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} (at {iso_date}) failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
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
#[cfg(feature = "test-support")]
pub mod function_xray_repo {
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct FunctionXrayRepo {
        pub dir: TempDir,
    }

    // `hot` is an 11-line function so that edits on line 2 and line 10 are
    // far enough apart to guarantee two separate hunks in a single commit.
    // Gap = lines 3–9 = 7 lines, which exceeds 2 × 3 (git's default context
    // window) = 6. A gap > 2×context guarantees the diff engine cannot bridge
    // the two edit regions into one hunk.
    //
    // Layout of src/target.rs:
    //   line 1:  pub fn hot() -> i32 {
    //   line 2:    let a = <A>;        ← edit region 1 (tweak-1/2/3 + tweak-mh)
    //   line 3:    let b = 10;
    //   line 4:    let c = 20;
    //   line 5:    let d = 30;         ← gap: 7 lines > 2×context(3)=6
    //   line 6:    let e = 40;
    //   line 7:    let f = 50;
    //   line 8:    let g = 60;
    //   line 9:    let h = 70;
    //   line 10:   <B>                 ← edit region 2 (tweak-mh + coupled)
    //   line 11: }
    //   line 12: (blank separator)
    //   line 13: pub fn cold() -> i32 {
    //   line 14:   42                  ← edit region (coupled commits)
    //   line 15: }
    //   line 16: (blank separator)
    //   line 17: pub fn meta() -> i32 {
    //   line 18:   0                   ← edit region (meta-tweak only)
    //   line 19: }
    //
    // Versions v0–v3 only change line 2 (single hunk per commit).
    // tweak-mh changes BOTH line 2 (a = 99) AND line 10 (return value) in
    // ONE commit → two hunks, but still only +1 to change_freq.
    // coupled-1/2/3 change line 10 of `hot` AND line 14 of `cold` together.
    // meta-tweak changes ONLY line 18 (`meta` body); hot and cold are untouched.

    const HOT_V0: &str = "pub fn hot() -> i32 {\n    let a = 0;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    a + b + c + d + e + f + g + h\n}\n";
    const HOT_V1: &str = "pub fn hot() -> i32 {\n    let a = 1;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    a + b + c + d + e + f + g + h\n}\n";
    const HOT_V2: &str = "pub fn hot() -> i32 {\n    let a = 2;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    a + b + c + d + e + f + g + h\n}\n";
    const HOT_V3: &str = "pub fn hot() -> i32 {\n    let a = 3;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    a + b + c + d + e + f + g + h\n}\n";
    // Multi-hunk: changes line 2 (a = 99) AND line 10 (return value).
    // Gap = 7 lines (lines 3-9) > 2×context(3)=6 → guaranteed two separate hunks.
    const HOT_VMH: &str = "pub fn hot() -> i32 {\n    let a = 99;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    99\n}\n";
    // Coupling versions: change line 10 (return value) in hot for each coupled commit.
    const HOT_VCPL1: &str = "pub fn hot() -> i32 {\n    let a = 99;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    100\n}\n";
    const HOT_VCPL2: &str = "pub fn hot() -> i32 {\n    let a = 99;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    101\n}\n";
    const HOT_VCPL3: &str = "pub fn hot() -> i32 {\n    let a = 99;\n    let b = 10;\n    let c = 20;\n    let d = 30;\n    let e = 40;\n    let f = 50;\n    let g = 60;\n    let h = 70;\n    102\n}\n";
    // `cold` versions for coupled commits — body changes on line 14.
    const COLD_V0: &str = "pub fn cold() -> i32 {\n    42\n}\n";
    const COLD_V1: &str = "pub fn cold() -> i32 {\n    43\n}\n";
    const COLD_V2: &str = "pub fn cold() -> i32 {\n    44\n}\n";
    const COLD_V3: &str = "pub fn cold() -> i32 {\n    45\n}\n";
    // `meta` versions: only the body differs; meta-tweak changes V0 → V1.
    // Neither `hot` nor `cold` is touched in meta-tweak, so the Fisher table
    // for the (hot, cold) pair gets `neither = 1` (this revision touches
    // neither of them), making the table non-degenerate.
    const META_V0: &str = "pub fn meta() -> i32 {\n    0\n}\n";
    const META_V1: &str = "pub fn meta() -> i32 {\n    1\n}\n";

    fn src(hot: &str, cold: &str, meta: &str) -> String {
        format!("{hot}\n{cold}\n{meta}")
    }

    /// # Panics
    ///
    /// Panics if the OS cannot create a temp directory or any `git`
    /// command fails.
    #[must_use]
    pub fn build() -> FunctionXrayRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "xray@example.com"]);
        run_git(&path, &["config", "user.name", "XRay"]);

        let dates = [
            "2026-06-01T10:00:00Z",
            "2026-06-02T10:00:00Z",
            "2026-06-03T10:00:00Z",
            "2026-06-04T10:00:00Z",
            "2026-06-05T10:00:00Z",
            "2026-06-06T10:00:00Z",
            "2026-06-07T10:00:00Z",
            "2026-06-08T10:00:00Z",
        ];

        // Seed: write all three functions.
        write(&path, "src/target.rs", &src(HOT_V0, COLD_V0, META_V0));
        run_git(&path, &["add", "."]);
        run_git_at(&path, dates[0], &["commit", "-m", "seed", "--quiet"]);

        // tweak-1: mutate hot's body (line 2 only → single hunk).
        write(&path, "src/target.rs", &src(HOT_V1, COLD_V0, META_V0));
        run_git_at(&path, dates[1], &["commit", "-am", "tweak-1", "--quiet"]);

        // tweak-2: mutate hot's body again (line 2 only → single hunk).
        write(&path, "src/target.rs", &src(HOT_V2, COLD_V0, META_V0));
        run_git_at(&path, dates[2], &["commit", "-am", "tweak-2", "--quiet"]);

        // tweak-3: mutate hot's body a third time (line 2 only → single hunk).
        write(&path, "src/target.rs", &src(HOT_V3, COLD_V0, META_V0));
        run_git_at(&path, dates[3], &["commit", "-am", "tweak-3", "--quiet"]);

        // tweak-mh: change BOTH line 2 (a = 99) AND line 10 (return value) in
        // ONE commit. The 7-line gap (lines 3-9) exceeds 2×context(3)=6,
        // guaranteeing two separate hunks. change_freq for `hot` must still
        // increment by 1 (not 2).
        write(&path, "src/target.rs", &src(HOT_VMH, COLD_V0, META_V0));
        run_git_at(&path, dates[4], &["commit", "-am", "tweak-mh", "--quiet"]);

        // coupled-1/2/3: change BOTH hot AND cold in the same commit.
        // hot changes at line 10 (return value); cold changes at line 14.
        // Both function spans are touched → function-coupling counts
        // (hot, cold) co_changes = 3.
        write(&path, "src/target.rs", &src(HOT_VCPL1, COLD_V1, META_V0));
        run_git_at(&path, dates[5], &["commit", "-am", "coupled-1", "--quiet"]);

        write(&path, "src/target.rs", &src(HOT_VCPL2, COLD_V2, META_V0));
        run_git_at(&path, dates[6], &["commit", "-am", "coupled-2", "--quiet"]);

        write(&path, "src/target.rs", &src(HOT_VCPL3, COLD_V3, META_V0));
        run_git_at(&path, dates[7], &["commit", "-am", "coupled-3", "--quiet"]);

        // meta-tweak-1 … meta-tweak-10: change ONLY the `meta` function body,
        // alternating between META_V0 and META_V1. Neither `hot` nor `cold` is
        // touched in any of these commits. Ten commits give `neither = 10`,
        // which drives the Fisher p-value for the (hot, cold) pair below 0.1:
        //
        //   n       = 17  (tweak-1/2/3 + tweak-mh + coupled-1/2/3 +
        //                   meta-tweak-1..10; seed is an Add → no hunks)
        //   co      = 3   (coupled-1/2/3)
        //   a_only  = 4   (tweak-1/2/3 + tweak-mh)
        //   b_only  = 0
        //   neither = 10  (meta-tweak-1..10)
        //   n - co - a_only - b_only = 17 - 3 - 4 - 0 = 10  ✓
        //
        // Fisher table [3, 4, 0, 10]:
        //   row1=7, row2=10, col1=3, col2=14, N=17
        //   P(k=3) = C(7,3)·C(10,0)/C(17,3) = 35/680 ≈ 0.051 < 0.1
        //   Two-tail sums tables with P ≤ P(observed=3): only k=3 itself
        //   (k=0: C(7,0)·C(10,3)/C(17,3)=120/680≈0.176 > 0.051, excluded).
        //   So p ≈ 0.051 < 0.1.
        //
        // Note: the `None`-sorts-first behavior is still correct for genuinely
        // degenerate tables on real repos (zero row/col marginal → p → 0).
        let meta_dates = [
            "2026-06-09T10:00:00Z",
            "2026-06-10T10:00:00Z",
            "2026-06-11T10:00:00Z",
            "2026-06-12T10:00:00Z",
            "2026-06-13T10:00:00Z",
            "2026-06-14T10:00:00Z",
            "2026-06-15T10:00:00Z",
            "2026-06-16T10:00:00Z",
            "2026-06-17T10:00:00Z",
            "2026-06-18T10:00:00Z",
        ];
        for (i, &date) in meta_dates.iter().enumerate() {
            let meta = if i % 2 == 0 { META_V1 } else { META_V0 };
            let msg = format!("meta-tweak-{}", i + 1);
            write(&path, "src/target.rs", &src(HOT_VCPL3, COLD_V3, meta));
            run_git_at(&path, date, &["commit", "-am", msg.as_str(), "--quiet"]);
        }

        FunctionXrayRepo { dir }
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn run_git_at(path: &std::path::Path, iso_date: &str, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", iso_date)
            .env("GIT_COMMITTER_DATE", iso_date)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} (at {iso_date}) failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
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
    //! reasonable `0.1.x` follow-up — see roadmap Tier 2.

    use std::process::Command;
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

        // Write the bundle to an OS-temp file outside the fixture's tempdir
        // so `git clone` sees an empty target. The NamedTempFile auto-deletes
        // on drop at end of this function — we don't need it after the clone.
        let bundle_file = tempfile::NamedTempFile::new().expect("bundle scratch tempfile");
        std::fs::write(bundle_file.path(), BUNDLE).expect("write bundle bytes");

        let status = Command::new("git")
            .args(["clone", "--quiet"])
            .arg(bundle_file.path())
            .arg(dir.path())
            .status()
            .expect("spawn git clone");
        assert!(status.success(), "git clone from embedded bundle failed");
        drop(bundle_file);

        let head_sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("spawn git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        DifferentialRepo { dir, head_sha }
    }
}

#[cfg(feature = "test-support")]
pub mod coupling_repo {
    //! A programmatically-built fixture with deliberate cross-module co-changes
    //! so the depth-2 coupling sankey produces visible edges.
    //!
    //! Structure: 3 modules (`src/alpha`, `src/beta`, `src/gamma`), each with two
    //! files. `alpha/svc.rs` and `beta/svc.rs` co-change together in ≥5 commits,
    //! guaranteeing a `src/alpha`↔`src/beta` edge at depth 2 even under the most
    //! conservative coupling thresholds. Additional single-file commits ensure
    //! every file accrues enough churn to appear in the hotspot table.

    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    pub struct CouplingRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Build a ~20-commit fixture with guaranteed cross-module co-change pairs.
    ///
    /// # Panics
    ///
    /// Panics if the OS cannot create a temporary directory or if any `git`
    /// command fails.
    // Long but linear: the fixture stages ~20 explicit commits so the
    // cross-module co-change history is legible commit-by-commit; splitting it
    // into helpers would obscure exactly what history the module-depth test
    // relies on.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn build() -> CouplingRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "coupling@example.com"]);
        run_git(&path, &["config", "user.name", "Coupling"]);
        // Disable auto-gc so a mid-build `gc --auto` can't prune loose objects
        // between an `add` and its `commit` (matches `medium_repo`).
        run_git(&path, &["config", "gc.auto", "0"]);

        // Seed all six files with real content so complexity ingest is non-trivial.
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 1 }\n",
        );
        write(
            &path,
            "src/alpha/util.rs",
            "pub fn alpha_util(x: i32) -> i32 { x * 2 }\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 1 }\n",
        );
        write(
            &path,
            "src/beta/util.rs",
            "pub fn beta_util(x: i32) -> i32 { x / 2 }\n",
        );
        write(
            &path,
            "src/gamma/svc.rs",
            "pub fn gamma_svc(x: i32) -> i32 { x * x }\n",
        );
        write(
            &path,
            "src/gamma/util.rs",
            "pub fn gamma_util(x: i32) -> i32 { x + x }\n",
        );
        run_git(&path, &["add", "."]);
        commit_at(&path, "2026-06-01T10:00:00Z", "seed all modules");

        // Co-change 1: alpha/svc + beta/svc together (→ depth-2 edge src/alpha↔src/beta).
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 2 }\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 2 }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-02T10:00:00Z", "co-change alpha+beta 1");

        // Co-change 2.
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 3 }\npub fn alpha_extra() {}\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 3 }\npub fn beta_extra() {}\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-03T10:00:00Z", "co-change alpha+beta 2");

        // Co-change 3.
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 4 }\npub fn alpha_extra() { let _ = 1; }\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 4 }\npub fn beta_extra() { let _ = 2; }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-04T10:00:00Z", "co-change alpha+beta 3");

        // Co-change 4.
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 5 }\npub fn alpha_v2(y: i32) -> i32 { y }\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 5 }\npub fn beta_v2(y: i32) -> i32 { y }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-05T10:00:00Z", "co-change alpha+beta 4");

        // Co-change 5.
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 6 }\npub fn alpha_v2(y: i32) -> i32 { y + 1 }\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 6 }\npub fn beta_v2(y: i32) -> i32 { y + 1 }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-06T10:00:00Z", "co-change alpha+beta 5");

        // Single-file commits so every file gets individual churn (hotspot rows).
        write(
            &path,
            "src/alpha/util.rs",
            "pub fn alpha_util(x: i32) -> i32 { x * 3 }\n",
        );
        run_git(&path, &["add", "src/alpha/util.rs"]);
        commit_at(&path, "2026-06-07T10:00:00Z", "touch alpha/util");

        write(
            &path,
            "src/beta/util.rs",
            "pub fn beta_util(x: i32) -> i32 { x / 3 }\n",
        );
        run_git(&path, &["add", "src/beta/util.rs"]);
        commit_at(&path, "2026-06-08T10:00:00Z", "touch beta/util");

        write(
            &path,
            "src/gamma/svc.rs",
            "pub fn gamma_svc(x: i32) -> i32 { x * x + 1 }\n",
        );
        run_git(&path, &["add", "src/gamma/svc.rs"]);
        commit_at(&path, "2026-06-09T10:00:00Z", "touch gamma/svc");

        write(
            &path,
            "src/gamma/util.rs",
            "pub fn gamma_util(x: i32) -> i32 { x + x + 1 }\n",
        );
        run_git(&path, &["add", "src/gamma/util.rs"]);
        commit_at(&path, "2026-06-10T10:00:00Z", "touch gamma/util");

        // More individual churn to push rev counts above any default floor.
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 7 }\npub fn alpha_v3() -> i32 { 42 }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs"]);
        commit_at(&path, "2026-06-11T10:00:00Z", "alpha/svc solo");

        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 7 }\npub fn beta_v3() -> i32 { 99 }\n",
        );
        run_git(&path, &["add", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-12T10:00:00Z", "beta/svc solo");

        write(
            &path,
            "src/alpha/util.rs",
            "pub fn alpha_util(x: i32) -> i32 { x * 4 }\npub fn alpha_util2() {}\n",
        );
        run_git(&path, &["add", "src/alpha/util.rs"]);
        commit_at(&path, "2026-06-13T10:00:00Z", "alpha/util 2");

        write(
            &path,
            "src/beta/util.rs",
            "pub fn beta_util(x: i32) -> i32 { x / 4 }\npub fn beta_util2() {}\n",
        );
        run_git(&path, &["add", "src/beta/util.rs"]);
        commit_at(&path, "2026-06-14T10:00:00Z", "beta/util 2");

        write(
            &path,
            "src/gamma/svc.rs",
            "pub fn gamma_svc(x: i32) -> i32 { x * x + 2 }\npub fn gamma_v2() {}\n",
        );
        run_git(&path, &["add", "src/gamma/svc.rs"]);
        commit_at(&path, "2026-06-15T10:00:00Z", "gamma/svc 2");

        // One more co-change for good measure (total 6 shared revisions).
        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 8 }\npub fn alpha_v3() -> i32 { 43 }\n",
        );
        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 8 }\npub fn beta_v3() -> i32 { 100 }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-16T10:00:00Z", "co-change alpha+beta 6");

        // Final solo touches to widen the hotspot spread.
        write(
            &path,
            "src/gamma/util.rs",
            "pub fn gamma_util(x: i32) -> i32 { x + x + 2 }\npub fn gamma_util2() {}\n",
        );
        run_git(&path, &["add", "src/gamma/util.rs"]);
        commit_at(&path, "2026-06-17T10:00:00Z", "gamma/util 2");

        write(
            &path,
            "src/alpha/svc.rs",
            "pub fn alpha_svc(x: i32) -> i32 { x + 9 }\npub fn alpha_v4() -> bool { true }\n",
        );
        run_git(&path, &["add", "src/alpha/svc.rs"]);
        commit_at(&path, "2026-06-18T10:00:00Z", "alpha/svc final");

        write(
            &path,
            "src/beta/svc.rs",
            "pub fn beta_svc(x: i32) -> i32 { x - 9 }\npub fn beta_v4() -> bool { false }\n",
        );
        run_git(&path, &["add", "src/beta/svc.rs"]);
        commit_at(&path, "2026-06-19T10:00:00Z", "beta/svc final");

        let head_sha = String::from_utf8(
            Command::new("git")
                .args(["-C", path.to_str().unwrap(), "rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        CouplingRepo { dir, head_sha }
    }

    fn commit_at(path: &std::path::Path, iso_date: &str, msg: &str) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", iso_date)
            .env("GIT_COMMITTER_DATE", iso_date)
            .status()
            .expect("git commit");
        assert!(status.success(), "commit '{msg}' at {iso_date} failed");
        // Pack this commit's loose objects immediately. `coupling_repo` is built
        // by FOUR browser tests in parallel (unlike `medium_repo`'s single
        // build), so under ~4x the I/O contention a later `write-tree` can miss
        // a recently-written loose object ("invalid object / Error building
        // trees"). `medium_repo` keeps its loose set small by repacking every 50
        // commits at 1x load; the parallel `coupling_repo` needs the tighter
        // per-commit cadence. `gc.auto=0` (set in `build`) keeps a concurrent
        // auto-gc from racing the same store.
        run_git(path, &["repack", "-d", "--quiet"]);
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }
}

#[cfg(feature = "test-support")]
pub mod medium_repo {
    //! 500-commit fixture for criterion benchmarks. Heavier than `differential_repo`
    //! (which is 50 commits, optimized for differential-test edge-case coverage) but
    //! still small enough for a CI bench to iterate on in under ~10 seconds.
    //!
    //! Structure: 500 commits, 3 authors, 25 files, round-robin author×file, no
    //! merges, no renames, deterministic dates. Intended use: measure ingest+walk
    //! throughput without any of the edge cases `differential_repo` exercises.

    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    pub struct MediumRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Build a 500-commit fixture for criterion benches.
    ///
    /// # Panics
    ///
    /// Panics if any git command fails.
    #[must_use]
    pub fn build() -> MediumRepo {
        const COMMIT_COUNT: usize = 500;
        const FILE_COUNT: usize = 25;

        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();
        run_git(&path, &["init", "-b", "main", "--quiet"]);
        run_git(&path, &["config", "user.email", "bench@example.com"]);
        run_git(&path, &["config", "user.name", "Bench"]);
        // Disable auto-gc: 500 rapid commits can trigger gc and prune loose
        // blobs between `git add` and the next `git commit`, producing
        // "invalid object" errors. Disable upfront for the bench fixture.
        run_git(&path, &["config", "gc.auto", "0"]);

        let authors = [
            ("Alice", "alice@example.com"),
            ("Bob", "bob@example.com"),
            ("Carol", "carol@example.com"),
        ];

        for i in 0..COMMIT_COUNT {
            let file_idx = i % FILE_COUNT;
            let rel = format!("src/mod_{file_idx:02}.rs");
            let content = format!(
                "// commit {i}\npub fn fn_{i}() -> u32 {{ {i} }}\n\n\
                 #[cfg(test)]\nmod tests {{\n    use super::*;\n    \
                 #[test] fn t_{i}() {{ assert_eq!(fn_{i}(), {i}); }}\n}}\n"
            );
            write(&path, &rel, &content);
            let (name, email) = authors[i % 3];
            commit_at(&path, name, email, i, &format!("touch {rel}"), &[&rel]);

            // Pack loose objects every 50 commits to prevent "invalid object"
            // errors on macOS/APFS where the OS may delay flushing loose blob
            // writes to the object store. `git repack -d` packs all loose
            // objects into a single packfile and prunes the loose originals,
            // eliminating the race between `git add` (write loose object) and
            // `git commit` (write-tree reads that same object). We use
            // `repack -d` instead of `gc --quiet` because `gc` has additional
            // phases (expire, reflog) that can fail when packfiles are created
            // concurrently on macOS/APFS.
            if i > 0 && i % 50 == 49 {
                run_git(&path, &["repack", "-d", "--quiet"]);
            }
        }

        let head_sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        MediumRepo { dir, head_sha }
    }

    fn commit_at(
        path: &std::path::Path,
        name: &str,
        email: &str,
        sequence: usize,
        msg: &str,
        files: &[&str],
    ) {
        for f in files {
            run_git(path, &["add", f]);
        }
        let date = format!(
            "2026-01-{:02}T{:02}:{:02}:00Z",
            1 + (sequence / (24 * 60)),
            (sequence / 60) % 24,
            sequence % 60
        );
        let author = format!("{name} <{email}>");
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--author", &author, "--quiet"])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .status()
            .expect("git commit");
        assert!(status.success(), "commit failed: {msg}");
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }
}

#[cfg(feature = "test-support")]
pub mod delivery_repo {
    //! A programmatically-built fixture covering delivery-metrics edge cases:
    //! 3 authors (Alice/Bob/Carol), annotated tags at ~day 10/50/110 plus one
    //! non-matching tag (`nightly-1`), two `--no-ff` merges with measurable
    //! branch durations, author-vs-committer date gaps on 2 commits, one
    //! rework signal (file modified then partially deleted within 5 days), and
    //! one file churned 62 days after first touch (outside the 21-day rework
    //! window).
    //!
    //! Merge commits are only ingested when `Options.include_merges` is true —
    //! the default drops them at the walker before they reach the `DuckDB`
    //! Appender, so `is_merge` rows will not appear with `Options::default()`.

    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    pub struct DeliveryRepo {
        pub dir: TempDir,
        pub head_sha: String,
    }

    /// Build a delivery-metrics fixture spanning ~110 days.
    ///
    /// # Panics
    ///
    /// Panics if the OS cannot create a temporary directory or if any `git`
    /// command fails.
    // Long but linear: each section corresponds to one named fixture event
    // (rework, branch, tag, author-gap) so the history is legible
    // commit-by-commit. Splitting into helpers would obscure which commits
    // carry which signals.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn build() -> DeliveryRepo {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().to_path_buf();

        run_git(&path, &["init", "-b", "main", "--quiet"]);
        // A default identity so git never falls back to hostname auto-detect;
        // individual commits override the author via --author / env vars.
        run_git(&path, &["config", "user.email", "delivery@example.com"]);
        run_git(&path, &["config", "user.name", "Delivery"]);
        // Prevent auto-gc from racing the object store between add and commit.
        run_git(&path, &["config", "gc.auto", "0"]);

        // ── Day 0 (2026-01-01): Alice seeds three files ──────────────────────
        write(&path, "src/core.rs", "pub fn core() -> u32 { 1 }\n");
        write(&path, "src/rework.rs", "pub fn rework() -> u32 { 1 }\n");
        write(&path, "src/stable.rs", "pub fn stable() -> u32 { 1 }\n");
        run_git(&path, &["add", "."]);
        commit_as(
            &path,
            "Alice",
            "alice@example.com",
            "2026-01-01T10:00:00Z",
            "2026-01-01T10:00:00Z",
            "seed: add initial files",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Day 5 (2026-01-06): Alice expands rework.rs ──────────────────────
        write(
            &path,
            "src/rework.rs",
            "pub fn rework() -> u32 { 1 }\npub fn extra() -> u32 { 2 }\npub fn more() -> u32 { 3 }\n",
        );
        run_git(&path, &["add", "src/rework.rs"]);
        commit_as(
            &path,
            "Alice",
            "alice@example.com",
            "2026-01-06T10:00:00Z",
            "2026-01-06T10:00:00Z",
            "feat: expand rework.rs",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Day 8 (2026-01-09): Bob partially deletes rework.rs ──────────────
        // Within 5 days of the previous touch → rework signal.
        // author_date = Jan 8, committer_date = Jan 9 → lead-proxy gap (1 of 2).
        write(&path, "src/rework.rs", "pub fn rework() -> u32 { 1 }\n");
        run_git(&path, &["add", "src/rework.rs"]);
        commit_as(
            &path,
            "Bob",
            "bob@example.com",
            "2026-01-08T10:00:00Z",
            "2026-01-09T10:00:00Z",
            "refactor: trim rework.rs",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Day 10 (2026-01-11): Alice updates core.rs; tag v0.1.0 ──────────
        write(&path, "src/core.rs", "pub fn core() -> u32 { 2 }\n");
        run_git(&path, &["add", "src/core.rs"]);
        commit_as(
            &path,
            "Alice",
            "alice@example.com",
            "2026-01-11T10:00:00Z",
            "2026-01-11T10:00:00Z",
            "feat: update core",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);
        tag_annotated(&path, "v0.1.0", "release v0.1.0", "2026-01-11T12:00:00Z");

        // ── Branch 1: 3 commits spanning 2 calendar days (Jan 16–17) ─────────
        run_git(&path, &["checkout", "-b", "feature/branch1", "--quiet"]);

        write(&path, "src/branch1.rs", "pub fn branch1_a() -> u32 { 1 }\n");
        run_git(&path, &["add", "src/branch1.rs"]);
        commit_as(
            &path,
            "Bob",
            "bob@example.com",
            "2026-01-16T10:00:00Z",
            "2026-01-16T10:00:00Z",
            "feat: add branch1 file",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        write(
            &path,
            "src/branch1.rs",
            "pub fn branch1_a() -> u32 { 1 }\npub fn branch1_b() -> u32 { 2 }\n",
        );
        run_git(&path, &["add", "src/branch1.rs"]);
        commit_as(
            &path,
            "Carol",
            "carol@example.com",
            "2026-01-17T08:00:00Z",
            "2026-01-17T08:00:00Z",
            "feat: expand branch1",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        write(
            &path,
            "src/branch1.rs",
            "pub fn branch1_a() -> u32 { 10 }\npub fn branch1_b() -> u32 { 20 }\n",
        );
        run_git(&path, &["add", "src/branch1.rs"]);
        commit_as(
            &path,
            "Bob",
            "bob@example.com",
            "2026-01-17T14:00:00Z",
            "2026-01-17T14:00:00Z",
            "fix: finalize branch1",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // Merge commit 1 of 2: --no-ff merge of branch1.
        run_git(&path, &["checkout", "main", "--quiet"]);
        merge_no_ff(
            &path,
            "Alice",
            "alice@example.com",
            "2026-01-18T10:00:00Z",
            "feature/branch1",
            "Merge branch 'feature/branch1' into main",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Day 40 (2026-02-10): Carol updates core.rs ───────────────────────
        write(&path, "src/core.rs", "pub fn core() -> u32 { 3 }\n");
        run_git(&path, &["add", "src/core.rs"]);
        commit_as(
            &path,
            "Carol",
            "carol@example.com",
            "2026-02-10T10:00:00Z",
            "2026-02-10T10:00:00Z",
            "feat: update core for v0.2",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Day 50 (2026-02-20): Alice preps v0.2.0; tag v0.2.0 ─────────────
        write(&path, "src/core.rs", "pub fn core() -> u32 { 4 }\n");
        run_git(&path, &["add", "src/core.rs"]);
        commit_as(
            &path,
            "Alice",
            "alice@example.com",
            "2026-02-20T10:00:00Z",
            "2026-02-20T10:00:00Z",
            "release: prep v0.2.0",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);
        tag_annotated(&path, "v0.2.0", "release v0.2.0", "2026-02-20T12:00:00Z");

        // ── Day 62 (2026-03-04): Bob churns stable.rs ────────────────────────
        // 62 days since stable.rs was first touched (day 0) → outside the
        // 21-day rework window; contrasts with the rework.rs signal above.
        // author_date = Mar 3, committer_date = Mar 4 → lead-proxy gap (2 of 2).
        write(&path, "src/stable.rs", "pub fn stable() -> u32 { 99 }\n");
        run_git(&path, &["add", "src/stable.rs"]);
        commit_as(
            &path,
            "Bob",
            "bob@example.com",
            "2026-03-03T10:00:00Z",
            "2026-03-04T10:00:00Z",
            "chore: churn stable.rs",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Branch 2: 1 commit, same-day merge (2026-03-07) ──────────────────
        run_git(&path, &["checkout", "-b", "feature/branch2", "--quiet"]);

        write(&path, "src/branch2.rs", "pub fn branch2() -> u32 { 1 }\n");
        run_git(&path, &["add", "src/branch2.rs"]);
        commit_as(
            &path,
            "Carol",
            "carol@example.com",
            "2026-03-07T10:00:00Z",
            "2026-03-07T10:00:00Z",
            "feat: add branch2 file",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // Merge commit 2 of 2: --no-ff merge of branch2, same calendar day.
        run_git(&path, &["checkout", "main", "--quiet"]);
        merge_no_ff(
            &path,
            "Bob",
            "bob@example.com",
            "2026-03-07T14:00:00Z",
            "feature/branch2",
            "Merge branch 'feature/branch2' into main",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);

        // ── Day 80 (2026-03-22): Carol touches core.rs; non-matching tag ─────
        write(&path, "src/core.rs", "pub fn core() -> u32 { 5 }\n");
        run_git(&path, &["add", "src/core.rs"]);
        commit_as(
            &path,
            "Carol",
            "carol@example.com",
            "2026-03-22T10:00:00Z",
            "2026-03-22T10:00:00Z",
            "chore: tweak core",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);
        // Non-matching tag: a later task tests glob filtering (e.g. `v*` patterns
        // must not match this tag).
        tag_annotated(
            &path,
            "nightly-1",
            "nightly build 1",
            "2026-03-22T12:00:00Z",
        );

        // ── Day 110 (2026-04-21): Alice preps v1.0.0; tag v1.0.0 ─────────────
        write(&path, "src/core.rs", "pub fn core() -> u32 { 6 }\n");
        run_git(&path, &["add", "src/core.rs"]);
        commit_as(
            &path,
            "Alice",
            "alice@example.com",
            "2026-04-21T10:00:00Z",
            "2026-04-21T10:00:00Z",
            "release: prep v1.0.0",
        );
        run_git(&path, &["repack", "-d", "--quiet"]);
        tag_annotated(&path, "v1.0.0", "release v1.0.0", "2026-04-21T12:00:00Z");
        // Lightweight tag (no `-a`): exercises the tag-date fallback path in
        // `Repo::tags()` — lightweight tags carry no tagger date, so both
        // backends must fall back to the target commit's committer date
        // (2026-04-21T10:00:00Z here, sorting BEFORE v1.0.0's 12:00 tagger date).
        run_git(&path, &["tag", "light-1"]);

        let head_sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        DeliveryRepo { dir, head_sha }
    }

    /// Commit with a specific author identity and independent author/committer
    /// dates. The `--author` flag overrides the author; the repo default user
    /// is used for the committer identity. When `author_date != committer_date`
    /// the commit models a review-then-land gap.
    fn commit_as(
        path: &std::path::Path,
        name: &str,
        email: &str,
        author_date: &str,
        committer_date: &str,
        msg: &str,
    ) {
        let author = format!("{name} <{email}>");
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--author", &author, "--quiet"])
            .env("GIT_AUTHOR_DATE", author_date)
            .env("GIT_COMMITTER_DATE", committer_date)
            .status()
            .expect("git commit");
        assert!(status.success(), "commit '{msg}' failed");
    }

    /// `--no-ff` merge commit pinned to a specific author/committer identity
    /// and date. `git merge` has no `--author` flag; identity is set via env
    /// vars.
    fn merge_no_ff(
        path: &std::path::Path,
        name: &str,
        email: &str,
        date: &str,
        branch: &str,
        msg: &str,
    ) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["merge", "--no-ff", branch, "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git merge");
        assert!(status.success(), "merge '{branch}' into main failed");
    }

    /// Create an annotated tag at HEAD with a deterministic tagger date.
    fn tag_annotated(path: &std::path::Path, tag: &str, msg: &str, date: &str) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["tag", "-a", tag, "-m", msg])
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git tag");
        assert!(status.success(), "tag '{tag}' failed");
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }
}
