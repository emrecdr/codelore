//! Guard: no publishing workflow triggers on the Action's floating major tag.
//!
//! `cut-release.sh` re-points `v1` — the Action's interface tag — onto each
//! release. That is a tag push like any other, so a workflow triggering on
//! `tags: ['v*']` fires for it.
//!
//! It did. The `v1` push ran the full release pipeline a second time and
//! published a GitHub Release literally named `v1`. GitHub's
//! `releases/latest` API then returned `v1`, and the Action's own
//! `version: latest` resolution rejected it as not-a-semver — so the
//! published Action failed for every consumer, and the Homebrew tap was
//! rewritten to a "codelore 1" whose download URLs pointed at that release.
//!
//! The fix is a narrower pattern (`v*.*.*`), which still matches every real
//! release tag including pre-release suffixes and matches no bare major.
//! This guard ties the two files together so the pattern cannot be widened
//! back to `v*` while the major tag still moves — the two are individually
//! reasonable and catastrophic together, which is exactly the coupling a
//! reviewer looking at one file will not see.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// Match a GitHub Actions tag-filter glob against a ref name.
///
/// Only `*` is supported, which is all these patterns use; it matches any
/// run of characters other than `/`. Backtracking is fine at this size.
fn glob_matches(pattern: &str, name: &str) -> bool {
    match pattern.find('*') {
        None => pattern == name,
        Some(star) => {
            let (lit, rest) = (&pattern[..star], &pattern[star + 1..]);
            if !name.starts_with(lit) {
                return false;
            }
            let tail = &name[lit.len()..];
            // `*` does not cross a path separator.
            let limit = tail.find('/').unwrap_or(tail.len());
            (0..=limit).any(|i| glob_matches(rest, &tail[i..]))
        }
    }
}

/// The tag `cut-release.sh` re-points on every release, read from the script
/// rather than duplicated here — a rename there must not silently orphan
/// this guard.
fn action_major_tag(root: &Path) -> String {
    let script = std::fs::read_to_string(root.join("scripts/cut-release.sh"))
        .expect("cut-release.sh is readable");
    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with("ACTION_MAJOR_TAG="))
        .expect(
            "cut-release.sh must define ACTION_MAJOR_TAG — if the floating \
             major tag was removed entirely, delete this guard with it",
        );
    line.split_once('=')
        .expect("assignment has an `=`")
        .1
        .trim()
        .trim_matches('"')
        .to_owned()
}

/// `(workflow, pattern)` for every tag pattern under an `on: push: tags:`.
fn tag_trigger_patterns(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root.join(".github/workflows")) else {
        return out;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
        .collect();
    files.sort();

    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            // Inline-sequence form, which is what these workflows use:
            //   tags: ['v*.*.*']
            let Some(rest) = trimmed.strip_prefix("tags:") else {
                continue;
            };
            for pat in rest
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
            {
                let pat = pat.trim().trim_matches(['\'', '"']);
                if !pat.is_empty() {
                    out.push((rel.clone(), pat.to_owned()));
                }
            }
        }
    }
    out
}

#[test]
fn no_workflow_triggers_on_the_action_major_tag() {
    let root = workspace_root();
    let major = action_major_tag(&root);
    let patterns = tag_trigger_patterns(&root);

    assert!(
        patterns.len() >= 2,
        "found {} tag-trigger pattern(s); expected at least the release and \
         container workflows. The scan is not seeing them, so this guard \
         would pass vacuously",
        patterns.len()
    );

    let offenders: Vec<String> = patterns
        .iter()
        .filter(|(_, pat)| glob_matches(pat, &major))
        .map(|(wf, pat)| format!("  {wf}: `tags: [{pat}]` matches the major tag `{major}`"))
        .collect();

    assert!(
        offenders.is_empty(),
        "{} workflow tag-trigger(s) fire when cut-release.sh re-points \
         `{major}`:\n{}\n\nThat publishes a release named `{major}`, which \
         makes the releases/latest API return it and breaks the Action's own \
         `version: latest` resolution. Use `v*.*.*`, which matches every real \
         release tag (pre-release suffixes included) and no bare major.",
        offenders.len(),
        offenders.join("\n"),
    );
}

#[test]
fn the_pattern_guard_matches_the_way_github_does() {
    // A guard that cannot fail is worth nothing — and this one rests entirely
    // on the glob matcher, so pin its behaviour against the exact patterns in
    // play, including the one that caused the incident.
    assert!(glob_matches("v*", "v1"), "the old pattern DID match v1");
    assert!(!glob_matches("v*.*.*", "v1"), "the fix must not match v1");
    assert!(!glob_matches("v*.*.*", "v2"), "nor any other bare major");

    for tag in ["v0.27.2", "v1.0.0", "v0.1.0-alpha.2", "v1.0.0-rc.1"] {
        assert!(
            glob_matches("v*.*.*", tag),
            "the fix must still match the real release tag {tag}"
        );
    }

    assert!(!glob_matches("v*", "release/v1"), "* does not cross a `/`");
    assert!(glob_matches("v1", "v1"), "a literal pattern matches itself");
    assert!(!glob_matches("v1", "v2"), "and nothing else");

    // The tag name is read from the script, not hardcoded; prove that works
    // rather than assuming, since a silent empty read would make the real
    // assertion above vacuous.
    let major = action_major_tag(&workspace_root());
    assert!(
        major.starts_with('v') && major.len() >= 2,
        "ACTION_MAJOR_TAG parsed from cut-release.sh looks wrong: {major:?}"
    );
}
