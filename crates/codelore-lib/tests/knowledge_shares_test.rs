//! Integration tests for `analyses::knowledge::shares::materialize_knowledge_shares`.
//!
//! Tests verify the four contractual invariants:
//! 1. `k_norm` sums to ~1.0 per path (within floating-point tolerance).
//! 2. A single-author path has `k_norm = 1.0` for that author.
//! 3. DOE ranks the file creator above a late minor contributor.
//! 4. Bot rows are absent from both temp tables.
//!
//! Additionally, a `delivery_repo` test asserts that decay ordering
//! holds: an author with a more-recent touch has higher `k` than one
//! whose only touch was much earlier.

#![allow(clippy::doc_markdown)]

use codelore_lib::Options;
use codelore_lib::analyses::knowledge::shares::materialize_knowledge_shares;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

// ---------------------------------------------------------------------------
// coupling_repo tests (single-author — all k_norm values are 1.0 per path)
// ---------------------------------------------------------------------------

/// k_norm for any path in coupling_repo must sum to 1.0 (only one author).
#[test]
fn knowledge_shares_k_norm_sums_to_one_per_path() {
    let coupling = codelore_lib::test_support::coupling_repo::build();
    let repo = GixRepo::open(coupling.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: coupling.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    let totals: Vec<(String, f64)> = db
        .prepare("SELECT path, SUM(k_norm) AS total FROM knowledge_shares GROUP BY path")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    assert!(
        !totals.is_empty(),
        "knowledge_shares must have at least one path"
    );
    for (path, total) in &totals {
        assert!(
            (total - 1.0_f64).abs() < 1e-9,
            "path {path}: k_norm sum = {total}, expected 1.0"
        );
    }
}

/// A single-author path must have k_norm = 1.0 for that author.
#[test]
fn knowledge_shares_single_author_path_gets_full_share() {
    let coupling = codelore_lib::test_support::coupling_repo::build();
    let repo = GixRepo::open(coupling.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: coupling.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    // coupling_repo has only one author so every path row must have k_norm = 1.0.
    let rows: Vec<(String, String, f64)> = db
        .prepare("SELECT path, author, k_norm FROM knowledge_shares WHERE k_norm < 0.9999999")
        .expect("prepare")
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
            ))
        })
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    assert!(
        rows.is_empty(),
        "all paths are single-author — expected k_norm=1.0 everywhere, \
         but found rows with k_norm < 1.0: {rows:?}"
    );
}

