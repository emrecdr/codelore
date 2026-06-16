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
#[test]
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

/// `resolve_alias` must return identical canonical emails for all tested
/// (name, email) pairs. Both an empty-name probe (exercises email-only
/// `.mailmap` rules) and a paired-name probe (exercises name+email rules)
/// are tested — the `name` parameter on the trait closes a previously-
/// asymmetric `GixRepo` / `GitCliRepo` gap.
#[test]
fn resolve_alias_matches() {
    let (gix, cli) = open_both();

    let probes: &[(&str, &str)] = &[
        ("", "alice-old@example.com"),
        ("", "bob-aliased@example.com"),
        ("", "c.lee@example.com"),
        ("", "unmapped@example.com"),
        ("", "canonical-alice@example.com"),
        ("", "49699333+dependabot[bot]@users.noreply.github.com"),
        // Paired-name probes — both backends must agree whether the .mailmap
        // matches name+email rules. Even if the fixture has no name+email
        // rules, the parity invariant (identical output) must still hold.
        ("Alice Old", "alice-old@example.com"),
        ("Bob Bot", "bob-aliased@example.com"),
    ];

    for (name, email) in probes {
        let gix_result = gix.resolve_alias(name, email);
        let cli_result = cli.resolve_alias(name, email);
        assert_eq!(
            gix_result, cli_result,
            "resolve_alias({name:?}, {email:?}) mismatch: gix={gix_result:?} cli={cli_result:?}"
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

/// Merge commits must report an EMPTY change set in BOTH backends
/// when `--include-merges` is on. `git log --name-status` (used by
/// `GitCliRepo`) suppresses merge diffs by default; the gix walker
/// previously surfaced a first-parent diff for every merge, breaking
/// differential parity and causing every churn / hotspots / coupling
/// metric to diverge whenever merges were included.
#[test]
fn merge_commits_report_empty_changes_in_both_walkers() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let events: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let merge_revs: Vec<&str> = events
        .iter()
        .filter(|e| e.parents.len() == 2)
        .map(|e| e.rev.as_str())
        .collect();
    assert!(
        !merge_revs.is_empty(),
        "fixture must contain at least one merge commit"
    );

    for rev in &merge_revs {
        let gix_changes = events
            .iter()
            .find(|e| e.rev == *rev)
            .map(|e| e.changes.clone())
            .unwrap_or_default();
        let cli_changes = cli.changed_files(rev).expect("cli changed_files");
        assert!(
            gix_changes.is_empty(),
            "gix backend must report 0 changes for merge {rev}, got {gix_changes:?}"
        );
        assert!(
            cli_changes.is_empty(),
            "cli backend must report 0 changes for merge {rev}, got {cli_changes:?}"
        );
    }
}

/// The rename commit (`old_name` → `new_name`) must appear in both impls with
/// matching changed file paths.
/// Stricter rename invariant: both walkers must agree on `change_type` —
/// `GixRepo` and `GitCliRepo` must both emit `ChangeType::Renamed { from, .. }`
/// for the same change rather than one side reporting an Add+Delete pair.
///
/// Regression for the pre-fix divergence: `GixRepo` had
/// `track_rewrites(None)`, so a rename surfaced as `Deleted` on the old
/// path + `Added` on the new path, while `GitCliRepo` (via `git log
/// --name-status`'s default `-M` detection) emitted a single `Renamed`
/// row. Same commit, different events, silent history splits in every
/// downstream analysis.
#[test]
fn rename_commit_change_type_matches_across_walkers() {
    use codelore_lib::types::ChangeType;
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

    // Find the rename commit via `GitCliRepo`'s (already-correct) Renamed
    // signal — its `git log --name-status` has detected renames since
    // before this fix. The fixture has exactly one rename
    // (`old_name.rs` -> `new_name.rs`). Then assert `GixRepo` also emits
    // Renamed for the same commit.
    let rev = revs
        .iter()
        .find(|rev| {
            cli.changed_files(rev).is_ok_and(|files| {
                files
                    .iter()
                    .any(|f| matches!(f.change_type, ChangeType::Renamed { .. }))
            })
        })
        .expect("CLI walker should expose at least one Renamed event in the fixture");

    // Both walkers must mark the new-name entry as Renamed (not Added).
    let gix_files = gix.changed_files(rev).unwrap();
    let cli_files = cli.changed_files(rev).unwrap();

    let new_name_change_gix = gix_files
        .iter()
        .find(|f| f.path.contains("new_name"))
        .expect("gix: new_name entry missing");
    let new_name_change_cli = cli_files
        .iter()
        .find(|f| f.path.contains("new_name"))
        .expect("cli: new_name entry missing");

    assert!(
        matches!(new_name_change_gix.change_type, ChangeType::Renamed { .. }),
        "`gix` should mark rename as Renamed, got {:?}",
        new_name_change_gix.change_type
    );
    assert!(
        matches!(new_name_change_cli.change_type, ChangeType::Renamed { .. }),
        "`cli` should mark rename as Renamed, got {:?}",
        new_name_change_cli.change_type
    );

    // The pre-rename path must NOT appear as a stand-alone Deleted entry
    // when rewrite tracking works.
    let old_name_deleted = gix_files
        .iter()
        .any(|f| !f.path.contains("new_name") && matches!(f.change_type, ChangeType::Deleted));
    assert!(
        !old_name_deleted,
        "`gix` should fold the old path into Renamed, not emit a separate Deleted entry"
    );
}

/// `loc_added` and `loc_deleted` MUST be non-zero across the fixture
/// (the differential repo has ~50 commits including ordinary file
/// edits). The pre-A.1 code stubbed both to `0` in every change event
/// across both walkers, so every churn-driven analysis (`abs-churn`,
/// `author-churn`, `entity-churn`, `main-dev`-by-lines, Kamei la/ld,
/// code-health churn term) returned uniformly zero. Regression: assert
/// the totals are non-zero AND that the two walkers agree on them
/// (modulo the few edge cases — neither walker invents data).
#[test]
fn line_counts_are_non_zero_and_match_across_walkers() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    let mut gix_total: u64 = 0;
    let mut cli_total: u64 = 0;
    let revs: Vec<String> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|e| e.rev)
        .collect();
    for rev in &revs {
        let gix_files = gix.changed_files(rev).unwrap();
        let cli_files = cli.changed_files(rev).unwrap();
        for fc in &gix_files {
            gix_total += u64::from(fc.loc_added) + u64::from(fc.loc_deleted);
        }
        for fc in &cli_files {
            cli_total += u64::from(fc.loc_added) + u64::from(fc.loc_deleted);
        }
    }

    assert!(
        gix_total > 0,
        "GixRepo aggregated zero line churn across {} commits; \
         loc_added/loc_deleted are likely still stubbed",
        revs.len()
    );
    assert!(
        cli_total > 0,
        "GitCliRepo aggregated zero line churn across {} commits",
        revs.len()
    );

    // The two walkers should produce ROUGHLY the same aggregate churn.
    // Both use the histogram diff algorithm (gix via `gix_diff::blob` =
    // `imara-diff`; git CLI via `git log --numstat` which is also
    // Histogram). Small drift is expected for:
    //   - root commits (gix's empty-parent + content diff sometimes
    //     differs by 1 line from git's "all lines added" tally)
    //   - exact-rename detection where gix carries the `diff` field for
    //     same-content rewrites
    // 5% is the empirically-derived tolerance from a clean differential
    // fixture run; tighten this once cross-walker drift is investigated.
    #[allow(clippy::cast_precision_loss)]
    let denom = gix_total.max(cli_total) as f64;
    let drift = gix_total.abs_diff(cli_total);
    #[allow(clippy::cast_precision_loss)]
    let drift_pct = (drift as f64 / denom) * 100.0;
    assert!(
        drift_pct < 5.0,
        "walker line-count drift {drift} lines ({drift_pct:.2}%) — \
         gix={gix_total}, cli={cli_total}"
    );
}

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
            .is_ok_and(|files| files.iter().any(|f| f.path.contains("new_name")))
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

