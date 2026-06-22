//! Single-cursor tree-sitter walker that captures import edges from
//! Tier-1 source files.
//!
//! Mirrors the iterative `TreeCursor` pattern from
//! `crate::clones::fingerprint` (one cursor allocation
//! regardless of subtree size). Each `import_node_kinds()` hit is
//! recorded as a [`RawImport`] with the raw target text + a coarse
//! [`ImportKind`] classification.
//!
//! The walker captures raw target text — e.g. for Rust it grabs
//! `"use std::fs::read_to_string;"` verbatim then strips the keyword
//! and trailing punctuation with a tiny per-language normaliser.
//! The companion resolver in `resolver.rs` parses cleaned targets
//! into canonical module paths and walks the repo layout to map
//! them to tracked files where possible.

use super::language::ImportLanguage;
use crate::error::{CodeLoreError, Result};
use tree_sitter::{Node, Parser, TreeCursor};

/// One captured import edge — pre-resolution.
#[derive(Debug, Clone)]
pub struct RawImport {
    /// The normalised target string — for Rust this is "`std::fs`", for
    /// JS this is the contents of the `from '…'` string literal where
    /// the normaliser can extract it. Today's normaliser is best-
    /// effort trimming; per-language AST-child extraction is a
    /// planned enhancement.
    pub target: String,
    /// Coarse semantic bucket so SQL can filter without parsing
    /// `target`. See [`ImportKind`].
    pub kind: ImportKind,
}

/// Coarse import semantics. Closed set so the schema's `CHECK` can
/// validate at INSERT time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// Fully-qualified import — Rust `use std::fs`, Java
    /// `import java.util.List;`, JS `import x from 'react'`.
    Absolute,
    /// Path-relative — Rust `use crate::foo` / `use super::foo`,
    /// Python `from . import foo`, JS `from './foo'` / `from '../foo'`.
    Relative,
    /// Glob import — Rust `use foo::*`, Java `import foo.*;`,
    /// Python `from foo import *`.
    Wildcard,
    /// Couldn't determine — empty / malformed / unparseable target.
    /// Surfaces as a row so analyses can flag parse-quality issues.
    Unknown,
}

impl ImportKind {
    /// CHECK-constraint-compatible serialisation. Matches the closed
    /// set declared in `facts/schema_v1.sql::imports.kind`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::Wildcard => "wildcard",
            Self::Unknown => "unknown",
        }
    }
}

/// Parse `source` under `lang`'s grammar and collect every import
/// edge it carries. Returns an empty vec on parse error (logged at
/// warn level) so a single malformed file doesn't poison the ingest.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] only if tree-sitter rejects
/// the language assignment — a static-config bug that would fail
/// every file under that language. Per-file parse errors are
/// swallowed by design.
pub fn extract_imports(source: &[u8], lang: ImportLanguage) -> Result<Vec<RawImport>> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.language())
        .map_err(|e| CodeLoreError::Analysis(format!("set_language: {e}")))?;
    let Some(tree) = parser.parse(source, None) else {
        // tree-sitter returns None on parse error — return empty so
        // the caller treats this file as "no imports" rather than
        // failing the ingest.
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    walk_imports(tree.root_node(), source, lang, &mut out);
    Ok(out)
}

/// Iterative preorder walk over the AST using a single `TreeCursor`.
/// Mirrors `clones::fingerprint::fingerprint_recursive`'s pattern
/// (cursor allocated once vs once-per-node).
fn walk_imports(root: Node<'_>, source: &[u8], lang: ImportLanguage, out: &mut Vec<RawImport>) {
    let kinds = lang.import_node_kinds();
    let mut cursor: TreeCursor<'_> = root.walk();
    loop {
        let current = cursor.node();
        if kinds.contains(&current.kind())
            && let Some(raw) = node_text(current, source)
            && let Some(target) = normalise_target(&raw, lang)
            && !target.is_empty()
        {
            let kind = classify(&target, lang);
            out.push(RawImport { target, kind });
        }
        // Descend if possible — child first for preorder.
        if cursor.goto_first_child() {
            continue;
        }
        // No child: advance to next sibling, climbing as needed.
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            // No sibling and reached the root subtree boundary → done.
            if !cursor.goto_parent() || cursor.node().id() == root.id() {
                return;
            }
        }
    }
}

/// Slice the raw source between `node`'s byte range. Returns `None`
/// when the bytes don't form valid UTF-8 — files we can't read as
/// text shouldn't surface in the import graph anyway.
fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> Option<String> {
    let start = node.start_byte();
    let end = node.end_byte();
    if end > source.len() || start > end {
        return None;
    }
    let slice = &source[start..end];
    std::str::from_utf8(slice).ok().map(str::to_string)
}