/// No bot rows in knowledge_shares.
#[test]
fn knowledge_shares_excludes_bot_authors() {
    let coupling = codelore_lib::test_support::coupling_repo::build();
    let repo = GixRepo::open(coupling.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: coupling.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    let bot_rows: Vec<String> = db
        .prepare(
            "SELECT ks.author FROM knowledge_shares ks
             JOIN author_aliases a ON a.canonical = ks.author AND a.is_bot",
        )
        .expect("prepare")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    assert!(
        bot_rows.is_empty(),
        "knowledge_shares must not contain bot authors; found: {bot_rows:?}"
    );
}

// ---------------------------------------------------------------------------
// DOE tests (coupling_repo — single-author, so file creator = only developer)
// ---------------------------------------------------------------------------

/// doe_scores must be populated and every row expert in a single-author repo.
#[test]
fn doe_file_creator_is_expert() {
    let coupling = codelore_lib::test_support::coupling_repo::build();
    let repo = GixRepo::open(coupling.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: coupling.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    let doe_count: i64 = db
        .query_row("SELECT COUNT(*) FROM doe_scores", [], |r| r.get(0))
        .expect("doe count");
    assert!(
        doe_count > 0,
        "doe_scores must be non-empty after materialize"
    );

    // Single author created every file (fa=1) and is trivially max for each
    // path — every row must be flagged expert.
    let non_expert_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM doe_scores WHERE NOT is_expert",
            [],
            |r| r.get(0),
        )
        .expect("non-expert count");

    assert_eq!(
        non_expert_count, 0,
        "single-author coupling_repo: every DOE row should be flagged expert; \
         found {non_expert_count} non-expert rows"
    );
}

// ---------------------------------------------------------------------------
// delivery_repo tests — decay ordering + sole-author path
// ---------------------------------------------------------------------------

/// `src/stable.rs` is created by Alice (Jan 1) and touched by Bob (Mar 3).
/// Bob's touch is closer to HEAD (Apr 21) so his k must exceed Alice's.
#[test]
fn knowledge_shares_decay_more_recent_touch_has_higher_k() {
    let delivery = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(delivery.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: delivery.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    let rows: Vec<(String, f64)> = db
        .prepare(
            "SELECT author, k FROM knowledge_shares WHERE path = 'src/stable.rs' ORDER BY author",
        )
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    let k_alice = rows
        .iter()
        .find(|(a, _)| a == "alice@example.com")
        .map(|(_, k)| *k)
        .expect("alice must have a knowledge_shares row for src/stable.rs");

    let k_bob = rows
        .iter()
        .find(|(a, _)| a == "bob@example.com")
        .map(|(_, k)| *k)
        .expect("bob must have a knowledge_shares row for src/stable.rs");

    assert!(
        k_bob > k_alice,
        "decay ordering: Bob touched stable.rs later (closer to HEAD) so \
         k_bob ({k_bob:.6}) must exceed k_alice ({k_alice:.6})"
    );
}

/// `src/branch2.rs` is only touched by Carol — her k_norm must be 1.0.
#[test]
fn knowledge_shares_sole_author_gets_full_share_delivery() {
    let delivery = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(delivery.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: delivery.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    let rows: Vec<(String, f64)> = db
        .prepare("SELECT author, k_norm FROM knowledge_shares WHERE path = 'src/branch2.rs'")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    assert_eq!(
        rows.len(),
        1,
        "src/branch2.rs should have exactly 1 knowledge_shares row (Carol only); got {rows:?}"
    );
    let (author, k_norm) = &rows[0];
    assert_eq!(author, "carol@example.com", "sole author must be Carol");
    assert!(
        (k_norm - 1.0_f64).abs() < 1e-9,
        "Carol's k_norm for src/branch2.rs must be 1.0, got {k_norm}"
    );
}

/// DOE ranking: the file's creator outranks a late minor contributor.
///
/// In `delivery_repo`, `src/rework.rs` is created and expanded by Alice
/// (fa = 1, large `adds`); Bob's only touch is the day-8 trim (fa = 0,
/// `adds` ≈ 1 line). Alice's DOE must therefore be strictly higher.
#[test]
fn doe_ranks_file_creator_above_late_minor_contributor() {
    let delivery = codelore_lib::test_support::delivery_repo::build();
    let repo = GixRepo::open(delivery.dir.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: delivery.dir.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    let rows: Vec<(String, f64)> = db
        .prepare("SELECT author, doe FROM doe_scores WHERE path = 'src/rework.rs'")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    let alice = rows
        .iter()
        .find(|(a, _)| a == "alice@example.com")
        .unwrap_or_else(|| {
            panic!("Alice must have a doe_scores row for src/rework.rs; got {rows:?}")
        });
    let bob = rows
        .iter()
        .find(|(a, _)| a == "bob@example.com")
        .unwrap_or_else(|| {
            panic!("Bob must have a doe_scores row for src/rework.rs; got {rows:?}")
        });

    assert!(
        alice.1 > bob.1,
        "creator (Alice, doe={}) must outrank late minor contributor (Bob, doe={})",
        alice.1,
        bob.1
    );
}

/// Run one git command in `path` with a fully deterministic identity
/// (author = committer, fixed dates). Used by the trailer fixture below.
fn git_as(path: &std::path::Path, args: &[&str], author: &str, email: &str, date: &str) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", author)
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

/// Reviewer-credit path end-to-end: a commit carrying a `Reviewed-by:`
/// trailer must (a) not fail materialization (DuckDB rejects window
/// functions inside UPDATE — the re-normalization rebuilds the temp
/// table instead), (b) credit the reviewer on paths they never authored,
/// and (c) leave `k_norm` summing to ~1.0 per path after re-normalization.
#[test]
fn reviewer_trailer_credits_reviewer_and_renormalizes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let git = |args: &[&str], author: &str, email: &str, date: &str| {
        git_as(path, args, author, email, date);
    };
    git(
        &["init", "--quiet"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );
    git(
        &["config", "gc.auto", "0"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );

    // Commit 1: Alice authors her own file (puts her in author_aliases).
    std::fs::write(path.join("alice.rs"), "pub fn a() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "alice.rs"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );
    git(
        &["commit", "--quiet", "-m", "feat: alice file"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );

    // Commit 2: Bob authors bob.rs; Alice is credited via Reviewed-by trailer.
    std::fs::write(path.join("bob.rs"), "pub fn b() -> u32 { 2 }\n").expect("write");
    git(
        &["add", "bob.rs"],
        "Bob",
        "bob@example.com",
        "2026-01-02T10:00:00Z",
    );
    git(
        &[
            "commit",
            "--quiet",
            "-m",
            "feat: bob file\n\nReviewed-by: Alice <alice@example.com>",
        ],
        "Bob",
        "bob@example.com",
        "2026-01-02T10:00:00Z",
    );
    git(
        &["repack", "-d", "--quiet"],
        "Alice",
        "alice@example.com",
        "2026-01-02T10:00:00Z",
    );

    let repo = GixRepo::open(path).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: path.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // (a) The reviewer path must not error.
    materialize_knowledge_shares(&db, &opts).expect("materialize with reviewer trailer");

    // (b) Alice must hold a share of bob.rs despite never authoring it.
    let alice_on_bob: f64 = db
        .prepare(
            "SELECT COALESCE(SUM(k_norm), 0) FROM knowledge_shares
             WHERE path = 'bob.rs' AND author = 'alice@example.com'",
        )
        .expect("prepare")
        .query_map([], |r| r.get::<_, f64>(0))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")[0];
    assert!(
        alice_on_bob > 0.0,
        "reviewer credit must give Alice a share of bob.rs; got {alice_on_bob}"
    );

    // (c) k_norm still sums to ~1.0 per path after re-normalization.
    let totals: Vec<(String, f64)> = db
        .prepare("SELECT path, SUM(k_norm) FROM knowledge_shares GROUP BY path")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    for (p, total) in &totals {
        assert!(
            (total - 1.0).abs() < 1e-9,
            "k_norm for {p} must sum to 1.0 after reviewer re-normalization, got {total}"
        );
    }
}

/// Build a 3-commit fixture where Alice both authors `shared.rs` (contributor
/// row) and is credited as reviewer on it via a trailer on Bob's later commit
/// to the same path. When `with_trailer` is false, Bob's final commit carries
/// no trailer, so Alice never gets reviewer credit — used as a control to
/// compare against the merged (contributor + reviewer) knowledge value.
fn build_overlap_repo(with_trailer: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let git = |args: &[&str], author: &str, email: &str, date: &str| {
        git_as(path, args, author, email, date);
    };
    git(
        &["init", "--quiet"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );
    git(
        &["config", "gc.auto", "0"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );

    // Commit 1: Alice authors alice.rs (registers her identity).
    std::fs::write(path.join("alice.rs"), "pub fn a() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "alice.rs"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );
    git(
        &["commit", "--quiet", "-m", "feat: alice file"],
        "Alice",
        "alice@example.com",
        "2026-01-01T10:00:00Z",
    );

    // Commit 2: Alice authors shared.rs — this is her contributor row.
    std::fs::write(path.join("shared.rs"), "pub fn s() -> u32 { 1 }\n").expect("write");
    git(
        &["add", "shared.rs"],
        "Alice",
        "alice@example.com",
        "2026-01-02T10:00:00Z",
    );
    git(
        &["commit", "--quiet", "-m", "feat: alice creates shared.rs"],
        "Alice",
        "alice@example.com",
        "2026-01-02T10:00:00Z",
    );

    // Commit 3: Bob edits shared.rs. With `with_trailer`, the message credits
    // Alice as reviewer via `Reviewed-by:` on the SAME path she already
    // authored — this is the overlap that must merge into one row.
    std::fs::write(
        path.join("shared.rs"),
        "pub fn s() -> u32 { 1 }\npub fn s2() -> u32 { 2 }\n",
    )
    .expect("write");
    git(
        &["add", "shared.rs"],
        "Bob",
        "bob@example.com",
        "2026-01-03T10:00:00Z",
    );
    let message = if with_trailer {
        "feat: bob extends shared.rs\n\nReviewed-by: Alice <alice@example.com>"
    } else {
        "feat: bob extends shared.rs"
    };
    git(
        &["commit", "--quiet", "-m", message],
        "Bob",
        "bob@example.com",
        "2026-01-03T10:00:00Z",
    );
    git(
        &["repack", "-d", "--quiet"],
        "Alice",
        "alice@example.com",
        "2026-01-03T10:00:00Z",
    );

    dir
}

/// An author who is BOTH a contributor and a reviewer-trailer on the same
/// path must yield exactly one `(path, author)` row in `knowledge_shares`,
/// with `k` reflecting the sum of her contributor and reviewer-credit
/// weights (not two un-merged rows, which would let `code_familiarity`'s
/// `ROW_NUMBER() OVER (PARTITION BY path ORDER BY k_norm DESC)` rank the
/// same person #1 and #2, and would bias `coordination_needs` fragmentation
/// `1 − Σk_norm²` high since `a² + b² < (a + b)²`).
#[test]
fn reviewer_and_contributor_shares_merge_into_one_row_per_path() {
    let with_trailer = build_overlap_repo(true);
    let repo = GixRepo::open(with_trailer.path()).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: with_trailer.path().to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    materialize_knowledge_shares(&db, &opts).expect("materialize with overlap");

    let rows: Vec<(f64, f64)> = db
        .prepare(
            "SELECT k, k_norm FROM knowledge_shares
             WHERE path = 'shared.rs' AND author = 'alice@example.com'",
        )
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");

    assert_eq!(
        rows.len(),
        1,
        "alice contributed to AND is a reviewer-trailer on shared.rs — must \
         merge into exactly one (path, author) row, got {rows:?}"
    );
    let k_merged = rows[0].0;

    // k_norm must still sum to ~1.0 per path after the merge.
    let totals: Vec<(String, f64)> = db
        .prepare("SELECT path, SUM(k_norm) FROM knowledge_shares GROUP BY path")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    for (p, total) in &totals {
        assert!(
            (total - 1.0).abs() < 1e-9,
            "k_norm for {p} must sum to 1.0 after merge, got {total}"
        );
    }

    // Control: without the trailer, Alice only holds her contributor k.
    let without_trailer = build_overlap_repo(false);
    let repo2 = GixRepo::open(without_trailer.path()).expect("GixRepo::open");
    let db2 = FactsDb::new_in_memory().expect("new_in_memory");
    let opts2 = Options {
        repo_path: without_trailer.path().to_path_buf(),
        ..Options::default()
    };
    db2.ingest(&repo2, &opts2).expect("ingest");
    materialize_knowledge_shares(&db2, &opts2).expect("materialize without overlap");

    let k_contributor_only: f64 = db2
        .prepare(
            "SELECT k FROM knowledge_shares
             WHERE path = 'shared.rs' AND author = 'alice@example.com'",
        )
        .expect("prepare")
        .query_map([], |r| r.get::<_, f64>(0))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")[0];

    assert!(
        k_merged > k_contributor_only,
        "merged k ({k_merged}) must exceed contributor-only k \
         ({k_contributor_only}) — the reviewer-credit weight must be added, \
         not dropped, when merging into one row"
    );
}

/// Two authors that share one commit email but carry different names, with a
/// `.mailmap` whose two name+email rules resolve them to distinct canonicals.
///
/// The alias table is keyed on `(raw_name, raw_email)`, so it can hold both
/// resolutions; an email-only key would keep only the first-walked identity
/// and silently drop the other's commits from every canonical-set consumer.
/// The discriminating assertion is on a file BOTH authors touch: with both
/// canonicals present it is co-owned (neither `k_norm` reaches the 0.8 island
/// threshold); had the loser's commits been dropped, the survivor would own
/// 100% of it — a spurious knowledge island.
#[test]
fn shared_commit_email_with_name_mailmap_keeps_both_canonicals() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let date = "2026-01-01T10:00:00Z";
    let git = |args: &[&str], author: &str, email: &str| {
        git_as(path, args, author, email, date);
    };

    git(&["init", "--quiet"], "Alice", "shared@c");
    git(&["config", "gc.auto", "0"], "Alice", "shared@c");

    // 4-token rules: "Alice <shared@c>" → alice@c, "Bob <shared@c>" → bob@c.
    // Same commit email, different names, different canonicals.
    std::fs::write(
        path.join(".mailmap"),
        "Alice Smith <alice@c> Alice <shared@c>\nBob Jones <bob@c> Bob <shared@c>\n",
    )
    .expect("write mailmap");

    // Commit 1 — Alice creates shared.rs (2 lines added).
    std::fs::write(
        path.join("shared.rs"),
        "pub fn a() -> u32 { 1 }\npub fn a2() -> u32 { 11 }\n",
    )
    .expect("write");
    git(&["add", ".mailmap", "shared.rs"], "Alice", "shared@c");
    git(
        &["commit", "--quiet", "-m", "feat: alice"],
        "Alice",
        "shared@c",
    );

    // Commit 2 — Bob extends shared.rs (2 lines added), same date so the
    // decayed weights are equal and the split is ~0.5/0.5.
    std::fs::write(
        path.join("shared.rs"),
        "pub fn a() -> u32 { 1 }\npub fn a2() -> u32 { 11 }\n\
         pub fn b() -> u32 { 2 }\npub fn b2() -> u32 { 22 }\n",
    )
    .expect("write");
    git(&["add", "shared.rs"], "Bob", "shared@c");
    git(&["commit", "--quiet", "-m", "feat: bob"], "Bob", "shared@c");

    let repo = GixRepo::open(path).expect("GixRepo::open");
    let db = FactsDb::new_in_memory().expect("new_in_memory");
    let opts = Options {
        repo_path: path.to_path_buf(),
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");

    // The alias table represents BOTH resolved canonicals, each non-bot.
    let aliases: Vec<(String, bool)> = db
        .prepare("SELECT canonical, is_bot FROM author_aliases ORDER BY canonical")
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    assert_eq!(
        aliases,
        vec![("alice@c".to_string(), false), ("bob@c".to_string(), false)],
        "both name+email identities must resolve to their own canonical row; \
         an email-only key would keep only one"
    );

    // Both canonicals reach the commits table.
    let commit_canons: Vec<String> = db
        .prepare("SELECT DISTINCT canonical_author FROM commits ORDER BY canonical_author")
        .expect("prepare")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    assert_eq!(commit_canons, vec!["alice@c", "bob@c"]);

    materialize_knowledge_shares(&db, &opts).expect("materialize");

    // shared.rs is co-owned: one row per canonical, neither an island.
    let shares: Vec<(String, f64)> = db
        .prepare(
            "SELECT author, k_norm FROM knowledge_shares \
             WHERE path = 'shared.rs' ORDER BY author",
        )
        .expect("prepare")
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .expect("query_map")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    assert_eq!(
        shares.len(),
        2,
        "shared.rs must be co-owned by both canonicals — the loser's commits \
         must not vanish; got {shares:?}"
    );
    assert_eq!(shares[0].0, "alice@c");
    assert_eq!(shares[1].0, "bob@c");
    let max_share = shares[0].1.max(shares[1].1);
    assert!(
        max_share < 0.8,
        "neither author may cross the 0.8 island threshold on a co-owned \
         file; got {shares:?}"
    );
    let total: f64 = shares[0].1 + shares[1].1;
    assert!(
        (total - 1.0).abs() < 1e-9,
        "k_norm must sum to 1.0 for shared.rs; got {total}"
    );
}