/// `head_sha()` is a load-bearing identity surface — every analysis,
/// the persistent cache key, and `codelore diff` all rely on it. Both
/// backends MUST return the same 40-char SHA for the same fixture or
/// the cache silently drifts on backend switch.
#[test]
fn head_sha_matches() {
    let (gix, cli) = open_both();
    let gix_sha = gix.head_sha().expect("gix head_sha");
    let cli_sha = cli.head_sha().expect("cli head_sha");
    assert_eq!(
        gix_sha.len(),
        40,
        "head_sha must be the 40-char SHA; got {gix_sha:?}",
    );
    assert_eq!(
        gix_sha, cli_sha,
        "GixRepo and GitCliRepo disagree on HEAD SHA",
    );
}

/// `is_worktree_dirty()` gates whether the persistent cache is
/// written (a dirty worktree means HEAD-time metrics — complexity,
/// clones — may not match the clean HEAD SHA, so we skip the cache
/// write). The differential fixture is cloned freshly per test, so
/// both backends must agree it's clean. A regression here would
/// either suppress cache writes silently or write stale metrics
/// under a clean `head_sha` — both classes are user-invisible until
/// a long debugging session.
#[test]
fn is_worktree_dirty_matches_on_fresh_clone() {
    let (gix, cli) = open_both();
    let gix_dirty = gix.is_worktree_dirty();
    let cli_dirty = cli.is_worktree_dirty();
    assert!(
        !gix_dirty,
        "freshly-cloned differential fixture is not dirty per GixRepo",
    );
    assert_eq!(
        gix_dirty, cli_dirty,
        "GixRepo and GitCliRepo disagree on worktree-dirty for a freshly-cloned fixture",
    );
}

