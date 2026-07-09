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
