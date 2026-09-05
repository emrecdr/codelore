//! `DuckDB` schema DDL. See spec §3.2.

pub const SCHEMA_V1: &str = include_str!("schema_v1.sql");

/// Current `DuckDB` schema version. Stamped into the `provenance` table on
/// every fresh ingest and re-validated on every `open_read_only` so an
/// operator who hands a stale `.duckdb` to `--cache-dir` directly gets a
/// typed parse-time error instead of cryptic SQL failures at analysis
/// time. Bump whenever a `CREATE TABLE` shape changes.
pub const CURRENT_SCHEMA_VERSION: &str = "8";

pub const INITIAL_PROVENANCE: &[(&str, &str)] = &[
    ("schema_version", CURRENT_SCHEMA_VERSION),
    ("codelore_version", env!("CARGO_PKG_VERSION")),
    ("arrow_version", crate::arrow_facade::ARROW_RUNTIME_VERSION),
];

/// Eligible-file count of the HEAD **complexity** scan, and the subset it
/// scored.
///
/// The key names carry the scan they describe because they are not the only
/// HEAD-time scan that tallies coverage — `clones` and `imports` do too, and
/// the module doc for `ScanCoverage` argues a thin `clones` matters more than
/// a thin `complexity_metrics`. Namespacing now costs two tokens; doing it
/// after these keys ship costs a `CACHE_EPOCH` bump.
///
/// These live in `provenance` rather than in an in-memory ingest stat because
/// the gate that reads them runs on cache hits too, and a cache hit never
/// re-executes the scan. The store *is* the cache, so a row written here
/// outlives the ingest that produced it; a counter does not.
///
/// Absent on any store written before these keys existed. Readers must treat
/// missing as "unknown" rather than as zero — a zero eligible count means a
/// docs-only tree, which is honestly complete, not blind.
pub const KEY_HEAD_SCAN_ELIGIBLE: &str = "head_scan_complexity_eligible";
/// Companion to [`KEY_HEAD_SCAN_ELIGIBLE`]; see its documentation.
pub const KEY_HEAD_SCAN_SCORED: &str = "head_scan_complexity_scored";