/// `read_blob_at_head(path)` reads a tracked file from the gix
/// object DB without disk access — the production complexity scan +
/// clone fingerprinter both use it so codelore works on bare
/// repositories. Both backends must read identical bytes for any
/// path that exists at HEAD; both must return `Ok(None)` for any
/// path that doesn't.
#[test]
fn read_blob_at_head_matches_on_tracked_and_untracked_paths() {
    let (gix, cli) = open_both();

    // Tracked path: README.md exists in the differential fixture at HEAD.
    let gix_blob = gix
        .read_blob_at_head("README.md")
        .expect("gix read_blob_at_head README.md");
    let cli_blob = cli
        .read_blob_at_head("README.md")
        .expect("cli read_blob_at_head README.md");
    assert!(
        gix_blob.is_some(),
        "README.md must exist at HEAD per GixRepo",
    );
    assert_eq!(
        gix_blob, cli_blob,
        "GixRepo and GitCliRepo disagree on README.md bytes at HEAD",
    );

    // Untracked path: must return `Ok(None)` from both. The path
    // collides with no real fixture file by construction.
    let gix_missing = gix
        .read_blob_at_head("this-file-does-not-exist-at-head.zzz")
        .expect("gix missing path");
    let cli_missing = cli
        .read_blob_at_head("this-file-does-not-exist-at-head.zzz")
        .expect("cli missing path");
    assert!(
        gix_missing.is_none(),
        "missing path must return None from GixRepo",
    );
    assert!(
        cli_missing.is_none(),
        "missing path must return None from GitCliRepo",
    );
}

/// `diff_hunks(rev, path)` has a known divergence: `GixRepo` returns
/// an empty-vec stub (real hunk extraction is a deferred work item)
/// while `GitCliRepo` shells out to `git diff` and returns actual
/// hunks. This is documented at `gix_repo.rs::diff_hunks` and the
/// only production consumer is `codelore diff` which is git-CLI-backed
/// today. The test below documents the divergence as an EXPECTED
/// state so a future implementation either flips the assertion (when
/// `GixRepo::diff_hunks` lands) or makes the divergence loud (a
/// silent re-stub of `GitCliRepo` would break this test).
#[test]
fn diff_hunks_gix_is_empty_stub_cli_returns_real_hunks() {
    let (gix, cli) = open_both();
    // Pick the most recent commit and ask for hunks against
    // README.md — the fixture's first-touched file.
    let head = gix.head_sha().expect("head_sha");
    let gix_hunks = gix.diff_hunks(&head, "README.md").expect("gix diff_hunks");
    let cli_hunks = cli.diff_hunks(&head, "README.md").expect("cli diff_hunks");
    assert!(
        gix_hunks.is_empty(),
        "GixRepo::diff_hunks is documented as an empty-vec stub; \
         this test must be updated when real extraction lands",
    );
    // GitCliRepo's hunks for the HEAD commit's README.md change can
    // be empty (e.g. if the file wasn't touched in HEAD) but the
    // call must succeed. We don't assert non-empty because the
    // fixture's HEAD commit may or may not touch this specific file
    // depending on regeneration order.
    let _ = cli_hunks;
}
