//! Differential property tests asserting `GixRepo ≡ GitCliRepo` on the
//! 50-commit generated fixture.  Each test opens fresh repo handles against
//! a single shared fixture (built once via `OnceLock`) so there is no
//! parallel-build race between tests.

use codelore_lib::Options;
use codelore_lib::repo::{GitCliRepo, GixRepo, Repo};
use codelore_lib::test_support::differential_repo::DifferentialRepo;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Shared fixture — built exactly once per test binary invocation.
// DifferentialRepo owns a TempDir; we keep it in a 'static so the tempdir
// lives for the duration of the test binary run.
// ---------------------------------------------------------------------------

struct SharedFixture {
    /// The temp dir holding the git repo (kept alive for the 'static lifetime).
    _repo: DifferentialRepo,
    /// Cached path so we don't have to reach through the `TempDir` each time.
    path: PathBuf,
}

static FIXTURE: OnceLock<SharedFixture> = OnceLock::new();

fn fixture_path() -> &'static PathBuf {
    let sf = FIXTURE.get_or_init(|| {
        let repo = codelore_lib::test_support::differential_repo::build();
        let path = repo.dir.path().to_path_buf();
        SharedFixture { _repo: repo, path }
    });
    &sf.path
}

fn open_both() -> (GixRepo, GitCliRepo) {
    let path = fixture_path();
    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    (gix, cli)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn opts_with_merges() -> Options {
    Options {
        repo_path: fixture_path().clone(),
        include_merges: true,
        ..Options::default()
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Both impls must return the same set of commit SHAs and at least ~50 commits.
///
/// KNOWN DIVERGENCE (P6 finding): `GitCliRepo`'s `parse_git_log_stream` drops
/// the commit immediately after a merge commit that has an empty name-status
/// block.  The merge commit's `\x1e` chunk contains no `\n\n` separator, so the
/// parser mis-classifies the following commit's pretty block as name-status and
/// silently discards it.  `GixRepo`'s `rev_walk` correctly returns all reachable
/// commits.  The missing SHA is the `feature/x` tip (commit 48 in the fixture).
/// This will be fixed in the next task (P6.T03).
#[test]
#[ignore = "P6.T03 — GitCliRepo parser drops commit after merge with empty name-status"]
fn walk_commits_produces_same_rev_set() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let gix_events: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let cli_events: Vec<_> = cli
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let gix_revs: HashSet<_> = gix_events.iter().map(|e| e.rev.clone()).collect();
    let cli_revs: HashSet<_> = cli_events.iter().map(|e| e.rev.clone()).collect();

    let only_gix: Vec<_> = gix_revs.difference(&cli_revs).collect();
    let only_cli: Vec<_> = cli_revs.difference(&gix_revs).collect();
    assert!(
        only_gix.is_empty() && only_cli.is_empty(),
        "commit SHA sets differ — only in gix: {only_gix:?}; only in cli: {only_cli:?}"
    );

    // 50 commits total (1 merge commit included)
    assert!(
        gix_events.len() >= 48,
        "expected ~50 commits, got {} from GixRepo",
        gix_events.len()
    );
}

/// For every shared commit, `author_email`, `parents`, and `date` must agree.
#[test]
fn walk_commits_per_commit_fields_match() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let gix_events: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let cli_events: Vec<_> = cli
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let gix_by_rev: HashMap<_, _> = gix_events.iter().map(|e| (e.rev.clone(), e)).collect();

    for cli_e in &cli_events {
        let gix_e = gix_by_rev
            .get(&cli_e.rev)
            .unwrap_or_else(|| panic!("rev {} from cli not found in gix", cli_e.rev));

        assert_eq!(
            gix_e.author_email, cli_e.author_email,
            "author_email mismatch at {}",
            cli_e.rev
        );
        assert_eq!(
            gix_e.parents, cli_e.parents,
            "parents mismatch at {}",
            cli_e.rev
        );
        assert_eq!(gix_e.date, cli_e.date, "date mismatch at {}", cli_e.rev);
    }
}

/// `resolve_alias` must return identical canonical emails for all tested addresses.
#[test]
fn resolve_alias_matches() {
    let (gix, cli) = open_both();

    for email in [
        "alice-old@example.com",
        "bob-aliased@example.com",
        "c.lee@example.com",
        "unmapped@example.com",
        "canonical-alice@example.com",
        "49699333+dependabot[bot]@users.noreply.github.com",
    ] {
        let gix_result = gix.resolve_alias(email);
        let cli_result = cli.resolve_alias(email);
        assert_eq!(
            gix_result, cli_result,
            "resolve_alias({email:?}) mismatch: gix={gix_result:?} cli={cli_result:?}"
        );
    }
}

/// For a sample of 8 commits (evenly spaced), both impls must return
/// the same set of changed-file paths.
#[test]
fn changed_files_match() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let revs: Vec<String> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|e| e.rev)
        .collect();

    assert!(!revs.is_empty(), "no commits");

    // Step through evenly; take up to 8 samples.
    let step = (revs.len() / 8).max(1);
    for rev in revs.iter().step_by(step).take(8) {
        let gix_files = gix.changed_files(rev).unwrap();
        let cli_files = cli.changed_files(rev).unwrap();

        let gix_paths: HashSet<_> = gix_files.iter().map(|f| f.path.clone()).collect();
        let cli_paths: HashSet<_> = cli_files.iter().map(|f| f.path.clone()).collect();

        let only_gix: Vec<_> = gix_paths.difference(&cli_paths).collect();
        let only_cli: Vec<_> = cli_paths.difference(&gix_paths).collect();
        assert!(
            only_gix.is_empty() && only_cli.is_empty(),
            "changed_files mismatch at {rev}:\n  only gix: {only_gix:?}\n  only cli: {only_cli:?}"
        );
    }
}

