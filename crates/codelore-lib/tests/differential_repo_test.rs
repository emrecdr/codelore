//! Differential property tests asserting `GixRepo ≡ GitCliRepo` on the
//! 50-commit generated fixture.  Each test opens fresh repo handles against
//! a single shared fixture (built once via `OnceLock`) so there is no
//! parallel-build race between tests.

use codelore_lib::Options;
use codelore_lib::repo::{GitCliRepo, GixRepo, Repo};
use codelore_lib::test_support::delivery_repo::DeliveryRepo;
use codelore_lib::test_support::differential_repo::DifferentialRepo;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Shared delivery_repo fixture — built once; used by the tags test only.
// ---------------------------------------------------------------------------

struct SharedDelivery {
    _repo: DeliveryRepo,
    path: PathBuf,
}

static DELIVERY: OnceLock<SharedDelivery> = OnceLock::new();

fn delivery_path() -> &'static PathBuf {
    let sd = DELIVERY.get_or_init(|| {
        let repo = codelore_lib::test_support::delivery_repo::build();
        let path = repo.dir.path().to_path_buf();
        SharedDelivery { _repo: repo, path }
    });
    &sd.path
}

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

/// For every shared commit, `author_email`, `parents`, `date`, and
/// `committer_date` must agree. The `committer_date` parity in
/// particular is load-bearing for the `lead-time` and
/// `delivery-friction` analyses — those derive their signal from
/// `(committer_date - date)` and silent divergence would produce
/// different friction scores depending on which backend ingested the
/// cache.
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
        assert_eq!(
            gix_e.committer_date, cli_e.committer_date,
            "committer_date mismatch at {}",
            cli_e.rev
        );
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
        // MATCHING-name probes for the fixture's two 4-token rules (confirmed
        // against the bundle's committed .mailmap: `Alice Canonical
        // <canonical-alice@example.com> Alice <alice-old@example.com>` and
        // `Carol Lee <carol@example.com> Carol <c.lee@example.com>` — both
        // gate on commit name AND email, unlike Bob's 3-token, email-only
        // rule). Every probe above pairs a non-matching name with an aliased
        // email, so both backends only ever exercised the NO-MATCH path —
        // an email-only resolution regression (e.g. a backend that passes
        // only the email to its mailmap lookup, ignoring name) would be
        // invisible. These two probes use the exact commit name each rule
        // requires, so a genuine 4-token match is exercised on both backends.
        ("Alice", "alice-old@example.com"),
        ("Carol", "c.lee@example.com"),
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

/// An untracked file must never mark the tree dirty in either backend —
/// HEAD-time metrics (`complexity`, `clones`) are computed over
/// `tracked_paths_at_head()` only, so a stray untracked file (e.g. a
/// screenshot dropped in the repo root) cannot affect them and must not
/// trip the `calibrate-defects` mining guard or suppress the persistent
/// cache. Each test below clones its own fixture copy (rather than the
/// shared `fixture_path()`) because it mutates the worktree.
#[test]
fn is_worktree_dirty_ignores_untracked_file() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();
    std::fs::write(path.join("untracked_scratch.txt"), b"scratch\n").expect("write untracked file");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_dirty = gix.is_worktree_dirty();
    let cli_dirty = cli.is_worktree_dirty();

    assert!(
        !gix_dirty,
        "an untracked-only worktree must report clean per GixRepo",
    );
    assert_eq!(
        gix_dirty, cli_dirty,
        "GixRepo and GitCliRepo disagree on an untracked-only worktree",
    );
}

/// Editing a tracked file without staging it (worktree vs. index differ)
/// must mark both backends dirty.
#[test]
fn is_worktree_dirty_detects_unstaged_tracked_modification() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();
    std::fs::write(path.join("README.md"), b"unstaged edit\n").expect("edit tracked file");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_dirty = gix.is_worktree_dirty();
    let cli_dirty = cli.is_worktree_dirty();

    assert!(
        gix_dirty,
        "an unstaged edit to a tracked file must mark GixRepo dirty",
    );
    assert_eq!(
        gix_dirty, cli_dirty,
        "GixRepo and GitCliRepo disagree on an unstaged tracked-file modification",
    );
}

