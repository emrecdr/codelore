//! Guard: the version the workspace declares has a CHANGELOG section.
//!
//! `[Unreleased]` sits directly above the newest released section, and both
//! are `##` headings one blank line apart. Every change lands by prepending
//! to the top of that file, which puts each edit one line away from the
//! boundary between "not shipped" and "shipped". Replace the released heading
//! instead of inserting above it and the section beneath is silently adopted
//! by `[Unreleased]`: the entries all survive, one level down, and the diff
//! reads as an ordinary prepend — one line out, one line in, at the top of a
//! file where every changelog commit adds lines at the top.
//!
//! Two consequences follow, neither of them visible in the file. The released
//! version loses its notes, so the newest version a reader finds is the one
//! before it. And the next cut re-announces the adopted section, because
//! `cut-release.sh` flips whatever sits under `[Unreleased]` into the new
//! version — work that already shipped goes out again under a second version
//! number.
//!
//! The invariant that catches it: the version in `[workspace.package]` must
//! have a matching section. `cut-release.sh` bumps that version and flips the
//! CHANGELOG in the same commit, so the two are never legitimately out of
//! step and there is no mid-release state to tolerate. The script already
//! checks this to decide whether prep work has been done; nothing enforced it
//! on the commits in between.
//!
//! `ledger_stamp_test` reads the same file and cannot see this. Its check is
//! one-directional by design — Unreleased stamps imply a non-empty Unreleased
//! section — so a section that absorbs another only makes it pass harder.
//!
//! Both files are embedded at compile time, so a broken path fails the build
//! rather than reading empty and passing.

const WORKSPACE_CARGO_TOML: &str = include_str!("../../../Cargo.toml");
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// `version` from the `[workspace.package]` table, scoped to that table so a
/// dependency's version cannot answer for it.
fn workspace_version(cargo_toml: &str) -> Option<&str> {
    let table = cargo_toml.split("[workspace.package]").nth(1)?;
    let table = table.split("\n[").next()?;
    let start = table.find("version = \"")? + "version = \"".len();
    let end = table[start..].find('"')? + start;
    Some(&table[start..end])
}

/// Whether the CHANGELOG carries a released section for `version`. The
/// closing bracket is part of the match, so a longer version sharing the same
/// leading digits cannot stand in for it.
fn has_release_section(changelog: &str, version: &str) -> bool {
    let heading = format!("## [{version}]");
    changelog.lines().any(|line| line.starts_with(&heading))
}

#[test]
fn the_declared_version_has_a_changelog_section() {
    let version = workspace_version(WORKSPACE_CARGO_TOML)
        .expect("workspace Cargo.toml declares a [workspace.package] version");

    assert!(
        has_release_section(CHANGELOG, version),
        "the workspace declares version {version}, but CHANGELOG.md has no \
         `## [{version}]` section.\n\n\
         If that version shipped, its heading has been consumed — most likely \
         by an edit that prepended to the top of the file and replaced the \
         heading rather than inserting above it. The entries are probably \
         still present, adopted by [Unreleased], where the next cut will \
         re-announce them under a new version. Restore the heading above the \
         first section belonging to the release; `git show <tag>:CHANGELOG.md` \
         is the reference for where it goes.\n\n\
         If the version was instead bumped by hand, that is the bug: \
         `scripts/cut-release.sh` bumps it and flips the CHANGELOG together.",
    );
}

#[test]
fn the_guard_reads_the_shapes_it_expects() {
    // The real check reduces to a heading lookup, so it would pass for the
    // wrong reason if the version parse returned something the CHANGELOG
    // happens to mention. Pin both halves against known shapes — including
    // the one this guard exists to catch — so neither can degrade into a
    // vacuous pass.
    let cargo = "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"1.2.3\"\nedition = \"2024\"\n";
    assert_eq!(
        workspace_version(cargo),
        Some("1.2.3"),
        "must read the version from the [workspace.package] table"
    );

    let decoy = "[workspace.dependencies]\nserde = { version = \"9.9.9\" }\n\n[workspace.package]\nversion = \"1.2.3\"\n";
    assert_eq!(
        workspace_version(decoy),
        Some("1.2.3"),
        "a dependency's version must not answer for the workspace's"
    );

    let intact = "## [Unreleased]\n\n## [1.2.3] - 2026-01-01\n\n### Fixed\n\n- **a** thing\n";
    assert!(
        has_release_section(intact, "1.2.3"),
        "an intact release heading must be found"
    );

    // The exact damage: the released heading removed, its body left behind
    // under [Unreleased]. Every entry survives, so the heading's absence is
    // the only thing separating this from the shape above.
    let absorbed = "## [Unreleased]\n\n### Fixed\n\n- **a** thing\n";
    assert!(
        !has_release_section(absorbed, "1.2.3"),
        "a consumed release heading must not be found — this is the shape the \
         guard exists to catch"
    );

    let sibling = "## [Unreleased]\n\n## [1.2.30] - 2026-01-01\n";
    assert!(
        !has_release_section(sibling, "1.2.3"),
        "a longer version sharing the same leading digits must not match"
    );
}
