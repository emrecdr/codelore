//! Regression coverage for pair-granular bot filtering.
//!
//! `author_aliases` is keyed on `(raw_name, raw_email)` and `is_bot` rides
//! that pair — the schema comment on `author_aliases` (`schema_v1.sql`)
//! documents that a human and a bot sharing one canonical identity must
//! classify independently. Several consumers used to collapse that to a
//! canonical-level `BOOL_OR(is_bot)` (or `HAVING NOT BOOL_OR(is_bot)`),
//! which erases (or mislabels) a human's contribution the instant ANY alias
//! sharing their canonical is bot-classified.
//!
//! This fixture forces exactly that mixed-canonical case via the
//! raw-email canonical fallback: two different `(name, email)` pairs that
//! share one email (`shared@example.com`) resolve to the same canonical
//! (no `.mailmap` involved), but only one of the two names matches a bot
//! pattern (`dependabot[bot]`). A separate pure-bot canonical and a
//! separate pure-human canonical are included as unaffected controls.

use codelore_lib::Options;
use codelore_lib::analyses::authors::run_authors;
use codelore_lib::analyses::bus_factor::run_bus_factor;
use codelore_lib::analyses::top_committers::run_top_committers;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;

/// Run one git command in `path` with an explicit author/committer identity
/// and fixed date. Mirrors `bus_factor_test.rs::git_as`.
fn git_as(path: &std::path::Path, date: &str, author: &str, email: &str, args: &[&str]) {
    use std::process::Command;
    let status = Command::new("git")
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

const SHARED_EMAIL: &str = "shared@example.com";
const HUMAN_NAME: &str = "Alice";
const BOT_NAME: &str = "dependabot[bot]";
const BOB_EMAIL: &str = "bob@example.com";
const PURE_BOT_EMAIL: &str = "renovate-bot@noreply.example.com";
const PURE_BOT_NAME: &str = "renovate[bot]";

/// Builds a repo with a single file (`src/a.rs`) touched by three canonical
/// identities:
///
/// - `bob@example.com` (pure human): 2 commits.
/// - `shared@example.com` (MIXED canonical, raw-email fallback): 3 commits
///   under the human-named pair (`Alice`, `shared@example.com`) + 5 commits
///   under the bot-named pair (`dependabot[bot]`, `shared@example.com`).
/// - `renovate-bot@noreply.example.com` (pure bot): 4 commits.
fn build_mixed_canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git_as(
        path,
        "2026-01-01T00:00:00Z",
        "Bob",
        BOB_EMAIL,
        &["init", "-b", "main", "--quiet"],
    );

    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(path.join("src/a.rs"), "fn a() {}\n").unwrap();
    git_as(
        path,
        "2026-01-01T12:00:00Z",
        "Bob",
        BOB_EMAIL,
        &["add", "src/a.rs"],
    );
    git_as(
        path,
        "2026-01-01T12:00:00Z",
        "Bob",
        BOB_EMAIL,
        &["commit", "-m", "b1", "--quiet"],
    );

    // Bob's second (and last) commit — b1 (add) + b2 (edit) = 2 total.
    std::fs::write(path.join("src/a.rs"), "fn a() {}\n// bob 1\n").unwrap();
    git_as(
        path,
        "2026-01-02T12:00:00Z",
        "Bob",
        BOB_EMAIL,
        &["commit", "-am", "b2", "--quiet"],
    );

    for (i, day) in [3u32, 4, 5].iter().enumerate() {
        std::fs::write(
            path.join("src/a.rs"),
            format!("fn a() {{}}\n// alice {i}\n"),
        )
        .unwrap();
        git_as(
            path,
            &format!("2026-01-{day:02}T12:00:00Z"),
            HUMAN_NAME,
            SHARED_EMAIL,
            &["commit", "-am", &format!("s{i}"), "--quiet"],
        );
    }

    for (i, day) in [6u32, 7, 8, 9, 10].iter().enumerate() {
        std::fs::write(path.join("src/a.rs"), format!("fn a() {{}}\n// bot {i}\n")).unwrap();
        git_as(
            path,
            &format!("2026-01-{day:02}T12:00:00Z"),
            BOT_NAME,
            SHARED_EMAIL,
            &["commit", "-am", &format!("d{i}"), "--quiet"],
        );
    }

    for (i, day) in [11u32, 12, 13, 14].iter().enumerate() {
        std::fs::write(
            path.join("src/a.rs"),
            format!("fn a() {{}}\n// renovate {i}\n"),
        )
        .unwrap();
        git_as(
            path,
            &format!("2026-01-{day:02}T12:00:00Z"),
            PURE_BOT_NAME,
            PURE_BOT_EMAIL,
            &["commit", "-am", &format!("r{i}"), "--quiet"],
        );
    }

    dir
}

fn open_and_ingest(dir: &tempfile::TempDir) -> (FactsDb, Options) {
    let repo = GixRepo::open(dir.path()).expect("open");
    let db = FactsDb::new_in_memory().expect("db");
    let opts = Options {
        repo_path: dir.path().to_path_buf(),
        min_revs: 1,
        ..Options::default()
    };
    db.ingest(&repo, &opts).expect("ingest");
    (db, opts)
}