/// Staging a tracked-file edit (index vs. `HEAD` differ; worktree matches
/// the index) must still mark both backends dirty.
#[test]
fn is_worktree_dirty_detects_staged_tracked_modification() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();
    std::fs::write(path.join("README.md"), b"staged edit\n").expect("edit tracked file");
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "README.md"])
        .status()
        .expect("spawn git add");
    assert!(status.success(), "git add README.md failed");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_dirty = gix.is_worktree_dirty();
    let cli_dirty = cli.is_worktree_dirty();

    assert!(
        gix_dirty,
        "a staged edit to a tracked file must mark GixRepo dirty",
    );
    assert_eq!(
        gix_dirty, cli_dirty,
        "GixRepo and GitCliRepo disagree on a staged tracked-file modification",
    );
}

/// A fresh clone has no merge, rebase, cherry-pick, or revert underway;
/// both backends must agree it is clean. The agent-loop briefing tools key
/// their ambiguous-HEAD disclosure off this signal, so a false positive here
/// would slap a spurious "merge in progress" note on every ordinary briefing.
#[test]
fn merge_or_rebase_state_clean_on_fresh_clone() {
    let (gix, cli) = open_both();
    assert!(
        !gix.merge_or_rebase_in_progress(),
        "a freshly-cloned fixture is not mid-merge/rebase per GixRepo",
    );
    assert_eq!(
        gix.merge_or_rebase_in_progress(),
        cli.merge_or_rebase_in_progress(),
        "GixRepo and GitCliRepo disagree on a clean fresh clone",
    );
}

/// A conflicted merge leaves `MERGE_HEAD` in the git dir. Writing that file
/// (the exact artifact `git merge` leaves behind on conflict) must flip both
/// backends to "in progress" and they must agree. Uses a private fixture
/// clone because it mutates the git dir.
#[test]
fn merge_or_rebase_state_detects_merge_head() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();
    // 40 zeros is a valid-looking object id; both backends (and git itself)
    // key on the marker's mere presence, not its contents.
    std::fs::write(path.join(".git").join("MERGE_HEAD"), "0".repeat(40)).expect("write MERGE_HEAD");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    assert!(gix.merge_or_rebase_in_progress(), "gix must see MERGE_HEAD");
    assert!(cli.merge_or_rebase_in_progress(), "cli must see MERGE_HEAD");
}

/// A rebase leaves a `rebase-merge/` DIRECTORY (not a single file) in the git
/// dir. Both backends must treat the directory marker the same as the file
/// markers — this exercises the directory-existence branch that the
/// `MERGE_HEAD` test (a plain file) does not.
#[test]
fn merge_or_rebase_state_detects_rebase_merge_dir() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();
    std::fs::create_dir(path.join(".git").join("rebase-merge")).expect("create rebase-merge dir");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    assert!(
        gix.merge_or_rebase_in_progress(),
        "gix must see the rebase-merge/ directory",
    );
    assert!(
        cli.merge_or_rebase_in_progress(),
        "cli must see the rebase-merge/ directory",
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

/// `read_blob_at(rev, path)` must agree across backends for an explicit
/// (non-symbolic) commit SHA — the historical-scan path (architecture-
/// trend) reads blobs at SHAs returned by `walk_commits`, so gix's
/// `rev_parse_single` and git's `git show <sha>:<path>` must resolve the
/// same bytes. Probes the OLDEST commit in the fixture so the rev is
/// genuinely not HEAD.
#[test]
fn read_blob_at_matches_for_an_explicit_old_rev() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();

    // Oldest commit = last in the (reverse-chronological) walk.
    let events: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let old_rev = events.last().expect("fixture has commits").rev.clone();

    // Probe several paths; for each, the two backends must return
    // byte-identical results (Some-or-None must match exactly) at that
    // historical rev. At least one probe must resolve to Some so the
    // test actually exercises the read path, not just the None branch.
    let probes = ["README.md", "src/main.rs", "Cargo.toml", "src/lib.rs"];
    let mut any_some = false;
    for path in probes {
        let g = gix
            .read_blob_at(&old_rev, path)
            .unwrap_or_else(|e| panic!("gix read_blob_at {old_rev}:{path}: {e}"));
        let c = cli
            .read_blob_at(&old_rev, path)
            .unwrap_or_else(|e| panic!("cli read_blob_at {old_rev}:{path}: {e}"));
        assert_eq!(
            g,
            c,
            "backends disagree on {path} at {old_rev} (gix Some={}, cli Some={})",
            g.is_some(),
            c.is_some(),
        );
        any_some = any_some || g.is_some();
    }
    assert!(
        any_some,
        "no probe resolved at {old_rev} — test never exercised a real blob read",
    );
}

