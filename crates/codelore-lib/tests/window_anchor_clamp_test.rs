//! End-to-end guard for the wall-clock window-anchor clamp.
//!
//! A single future-dated commit must not become the "now" that every
//! trailing-window and time-decay term anchors on. Here a real three-author
//! history is poisoned with one commit dated in the year 2099. Without the
//! clamp, `MAX(commits.date)` is 2099, the trailing active-author window opens
//! in ~2098, every real author falls outside it, and `active_authors`
//! collapses to the single future-dated author while every real author's
//! knowledge-decay term underflows. Clamped, the anchor is the wall clock, so
//! all real authors stay active and their shares stay finite.

#![cfg(feature = "test-support")]

use codelore_lib::Options;
use codelore_lib::analyses::code_familiarity::run_code_familiarity;
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::gix_repo::GixRepo;

/// Run one git command with a fixed identity and author/committer date.
fn git(path: &std::path::Path, args: &[&str], author: &str, email: &str, date: &str) {
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

/// ISO-8601 UTC timestamp `days` days before the current instant, hand-
/// formatted so the `time` crate's `formatting` feature stays off (matching
/// the ingest-side timestamp formatter). Real fixture commits are dated
/// relative to now so they always fall inside the default trailing window,
/// whatever day the suite runs.
fn iso_days_ago(days: i64) -> String {
    let t = time::OffsetDateTime::now_utc() - time::Duration::days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
    )
}

#[test]
fn a_future_dated_commit_does_not_collapse_the_active_author_window() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    let init_date = iso_days_ago(60);
    git(
        path,
        &["init", "--quiet"],
        "Alice",
        "alice@example.com",
        &init_date,
    );
    git(
        path,
        &["config", "gc.auto", "0"],
        "Alice",
        "alice@example.com",
        &init_date,
    );

    // Three real authors, each touching their own source file within the last
    // two months. A fourth author lands a commit stamped in 2099.
    let src = |body: &str| format!("pub fn f() -> i32 {{ {body} }}\n");
    let commits = [
        ("alpha.rs", "Alice", "alice@example.com", iso_days_ago(40)),
        ("beta.rs", "Bob", "bob@example.com", iso_days_ago(20)),
        ("gamma.rs", "Carol", "carol@example.com", iso_days_ago(5)),
        // The poison: a far-future author date.
        (
            "future.rs",
            "Dave",
            "dave@example.com",
            "2099-01-01T00:00:00Z".to_string(),
        ),
    ];
    for (i, (file, author, email, date)) in commits.iter().enumerate() {
        std::fs::write(path.join(file), src(&i.to_string())).expect("write source");
        git(path, &["add", file], author, email, date);
        git(
            path,
            &["commit", "-m", &format!("add {file}")],
            author,
            email,
            date,
        );
    }

    let db = FactsDb::new_in_memory().expect("in-memory db");
    let repo = GixRepo::open(path).expect("open repo");
    let opts = Options {
        repo_path: path.to_path_buf(),
        window_days: 365,
        min_revs: 1,
        ..Options::default()
    };
    // Ingest also emits the once-per-ingest future-date warning for the 2099
    // commit (asserted directly in the ingest module's unit tests).
    db.ingest(&repo, &opts).expect("ingest");

    let rows = run_code_familiarity(&db, &opts).expect("run code-familiarity");
    let row = rows.first().expect("a repo-scope familiarity row");

    // All four authors committed within [now − 365 d, now] once the anchor is
    // clamped to the wall clock. Without the clamp the window opens in ~2098
    // and this collapses to 1 (only Dave, the future-dated author).
    assert_eq!(
        row.active_authors, 4,
        "the clamp must keep every real author active; a raw MAX(date) anchor \
         would collapse this to the single 2099 author"
    );
    // The real authors' decay terms stay finite and positive rather than
    // underflowing against a 2099 anchor.
    assert!(
        row.familiarity_pct.is_finite() && row.familiarity_pct > 0.0,
        "familiarity_pct must stay finite and positive, got {}",
        row.familiarity_pct
    );
}
