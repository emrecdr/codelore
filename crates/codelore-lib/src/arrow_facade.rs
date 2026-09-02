//! The workspace's record of which arrow-rs version is linked at runtime.
//!
//! Arrow reaches this workspace only transitively, through the `duckdb`
//! crate — parquet output goes through DuckDB's own `COPY … TO (FORMAT
//! PARQUET)`, and the ingest uses the row-level `Appender`, so no Rust
//! code here touches arrow types. A direct `arrow` dependency used to
//! live beside duckdb's and drifted a major ahead of it, putting TWO
//! arrow generations in the build graph while this module's constant —
//! stamped into every provenance sidecar and the fact store's provenance
//! table — kept describing the other one. The drift guard read the first
//! lockfile match and passed. The direct dependency is gone; the constant
//! below tracks duckdb's pinned arrow, and the guard now fails loudly on
//! duplicate lockfile entries instead of picking one.

/// Version reported by the runtime (for provenance manifests). Must match
/// the single `arrow` entry in `Cargo.lock` — the one `duckdb` pins;
/// `dep_versions_drift_test` enforces both the match and the singleness.
pub const ARROW_RUNTIME_VERSION: &str = "58.3.0";
