//! Single source of truth for `Options` default values + `clap`
//! `default_value_t` annotations. Without this module the same number
//! had to be edited in two places, and drift went unnoticed at compile
//! time (the v0.1.0 audit caught the matching state — but only after a
//! manual cross-reference of every default).
//!
//! Add new defaults here; reference from `Options::default()` AND from
//! the corresponding `#[arg(default_value_t = …)]` annotation in
//! `codelore-cli`. Tests that want the default value can `use
//! codelore_lib::constants::*;` and reference the constant by name —
//! readers learn the semantics from the name instead of from a magic
//! number with a one-line comment.

/// Minimum revisions per entity to surface in any per-file analysis.
/// Matches code-maat's `-n` default. Filters away long-tail noise (files
/// changed once or twice) from hotspots and ownership reports.
pub const DEFAULT_MIN_REVS: u32 = 5;

/// Minimum shared revisions for a coupling pair. Below this floor, the
/// `(A, B)` pair is dropped before the Fisher exact test even runs.
/// Matches code-maat's `-m` default.
pub const DEFAULT_MIN_SHARED_REVS: u32 = 5;

/// Lower coupling-degree threshold (percent). Matches code-maat's `-i`
/// default. Pairs below this floor don't surface in `coupling` output.
pub const DEFAULT_MIN_COUPLING_PCT: u8 = 30;

/// Upper coupling-degree ceiling (percent). Matches code-maat's `-x`
/// default. Pairs at 100% (every commit modifies both files) are usually
/// file splits or copy/rename artifacts; the upper bound exists to drop
/// them when desired.
pub const DEFAULT_MAX_COUPLING_PCT: u8 = 100;

/// Drop commits touching more than N files from coupling / SOC analyses.
/// Matches code-maat's `-s` default. Filters refactor sweeps that
/// create spurious coupling noise (renames, mass formatter passes).
pub const DEFAULT_MAX_CHANGESET_SIZE: u32 = 30;

/// Two-tailed Fisher exact p-value cutoff above which a coupling pair
/// is considered statistically insignificant and dropped. 0.05 is the
/// conventional alpha; `CodeLore` inherits it.
pub const DEFAULT_FISHER_SIGNIFICANCE: f64 = 0.05;

/// Minimum AST-node count for a clone candidate. Functions smaller than
/// this are usually trivial getters / setters / accessor wrappers that
/// produce noisy false positives.
pub const DEFAULT_MIN_CLONE_NODE_COUNT: u32 = 30;

/// Minimum shared revisions for a clone pair to surface in
/// `clone-coupling`. Lower than `min_shared_revs` because clones are
/// rarer; we want the signal even for occasional co-changes.
pub const DEFAULT_MIN_CLONE_SHARED_REVS: u32 = 3;

/// Minimum AST-node similarity (0.0 - 1.0) for two functions to be
/// considered structurally a clone. 0.70 matches the floor used by
/// `CodeLore`'s Type-2 detection in the original Plan 7 design.
pub const DEFAULT_CLONE_SIMILARITY_FLOOR: f64 = 0.70;

/// Skip clone pairs whose two members live in the same directory. ON by
/// default because same-dir clones are usually intentional helpers
/// (e.g., test-fixture duplicates), and they crowd out the cross-module
/// signal. Users who want them can opt back in.
pub const DEFAULT_CLONE_SKIP_SAME_DIR: bool = true;
