//! Single source of truth for `Options` default values + `clap`
//! `default_value_t` annotations. Without this module the same number
//! had to be edited in two places, and drift went unnoticed at compile
//! time — a hand-cross-reference of every default was the only catch.
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
/// `CodeLore`'s Type-2 detection in the original design.
pub const DEFAULT_CLONE_SIMILARITY_FLOOR: f64 = 0.70;

/// Skip clone pairs whose two members live in the same directory. ON by
/// default because same-dir clones are usually intentional helpers
/// (e.g., test-fixture duplicates), and they crowd out the cross-module
/// signal. Users who want them can opt back in.
pub const DEFAULT_CLONE_SKIP_SAME_DIR: bool = true;

/// Maximum source-file size in bytes that we'll feed to tree-sitter
/// for complexity / clones extraction at HEAD. Files larger than this
/// are skipped with a tracing-debug log entry and excluded from
/// AST-based metrics.
///
/// 2 MB covers real hand-written source (Linux's `block.c` ~195 KB;
/// V8 monorepo's largest hand-written .cc ~1.5 MB) while reliably
/// catching the common offenders: minified JS bundles (5–50 MB),
/// protobuf-generated .pb.cc (often 10+ MB), and vendored single-file
/// libraries (sqlite3.c is ~9 MB). Without this cap tree-sitter
/// occasionally hits stack-overflow or OOM on deeply-nested generated
/// code; the cap turns those failures into graceful skips.
pub const DEFAULT_MAX_AST_FILE_BYTES: usize = 2 * 1024 * 1024;

/// T8: An author is considered "departed" if their most recent commit
/// anywhere in the repo is older than this many days at the anchor
/// moment. 90 days is the default — chosen as the empirical
/// "person has stopped contributing in a meaningful way" threshold
/// based on industry retention/sabbatical patterns (most engineers who
/// leave permanently stop committing within 60 days; a 90-day window
/// avoids flagging contributors on extended leave / between projects).
/// Tunable via `--departed-threshold-days N` for organisations with
/// longer (academia / OSS maintainers) or shorter (fast-moving startup)
/// project cadences.
///
/// Critical for the `knowledge-islands` analysis (Bird et al. 2011
/// "Don't Touch My Code!" — risk indicator combined with departure).
/// `CodeScene` requires manual marking of "Ex-Developers"; `CodeLore`
/// detects automatically via commit-date falloff.
pub const DEFAULT_DEPARTED_THRESHOLD_DAYS: u32 = 90;

/// Default trailing-window length (in days) for activity-scoped analyses.
/// Anchored to the repo's most recent commit date, not wall-clock time, so
/// results are reproducible on archived repos. Used by any analysis that
/// should focus on recent activity rather than full history. 90 days is the
/// prevailing "active contributor" window in contributor-lifecycle research
/// and long enough to smooth vacation/release-cycle gaps.
pub const DEFAULT_WINDOW_DAYS: u32 = 90;

/// Default rework-detection window in days. Hunk pairs on the same path
/// where the second commit's author-date is within this many days of the
/// first are considered rework candidates. 21 days aligns with Pluralsight
/// Flow's convention (`GitClear` uses 14; ours is configurable via
/// `--rework-window-days`). Output is labelled "approximate" because line
/// drift between commits is not tracked.
pub const DEFAULT_REWORK_WINDOW_DAYS: u32 = 21;

/// Default glob pattern for selecting release tags in `release-cadence`.
/// Only tags whose short name matches this pattern are counted as releases.
/// Uses standard glob syntax (`*` = any sequence, `?` = single char,
/// `[...]` = character class). Default `v*` captures conventional semver
/// tags (`v1.0.0`, `v2.3.1`) while excluding nightly/prerelease tags
/// that do not represent formal releases.
pub const DEFAULT_RELEASE_TAG_GLOB: &str = "v*";

/// Default `DuckDB` `memory_limit` PRAGMA applied to every connection this
/// binary opens (see `facts::mod::apply_memory_pragmas`). Matches the v1
/// performance target of "peak memory under 4 GB (`DuckDB` spill enabled)"
/// documented in `docs/superpowers/specs/2026-06-06-codelore-design.md`
/// (§ Performance targets) and measured in `docs/perf-evidence-v1.md`.
/// Once a query's resident state would exceed this ceiling, `DuckDB` spills
/// intermediate hash-join / sort / aggregation state to `temp_directory`
/// instead of growing unbounded and inviting the OS OOM killer.
pub const DEFAULT_DUCKDB_MEMORY_LIMIT: &str = "4GB";

/// T8: Threshold for "substantial author" — an author who isn't the
/// main author but owns at least this fraction of file's `LoC`. Used to
/// count `n_substantial_others` per file in `knowledge-islands`. A
/// file with 0 substantial other owners + departed main author is the
/// most actionable knowledge-loss signal; 1+ substantial others means
/// there's still someone who genuinely knows the code.
///
/// 0.10 (10%) is the default — matches the academic literature's
/// definition of "non-trivial contribution" (Bird et al., Mockus &
/// Herbsleb).
pub const DEFAULT_SUBSTANTIAL_OWNER_THRESHOLD: f64 = 0.10;
