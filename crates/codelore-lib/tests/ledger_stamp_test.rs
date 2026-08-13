//! Guard: a ledger stamp claiming unreleased work must be backed by a
//! non-empty `[Unreleased]` CHANGELOG section.
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

/// True where a finding's status is written: the parenthetical of a `### F…`
/// heading, or a `**Status**:` bullet in the carried-forward sections. Prose
/// that merely discusses the section is not a stamp and must not count.
fn is_stamp_line(line: &str) -> bool {
    line.starts_with("### F") || line.contains("**Status**:")
}

/// Stamps claiming work that has not shipped.
///
/// Asking whether a stamp *mentions* `Unreleased` — rather than matching a
/// list of exact spellings — is the point. The list this replaced held two,
/// and a compound stamp, one half shipped in a release and the other still
/// pending, matched neither. A row claiming unreleased work was therefore
/// invisible to the guard written to find exactly that claim, and stayed
/// wrong for two releases after the work had shipped. A rule that enumerates
/// the spellings it has seen fails on the next one.
fn unreleased_stamp_count(ledger: &str) -> usize {
    ledger
        .lines()
        .filter(|l| is_stamp_line(l) && l.contains("Unreleased"))
        .count()
}

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

    let stamped = unreleased_stamp_count(&ledger);
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

    // Stamp recognition, pinned against the spellings the ledger actually
    // uses AND the compound form that defeated the previous exact-match list.
    // IDs are assembled at runtime. This file is scanned by the comment
    // hygiene guard, so a literal finding ID written here as an illustration
    // is a violation of the rule that guard enforces — and was one, the first
    // time this fixture was written.
    let f = |n: u32| format!("F{n}");
    let stamps = format!(
        "### {} (Fixed — Unreleased) — a\n\
         ### {} (Fixed (Unreleased)) — b\n\
         ### {} (Fixed — v0.27.2 + Unreleased) — c\n\
         ### {} (Fixed — v0.27.3) — d\n\
         *   **Status**: Fixed (Unreleased)\n\
         *   **Status**: Active\n",
        f(1),
        f(2),
        f(3),
        f(4)
    );
    assert_eq!(
        unreleased_stamp_count(&stamps),
        4,
        "every stamp mentioning Unreleased counts, including a compound one \
         and a Status bullet; a shipped-version stamp does not"
    );

    // Prose discussing the section is not a stamp. Prophylactic rather than
    // a fixed miscount: the previous rule scanned the whole document for two
    // exact spellings, so a sentence quoting one verbatim would have counted
    // — no such sentence existed, and scoping to stamp lines means none can.
    let prose = "The stamp read \"+ Unreleased\" for two releases after the \
                 work shipped, which is the rot this guard exists to catch.\n";
    assert_eq!(
        unreleased_stamp_count(prose),
        0,
        "a sentence about unreleased stamps is not itself a stamp"
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
