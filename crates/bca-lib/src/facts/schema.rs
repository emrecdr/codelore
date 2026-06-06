//! `DuckDB` schema DDL. See spec §3.2.

pub const SCHEMA_V1: &str = include_str!("schema_v1.sql");

pub const INITIAL_PROVENANCE: &[(&str, &str)] = &[
    ("schema_version", "1"),
    ("bca_version", env!("CARGO_PKG_VERSION")),
    ("arrow_version", crate::arrow_facade::ARROW_RUNTIME_VERSION),
];
