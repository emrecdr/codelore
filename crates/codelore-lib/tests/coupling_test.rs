use codelore_lib::Options;
use codelore_lib::analyses::coupling::run_coupling;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

#[test]
fn coupling_for_tiny_repo() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        fisher_significance: 1.0, // allow all p-values for tiny fixture
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_coupling(&db, &opts).expect("run");

    // tiny_repo has src/main.rs (4 commits) and src/lib.rs (1 commit).
    // src/lib.rs's only commit ("add lib", commit 3) touches only src/lib.rs.
    // src/main.rs is touched in commits 1, 2, 4, 5.
    // There are NO shared commits between the two files, so no coupling pair
    // should be produced even with min_shared_revs=1.
    assert!(
        rows.is_empty() || rows.iter().all(|r| r.shared >= 1),
        "any coupling row must have at least 1 shared commit"
    );
}

/// Build a throwaway repo whose history yields three Fisher-significant
/// coupling pairs — (a,b), (a,c), (b,c). The three files co-change in
/// `n_shared` commits; each file then gets its OWN solo commits and a
/// fourth file `x` provides "neither" rows, so every pair's 2x2 contingency
/// table has off-diagonal mass (p < 1) rather than the degenerate p = 1 a
/// perfectly-correlated pair would produce. This gives the memo tests a
/// fixture with multiple surviving pairs for truncation + keyed-recompute.
fn build_trio_repo(n_shared: usize) -> tempfile::TempDir {
    use std::process::Command;
    fn git(path: &std::path::Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git");
        assert!(status.success());
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(
        path,
        "2026-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    git(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    git(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.name", "Trio"],
    );
    let mut day = 1u32;
    let mut commit = |files: &[&str], tag: &str| {
        let date = format!("2026-01-{day:02}T12:00:00Z");
        for name in files {
            std::fs::write(path.join(name), format!("{name}-{tag}")).unwrap();
        }
        git(path, &date, &["add", "."]);
        git(path, &date, &["commit", "-m", tag, "--quiet"]);
        day += 1;
    };
    // n_shared commits touching a, b, c together.
    for i in 1..=n_shared {
        commit(&["a.txt", "b.txt", "c.txt"], &format!("trio{i}"));
    }
    // Per-file solo commits break perfect correlation so each pair's Fisher
    // table has a non-zero "one but not the other" cell.
    for i in 1..=2 {
        commit(&["a.txt"], &format!("soloA{i}"));
    }
    for i in 1..=2 {
        commit(&["b.txt"], &format!("soloB{i}"));
    }
    for i in 1..=2 {
        commit(&["c.txt"], &format!("soloC{i}"));
    }
    // "Neither" rows: an unrelated file changing alone supplies the d-cell.
    for i in 1..=3 {
        commit(&["x.txt"], &format!("x{i}"));
    }
    dir
}

