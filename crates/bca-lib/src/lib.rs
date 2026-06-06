//! bca-lib — Behavioral Code Analyzer library.
//!
//! See `docs/superpowers/specs/2026-06-06-bca-design.md`.

#![doc(html_no_source)]

pub mod error;
pub mod types;

pub use error::{BcaError, Result};
pub use types::{ChangeType, CommitEvent, FileChange, Hunk, KameiFeatures, SCHEMA_VERSION};
