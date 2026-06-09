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
    let start = lock.find(&needle)? + needle.len();
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
