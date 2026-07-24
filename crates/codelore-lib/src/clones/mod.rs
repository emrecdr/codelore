//! Clone detection.
//!
//! Identifies Type 1 (exact) and Type 2 (renamed/parameterized) clones via
//! AST structural hashing on tree-sitter parses. The fingerprint walks the
//! AST in pre-order emitting `(kind_name, child_count)` pairs while
//! skipping identifier, literal, and comment nodes — this normalization is
//! what makes the hash Type 2-aware.
//!
//! Optional Type 3 (near-miss) support via `MinHash` + LSH is not yet
//! implemented.
//!
//! The clone-coupling intersection — clone families that ALSO co-change
//! at Fisher-significant rates — is the differentiating signal `CodeScene`
//! calls "X-Ray", with our published-formula transparency wedge applied.

pub mod extractor;
pub mod fingerprint;
pub mod language;

pub use extractor::{CloneGroup, FunctionFingerprint, extract_functions, group_clones};
pub use fingerprint::{Fingerprint, fingerprint_source};
pub use language::CloneLanguage;