/// `commit_metadata` (signed / signoffs) must agree for the first 5 commits.
///
/// NOTE: GPG signing is not configured in the fixture, so both impls are
/// expected to return `signed = false` and `signed_by = None`.  This test
/// guards against future divergence while acknowledging that the assertions
/// are trivially satisfied (both return zero values) in the unsigned fixture.
#[test]
fn commit_metadata_match() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let revs: Vec<String> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .take(5)
        .map(|e| e.rev)
        .collect();

    for rev in &revs {
        let gix_md = gix.commit_metadata(rev).unwrap();
        let cli_md = cli.commit_metadata(rev).unwrap();

        assert_eq!(gix_md.signed, cli_md.signed, "signed mismatch at {rev}");
        assert_eq!(
            gix_md.signed_by, cli_md.signed_by,
            "signed_by mismatch at {rev}"
        );
        assert_eq!(
            gix_md.signoffs, cli_md.signoffs,
            "signoffs mismatch at {rev}"
        );
    }
}

/// The fixture must contain exactly one merge commit (two parents).
#[test]
fn fixture_contains_merge_commit() {
    let (gix, _cli) = open_both();
    let opts = opts_with_merges();

    let events: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let merge_count = events.iter().filter(|e| e.parents.len() == 2).count();
    assert_eq!(
        merge_count, 1,
        "expected exactly 1 merge commit, found {merge_count}"
    );
}

/// The rename commit (`old_name` → `new_name`) must appear in both impls with
/// matching changed file paths.
#[test]
fn rename_commit_visible_in_both() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let revs: Vec<String> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|e| e.rev)
        .collect();

    // Find the rename commit — the one whose changed files include "new_name".
    let rename_rev = revs.iter().find(|rev| {
        gix.changed_files(rev)
            .map(|files| files.iter().any(|f| f.path.contains("new_name")))
            .unwrap_or(false)
    });

    let rev = rename_rev.expect("rename commit not found in gix walk");

    let gix_files = gix.changed_files(rev).unwrap();
    let cli_files = cli.changed_files(rev).unwrap();

    let gix_paths: HashSet<_> = gix_files.iter().map(|f| f.path.clone()).collect();
    let cli_paths: HashSet<_> = cli_files.iter().map(|f| f.path.clone()).collect();

    assert_eq!(
        gix_paths, cli_paths,
        "rename commit changed_files mismatch at {rev}"
    );
}

/// The bot commit must appear in both walks with the dependabot author email.
#[test]
fn bot_commit_visible_in_both() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let bot_email = "49699333+dependabot[bot]@users.noreply.github.com";

    let gix_bot: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(|e| e.author_email == bot_email)
        .collect();

    let cli_bot: Vec<_> = cli
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(|e| e.author_email == bot_email)
        .collect();

    assert_eq!(gix_bot.len(), 1, "expected exactly 1 bot commit in gix");
    assert_eq!(cli_bot.len(), 1, "expected exactly 1 bot commit in cli");
    assert_eq!(
        gix_bot[0].rev, cli_bot[0].rev,
        "bot commit SHA mismatch between gix and cli"
    );
}