/// A directory path (any non-blob tree entry) must resolve to `Ok(None)`
/// on BOTH backends — the trait contract (`repo/mod.rs`). `GixRepo`
/// enforces it via `entry.mode().is_blob()`; `GitCliRepo` must reject
/// non-blobs too (`git cat-file blob <rev>:<dir>` errors), else
/// `git show <rev>:<dir>` would succeed and return a tree listing,
/// silently diverging.
#[test]
fn read_blob_at_returns_none_for_a_directory_path() {
    let (gix, cli) = open_both();
    let opts = opts_with_merges();
    let events: Vec<_> = gix
        .walk_commits(&opts)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let head_rev = events.first().expect("fixture has commits").rev.clone();

    // Find a tracked file nested in a directory at HEAD, then probe its
    // parent directory (derived, so the test adapts to the fixture layout).
    let nested = ["src/main.rs", "src/lib.rs", "src/repo/mod.rs"]
        .into_iter()
        .find(|p| gix.read_blob_at(&head_rev, p).ok().flatten().is_some())
        .expect("fixture should have a file nested in a directory at HEAD");
    let dir = &nested[..nested.rfind('/').unwrap()];

    let g = gix
        .read_blob_at(&head_rev, dir)
        .expect("gix read_blob_at dir");
    let c = cli
        .read_blob_at(&head_rev, dir)
        .expect("cli read_blob_at dir");
    assert_eq!(g, None, "gix must return None for directory {dir}");
    assert_eq!(c, None, "cli must return None for directory {dir}");
    assert_eq!(g, c, "backends must agree a directory is not a blob");
}

/// `tracked_paths_at_head()` must return the identical path list from
/// both backends: every regular-file blob at HEAD, repo-relative,
/// `/`-separated, sorted ascending with no duplicates. The head-only
/// ingest mode substitutes this for the walk-derived live-path
/// reconstruction, so cross-backend divergence here would silently
/// change which files the calibration complexity scan reads.
#[test]
fn tracked_paths_at_head_matches() {
    let (gix, cli) = open_both();

    let gix_paths = gix
        .tracked_paths_at_head()
        .expect("gix tracked_paths_at_head");
    let cli_paths = cli
        .tracked_paths_at_head()
        .expect("cli tracked_paths_at_head");

    assert!(
        !gix_paths.is_empty(),
        "fixture must have tracked files at HEAD"
    );
    assert!(
        gix_paths.windows(2).all(|w| w[0] < w[1]),
        "paths must be strictly ascending (sorted, no duplicates): {gix_paths:?}"
    );
    assert_eq!(
        gix_paths, cli_paths,
        "GixRepo and GitCliRepo disagree on tracked paths at HEAD"
    );
    // README.md is known-tracked at HEAD (the read_blob_at_head test
    // relies on it) — anchor the list to a concrete fixture file so an
    // accidentally-empty-but-equal regression can't pass.
    assert!(
        gix_paths.iter().any(|p| p == "README.md"),
        "README.md must be enumerated at HEAD; got {gix_paths:?}"
    );
}

