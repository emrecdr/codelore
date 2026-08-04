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
