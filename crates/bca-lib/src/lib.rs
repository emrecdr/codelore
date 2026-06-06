//! bca-lib — Behavioral Code Analyzer library.
//!
//! See `docs/superpowers/specs/2026-06-06-bca-design.md`.

#![doc(html_no_source)]

pub mod analysis;
pub mod arrow_facade;
pub mod error;
pub mod options;
pub mod types;

pub use analysis::{AnalysisName, UnknownAnalysisError};
pub use error::{BcaError, Result};
pub use options::Options;
pub use types::{ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION};