/// Precondition: both the human-named and bot-named `shared@example.com`
/// pairs resolve to the SAME canonical (raw-email fallback, no `.mailmap`),
/// and only the bot-named pair is flagged `is_bot`.
fn assert_mixed_canonical_registered(db: &FactsDb) {
    let alice: (String, bool) = db
        .query_row(
            "SELECT canonical, is_bot FROM author_aliases \
             WHERE raw_name = ? AND raw_email = ?",
            duckdb::params![HUMAN_NAME, SHARED_EMAIL],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
        )
        .expect("alice alias row present");
    let bot: (String, bool) = db
        .query_row(
            "SELECT canonical, is_bot FROM author_aliases \
             WHERE raw_name = ? AND raw_email = ?",
            duckdb::params![BOT_NAME, SHARED_EMAIL],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
        )
        .expect("bot alias row present");
    assert_eq!(
        alice.0, bot.0,
        "both pairs must resolve to the same canonical (raw-email fallback)"
    );
    assert!(!alice.1, "the human-named pair must not be bot-classified");
    assert!(bot.1, "the bot-named pair must be bot-classified");
}

#[test]
fn authors_counts_mixed_canonical_as_human_via_its_human_pair() {
    let fixture = build_mixed_canonical_repo();
    let (db, opts) = open_and_ingest(&fixture);
    assert_mixed_canonical_registered(&db);

    let rows = run_authors(&db, &opts).expect("run authors");
    let row = rows
        .iter()
        .find(|r| r.entity == "src/a.rs")
        .expect("src/a.rs row present");

    // n_authors counts every distinct canonical touching the file,
    // regardless of bot status — unaffected by the fix.
    assert_eq!(row.n_authors, 3, "bob, shared@example.com, renovate-bot");
    // n_revs sums every commit regardless of bot status — unaffected.
    assert_eq!(row.n_revs, 2 + 3 + 5 + 4);

    // The pair-granular fix: `shared@example.com` has a human alias
    // (Alice), so it must classify as human despite ALSO owning a
    // bot-classified alias on the same canonical. Bob (pure human) stays
    // human. `renovate-bot@...` (pure bot) stays bot.
    assert_eq!(
        row.n_humans, 2,
        "bob + shared@example.com must both count as human; got n_humans={} n_bots={}",
        row.n_humans, row.n_bots
    );
    assert_eq!(
        row.n_bots, 1,
        "only the pure-bot canonical (renovate-bot) may count as bot; got n_bots={}",
        row.n_bots
    );
}

#[test]
fn top_committers_labels_mixed_canonical_as_non_bot() {
    let fixture = build_mixed_canonical_repo();
    let (db, opts) = open_and_ingest(&fixture);
    assert_mixed_canonical_registered(&db);

    let rows = run_top_committers(&db, &opts).expect("run top-committers");

    let shared = rows
        .iter()
        .find(|r| r.author == SHARED_EMAIL)
        .expect("shared@example.com row present");
    // top-committers never drops rows — it only labels. The commit count
    // must include EVERY commit under the canonical (human-paired +
    // bot-paired), but the `is_bot` label must reflect that a human alias
    // exists for this canonical.
    assert_eq!(
        shared.commits,
        3 + 5,
        "commits are counted regardless of pair"
    );
    assert!(
        !shared.is_bot,
        "a canonical with at least one human alias must not be labeled is_bot"
    );

    let bob = rows
        .iter()
        .find(|r| r.author == BOB_EMAIL)
        .expect("bob row present");
    assert!(!bob.is_bot, "pure-human canonical must remain unaffected");

    let pure_bot = rows
        .iter()
        .find(|r| r.author == PURE_BOT_EMAIL)
        .expect("pure-bot row present");
    assert!(
        pure_bot.is_bot,
        "a canonical with ONLY bot aliases must remain labeled is_bot"
    );
}

#[test]
fn bus_factor_counts_only_the_human_pairs_commits_for_mixed_canonical() {
    let fixture = build_mixed_canonical_repo();
    let (db, opts) = open_and_ingest(&fixture);
    assert_mixed_canonical_registered(&db);

    let rows = run_bus_factor(&db, &opts).expect("run bus-factor");
    let src = rows
        .iter()
        .find(|r| r.module == "src")
        .expect("src module present");

    // Module total must count Bob's 2 human commits plus ONLY Alice's 3
    // human-paired commits on `shared@example.com` — never the 5
    // bot-paired commits under that same canonical, and never any of the
    // pure-bot canonical's 4 commits.
    assert_eq!(
        src.total_commits,
        2 + 3,
        "must exclude the bot-paired commits row-wise, and the pure-bot canonical entirely"
    );
    assert_eq!(
        src.top_contributor, SHARED_EMAIL,
        "shared@example.com's 3 surviving human commits outrank bob's 2"
    );
    assert!(
        (src.top_contributor_share - 0.6).abs() < 1e-9,
        "3 of 5 counted commits => 0.6 share, got {}",
        src.top_contributor_share
    );
}
