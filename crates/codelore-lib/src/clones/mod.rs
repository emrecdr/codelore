//! Clone detection (Plan 7).
//!
//! Identifies Type 1 (exact) and Type 2 (renamed/parameterized) clones via
//! AST structural hashing on tree-sitter parses. The fingerprint walks the
//! AST in pre-order emitting `(node_kind_id, child_count)` pairs while
//! skipping identifier + literal nodes — this normalization is what makes
//! the hash Type 2-aware.
//!
//! Optional Type 3 (near-miss) support via `MinHash` + LSH lands in Plan 7
//! Task 4 (or v1.x).
//!
//! The clone-coupling intersection — clone families that ALSO co-change
//! at Fisher-significant rates — is the differentiating signal `CodeScene`
//! calls "X-Ray", with our published-formula transparency wedge applied.

pub mod fingerprint;
pub mod language;

pub use fingerprint::{Fingerprint, fingerprint_source};
pub use language::CloneLanguage;
