//! Guards the property that no commit on `main` can have its CI cancelled by
//! a later one.
//!
//! Every commit on `main` is a permanent, releasable point: `self-gate` stamps
//! it and `dogfood` publishes from it. A commit whose CI run was cancelled is
//! unverified forever, because no later run covers it.
//!
//! Main commits genuinely do arrive back-to-back — `merge-on-green` has to
//! re-dispatch main's CI after each auto-merge, since a `GITHUB_TOKEN` push
//! does not fire `on: push`. Under a ref-scoped concurrency group those
//! dispatches cancelled each other, and three merges landing together left the
//! middle two commits permanently unverified.
//!
//! The fix is to scope main's group by SHA so each commit runs alone. Note
//! that `cancel-in-progress: false` does NOT achieve this: GitHub keeps at
//! most one running and one *pending* run per group and cancels the older
//! pending one, so the middle commit of three would still lose its run. Only
//! distinct groups work, which is why this guard checks the group expression
//! rather than the cancel flag.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> has a workspace root two levels up")
        .to_path_buf()
}

/// Extracts the `group:` value from a workflow's top-level `concurrency:`
/// block. Returns `None` when the workflow declares no concurrency at all,
/// which is a valid configuration this guard says nothing about.
fn concurrency_group(workflow: &str) -> Option<String> {
    let mut in_block = false;
    for line in workflow.lines() {
        if line.starts_with("concurrency:") {
            in_block = true;
            continue;
        }
        if in_block {
            // A non-indented, non-blank line ends the block.
            if !line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
                break;
            }
            if let Some(rest) = line.trim().strip_prefix("group:") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

/// The property under test: two different commits on `main` must land in two
/// different concurrency groups.
///
/// A group qualified by the commit SHA under a `refs/heads/main` condition
/// satisfies this; one scoped only to the ref does not, because every main
/// commit shares `refs/heads/main`.
fn main_commits_get_distinct_groups(group: &str) -> bool {
    group.contains("github.sha") && group.contains("refs/heads/main")
}

#[test]
fn main_commits_cannot_cancel_each_others_ci() {
    let root = workspace_root();
    let ci = root.join(".github/workflows/ci.yml");
    let text = std::fs::read_to_string(&ci).expect("read .github/workflows/ci.yml");

    let group = concurrency_group(&text).expect(
        "ci.yml declares a top-level `concurrency:` block with a `group:` key — \
         if that changed, this guard needs updating rather than deleting",
    );

    assert!(
        main_commits_get_distinct_groups(&group),
        "ci.yml's concurrency group does not give each `main` commit its own \
         group, so one main commit's CI can cancel another's, leaving it \
         permanently unverified.\n  group: {group}\n\
         Expected the expression to qualify the group by `github.sha` when \
         `github.ref` is `refs/heads/main`."
    );
}

#[test]
fn the_guard_rejects_the_ref_only_group_and_accepts_the_sha_qualified_one() {
    // The exact shape that caused three commits to lose their CI runs. It is
    // also what the tempting non-fix leaves behind: flipping
    // `cancel-in-progress` to `false` keeps the group ref-scoped, so the
    // pending-run supersede still drops the middle commit. The group is the
    // thing that has to change, which is why this is what gets rejected.
    let ref_only = "${{ github.workflow }}-${{ github.ref }}";
    assert!(
        !main_commits_get_distinct_groups(ref_only),
        "the ref-only group must be rejected — it is the defect this guards"
    );

    let sha_qualified = "${{ github.workflow }}-${{ github.ref }}-${{ github.ref == 'refs/heads/main' && github.sha || '' }}";
    assert!(
        main_commits_get_distinct_groups(sha_qualified),
        "the shipped group must be accepted"
    );

    // The self-test runs the same matcher the gate above runs, so the two
    // cannot drift apart.
    let root = workspace_root();
    let shipped = concurrency_group(
        &std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml"),
    )
    .expect("ci.yml has a concurrency group");
    assert_eq!(
        shipped, sha_qualified,
        "the shipped group changed; update this self-test's pinned copy so it \
         keeps proving the matcher works against what actually ships"
    );
}

#[test]
fn the_block_parser_finds_the_group_and_stops_at_the_block_end() {
    let sample = "name: X\nconcurrency:\n  group: abc\n  cancel-in-progress: true\n\npermissions:\n  group: NOT-THIS\n";
    assert_eq!(concurrency_group(sample).as_deref(), Some("abc"));

    // No concurrency block at all is a valid configuration, not a match.
    assert_eq!(
        concurrency_group("name: X\njobs:\n  a:\n    runs-on: x\n"),
        None
    );
}
