//! codelore-lib — Behavioral Code Analyzer library.
//!
//! See `docs/superpowers/specs/2026-06-06-codelore-design.md`.

#![doc(html_no_source)]

pub mod analyses;
pub mod analysis;
pub mod arch_rules;
pub mod arrow_facade;
pub mod bands;
pub mod cache;
pub mod cli_api;
pub mod clones;
pub mod complexity;
pub mod constants;
pub mod error;
pub mod external;
pub mod facts;
pub mod hashing;
pub mod identity;
pub mod imports;
pub mod kamei;
pub mod options;
pub mod output;
pub mod paths;
pub mod paths_filter;
pub mod provenance;
pub mod quality_gates;
pub mod repo;
pub mod stats;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{CodeLoreError, Result};
pub use facts::FactsDb;
pub use options::Options;
pub use repo::{Repo, TagInfo};
pub use types::{ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION};
