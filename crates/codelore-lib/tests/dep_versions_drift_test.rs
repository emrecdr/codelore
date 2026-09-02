//! Asserts that the hardcoded dependency-version strings in the provenance
//! and arrow-facade modules match the versions actually resolved by
//! `Cargo.lock`. A dependency bump without updating these constants drops
//! a stale version into every provenance JSON sidecar codelore emits —
//! the provenance receipt becomes a lie. This test catches the drift at
//! CI time instead of in a downstream user's report.
//!
//! Implementation: `include_str!` the workspace `Cargo.lock` (resolved at
//! compile time via `CARGO_MANIFEST_DIR`), find the `name = "<pkg>"` entry
//! and grab the immediately-following `version = "..."` line.

const CARGO_LOCK: &str = include_str!("../../../Cargo.lock");

/// Look up the version string Cargo resolved for `package` at compile time.
/// Returns `None` if the package isn't in the lockfile (which would itself
/// be a test failure — the package wouldn't compile in the first place).
fn locked_version<'a>(lock: &'a str, package: &str) -> Option<&'a str> {
    let needle = format!("\nname = \"{package}\"\nversion = \"");
    // First-match-wins is exactly how the arrow 58/59 drift hid: a direct
    // dependency added a SECOND generation to the lockfile, this helper
    // returned whichever sorted first, and the guard compared the constant
    // against the wrong one. Two entries is itself the defect — fail
    // naming both versions instead of picking one.
    let occurrences: Vec<usize> = lock
        .match_indices(&needle)
        .map(|(i, _)| i + needle.len())
        .collect();
    assert!(
        occurrences.len() <= 1,
        "Cargo.lock holds {} entries for package {package:?} — versions: {:?}. \
         A duplicated dependency generation means some direct dependency \
         desynced from the pinned one; deduplicate the graph instead of \
         letting this guard compare against an arbitrary copy.",
        occurrences.len(),
        occurrences
            .iter()
            .map(|&s| &lock[s..s + lock[s..].find('"').unwrap_or(0)])
            .collect::<Vec<_>>()
    );
    let start = *occurrences.first()?;
    let end = lock[start..].find('"')? + start;
    Some(&lock[start..end])
}

#[test]
fn arrow_runtime_version_matches_cargo_lock() {
    let resolved = locked_version(CARGO_LOCK, "arrow").expect("arrow in Cargo.lock");
    assert_eq!(
        resolved,
        codelore_lib::arrow_facade::ARROW_RUNTIME_VERSION,
        "arrow_facade::ARROW_RUNTIME_VERSION drifted from Cargo.lock — \
         bump the constant or pin the version"
    );
}

#[test]
fn provenance_gix_version_matches_cargo_lock() {
    let resolved = locked_version(CARGO_LOCK, "gix").expect("gix in Cargo.lock");
    assert_eq!(
        resolved,
        codelore_lib::provenance::GIX_VERSION,
        "provenance::GIX_VERSION drifted from Cargo.lock — bump the constant or pin the dep"
    );
}

#[test]
fn provenance_duckdb_version_matches_cargo_lock() {
    let resolved = locked_version(CARGO_LOCK, "duckdb").expect("duckdb in Cargo.lock");
    assert_eq!(
        resolved,
        codelore_lib::provenance::DUCKDB_VERSION,
        "provenance::DUCKDB_VERSION drifted from Cargo.lock — bump the constant or pin the dep"
    );
}

/// The guard's own matcher, probed with a synthetic duplicate: two lockfile
/// entries for one package must panic naming the count — first-match-wins
/// is exactly how the arrow 58/59 drift hid. (Probed on a string because
/// cargo regenerates the real lockfile before any test reads it, silently
/// scrubbing an appended fake entry.)
#[test]
#[should_panic(expected = "2 entries for package")]
fn locked_version_fails_loudly_on_duplicate_entries() {
    let lock = "\nname = \"arrow\"\nversion = \"58.3.0\"\nother = 1\n\nname = \"arrow\"\nversion = \"59.2.0\"\n";
    let _ = locked_version(lock, "arrow");
}

/// And the single-entry happy path stays a plain lookup.
#[test]
fn locked_version_returns_the_single_entry() {
    let lock = "\nname = \"arrow\"\nversion = \"58.3.0\"\n";
    assert_eq!(locked_version(lock, "arrow"), Some("58.3.0"));
}
