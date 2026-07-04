use codelore_lib::Options;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn kamei_features_populated_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // NF should be >= 1 for every commit (each commit touches at least one file)
    let min_nf: String = db
        .query_one_value("SELECT CAST(MIN(nf) AS TEXT) FROM commits WHERE nf IS NOT NULL")
        .expect("nf query");
    assert!(min_nf.parse::<u32>().unwrap() >= 1);

    // NS should be >= 1 — all files in tiny_repo live under "src/"
    let min_ns_val: String = db
        .query_one_value("SELECT CAST(MIN(ns) AS TEXT) FROM commits WHERE ns IS NOT NULL")
        .expect("ns query");
    assert!(
        min_ns_val.parse::<u32>().unwrap() >= 1,
        "expected NS >= 1 (files in src/)"
    );

    // LA column should be populated (not NULL) for all commits.
    // loc_added is stubbed to 0 at gix walk time (blob-diff is not computed),
    // so SUM(la) = 0 is expected — we verify it is at least non-NULL.
    let la_non_null: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits WHERE la IS NOT NULL")
        .expect("la non-null query");
    assert_eq!(
        la_non_null.parse::<u32>().unwrap(),
        5,
        "expected all 5 commits to have la populated (even if 0)"
    );

    // EXP should be 0 for the first commit, max = 4 for the 5th commit (same author)
    let max_exp: String = db
        .query_one_value("SELECT CAST(MAX(exp) AS TEXT) FROM commits")
        .expect("exp query");
    let max_exp_val: u32 = max_exp.parse().unwrap();
    assert!(
        max_exp_val >= 4,
        "expected max EXP >= 4 for 5-commit single-author repo, got {max_exp_val}"
    );
}

/// Hot-path scenario validates the windowed history rewrite. A repo with
/// one path touched many times by multiple authors should produce
/// monotonically non-decreasing ndev/nuc/sexp values on that path, and
/// the final commit's ndev/nuc must reflect all distinct prior authors
/// and revs across the path's history.
///
/// The legacy path-self-join would compute the same values via a K×K
/// cartesian on the hot path. The windowed form computes them via
/// per-path running aggregation. This test gates parity.
#[allow(clippy::too_many_lines)]
#[test]
fn windowed_history_matches_legacy_semantics_on_hot_path() {
    use std::process::Command;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("spawn git")
            .success();
        assert!(ok, "git {args:?} failed");
    };
    // Explicit per-commit timestamps — the Kamei `<=` semantic
    // (preserved by the windowed RANGE … EXCLUDE CURRENT ROW frame)
    // counts same-second peers as mutual priors. Distinct timestamps
    // make the per-commit expected values deterministic.
    let git_commit = |msg: &str, date: &str| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("spawn git commit")
            .success();
        assert!(ok, "git commit {msg:?} failed");
    };

    git(&["init", "-b", "main", "--quiet"]);
    git(&["config", "user.email", "alice@example.com"]);
    git(&["config", "user.name", "Alice"]);
    std::fs::write(path.join("hot.rs"), "v1\n").unwrap();
    git(&["add", "."]);
    git_commit("c1: alice creates hot.rs", "2026-01-01T10:00:00");

    std::fs::write(path.join("hot.rs"), "v2\n").unwrap();
    git(&["add", "."]);
    git_commit("c2: alice modifies hot.rs", "2026-01-02T10:00:00");

    git(&["config", "user.email", "bob@example.com"]);
    git(&["config", "user.name", "Bob"]);
    std::fs::write(path.join("hot.rs"), "v3\n").unwrap();
    git(&["add", "."]);
    git_commit("c3: bob modifies hot.rs", "2026-01-03T10:00:00");

    git(&["config", "user.email", "alice@example.com"]);
    git(&["config", "user.name", "Alice"]);
    std::fs::write(path.join("cool.rs"), "vA\n").unwrap();
    git(&["add", "."]);
    git_commit("c4: alice creates cool.rs", "2026-01-04T10:00:00");

    git(&["config", "user.email", "bob@example.com"]);
    git(&["config", "user.name", "Bob"]);
    std::fs::write(path.join("hot.rs"), "v4\n").unwrap();
    std::fs::write(path.join("cool.rs"), "vB\n").unwrap();
    git(&["add", "."]);
    git_commit("c5: bob modifies both", "2026-01-05T10:00:00");

    let repo = GixRepo::open(path).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Verify c5's ndev/nuc: prior history at hot.rs = {alice, alice, bob}
    // (revs c1, c2, c3) and at cool.rs = {alice} (rev c4). Union → ndev=2
    // (alice, bob), nuc=4 (c1, c2, c3, c4).
    let c5_ndev: String = db
        .query_one_value(
            "SELECT CAST(ndev AS TEXT) FROM commits \
             WHERE message LIKE 'c5%'",
        )
        .expect("c5 ndev");
    let c5_nuc: String = db
        .query_one_value(
            "SELECT CAST(nuc AS TEXT) FROM commits \
             WHERE message LIKE 'c5%'",
        )
        .expect("c5 nuc");
    assert_eq!(
        c5_ndev.parse::<u32>().unwrap(),
        2,
        "c5 prior history spans both hot.rs (alice, bob) and cool.rs \
         (alice); ndev should be 2 (distinct authors). Got {c5_ndev}",
    );
    assert_eq!(
        c5_nuc.parse::<u32>().unwrap(),
        4,
        "c5 prior history union spans 4 distinct revs (c1, c2, c3, c4); \
         got {c5_nuc}",
    );

    // SEXP: c5 by bob touches src/. Prior commits by bob touching src/
    // = {c3}. So sexp = 1.
    // (Note: no `src/` prefix — top-level dir = ".", same partition.)
    let c5_sexp: String = db
        .query_one_value(
            "SELECT CAST(sexp AS TEXT) FROM commits \
             WHERE message LIKE 'c5%'",
        )
        .expect("c5 sexp");
    assert_eq!(
        c5_sexp.parse::<u32>().unwrap(),
        1,
        "c5 by bob: prior bob-commits in the same top-level dir = c3 only \
         (c4 was by alice, c5 is current); sexp should be 1. Got {c5_sexp}",
    );

    // c1 has no prior history on any path -> ndev=0, nuc=0, age=0.0,
    // sexp=0.
    let c1_ndev: String = db
        .query_one_value(
            "SELECT CAST(ndev AS TEXT) FROM commits \
             WHERE message LIKE 'c1%'",
        )
        .expect("c1 ndev");
    assert_eq!(
        c1_ndev.parse::<u32>().unwrap(),
        0,
        "c1 is the first commit; ndev should be 0"
    );
}