/// The per-`FactsDb` coupling memo must be transparent: a second identical
/// call returns a result equal to the first (same pairs, same order, same
/// p-values), and a call with a DIFFERENT coupling-affecting option must NOT
/// serve the first call's cached entry.
#[test]
fn coupling_memo_is_transparent_and_keyed() {
    let dir = build_trio_repo(8);
    let repo = GixRepo::open(dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        fisher_significance: 1.0,
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // First call populates the memo; second call must hit it and return an
    // equal result. Compare on the field tuple so a future field addition to
    // CouplingRow is caught.
    let first = run_coupling(&db, &opts).expect("first");
    let second = run_coupling(&db, &opts).expect("second (memo hit)");
    assert_eq!(first.len(), 3, "trio repo should yield (a,b),(a,c),(b,c)");
    assert_eq!(first.len(), second.len(), "memo hit changed row count");
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.entity_a, b.entity_a);
        assert_eq!(a.entity_b, b.entity_b);
        assert_eq!(a.shared, b.shared);
        assert_eq!(a.revs_a, b.revs_a);
        assert_eq!(a.revs_b, b.revs_b);
        assert_eq!(a.average_revs, b.average_revs);
        assert!((a.degree - b.degree).abs() < f64::EPSILON);
        assert!((a.fisher_p - b.fisher_p).abs() < f64::EPSILON);
    }

    // A different coupling-affecting option — here driving `fisher_significance`
    // to 0.0 — must NOT reuse the prior cache entry. p-values are strictly
    // positive, so a 0.0 threshold rejects every pair: a stale-memo bug would
    // wrongly return the 3-row baseline.
    let opts_no_sig = Options {
        fisher_significance: 0.0,
        ..opts.clone()
    };
    let no_sig = run_coupling(&db, &opts_no_sig).expect("no-sig");
    assert!(
        no_sig.is_empty(),
        "fisher_significance=0.0 is a distinct key and must recompute to empty, \
         not serve the cached 3-row baseline; got {} rows",
        no_sig.len()
    );

    // The original opts must still memo-hit to its UNCHANGED baseline after
    // the distinct-key call cached its own (empty) entry — no overwrite, no
    // aliasing between the two keys.
    let third = run_coupling(&db, &opts).expect("third (original key)");
    assert_eq!(
        third.len(),
        first.len(),
        "original key's entry was clobbered by the distinct-key call"
    );
}

/// `--rows N` must truncate the returned vec but NOT the shared memo entry,
/// so a later un-capped call still sees the full graph.
#[test]
fn coupling_memo_row_limit_does_not_poison_full_result() {
    let dir = build_trio_repo(8);
    let repo = GixRepo::open(dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let base = Options {
        repo_path: dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        fisher_significance: 1.0,
        use_canonical_lineage: false,
        ..Options::default()
    };
    db.ingest(&repo, &base).expect("ingest");

    let full = run_coupling(&db, &base).expect("full");
    assert_eq!(full.len(), 3, "trio repo should yield 3 pairs");

    // Capped call (same key, only rows_limit differs) returns a prefix.
    let capped = run_coupling(
        &db,
        &Options {
            rows_limit: Some(1),
            ..base.clone()
        },
    )
    .expect("capped");
    assert_eq!(capped.len(), 1, "rows_limit=1 must truncate to 1");
    assert_eq!(capped[0].entity_a, full[0].entity_a);
    assert_eq!(capped[0].entity_b, full[0].entity_b);

    // A later un-capped call must STILL return the full graph — proving the
    // cached entry was never truncated.
    let full_again = run_coupling(&db, &base).expect("full again");
    assert_eq!(
        full_again.len(),
        full.len(),
        "rows_limit poisoned the shared memo entry"
    );
}

#[test]
fn coupling_struct_shape() {
    use codelore_lib::analyses::coupling::CouplingRow;
    let row = CouplingRow {
        entity_a: "a.rs".into(),
        entity_b: "b.rs".into(),
        shared: 4,
        revs_a: 5,
        revs_b: 5,
        average_revs: 5,
        degree: 80.0,
        fisher_p: 0.01,
    };
    assert_eq!(row.shared, 4);
    assert!(row.degree > 70.0);
    assert!(row.fisher_p < 0.05);
    assert_eq!(row.entity_a, "a.rs");
    assert_eq!(row.entity_b, "b.rs");
}

#[test]
fn coupling_respects_min_shared_revs() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 2, // stricter: require ≥2 shared
        min_coupling_pct: 0,
        fisher_significance: 1.0,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_coupling(&db, &opts).expect("run");
    for row in &rows {
        assert!(
            row.shared >= 2,
            "min_shared_revs=2 violated: shared={} for {}<->{}",
            row.shared,
            row.entity_a,
            row.entity_b
        );
    }
}

