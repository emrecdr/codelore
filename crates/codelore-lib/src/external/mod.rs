//! External-findings integration — parse, store, and fuse external SARIF
//! scanner output with `CodeLore`'s behavioral signal.
//!
//! # Entry point
//!
//! [`sarif_parse::parse_sarif`] converts a raw SARIF 2.1.0 JSON string into
//! a `Vec<ExternalFinding>`.  Each finding is normalized to a repo-relative
//! path and carries a stable fingerprint derived from the dialect-appropriate
//! source (partialFingerprints → fingerprints → self-hash fallback, per the
//! §VALIDATED dialect-variance spec).

pub mod sarif_parse;
pub mod store;

pub use sarif_parse::{ExternalFinding, parse_sarif};
pub use store::{ExternalStore, PathFindings};

/// Group a flat list of findings by engine name.
///
/// Returns a `HashMap` keyed on `finding.engine`, with values being all
/// findings from that engine in input order. This is the grouping step that
/// precedes [`ExternalStore::replace_engine`]: the caller parses one or more
/// SARIF files into a flat `Vec<ExternalFinding>`, groups by engine, then
/// calls `replace_engine` once per engine — ensuring that two SARIF files
/// from the same engine are combined, not overwritten.
#[must_use]
pub fn group_findings_by_engine(
    findings: Vec<ExternalFinding>,
) -> std::collections::HashMap<String, Vec<ExternalFinding>> {
    let mut map: std::collections::HashMap<String, Vec<ExternalFinding>> =
        std::collections::HashMap::new();
    for f in findings {
        map.entry(f.engine.clone()).or_default().push(f);
    }
    map
}