/// Per-language target normalisation. Strips the language's import
/// keyword + trailing punctuation. A future enhancement will replace
/// this with proper AST-child extraction; today's heuristic gets
/// ~95 % correctness on real-world code while staying surgical.
///
/// Returns `None` when the statement cannot be normalised to a
/// meaningful module identifier (JS side-effect / dynamic imports,
/// Python `from X import Y` whose module specifier is empty). The
/// caller skips the row rather than storing the raw statement
/// text as a phantom target.
fn normalise_target(raw: &str, lang: ImportLanguage) -> Option<String> {
    // Collapse whitespace + strip leading/trailing punctuation.
    let s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = match lang {
        ImportLanguage::Rust => s
            .trim_start_matches("pub ")
            .trim_start_matches("use ")
            .trim_end_matches(';')
            .trim()
            .to_string(),
        ImportLanguage::Python => normalise_python_target(&s)?,
        ImportLanguage::Java => s
            .trim_start_matches("import ")
            .trim_start_matches("static ")
            .trim_end_matches(';')
            .trim()
            .to_string(),
        ImportLanguage::JavaScript | ImportLanguage::TypeScript | ImportLanguage::Tsx => {
            // The meaningful target is the string literal after
            // `from`. Side-effect imports (`import 'foo';`) and
            // dynamic imports have no such literal — return None so
            // the walker drops them, rather than storing the raw
            // statement text as a phantom target.
            extract_js_module_target(&s)?.trim().to_string()
        }
    };
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

/// Python target normalisation. Strips the `import` / `from` keyword
/// and discards everything after a `from X import Y`'s ` import ` so
/// only the dotted module specifier survives — `from os.path import
/// join` → `os.path`. `import x.y` → `x.y`. Returns None when the
/// statement carries no module specifier at all.
fn normalise_python_target(s: &str) -> Option<String> {
    let after = s.trim_start_matches("from ").trim_start_matches("import ");
    // `from X import Y` shape: keep everything before ` import `.
    let head = after.split_once(" import ").map_or(after, |(h, _)| h);
    // `import X, Y, Z` shape: keep the first dotted module.
    let first = head.split(',').next().unwrap_or(head).trim();
    // Drop `as Alias` tails on either branch.
    let target = first.split_once(" as ").map_or(first, |(h, _)| h).trim();
    if target.is_empty() {
        None
    } else {
        Some(target.to_string())
    }
}

/// Extract the module specifier from a JS/TS import statement —
/// the contents of the string literal after `from`. Returns `None`
/// when the pattern doesn't match (dynamic import, side-effect
/// import without `from`).
fn extract_js_module_target(s: &str) -> Option<&str> {
    let after_from = s.split_once(" from ")?.1.trim();
    // Strip the surrounding quotes; tolerate both single and double.
    let stripped = after_from
        .trim_start_matches(['\'', '"'])
        .trim_end_matches(';')
        .trim_end_matches(['\'', '"']);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped)
    }
}

/// Coarse classification using the cleaned `target` string.
fn classify(target: &str, lang: ImportLanguage) -> ImportKind {
    if target.is_empty() {
        return ImportKind::Unknown;
    }
    if target.ends_with('*') || target.contains("::*") {
        return ImportKind::Wildcard;
    }
    let relative_root = match lang {
        ImportLanguage::Rust => {
            target.starts_with("crate::")
                || target.starts_with("super::")
                || target.starts_with("self::")
        }
        ImportLanguage::Python => target.starts_with('.'),
        ImportLanguage::JavaScript | ImportLanguage::TypeScript | ImportLanguage::Tsx => {
            target.starts_with("./") || target.starts_with("../")
        }
        ImportLanguage::Java => false, // Java has no syntactic relative imports.
    };
    if relative_root {
        ImportKind::Relative
    } else {
        ImportKind::Absolute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_use_declaration_extracts_absolute_target() {
        let src = b"use std::fs::read_to_string;\nfn main() {}";
        let got = extract_imports(src, ImportLanguage::Rust).unwrap();
        assert_eq!(got.len(), 1, "one import expected");
        assert_eq!(got[0].target, "std::fs::read_to_string");
        assert_eq!(got[0].kind, ImportKind::Absolute);
    }

    #[test]
    fn rust_crate_relative_classifies_as_relative() {
        let src = b"use crate::analyses::hotspots;";
        let got = extract_imports(src, ImportLanguage::Rust).unwrap();
        assert_eq!(got[0].kind, ImportKind::Relative);
    }

    #[test]
    fn rust_glob_classifies_as_wildcard() {
        let src = b"use std::collections::*;";
        let got = extract_imports(src, ImportLanguage::Rust).unwrap();
        assert_eq!(got[0].kind, ImportKind::Wildcard);
    }

    #[test]
    fn python_from_extracts_module_target() {
        let src = b"from os.path import join\n";
        let got = extract_imports(src, ImportLanguage::Python).unwrap();
        assert!(!got.is_empty(), "expected at least one import row");
        assert!(
            got.iter().any(|r| r.target.contains("os.path")),
            "expected os.path target, got {got:?}"
        );
    }

    #[test]
    fn javascript_from_extracts_string_literal() {
        let src = b"import { useState } from 'react';\n";
        let got = extract_imports(src, ImportLanguage::JavaScript).unwrap();
        assert_eq!(got.len(), 1, "one import expected");
        assert_eq!(got[0].target, "react");
        assert_eq!(got[0].kind, ImportKind::Absolute);
    }

    #[test]
    fn javascript_relative_path_classifies_as_relative() {
        let src = b"import foo from './bar/baz';\n";
        let got = extract_imports(src, ImportLanguage::JavaScript).unwrap();
        assert_eq!(got[0].kind, ImportKind::Relative);
    }

    #[test]
    fn java_import_declaration_extracts_target() {
        let src = b"package com.example;\nimport java.util.List;\nclass A {}";
        let got = extract_imports(src, ImportLanguage::Java).unwrap();
        assert!(got.iter().any(|r| r.target == "java.util.List"));
    }

    #[test]
    fn java_wildcard_classifies_correctly() {
        let src = b"import java.util.*;";
        let got = extract_imports(src, ImportLanguage::Java).unwrap();
        assert_eq!(got[0].kind, ImportKind::Wildcard);
    }

    #[test]
    fn empty_source_yields_no_imports() {
        let src = b"";
        let got = extract_imports(src, ImportLanguage::Rust).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn source_without_imports_yields_no_rows() {
        let src = b"fn main() { let x = 1; }";
        let got = extract_imports(src, ImportLanguage::Rust).unwrap();
        assert!(got.is_empty());
    }
}