/// `max_coupling_pct` was wired through `Options` but the SQL only bound
/// the lower bound, so `--max-coupling N` was silently ignored. Regression:
/// assert the upper bound actually filters pairs.
#[test]
fn coupling_respects_max_coupling_pct() {
    let diff_repo = codelore_lib::test_support::differential_repo::build();
    let repo = GixRepo::open(diff_repo.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");

    // Baseline: collect ALL pairs (degree >= 0) to find what's there.
    let opts_all = Options {
        repo_path: diff_repo.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        max_coupling_pct: 100,
        fisher_significance: 1.0,
        ..Options::default()
    };
    db.ingest(&repo, &opts_all).expect("ingest");
    let baseline = run_coupling(&db, &opts_all).expect("baseline");
    let max_observed = baseline.iter().map(|r| r.degree).fold(0.0_f64, f64::max);
    assert!(
        max_observed > 0.0,
        "differential_repo should produce at least one coupled pair with degree > 0; \
         got {} rows max degree = {max_observed}",
        baseline.len()
    );

    // Cap below the observed max — MUST drop at least the top pair.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cap_pct = (max_observed / 2.0).floor() as u8;
    let opts_capped = Options {
        max_coupling_pct: cap_pct,
        ..opts_all.clone()
    };
    let capped = run_coupling(&db, &opts_capped).expect("capped");

    assert!(
        capped.len() < baseline.len(),
        "max_coupling_pct={cap_pct} should drop ≥1 pair; baseline={}, capped={}",
        baseline.len(),
        capped.len()
    );
    for row in &capped {
        assert!(
            row.degree <= f64::from(cap_pct),
            "row degree={} exceeds cap={cap_pct} for {}<->{}",
            row.degree,
            row.entity_a,
            row.entity_b
        );
    }
}

#[test]
fn coupling_fisher_significance_filter() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    let repo = GixRepo::open(tiny.dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    // With fisher_significance=0.0, no pair can pass (p-value is always > 0)
    let opts = Options {
        repo_path: tiny.dir.path().to_path_buf(),
        min_revs: 1,
        min_shared_revs: 1,
        min_coupling_pct: 0,
        fisher_significance: 0.0,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_coupling(&db, &opts).expect("run");
    assert!(
        rows.is_empty(),
        "fisher_significance=0.0 should reject all pairs, got {} rows",
        rows.len()
    );
}

/// PAR-6 regression: the `min_revs` filter applies at different
/// pivot points depending on `--code-maat-compat`.
///
/// Fixture: a fresh repo with four files. Asymmetry between `a` and
/// `b`/`c` is required so Fisher's exact test produces interpretable
/// p-values (strictly-always-together pairs reduce to a degenerate 2×2
/// where p = 1.0). We set `fisher_significance: 2.0` to remove the
/// Fisher filter entirely — this test is about the `min_revs` pivot,
/// not significance gating.
///
///   - `a.txt`: 10 commits (8 with b, 6 with c, 2 with only `x.txt`)
///   - `b.txt`: 8 commits, all alongside `a.txt`
///   - `c.txt`: 4 commits, all alongside `a.txt`
///   - `x.txt`: 5 commits — 2 alone (provides "neither a nor b"
///     observations for the Fisher table), 3 alongside `a.txt`
///
/// Under `--min-revs 5`:
///   - **Default (per-file gate)**: `c.txt` has only 4 individual
///     revs and is dropped from `file_revs`. The `(a, c)` pair never
///     forms. `b.txt` has 8 revs and survives.
///   - **Compat (per-pair-average gate)**: `c.txt` survives the
///     non-existent file gate; the `(a, c)` pair forms with
///     average = (10 + 4) / 2 = 7 ≥ 5 → pair surfaces.
///
/// In both modes `(a, b)` surfaces (both files above either pivot).
#[test]
#[allow(
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args
)]
fn par6_min_revs_pivot_differs_under_code_maat_compat() {
    use std::process::Command;
    fn run_git_at(path: &std::path::Path, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git");
        assert!(status.success());
    }
    fn commit(path: &std::path::Path, day: usize, files: &[(&str, &str)]) {
        let date = format!("2026-01-{day:02}T12:00:00Z");
        for (name, content) in files {
            std::fs::write(path.join(name), content).unwrap();
        }
        run_git_at(path, &date, &["add", "."]);
        run_git_at(
            path,
            &date,
            &["commit", "-m", &format!("d{day}"), "--quiet"],
        );
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    run_git_at(
        path,
        "2026-01-01T00:00:00Z",
        &["init", "-b", "main", "--quiet"],
    );
    run_git_at(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.email", "t@t"],
    );
    run_git_at(
        path,
        "2026-01-01T00:00:00Z",
        &["config", "user.name", "Tiny"],
    );

    // Days 1-4: a + b + c (4 commits — all three files together)
    for i in 1..=4 {
        commit(
            path,
            i,
            &[
                ("a.txt", &format!("a{i}")),
                ("b.txt", &format!("b{i}")),
                ("c.txt", &format!("c{i}")),
            ],
        );
    }
    // Days 5-8: a + b (4 more commits — a and b but no c)
    for i in 5..=8 {
        commit(
            path,
            i,
            &[("a.txt", &format!("a{i}")), ("b.txt", &format!("b{i}"))],
        );
    }
    // Days 9-11: a + x (3 commits — a alone with an unrelated file)
    for i in 9..=11 {
        commit(
            path,
            i,
            &[("a.txt", &format!("a{i}")), ("x.txt", &format!("x{i}"))],
        );
    }
    // Days 12-13: x only (2 commits — creates "neither a nor b" rows
    // for the Fisher 2×2 contingency table).
    for i in 12..=13 {
        commit(path, i, &[("x.txt", &format!("x{i}"))]);
    }
    // Tally check: a=10 commits (1-11), b=8 (1-8), c=4 (1-4), x=5 (9-13).

    let repo = GixRepo::open(path).expect("open");

    // Default mode: c.txt dropped by per-file gate.
    {
        let db = FactsDb::new_in_memory().expect("db");
        let opts = Options {
            repo_path: path.to_path_buf(),
            min_revs: 5,
            min_shared_revs: 1,
            min_coupling_pct: 0,
            fisher_significance: 2.0,
            // Disable lineage so the SQL hits `changes` directly — the
            // fixture has no renames so lineage is a no-op semantically
            // but materialise_source still routes through `changes_lineage`,
            // which would obscure the placeholder-binding semantics
            // under test.
            use_canonical_lineage: false,
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");
        let rows = run_coupling(&db, &opts).expect("run default");
        let has_ab = rows
            .iter()
            .any(|r| r.entity_a == "a.txt" && r.entity_b == "b.txt");
        let has_ac = rows
            .iter()
            .any(|r| r.entity_a == "a.txt" && r.entity_b == "c.txt");
        assert!(has_ab, "default: (a, b) pair must surface; got {rows:?}");
        assert!(
            !has_ac,
            "default per-file gate: (a, c) must be dropped (c has 4 revs < 5); got {rows:?}",
        );
    }

    // Compat mode: c.txt survives because pair-average is 7.
    {
        let db = FactsDb::new_in_memory().expect("db");
        let opts = Options {
            repo_path: path.to_path_buf(),
            min_revs: 5,
            min_shared_revs: 1,
            min_coupling_pct: 0,
            fisher_significance: 2.0,
            code_maat_compat: true,
            use_canonical_lineage: false,
            ..Options::default()
        };
        db.ingest(&repo, &opts).expect("ingest");
        let rows = run_coupling(&db, &opts).expect("run compat");
        let has_ab = rows
            .iter()
            .any(|r| r.entity_a == "a.txt" && r.entity_b == "b.txt");
        let has_ac = rows
            .iter()
            .any(|r| r.entity_a == "a.txt" && r.entity_b == "c.txt");
        assert!(has_ab, "compat: (a, b) pair must surface; got {rows:?}");
        assert!(
            has_ac,
            "compat per-pair-average gate: (a, c) must surface (avg = 7 ≥ 5); got {rows:?}",
        );
    }
}