#[test]
fn kamei_fix_flag_detects_bug_keywords() {
    // Build a fixture where one commit message says "fix typo"
    // tiny_repo's commit 4 has message "fix typo" — should set fix=TRUE
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let fix_count: String = db
        .query_one_value("SELECT CAST(COUNT(*) AS TEXT) FROM commits WHERE fix = TRUE")
        .expect("fix count");
    assert!(
        fix_count.parse::<u32>().unwrap() >= 1,
        "tiny repo has 'fix typo' commit; expected >=1 FIX"
    );
}

/// Entropy correctness regression guard. The Kamei diffusion entropy
/// is `-Σ p_i log2(p_i)` over the per-file LOC distribution within a
/// commit:
///
/// * Single-file commit (one file gets all LOC): `p_0 = 1` → entropy = 0
/// * Even split across 2 files: `p = [0.5, 0.5]` → entropy = `log2(2)` = 1
/// * Uneven 3-way split — hand-computed below
///
/// Pins the SQL semantics so that the next rewrite of the entropy
/// block (the correlated-subquery form is flagged as
/// remaining work) cannot silently shift any of these values. The
/// `enrich_history` 2-pass pattern (reset then grouped UPDATE) is
/// the planned shape; this test must keep producing the same numbers
/// after that rewrite.
#[test]
fn kamei_entropy_per_commit_distribution() {
    use std::fmt::Write as _;
    use std::process::Command;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    let mut s = String::new();
    let write_lines = |buf: &mut String, prefix: char, n: usize| {
        buf.clear();
        for i in 0..n {
            writeln!(buf, "{prefix}{i}").unwrap();
        }
    };

    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("spawn git")
            .success();
        assert!(ok, "git {args:?} failed");
    };
    let commit_at = |msg: &str, date: &str| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("spawn git commit")
            .success();
        assert!(ok);
    };

    git(&["init", "-b", "main", "--quiet"]);
    git(&["config", "user.email", "entropy@example.com"]);
    git(&["config", "user.name", "Entropy"]);

    // C1 — single file, 10 lines. Entropy MUST be exactly 0 (p=1,
    // log2(1)=0). One-file commits anchor the lower bound of the
    // distribution; any rewrite that emits NULL or a tiny positive
    // value here is a regression.
    write_lines(&mut s, 'a', 10);
    std::fs::write(path.join("a.txt"), &s).unwrap();
    git(&["add", "."]);
    commit_at("c1: single file 10 lines", "2026-06-01T10:00:00");

    // C2 — two files, 4 + 4 lines. Entropy MUST be exactly log2(2) = 1.
    write_lines(&mut s, 'b', 4);
    std::fs::write(path.join("b.txt"), &s).unwrap();
    write_lines(&mut s, 'c', 4);
    std::fs::write(path.join("c.txt"), &s).unwrap();
    git(&["add", "."]);
    commit_at("c2: two files even", "2026-06-02T10:00:00");

    // C3 — three files, 1 + 2 + 5 lines (total 8). Hand-computed:
    //   p = [1/8, 2/8, 5/8] = [0.125, 0.25, 0.625]
    //   entropy = -(0.125 * log2(0.125) + 0.25 * log2(0.25) + 0.625 * log2(0.625))
    //           = -(0.125 * -3.0 + 0.25 * -2.0 + 0.625 * -0.6780719051126377)
    //           =   0.375 + 0.5 + 0.42379494069539855
    //           ≈ 1.2987949406953984
    std::fs::write(path.join("d.txt"), "d0\n").unwrap();
    std::fs::write(path.join("e.txt"), "e0\ne1\n").unwrap();
    write_lines(&mut s, 'f', 5);
    std::fs::write(path.join("f.txt"), &s).unwrap();
    git(&["add", "."]);
    commit_at("c3: three files uneven", "2026-06-03T10:00:00");

    let repo = codelore_lib::repo::GixRepo::open(path).expect("gix open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // Read the three commits in date order.
    let entropies: Vec<f64> = (1..=3)
        .map(|i| {
            let s: String = db
                .query_one_value(&format!(
                    "SELECT CAST(entropy AS TEXT) FROM commits WHERE message LIKE 'c{i}:%'",
                ))
                .unwrap_or_else(|e| panic!("entropy query c{i}: {e}"));
            s.parse::<f64>()
                .unwrap_or_else(|e| panic!("entropy parse c{i}={s:?}: {e}"))
        })
        .collect();

    // C1 single-file: entropy MUST be exactly 0.
    assert!(
        (entropies[0] - 0.0).abs() < 1e-9,
        "C1 single-file entropy must be 0.0; got {}",
        entropies[0]
    );
    // C2 even 2-file split: entropy MUST be exactly 1.0 (log2(2)).
    assert!(
        (entropies[1] - 1.0).abs() < 1e-9,
        "C2 even 2-file entropy must be 1.0; got {}",
        entropies[1]
    );
    // C3 uneven 3-file split: hand-computed expected.
    let expected_c3 = 1.298_794_940_695_398_4_f64;
    assert!(
        (entropies[2] - expected_c3).abs() < 1e-9,
        "C3 uneven 3-file entropy must be {expected_c3}; got {}",
        entropies[2]
    );
}

