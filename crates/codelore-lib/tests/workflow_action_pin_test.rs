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

/// Patterns `.github/zizmor.yml` permits to be referenced by tag.
///
/// Scanned textually, the way `rust_version_pins_test` reads the other pin
/// sites, rather than by pulling in a YAML parser the workspace does not
/// otherwise need.
fn zizmor_ref_pin_patterns(config: &str) -> Vec<String> {
    config
        .lines()
        .filter_map(|line| line.trim().strip_suffix(": ref-pin"))
        .map(|pattern| pattern.trim_matches('"').to_owned())
        .collect()
}

#[test]
fn the_external_auditor_permits_exactly_what_this_guard_exempts() {
    // Two gates now enforce this one policy: this test, and `zizmor`'s
    // `unpinned-uses` audit via `.github/zizmor.yml`. Two gates that can
    // disagree are worse than either alone, because a contributor gets told
    // to pin by one and told it is fine by the other, and neither says which
    // is right. The config argues in prose that it is kept in step with this
    // function; this is that claim, checked.
    //
    // The same shape `rust_version_pins_test` uses for the toolchain pinned
    // in five places: name a source of truth, read the other statements of
    // it, and fail listing the disagreements.
    let root = workspace_root();
    let config =
        std::fs::read_to_string(root.join(".github/zizmor.yml")).expect("read .github/zizmor.yml");

    let mut permitted = zizmor_ref_pin_patterns(&config);
    permitted.sort();
    assert!(
        !permitted.is_empty(),
        "parsed no `ref-pin` patterns out of .github/zizmor.yml — the config \
         changed shape and this guard is reading nothing"
    );

    // `actions/*` is zizmor's glob for the namespace `is_exempt` matches by
    // prefix; the local-action exemption has no counterpart, because
    // zizmor's patterns are `owner/repo`-shaped and `./…` is not in that space.
    let disagreements: Vec<&String> = permitted
        .iter()
        .filter(|pattern| !is_exempt(&pattern.replace('*', "")))
        .collect();
    assert!(
        disagreements.is_empty(),
        "`.github/zizmor.yml` permits {disagreements:?} to be tag-referenced, \
         but `is_exempt` does not. Whichever is right, they must say the same \
         thing — a contributor reads whichever gate fails first."
    );

    for exempt in ["actions/", "dtolnay/rust-toolchain"] {
        assert!(
            permitted.iter().any(|p| p.replace('*', "") == exempt),
            "`is_exempt` allows `{exempt}` by tag but `.github/zizmor.yml` \
             does not, so zizmor will fail a reference this guard accepts"
        );
    }
}
