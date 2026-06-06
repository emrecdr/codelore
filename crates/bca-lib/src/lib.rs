//! bca-lib — Behavioral Code Analyzer library.
//!
//! See `docs/superpowers/specs/2026-06-06-bca-design.md`.

#![doc(html_no_source)]

pub mod analyses;
pub mod analysis;
pub mod arrow_facade;
pub mod complexity;
pub mod error;
pub mod facts;
pub mod options;
pub mod output;
pub mod repo;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{BcaError, Result};
pub use facts::FactsDb;
pub use options::Options;
pub use repo::Repo;
pub use types::{ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION};
