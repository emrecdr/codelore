//! Per-language complexity metric computation via vendored bca-rca.
//!
//! Plan 3 scope: HEAD-only file-level + function-level entity extraction
//! for Tier-1 languages (Rust, TS/JS, Python, Java).
//! See spec §4 and Plan 3 §3.

pub mod language;

pub use language::Tier1Language;

use crate::Result;
use bca_rca::{
    FuncSpace, JavaParser, JavascriptParser, ParserTrait, PythonParser, RustParser, SpaceKind,
    TypescriptParser, metrics,
};
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
    /// Nesting metrics are not directly exposed by bca-rca; set to 0.
    pub max_nesting: u32,
    pub mean_nesting: f64,
    pub sd_nesting: f64,
    pub total_nesting: u32,
}

fn space_kind_str(kind: SpaceKind) -> &'static str {
    match kind {
        SpaceKind::Function => "function",
        SpaceKind::Class => "class",
        SpaceKind::Struct => "struct",
        SpaceKind::Trait => "trait",
        SpaceKind::Impl => "impl",
        SpaceKind::Unit => "unit",
        SpaceKind::Namespace => "namespace",
        SpaceKind::Interface => "interface",
        SpaceKind::Unknown => "other",
    }
}

/// Recursively traverse a `FuncSpace` tree, collecting `ComplexityEntity` entries.
fn collect_entities(space: &FuncSpace, path: &str, out: &mut Vec<ComplexityEntity>) {
    let m = &space.metrics;

    // Halstead — guard against NaN/Inf (empty source produces volume = NaN)
    let h_volume = {
        let v = m.halstead.volume();
        if v.is_finite() { Some(v) } else { None }
    };
    let h_difficulty = {
        let d = m.halstead.difficulty();
        if d.is_finite() { Some(d) } else { None }
    };
    let h_effort = {
        let e = m.halstead.effort();
        if e.is_finite() { Some(e) } else { None }
    };

    // MI — guard for NaN (log of 0 → -Inf → whole formula invalid)
    let mi_val = {
        let v = m.mi.mi_sei();
        if v.is_finite() { Some(v) } else { None }
    };

    // Helper: convert f64 metrics (which are non-negative counts) to u32 safely.
    // We clamp to u32::MAX before truncating to avoid sign-loss and truncation.
    let f_to_u32 = |v: f64| -> u32 {
        if v.is_finite() && v >= 0.0 {
            let clamped = v.round().min(f64::from(u32::MAX));
            // SAFETY: clamped is in [0.0, u32::MAX], so truncation is safe.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                clamped as u32
            }
        } else {
            0
        }
    };

    let entity = ComplexityEntity {
        path: path.to_owned(),
        name: space
            .name
            .clone()
            .unwrap_or_else(|| "<anonymous>".to_owned()),
        kind: space_kind_str(space.kind).to_owned(),
        start_line: u32::try_from(space.start_line).unwrap_or(u32::MAX),
        end_line: u32::try_from(space.end_line).unwrap_or(u32::MAX),
        cyclomatic: m.cyclomatic.cyclomatic(),
        cognitive: m.cognitive.cognitive(),
        halstead_volume: h_volume,
        halstead_difficulty: h_difficulty,
        halstead_effort: h_effort,
        mi: mi_val,
        nom: f_to_u32(m.nom.total()),
        nexits: f_to_u32(m.nexits.exit_sum()),
        loc: f_to_u32(m.loc.sloc()),
        sloc: f_to_u32(m.loc.sloc()),
        // Nesting is not exposed as a standalone stat in bca-rca; left as 0.
        max_nesting: 0,
        mean_nesting: 0.0,
        sd_nesting: 0.0,
        total_nesting: 0,
    };
    out.push(entity);

    for child in &space.spaces {
        collect_entities(child, path, out);
    }
}

/// Compute complexity entities for a Tier-1 source file.
pub fn compute_for_file(
    path: &Path,
    source: &[u8],
    lang: Tier1Language,
) -> Result<Vec<ComplexityEntity>> {
    let code = source.to_vec();
    let path_str = path.to_str().unwrap_or("");

    let root: Option<FuncSpace> = match lang {
        Tier1Language::Rust => {
            let parser = RustParser::new(code, path, None);
            metrics(&parser, path)
        }
        Tier1Language::Python => {
            let parser = PythonParser::new(code, path, None);
            metrics(&parser, path)
        }
        Tier1Language::Java => {
            let parser = JavaParser::new(code, path, None);
            metrics(&parser, path)
        }
        Tier1Language::JavaScript => {
            let parser = JavascriptParser::new(code, path, None);
            metrics(&parser, path)
        }
        Tier1Language::TypeScript => {
            let parser = TypescriptParser::new(code, path, None);
            metrics(&parser, path)
        }
    };

    let mut entities = Vec::new();
    if let Some(space) = root {
        collect_entities(&space, path_str, &mut entities);
    }
    Ok(entities)
}