/// `diff_hunks(rev, path)` must agree byte-for-byte across the two
/// backends: gix-imara-diff's `Diff::hunks()` iterator converted to
/// git's 1-indexed `@@ -old_start,old_lines +new_start,new_lines @@`
/// convention vs `git show -p --unified=0`'s parsed hunk headers.
/// Both backends now produce real hunks (gix via `count_loc_and_hunks`
/// which extends `count_loc` with `Diff::hunks()` walk; cli via shell-
/// out to `git show -p --unified=0`).
#[test]
fn diff_hunks_match_across_backends() {
    let (gix, cli) = open_both();
    let head = gix.head_sha().expect("head_sha");
    // Probe a handful of paths from this repo (the fixture is
    // `open_both()` of THIS workspace). README.md is touched in some
    // commits; for paths that don't appear in HEAD the call returns
    // an empty vec on both sides, which is fine — the assertion is
    // EQUALITY, not non-emptiness.
    for path in &["README.md", "Cargo.toml", "CHANGELOG.md"] {
        let gix_hunks = gix
            .diff_hunks(&head, path)
            .unwrap_or_else(|e| panic!("gix diff_hunks for {path}: {e}"));
        let cli_hunks = cli
            .diff_hunks(&head, path)
            .unwrap_or_else(|e| panic!("cli diff_hunks for {path}: {e}"));
        assert_eq!(
            gix_hunks, cli_hunks,
            "diff_hunks divergence between gix and cli for {path} at {head}: \
             gix={gix_hunks:?} cli={cli_hunks:?}",
        );
    }
}

/// `worktree_changes()` on a freshly-cloned fixture must be empty on both
/// backends — the baseline every mutating test below perturbs. Uses the
/// shared fixture (read-only probe).
#[test]
fn worktree_changes_empty_on_fresh_clone() {
    let (gix, cli) = open_both();
    let gix_changes = gix.worktree_changes().expect("gix worktree_changes");
    let cli_changes = cli.worktree_changes().expect("cli worktree_changes");
    assert_eq!(
        gix_changes,
        Vec::new(),
        "fresh clone must have no worktree changes per GixRepo"
    );
    assert_eq!(
        gix_changes, cli_changes,
        "GixRepo and GitCliRepo disagree on a fresh clone"
    );
}

/// Unstaged, staged, and staged-then-re-edited modifications must each
/// surface exactly once with kind `Modified` — the union of the two stages
/// merged by path, never a duplicate entry for the both-stages case.
#[test]
fn worktree_changes_detects_staged_and_unstaged_edits() {
    use codelore_lib::repo::types::{WorktreeChange, WorktreeChangeKind};

    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();

    // Unstaged: worktree differs from index.
    std::fs::write(path.join("README.md"), b"unstaged edit\n").expect("edit README.md");
    // Staged: index differs from HEAD, worktree matches index.
    std::fs::write(path.join("Cargo.lock"), b"# staged edit\n").expect("edit Cargo.lock");
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "Cargo.lock"])
        .status()
        .expect("spawn git add");
    assert!(status.success(), "git add Cargo.lock failed");
    // Both stages: staged edit + a further unstaged edit on top.
    std::fs::write(path.join("src/lib.rs"), b"// staged\n").expect("edit src/lib.rs");
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "src/lib.rs"])
        .status()
        .expect("spawn git add");
    assert!(status.success(), "git add src/lib.rs failed");
    std::fs::write(path.join("src/lib.rs"), b"// staged then edited again\n")
        .expect("re-edit src/lib.rs");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_changes = gix.worktree_changes().expect("gix worktree_changes");
    let cli_changes = cli.worktree_changes().expect("cli worktree_changes");

    let expected = vec![
        WorktreeChange {
            path: "Cargo.lock".to_string(),
            kind: WorktreeChangeKind::Modified,
            rename_from: None,
        },
        WorktreeChange {
            path: "README.md".to_string(),
            kind: WorktreeChangeKind::Modified,
            rename_from: None,
        },
        WorktreeChange {
            path: "src/lib.rs".to_string(),
            kind: WorktreeChangeKind::Modified,
            rename_from: None,
        },
    ];
    assert_eq!(
        gix_changes, expected,
        "GixRepo must report the three edits once each, kind Modified, sorted"
    );
    assert_eq!(
        gix_changes, cli_changes,
        "GixRepo and GitCliRepo disagree on staged/unstaged edits"
    );
}

