//! Guard: a `Fixed (Unreleased)` ledger stamp must be backed by a non-empty
//! `[Unreleased]` CHANGELOG section.
//!
//! The stamp is a claim ABOUT that section — "this shipped, and the release
//! notes for it are sitting in `[Unreleased]`". Cutting a release drains the
//! section into a versioned one, which silently makes every such row false:
//! the work shipped, and the ledger the team reads to know what is done still
//! says it did not.
//!
//! That is not hypothetical. It rotted after v0.26.0, was reconciled by hand,
//! and rotted again after v0.27.0 — 42 rows against an empty section — because
//! both repairs fixed the rows rather than the mechanism. `cut-release.sh` now
//! re-stamps at the cut; this guard is what makes that step's absence
//! detectable rather than something noticed two releases later.
//!
//! The check is deliberately one-directional. Unreleased rows with a populated
//! section are normal mid-cycle state. Zero rows is always fine. Only rows
//! with nothing behind them are wrong.

use std::path::{Path, PathBuf};

const LEDGER: &str = "docs/reports/deep_analysis_report.md";
const CHANGELOG: &str = "CHANGELOG.md";

/// Both spellings in use. A sweep that handled only one would leave half the
/// rows stale while looking finished, so the guard counts both.
const UNRELEASED_MARKS: &[&str] = &["Fixed (Unreleased)", "Fixed — Unreleased"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root two levels above crates/codelore-lib")
        .to_path_buf()
}

/// Entries in the `[Unreleased]` section — the bullet lines, not blank lines
/// or the heading, so an empty-but-present section reads as zero.
fn unreleased_entry_count(changelog: &str) -> usize {
    changelog
        .lines()
        .skip_while(|l| !l.starts_with("## [Unreleased]"))
        .skip(1)
        .take_while(|l| !l.starts_with("## ["))
        .filter(|l| l.trim_start().starts_with("- "))
        .count()
}

#[test]
fn unreleased_ledger_stamps_have_changelog_entries_behind_them() {
    let root = workspace_root();
    let ledger = std::fs::read_to_string(root.join(LEDGER)).expect("read findings ledger");
    let changelog = std::fs::read_to_string(root.join(CHANGELOG)).expect("read CHANGELOG");

    let stamped: usize = UNRELEASED_MARKS
        .iter()
        .map(|m| ledger.matches(m).count())
        .sum();
    let entries = unreleased_entry_count(&changelog);

    assert!(
        stamped == 0 || entries > 0,
        "{stamped} ledger row(s) are stamped Unreleased, but CHANGELOG's \
         [Unreleased] section is empty — so every one of them claims to be \
         backed by release notes that have already been cut into a version.\n\n\
         This is what a release cut leaves behind when the ledger is not \
         re-stamped with it. `scripts/cut-release.sh` does that at cut time; \
         if these rows appeared without a cut, the stamps are simply wrong. \
         Rewrite them to the version that shipped the work.",
    );
}

#[test]
fn the_guard_reads_the_changelog_shape_it_expects() {
    // The assertion above is a one-directional implication, so it passes
    // whenever `stamped == 0` — including if the ledger path broke and the
    // file read empty. Pin the parser against known shapes so a silent
    // mis-parse cannot make the real check vacuous.
    let populated = "## [Unreleased]\n\n### Fixed\n\n- **a** thing\n- **b** thing\n\n## [0.27.0] - 2026-08-06\n\n- **old** entry\n";
    assert_eq!(
        unreleased_entry_count(populated),
        2,
        "must count only entries inside [Unreleased], not later sections"
    );

    let drained = "## [Unreleased]\n\n## [0.27.0] - 2026-08-06\n\n- **old** entry\n";
    assert_eq!(
        unreleased_entry_count(drained),
        0,
        "a drained section has no entries even though later sections do"
    );

    // And the ledger itself must be readable and non-trivial, or the real
    // check is counting zero stamps for the wrong reason.
    let ledger = std::fs::read_to_string(workspace_root().join(LEDGER)).expect("read ledger");
    assert!(
        ledger.len() > 10_000,
        "findings ledger read as {} bytes — path resolution is broken",
        ledger.len()
    );
}
