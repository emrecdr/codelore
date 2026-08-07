//! Guard: every third-party GitHub Action is pinned to a full commit SHA.
//!
//! A tag is a mutable pointer the action's owner controls. `@v2` on a job with
//! write access means trusting whatever that owner repoints it at later, which
//! is the standard supply-chain exposure — and the dogfood job that publishes
//! to `gh-pages` runs with `contents: write`.
//!
//! Most actions here were already SHA-pinned; the ones that were not had simply
//! been added by someone who did not know the convention, which is exactly the
//! failure mode a convention without a check has. This makes it a rule.

use std::path::{Path, PathBuf};

/// `CARGO_MANIFEST_DIR` is `<root>/crates/codelore-lib`; two levels up is the
/// workspace root. Embedded at compile time, so it resolves under CI too.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// Actions exempt from the SHA requirement, each for a stated reason.
fn is_exempt(action: &str) -> bool {
    // GitHub's own first-party namespace.
    action.starts_with("actions/")
        // A local composite action in this repository.
        || action.starts_with("./")
        // The tag names the TOOLCHAIN to install, not a release of the action.
        // Pinning it to a SHA would freeze the action while changing nothing
        // about the Rust version — the opposite of the intent.
        // `rust-toolchain.toml` is the real source of truth, and the dependabot
        // config records the same reasoning.
        || action == "dtolnay/rust-toolchain"
}

fn is_sha(reference: &str) -> bool {
    reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit())
}

/// `(file, line, spec)` for every `uses:` reference across the workflows and
/// the composite action.
fn action_refs(root: &Path) -> Vec<(String, usize, String)> {
    let mut files = vec![root.join("action.yml")];
    if let Ok(entries) = std::fs::read_dir(root.join(".github/workflows")) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("yml") {
                files.push(p);
            }
        }
    }
    let mut out = Vec::new();
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
            // Both step forms reach here. A step may be written as a
            // one-liner (`- uses: x`) or with the key on its own line under a
            // `- name:` (`  uses: x`); matching only the latter silently
            // skipped every one-liner, which is most of them.
            let trimmed = line.trim().trim_start_matches("- ").trim_start();
            if let Some(rest) = trimmed.strip_prefix("uses:")
                && let Some(spec) = rest.split_whitespace().next()
            {
                out.push((rel.clone(), idx + 1, spec.to_owned()));
            }
        }
    }
    out
}

#[test]
fn third_party_actions_are_pinned_to_a_sha() {
    let root = workspace_root();
    let refs = action_refs(&root);
    assert!(
        refs.len() > 5,
        "found only {} `uses:` references — path resolution is broken, so this \
         guard would pass vacuously",
        refs.len()
    );

    let floating: Vec<String> = refs
        .iter()
        .filter_map(|(file, line, spec)| {
            let (action, reference) = spec.rsplit_once('@')?;
            (!is_exempt(action) && !is_sha(reference)).then(|| format!("  {file}:{line}: {spec}"))
        })
        .collect();

    assert!(
        floating.is_empty(),
        "{} third-party action(s) pinned to a mutable tag rather than a commit \
         SHA:\n{}\n\nResolve the tag and pin it, keeping the version in a \
         trailing comment so the intent stays readable:\n  gh api \
         repos/<owner>/<repo>/commits/<tag> --jq .sha",
        floating.len(),
        floating.join("\n"),
    );
}

#[test]
fn the_pin_guard_rejects_a_tag_and_accepts_a_sha() {
    // A guard that cannot fail is worth nothing. Exercise the predicate pair
    // directly, including both exemptions, so the assertion above is known to
    // be discriminating rather than merely quiet.
    assert!(!is_sha("v2"), "a bare major tag is not a pin");
    assert!(!is_sha("v2.85.5"), "a version tag is still mutable");
    assert!(
        is_sha("6323deb102c322ba6fcbdcafc7e3dddab59af2b6"),
        "a 40-char hex ref is a pin"
    );
    assert!(is_exempt("actions/checkout"), "first-party is exempt");
    assert!(
        is_exempt("dtolnay/rust-toolchain"),
        "the toolchain action's tag names the toolchain, not a release"
    );
    assert!(
        !is_exempt("Swatinem/rust-cache"),
        "an ordinary third-party action is not exempt"
    );
}
