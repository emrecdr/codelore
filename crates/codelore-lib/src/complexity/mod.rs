//! Per-language complexity metric computation via vendored codelore-rca.
//!
//! HEAD-only file-level + function-level entity extraction
//! for Tier-1 languages (Rust, TS/JS, Python, Java).
//! See spec §4.

pub mod language;

pub use language::Tier1Language;

use crate::Result;
use codelore_rca::{
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
    /// Number of arguments of this function/closure (0 for non-callable spaces).
    pub nargs: u32,
    pub loc: u32,
    pub sloc: u32,
    /// Deepest control-flow nesting reached in this space (three nested `if`s
    /// yield 3; 0 when the space has no branching/looping construct).
    pub max_nesting: u32,
    /// Per-construct nesting distribution is not retained by codelore-rca, so
    /// mean/sd nesting are left at 0.0 (honest-absence).
    pub mean_nesting: f64,
    pub sd_nesting: f64,
    /// Sum of the nesting depth of every branching/looping construct.
    pub total_nesting: u32,
    /// Count of boolean-operator sequences in conditions (`a && b || c` is 2;
    /// `a && b && c` is 1) — the complex-conditional driver.
    pub bool_ops: u32,
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
        // A leaf function/closure space carries its own argument count in
        // `fn_args`/`closure_args`; non-callable spaces (file unit, impl,
        // class) report 0 here.
        nargs: f_to_u32(m.nargs.fn_args() + m.nargs.closure_args()),
        // `loc` = physical lines of code (including comments + blanks).
        // `sloc` = source lines of code (the "code-only" subset).
        // Before this fix both columns received `sloc()`, silently
        // discarding the physical-LOC count for every ingested file and
        // leaving the `loc` column with duplicate `sloc` data. The
        // `ploc()` API was already exposed by `rust-code-analysis` (see
        // `crates/codelore-rca/src/metrics/loc.rs`) — just never called.
        loc: f_to_u32(m.loc.ploc()),
        sloc: f_to_u32(m.loc.sloc()),
        // Nesting/bool-ops are derived from the cognitive pass, which walks the
        // AST once and already tracks nesting depth and boolean sequences.
        // Per-construct nesting distribution is not retained, so mean/sd stay 0.
        max_nesting: f_to_u32(m.cognitive.max_nesting()),
        mean_nesting: 0.0,
        sd_nesting: 0.0,
        total_nesting: f_to_u32(m.cognitive.total_nesting()),
        bool_ops: f_to_u32(m.cognitive.bool_ops()),
    };
    out.push(entity);

    for child in &space.spaces {
        collect_entities(child, path, out);
    }
}

/// Compute complexity entities for a Tier-1 source file.
///
/// `source` is taken by value because every `codelore-rca` parser constructor
/// consumes the byte buffer (`*Parser::new(Vec<u8>, &Path, Option<…>)`). The
/// caller in `facts/ingest.rs::ingest_complexity_at_head` already owns a
/// fresh `Vec<u8>` from `Repo::read_blob_at_head`, so the move is free; a
/// `&[u8]` parameter would force a redundant `.to_vec()` per file (one
/// full-source clone per Tier-1 file ingested at HEAD).
pub fn compute_for_file(
    path: &Path,
    source: Vec<u8>,
    lang: Tier1Language,
) -> Result<Vec<ComplexityEntity>> {
    let path_str = path.to_str().unwrap_or("");

    let root: Option<FuncSpace> = match lang {
        Tier1Language::Rust => {
            let parser = RustParser::new(source, path, None);
            metrics(&parser, path)
        }
        Tier1Language::Python => {
            let parser = PythonParser::new(source, path, None);
            metrics(&parser, path)
        }
        Tier1Language::Java => {
            let parser = JavaParser::new(source, path, None);
            metrics(&parser, path)
        }
        Tier1Language::JavaScript => {
            let parser = JavascriptParser::new(source, path, None);
            metrics(&parser, path)
        }
        Tier1Language::TypeScript => {
            let parser = TypescriptParser::new(source, path, None);
            metrics(&parser, path)
        }
    };

    let mut entities = Vec::new();
    if let Some(space) = root {
        collect_entities(&space, path_str, &mut entities);
    }
    Ok(entities)
}
