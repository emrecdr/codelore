//! Guard: every Rust-toolchain pin in the repository names the same version.
//!
//! The toolchain is pinned in five independent places — `rust-toolchain.toml`,
//! the workspace `rust-version`, `clippy.toml`'s `msrv`, the `Containerfile`'s
//! `ARG RUST_VERSION`, and the `dtolnay/rust-toolchain` action tags across the
//! workflows. Nothing bumps them together: the release script does not, and a
//! partial bump fails in a way that is hard to attribute — moving only the
//! action tag is a silent no-op, because `rust-toolchain.toml` overrides what
//! the action installs.
//!
//! `rust-toolchain.toml` is the source of truth; every other site is compared
//! against it. Pins are written at different precisions on purpose (the
//! toolchain channel and the action tags are `X.Y.Z`, the MSRV declarations are
//! `X.Y`), so agreement is checked on major.minor.
//!
//! The four file pins are embedded at compile time. The action tags are read at
//! run time from the workflows directory, so a workflow added later is covered
//! without editing this test.
//!
//! One pin is deliberately out of reach: the `Containerfile` builder base is
//! `rust:${RUST_VERSION}-${DEBIAN_RELEASE}@sha256:…`, and a digest wins over the
//! tag it accompanies. Bumping `ARG RUST_VERSION` without also refreshing that
//! digest silently keeps building on the old toolchain, and no textual check can
//! see it. The failure message says so rather than implying it is covered.

use std::path::{Path, PathBuf};

const RUST_TOOLCHAIN_TOML: &str = include_str!("../../../rust-toolchain.toml");
const WORKSPACE_CARGO_TOML: &str = include_str!("../../../Cargo.toml");
const CLIPPY_TOML: &str = include_str!("../../../clippy.toml");
const CONTAINERFILE: &str = include_str!("../../../Containerfile");

/// The action whose tag names a toolchain rather than a release of itself.
const TOOLCHAIN_ACTION: &str = "dtolnay/rust-toolchain@";

/// Value of the first `<key> = "<value>"` assignment in `text`.
fn quoted_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key} = \"");
    let start = text.find(&needle)? + needle.len();
    let end = text[start..].find('"')? + start;
    Some(&text[start..end])
}

/// Value of a Containerfile `ARG <key>=<value>` line.
fn arg_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("ARG {key}=");
    text.lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .map(str::trim)
}

/// Reduce a version to `major.minor`, so `1.96.0` and `1.96` compare equal.
fn major_minor(version: &str) -> String {
    version.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// `CARGO_MANIFEST_DIR` is `<root>/crates/codelore-lib`; two levels up is the
/// workspace root. Embedded at compile time, so it resolves under CI too.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// Every toolchain-action reference across the workflows, as
/// `(workflow file name, version ref)`. Read at run time so a workflow added
/// later is covered without touching this test.
fn action_toolchain_refs(workflows: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(workflows) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let text = std::fs::read_to_string(&path).expect("read workflow file");
        for line in text.lines() {
            if let Some(idx) = line.find(TOOLCHAIN_ACTION) {
                let tag = line[idx + TOOLCHAIN_ACTION.len()..]
                    .split_whitespace()
                    .next()
                    .unwrap_or_default();
                refs.push((name.clone(), tag.to_string()));
            }
        }
    }
    refs
}

/// Record `site` when its pin disagrees with `msrv` on major.minor.
fn record_mismatch(out: &mut Vec<String>, msrv: &str, site: &str, found: &str) {
    if major_minor(found) != msrv {
        out.push(format!("  {site} pins {found}"));
    }
}

#[test]
fn every_rust_version_pin_agrees_with_the_toolchain_file() {
    let channel = quoted_value(RUST_TOOLCHAIN_TOML, "channel")
        .expect("rust-toolchain.toml declares a channel");
    let msrv = major_minor(channel);

    let mut mismatches = Vec::new();
    record_mismatch(
        &mut mismatches,
        &msrv,
        "workspace Cargo.toml rust-version",
        quoted_value(WORKSPACE_CARGO_TOML, "rust-version")
            .expect("workspace Cargo.toml declares rust-version"),
    );
    record_mismatch(
        &mut mismatches,
        &msrv,
        "clippy.toml msrv",
        quoted_value(CLIPPY_TOML, "msrv").expect("clippy.toml declares msrv"),
    );
    record_mismatch(
        &mut mismatches,
        &msrv,
        "Containerfile ARG RUST_VERSION",
        arg_value(CONTAINERFILE, "RUST_VERSION").expect("Containerfile declares ARG RUST_VERSION"),
    );

    let workflows = workspace_root().join(".github/workflows");
    let refs = action_toolchain_refs(&workflows);
    assert!(
        !refs.is_empty(),
        "found no {TOOLCHAIN_ACTION} references under {} — workflow-path resolution is broken, \
         so this guard would pass vacuously",
        workflows.display()
    );
    for (file, tag) in &refs {
        record_mismatch(
            &mut mismatches,
            &msrv,
            &format!(".github/workflows/{file}"),
            tag,
        );
    }

    assert!(
        mismatches.is_empty(),
        "rust-toolchain.toml pins {channel} (major.minor {msrv}), but {} pin site(s) disagree:\n{}\n\n\
         rust-toolchain.toml is the source of truth — bump every site to match it. Bumping only \
         the workflow action tag is a silent no-op, because rust-toolchain.toml overrides what \
         the action installs.\n\n\
         Not covered by this check: the Containerfile builder base carries an inline \
         `@sha256:` digest, and a digest wins over the tag beside it. A RUST_VERSION bump without \
         a matching digest refresh keeps building on the old toolchain, invisibly to any textual \
         check — refresh it by hand or via the Dependabot docker stanza.",
        mismatches.len(),
        mismatches.join("\n"),
    );
}
