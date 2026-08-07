//! Guard: every `uses: emrecdr/codelore@<ref>` in the docs names a ref that
//! this repository actually publishes.
//!
//! The documentation told readers to write `uses: emrecdr/codelore@v1` in
//! fourteen places while no `v1` tag or branch existed and nothing in the
//! release process created one. A workflow copied from those examples fails at
//! the step — `Unable to resolve action emrecdr/codelore@v1` — and the action
//! never runs.
//!
//! Two audit cycles missed it because both asked what `action.yml` *contains*,
//! and CI exercises the action as `uses: ./`. A local path proves the mechanics
//! and says nothing about whether a consumer can reach it. This checks the
//! reference form a consumer actually types.
//!
//! Resolution is against the local clone's refs, so the check is offline and
//! deterministic. A tag that exists upstream but was never fetched would read
//! as missing, which is why the failure message says to fetch before believing
//! it — a false alarm here is cheap, and the alternative is trusting the docs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The action's own coordinates. A doc example naming a different repository
/// is quoting someone else's action and is not ours to verify.
const SELF_ACTION: &str = "emrecdr/codelore@";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            // Dated reports and frozen plans quote whatever was true when
            // written, including refs that have since moved.
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name == "reports" || name == "superpowers" {
                continue;
            }
            markdown_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(p);
        }
    }
}

/// Every ref name this clone knows: tags, local branches, and remote-tracking
/// branches with the remote prefix stripped.
fn known_refs(root: &Path) -> HashSet<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/tags",
            "refs/heads",
            "refs/remotes",
        ])
        .output()
        .expect("spawn git for-each-ref");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_owned())
        .flat_map(|r| {
            // `origin/main` also satisfies a documented `@main`.
            let short = r.split_once('/').map(|(_, rest)| rest.to_owned());
            std::iter::once(r).chain(short)
        })
        .collect()
}

/// `(file, line, ref)` for every self-referencing `uses:` in the docs.
fn documented_refs(root: &Path) -> Vec<(String, usize, String)> {
    let mut files = vec![root.join("README.md")];
    markdown_files(&root.join("docs"), &mut files);

    let mut found = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        for (idx, line) in text.lines().enumerate() {
            let Some(at) = line.find(SELF_ACTION) else {
                continue;
            };
            let rest = &line[at + SELF_ACTION.len()..];
            let reference: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
                .collect();
            if !reference.is_empty() {
                found.push((rel.clone(), idx + 1, reference));
            }
        }
    }
    found
}

#[test]
fn documented_action_refs_resolve_to_a_published_ref() {
    let root = workspace_root();
    let refs = documented_refs(&root);
    assert!(
        !refs.is_empty(),
        "found no `{SELF_ACTION}<ref>` references in the docs — either the \
         documentation stopped showing how to use the Action, or this scan is \
         looking in the wrong place. Both are worth knowing."
    );

    let known = known_refs(&root);
    assert!(
        known.len() > 3,
        "git reported {} refs — the repository looks unfetched, so this guard \
         would report every documented ref as missing",
        known.len()
    );

    let unresolved: Vec<String> = refs
        .iter()
        .filter(|(_, _, r)| !known.contains(r.as_str()))
        .map(|(f, l, r)| format!("  {f}:{l}: emrecdr/codelore@{r}"))
        .collect();

    assert!(
        unresolved.is_empty(),
        "{} documented Action reference(s) name a ref this repository does not \
         publish:\n{}\n\nA workflow copied from these examples fails with \
         `Unable to resolve action`. Either publish the ref (a floating major \
         tag needs moving on every release, or it stops existing the way this \
         one did) or document an exact release tag instead.\n\nIf you believe \
         the ref exists upstream, run `git fetch --tags` first — this resolves \
         against the local clone.",
        unresolved.len(),
        unresolved.join("\n"),
    );
}

#[test]
fn the_guard_reads_refs_and_docs_the_way_it_claims() {
    // The real assertion passes when nothing is unresolved, which is also what
    // happens if either half silently reads empty. Pin both.
    let root = workspace_root();

    let known = known_refs(&root);
    assert!(
        known.iter().any(|r| r == "main"),
        "ref enumeration must find `main`; got {} refs",
        known.len()
    );
    assert!(
        known.iter().any(|r| r.starts_with("v0.")),
        "ref enumeration must find the release tags"
    );

    let refs = documented_refs(&root);
    assert!(
        refs.iter().all(|(_, _, r)| !r.is_empty()),
        "a parsed ref must never be empty"
    );
}
