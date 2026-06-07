//! Per-language tree-sitter loader for clone fingerprinting.
//!
//! Mirrors the dispatch shape of `crate::complexity::language::Tier1Language`
//! but builds raw `tree_sitter::Parser` instances so the AST walker in
//! `fingerprint` can traverse `Node` directly. We do not go through
//! `codelore-rca`'s `FuncSpace` because `FuncSpace` doesn't carry the raw
//! `tree_sitter::Node` (its lifetime is tied to the parser's tree, which
//! `FuncSpace` summarises into `start_line`/`end_line` + metrics only).

use std::path::Path;

/// Tier-1 languages `CodeLore` clone-detects. Mirrors
/// `crate::complexity::language::Tier1Language` 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloneLanguage {
    Rust,
    Python,
    Java,
    JavaScript,
    TypeScript,
}

impl CloneLanguage {
    /// Map a file extension to its `CloneLanguage`. Returns `None` for
    /// non-Tier-1 files (we silently skip them during the clone pass).
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        match ext {
            "rs" => Some(Self::Rust),
            "py" => Some(Self::Python),
            "java" => Some(Self::Java),
            "js" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "tsx" => Some(Self::TypeScript),
            _ => None,
        }
    }

    /// Build a tree-sitter `Language` for this `CloneLanguage`. The grammars
    /// are the same exact-pinned crates `codelore-rca` uses, exposed via
    /// `codelore-rca`'s re-exports so we get parser-ABI compatibility.
    #[must_use]
    pub fn language(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        }
    }

    /// Per-language set of node-kind names to skip when fingerprinting.
    /// Identifiers and literals are normalized away — this is what makes
    /// the fingerprint Type 2-aware. The names are tree-sitter `kind()`
    /// strings, not numeric kind ids (kind ids are language-specific
    /// and can shift across grammar revisions; names are stable).
    #[must_use]
    pub fn skip_kinds(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &[
                "identifier",
                "type_identifier",
                "field_identifier",
                "primitive_type",
                "integer_literal",
                "float_literal",
                "string_literal",
                "char_literal",
                "boolean_literal",
                "raw_string_literal",
                "byte_literal",
                "byte_string_literal",
            ],
            Self::Python => &[
                "identifier",
                "integer",
                "float",
                "string",
                "true",
                "false",
                "none",
                "concatenated_string",
            ],
            Self::Java => &[
                "identifier",
                "type_identifier",
                "decimal_integer_literal",
                "hex_integer_literal",
                "decimal_floating_point_literal",
                "string_literal",
                "character_literal",
                "true",
                "false",
                "null_literal",
            ],
            Self::JavaScript | Self::TypeScript => &[
                "identifier",
                "type_identifier",
                "property_identifier",
                "shorthand_property_identifier",
                "number",
                "string",
                "template_string",
                "true",
                "false",
                "null",
                "undefined",
                "regex",
            ],
        }
    }

    /// Per-language set of node-kind names that mark a *function* boundary.
    /// Each match becomes a standalone clone-detection unit.
    #[must_use]
    pub fn function_kinds(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &[
                "function_item",
                "function_signature_item",
                "closure_expression",
            ],
            Self::Python => &["function_definition"],
            Self::Java => &["method_declaration", "constructor_declaration"],
            Self::JavaScript | Self::TypeScript => &[
                "function_declaration",
                "method_definition",
                "arrow_function",
                "function_expression",
                "generator_function",
                "generator_function_declaration",
            ],
        }
    }
}
