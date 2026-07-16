//! Advisory enrichment layer — deterministic fact sheets and (in later units)
//! grounded LLM narratives assembled on top of the existing analyses.
//!
//! Everything here is strictly advisory. No module in the scoring path
//! (`analyses`, `quality_gates`, `facts`, `calibration`, `defect_calibration`,
//! `provenance`, `output`) imports it, so enrichment can never perturb an
//! analysis row, a gate verdict, an exit code, or a fact-store cache key. That
//! one-way dependency boundary is guarded by the `enrichment_isolation_test`
//! integration test.

/// Fact-sheet serialization version. A bump signals a change in what a fact
/// sheet carries or how it is rendered; it feeds the sidecar narrative-cache
/// key, so bumping it invalidates every cached narrative naturally.
pub const SCHEMA_VERSION: u32 = 1;

pub mod fact_sheet;