/// A staged deletion surfaces as `Deleted`; a `git mv` surfaces as the
/// destination `Added` with `rename_from` naming the source plus the source
/// as its own `Deleted` entry — and both backends must agree on all three.
#[test]
fn worktree_changes_detects_delete_and_rename() {
    use codelore_lib::repo::types::{WorktreeChange, WorktreeChangeKind};

    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();

    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rm", "-q", "src/main.rs"])
        .status()
        .expect("spawn git rm");
    assert!(status.success(), "git rm src/main.rs failed");
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["mv", "Cargo.lock", "Cargo2.lock"])
        .status()
        .expect("spawn git mv");
    assert!(status.success(), "git mv Cargo.lock Cargo2.lock failed");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_changes = gix.worktree_changes().expect("gix worktree_changes");
    let cli_changes = cli.worktree_changes().expect("cli worktree_changes");

    let expected = vec![
        WorktreeChange {
            path: "Cargo.lock".to_string(),
            kind: WorktreeChangeKind::Deleted,
            rename_from: None,
        },
        WorktreeChange {
            path: "Cargo2.lock".to_string(),
            kind: WorktreeChangeKind::Added,
            rename_from: Some("Cargo.lock".to_string()),
        },
        WorktreeChange {
            path: "src/main.rs".to_string(),
            kind: WorktreeChangeKind::Deleted,
            rename_from: None,
        },
    ];
    assert_eq!(
        gix_changes, expected,
        "GixRepo must report the rename pair plus the staged deletion"
    );
    assert_eq!(
        gix_changes, cli_changes,
        "GixRepo and GitCliRepo disagree on delete/rename"
    );
}

/// Differential coverage: a rename staged via literal `git rm` (not
/// `git mv`) plus a separate `git add` of the new path must be detected as a
/// rename exactly like `worktree_changes_detects_delete_and_rename`'s `git
/// mv` case — `git mv` is itself implemented as this same rm-then-add
/// sequence, so this test proves the pairing does not depend on the `mv`
/// convenience wrapper. Current behavior is CORRECT: the destination is not
/// dropped.
#[test]
fn worktree_changes_detects_rm_then_add_rename() {
    use codelore_lib::repo::types::{WorktreeChange, WorktreeChangeKind};

    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();

    // Byte-identical content (git's rename detection pairs by similarity;
    // identical content is unambiguously a 100%-similarity match).
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rm", "-q", "README.md"])
        .status()
        .expect("spawn git rm");
    assert!(status.success(), "git rm README.md failed");
    // README.md's blob content was removed from the worktree by `git rm`
    // (unlike `git mv`, plain `git rm` also deletes the worktree file), so
    // recreate the destination from the HEAD blob to keep content identical.
    let head_readme = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["show", "HEAD:README.md"])
        .output()
        .expect("git show HEAD:README.md");
    assert!(
        head_readme.status.success(),
        "git show HEAD:README.md failed"
    );
    std::fs::write(path.join("RENAMED.md"), &head_readme.stdout).expect("write RENAMED.md");
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "RENAMED.md"])
        .status()
        .expect("spawn git add");
    assert!(status.success(), "git add RENAMED.md failed");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_changes = gix.worktree_changes().expect("gix worktree_changes");
    let cli_changes = cli.worktree_changes().expect("cli worktree_changes");

    let expected = vec![
        WorktreeChange {
            path: "README.md".to_string(),
            kind: WorktreeChangeKind::Deleted,
            rename_from: None,
        },
        WorktreeChange {
            path: "RENAMED.md".to_string(),
            kind: WorktreeChangeKind::Added,
            rename_from: Some("README.md".to_string()),
        },
    ];
    assert_eq!(
        gix_changes, expected,
        "a literal `git rm` + `git add` rename must pair identically to `git mv`"
    );
    assert_eq!(
        gix_changes, cli_changes,
        "GixRepo and GitCliRepo disagree on the rm-then-add rename"
    );
}