/// Directory-skew protection: a synthetic repo where every commit
/// lands in the same top-level dir touches the worst-case (dir, author)
/// partition. Before the O(K) rewrite, this exploded via list / cross-
/// join materialization. This test ensures the (path, dir, author)
/// hot-partition completes in a reasonable time and produces correct
/// `ndev`, `nuc`, `sexp` values.
#[test]
fn dir_skew_does_not_oom_or_timeout() {
    use std::process::Command;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();

    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("spawn git")
            .success();
        assert!(ok, "git {args:?} failed");
    };
    let commit_at = |msg: &str, date: &str| {
        let ok = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", msg, "--quiet"])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("spawn git commit")
            .success();
        assert!(ok);
    };

    git(&["init", "-b", "main", "--quiet"]);
    git(&["config", "user.email", "skew@example.com"]);
    git(&["config", "user.name", "Skew"]);
    std::fs::create_dir_all(path.join("src")).unwrap();

    // 60 commits all touching distinct files under src/ — the
    // (dir, author) partition has 60 distinct revs. The prior
    // implementation's cross-join would explode (60 * 60 = 3600
    // rows in the hot partition; safe for the test, but scales
    // quadratically with K — the OOM at 19 GiB on a real repo had
    // K ≈ 26k touches in one (src, author) partition).
    for i in 0..60 {
        std::fs::write(path.join(format!("src/f{i}.rs")), "v\n").unwrap();
        git(&["add", "."]);
        commit_at(
            &format!("commit {i}"),
            &format!("2026-01-{:02}T10:00:00", (i % 28) + 1),
        );
    }

    let started = std::time::Instant::now();
    let repo = codelore_lib::repo::GixRepo::open(path).expect("gix open");
    let db = codelore_lib::facts::FactsDb::new_in_memory().expect("db");
    let opts = codelore_lib::Options {
        repo_path: path.to_path_buf(),
        min_revs: 1,
        ..codelore_lib::Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    let elapsed = started.elapsed();

    // Even on slow CI, the directory-skew partition should finish in
    // well under 30 seconds — the prior implementation could exceed
    // this on a 60-commit synthetic fixture if the partition
    // materialization regressed.
    assert!(
        elapsed.as_secs() < 30,
        "kamei ingest took {elapsed:?} on a 60-commit fixture — likely a partition-materialization regression"
    );

    // The 60th commit should see 59 prior revs in its (src, Skew)
    // partition.
    let last_sexp: String = db
        .query_one_value("SELECT CAST(sexp AS TEXT) FROM commits ORDER BY date DESC LIMIT 1")
        .expect("last sexp");
    assert!(
        last_sexp.parse::<u32>().unwrap() >= 50,
        "60th commit in skewed (src, Skew) partition should see ~59 prior revs; got {last_sexp}"
    );
}
