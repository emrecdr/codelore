//! Per-language tree-sitter loader for import-edge extraction.
//!
//! Mirrors the dispatch shape of `crate::clones::language::CloneLanguage`
//! and `crate::complexity::language::Tier1Language` for extension
//! recognition. Splits TSX out as its own variant for the same reason
//! the clones module does: `LANGUAGE_TYPESCRIPT` errors on JSX tags,
//! `LANGUAGE_TSX` doesn't.
//!
//! The per-language `import_node_kinds()` set lists the tree-sitter
//! node-kind names that mark an import statement. Kind *names* are
//! stable across grammar versions; kind *ids* aren't (per the cache-
//! invalidation rationale in CLAUDE.md).

use std::path::Path;

/// Tier-1 languages `CodeLore` extracts import edges from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportLanguage {
    Rust,
    Python,
    Java,
    JavaScript,
    TypeScript,
    Tsx,
}

impl ImportLanguage {
    /// Map a file extension to its `ImportLanguage`. Returns `None`
    /// for non-Tier-1 files (the import pass silently skips them).
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        match ext {
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            "java" => Some(Self::Java),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            _ => None,
        }
    }

    /// Build a tree-sitter `Language` for this `ImportLanguage`. Uses
    /// the exact-pinned grammar crates the rest of the workspace
    /// already loads (parser-ABI compatible with both the complexity
    /// pass and the clones pass).
    #[must_use]
    pub fn language(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// Tree-sitter node-kind names that mark an import statement.
    /// Walker hits any node whose `kind()` is in this set and records
    /// the import edge from its raw source text.
    ///
    /// Verified against each grammar's `grammar.js` for the pinned
    /// versions in workspace `Cargo.toml` (`tree-sitter-* = "=0.23.x"`):
    ///   - Rust:    `use_declaration`
    ///   - Python:  `import_statement` (bare) + `import_from_statement` (from-style)
    ///   - Java:    `import_declaration`
    ///   - JS/TS:   `import_statement` (default / named / namespace / side-effect /
    ///     `import x = require(…)`), `export_statement` (re-exports:
    ///     `export … from`), and `call_expression` (dynamic `import(…)` and
    ///     `CommonJS` `require(…)`). The extractor filters call expressions down
    ///     to the two callee shapes that name a module.
    #[must_use]
    pub fn import_node_kinds(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["use_declaration"],
            Self::Python => &["import_statement", "import_from_statement"],
            Self::Java => &["import_declaration"],
            Self::JavaScript | Self::TypeScript | Self::Tsx => {
                &["import_statement", "export_statement", "call_expression"]
            }
        }
    }
}