/// Differential coverage: an UNSTAGED rename (a plain filesystem `mv`,
/// no `git add`) is NOT detected as a rename by either backend — the
/// destination is untracked, and untracked files are excluded from
/// `worktree_changes` by design (spec contract: "untracked files excluded on
/// both" backends; `GixRepo::worktree_changes` disables the directory walk
/// entirely via `UntrackedFiles::None`, and `GitCliRepo` passes
/// `--untracked-files=no`). This is confirmed-correct existing behavior, not
/// a bug: only the source shows up, as a plain `Deleted` entry.
#[test]
fn worktree_changes_unstaged_rename_drops_untracked_destination_by_design() {
    use codelore_lib::repo::types::{WorktreeChange, WorktreeChangeKind};

    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();

    std::fs::rename(path.join("README.md"), path.join("RENAMED2.md"))
        .expect("plain filesystem rename (no git mv/add)");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_changes = gix.worktree_changes().expect("gix worktree_changes");
    let cli_changes = cli.worktree_changes().expect("cli worktree_changes");

    let expected = vec![WorktreeChange {
        path: "README.md".to_string(),
        kind: WorktreeChangeKind::Deleted,
        rename_from: None,
    }];
    assert_eq!(
        gix_changes, expected,
        "an unstaged rename's untracked destination must be excluded by design; \
         only the tracked source's deletion is reported"
    );
    assert_eq!(
        gix_changes, cli_changes,
        "GixRepo and GitCliRepo disagree on the unstaged-rename case"
    );
}

/// A file added to the index then removed from the worktree (status `AD`)
/// nets out to no change vs HEAD and must be dropped; untracked files must
/// never appear. Both backends must agree the change list is empty.
#[test]
fn worktree_changes_drops_add_then_delete_and_untracked() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();

    // The AD case: stage a brand-new file, then delete it from the worktree.
    std::fs::write(path.join("ephemeral.rs"), b"// briefly staged\n").expect("write ephemeral.rs");
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["add", "ephemeral.rs"])
        .status()
        .expect("spawn git add");
    assert!(status.success(), "git add ephemeral.rs failed");
    std::fs::remove_file(path.join("ephemeral.rs")).expect("remove ephemeral.rs");

    // Untracked: never a candidate.
    std::fs::write(path.join("untracked_scratch.txt"), b"scratch\n").expect("write untracked");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");
    let gix_changes = gix.worktree_changes().expect("gix worktree_changes");
    let cli_changes = cli.worktree_changes().expect("cli worktree_changes");

    assert_eq!(
        gix_changes,
        Vec::new(),
        "add-then-delete nets to nothing and untracked files are excluded (gix)"
    );
    assert_eq!(
        gix_changes, cli_changes,
        "GixRepo and GitCliRepo disagree on the AD/untracked case"
    );
}

