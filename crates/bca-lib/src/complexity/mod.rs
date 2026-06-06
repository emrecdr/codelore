//! Per-language complexity metric computation via vendored bca-rca.
//!
//! Plan 3 scope: HEAD-only file-level + function-level entity extraction
//! for Tier-1 languages (Rust, TS/JS, Python, Java).
//! See spec §4 and Plan 3 §3.

pub mod language;

pub use language::Tier1Language;

use crate::Result;
use std::path::Path;

/// One function (or class, or file-level unit) with its complexity metrics.
#[derive(Debug, Clone)]
pub struct ComplexityEntity {
    pub path: String,
    pub name: String,
    pub kind: String, // "function", "method", "class", "file"
    pub start_line: u32,
    pub end_line: u32,
    pub cyclomatic: f64,
    pub cognitive: f64,
    pub halstead_volume: Option<f64>,
    pub halstead_difficulty: Option<f64>,
    pub halstead_effort: Option<f64>,
    pub mi: Option<f64>,
    pub nom: u32,
    pub nexits: u32,
    pub loc: u32,
    pub sloc: u32,
    pub max_nesting: u32,
    pub mean_nesting: f64,
    pub sd_nesting: f64,
    pub total_nesting: u32,
}

/// Compute complexity entities for a Tier-1 source file.
/// Plan 3 stub; real impl in Task 3.
pub fn compute_for_file(
    _path: &Path,
    _source: &[u8],
    _lang: Tier1Language,
) -> Result<Vec<ComplexityEntity>> {
    Ok(vec![])
}