/// A working tree with unmerged (conflicted) paths cannot be net-classified
/// against HEAD, so both backends must return `Err` — the error contract is
/// part of the dual-backend parity guarantee. The conflict is a regular file,
/// which both backends surface (gix via `EntryStatus::Conflict`, CLI via the
/// porcelain `u ` record).
#[test]
fn worktree_changes_errors_alike_on_merge_conflict() {
    let repo = codelore_lib::test_support::differential_repo::build();
    let path = repo.dir.path();

    // Supply a committer identity on every call: the fixture is a fresh clone
    // and the two `commit` calls below would otherwise fail on any machine
    // without an ambient `user.name`/`user.email` (e.g. a clean CI runner).
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["-c", "user.email=codelore-test@example.com"])
            .args(["-c", "user.name=CodeLore Test"])
            .args(args)
            .output()
            .expect("spawn git")
    };

    let base = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout)
        .expect("rev-parse utf8")
        .trim()
        .to_string();

    // Two branches from the same base overwrite README.md with different
    // whole-file content; merging them conflicts on that file.
    assert!(
        git(&["checkout", "-q", "-b", "conflict-left"])
            .status
            .success()
    );
    std::fs::write(path.join("README.md"), b"left side\n").expect("write left");
    assert!(git(&["commit", "-aqm", "left"]).status.success());

    assert!(
        git(&["checkout", "-q", "-b", "conflict-right", &base])
            .status
            .success()
    );
    std::fs::write(path.join("README.md"), b"right side\n").expect("write right");
    assert!(git(&["commit", "-aqm", "right"]).status.success());

    // The merge is expected to FAIL (conflict) and leave unmerged entries.
    let merge = git(&["merge", "conflict-left"]);
    assert!(!merge.status.success(), "merge should have conflicted");

    let gix = GixRepo::open(path).expect("GixRepo::open");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open");

    assert!(
        gix.worktree_changes().is_err(),
        "GixRepo must error on a conflicted working tree"
    );
    assert!(
        cli.worktree_changes().is_err(),
        "GitCliRepo must error on a conflicted working tree"
    );
}

/// `Repo::tags()` must return byte-identical results from both backends.
/// Uses the `delivery_repo` fixture which has 4 annotated tags plus one
/// LIGHTWEIGHT tag (`light-1`, exercising the committer-date fallback path)
/// in known chronological order: v0.1.0 (Jan), v0.2.0 (Feb), nightly-1 (Mar),
/// light-1 (Apr 21 10:00, target commit's committer date), v1.0.0 (Apr 21
/// 12:00, tagger date).
#[test]
fn tags_match_across_backends() {
    let path = delivery_path();
    let gix = GixRepo::open(path).expect("GixRepo::open delivery_repo");
    let cli = GitCliRepo::open(path).expect("GitCliRepo::open delivery_repo");

    let gix_tags = gix.tags().expect("GixRepo::tags");
    let cli_tags = cli.tags().expect("GitCliRepo::tags");

    assert_eq!(
        gix_tags.len(),
        5,
        "expected 4 annotated + 1 lightweight tag; got {gix_tags:?}",
    );
    assert_eq!(
        cli_tags.len(),
        5,
        "expected 4 annotated + 1 lightweight tag; got {cli_tags:?}",
    );

    // Both backends must return identical results: same order, same OIDs,
    // same dates. A divergence here means the two backends parse tag metadata
    // differently — a bug by design.
    assert_eq!(
        gix_tags, cli_tags,
        "tags() diverged between GixRepo and GitCliRepo:\n  gix={gix_tags:#?}\n  cli={cli_tags:#?}"
    );

    // Verify sort order and names are correct (ascending by date: tagger
    // date for annotated tags, target committer date for lightweight —
    // which is why light-1 (commit 10:00) sorts before v1.0.0 (tag 12:00)).
    let names: Vec<&str> = gix_tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["v0.1.0", "v0.2.0", "nightly-1", "light-1", "v1.0.0"],
        "tags not in expected (date, name) order"
    );

    // Each target_rev must be a full 40-char hex SHA.
    for tag in &gix_tags {
        assert_eq!(
            tag.target_rev.len(),
            40,
            "tag {} target_rev is not a 40-char SHA: {:?}",
            tag.name,
            tag.target_rev
        );
        assert!(
            tag.target_rev.chars().all(|c| c.is_ascii_hexdigit()),
            "tag {} target_rev contains non-hex chars: {:?}",
            tag.name,
            tag.target_rev
        );
    }
}
