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
/// `crate::complexity::language::Tier1Language` for extension recognition,
/// but splits TSX out as its own variant: tree-sitter-typescript ships a
/// dedicated `LANGUAGE_TSX` grammar for `.tsx` files (the plain
/// `LANGUAGE_TYPESCRIPT` grammar errors on JSX tags). Complexity rolls TSX
/// into TypeScript because `codelore-rca`'s Coleman MI scoring is the same
/// for both — clone fingerprinting can't make that simplification because
/// it parses the raw AST and JSX tags become real node kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloneLanguage {
    Rust,
    Python,
    Java,
    JavaScript,
    TypeScript,
    Tsx,
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
            // tree-sitter-javascript's grammar accepts JSX natively, so
            // `.jsx` shares the JavaScript variant. This matches what the
            // complexity pass does at `complexity/language.rs`.
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
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
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// Per-language set of node-kind names to skip when fingerprinting.
    /// Identifiers, literals, and comments are normalized away — dropping
    /// identifiers/literals is what makes the fingerprint Type 2-aware, and
    /// dropping comments keeps a lone `// TODO` from defeating an otherwise
    /// exact match. The names are tree-sitter `kind()` strings, not numeric
    /// kind ids (kind ids are language-specific and can shift across grammar
    /// revisions; names are stable). Only the top-level comment kinds appear
    /// here: the walk prunes a comment's whole subtree (see
    /// [`Self::comment_kinds`]), so a `///` doc comment's inner marker/text
    /// children are never reached and need no entry.
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
                "line_comment",
                "block_comment",
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
                "comment",
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
                "line_comment",
                "block_comment",
            ],
            Self::JavaScript | Self::TypeScript | Self::Tsx => &[
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
                "comment",
                "html_comment",
            ],
        }
    }

    /// Per-language set of comment node-kind names. A subset of
    /// [`Self::skip_kinds`], but tracked separately because comments must be
    /// *fully transparent* to the fingerprint: unlike identifiers and
    /// literals (which are dropped from the emitted sequence yet still count
    /// toward their parent's arity so shape is preserved), a comment must
    /// also not inflate its parent's child count — otherwise a lone
    /// `// TODO` between two statements would perturb the enclosing block's
    /// arity and defeat the match. Only the top-level comment kinds appear
    /// here; Rust's doc-marker/`doc_comment` children live inside a
    /// `line_comment`/`block_comment` and never as a direct child of a code
    /// node, so they need no arity handling.
    #[must_use]
    pub fn comment_kinds(self) -> &'static [&'static str] {
        match self {
            Self::Rust | Self::Java => &["line_comment", "block_comment"],
            Self::Python => &["comment"],
            Self::JavaScript | Self::TypeScript | Self::Tsx => &["comment", "html_comment"],
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
            Self::JavaScript | Self::TypeScript | Self::Tsx => &[
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extension_dispatch() {
        // Plain TypeScript and TSX must dispatch to different grammars —
        // tree-sitter-typescript's plain TypeScript parser errors on JSX
        // tags. The JSX extension shares JavaScript's grammar, which
        // accepts JSX natively.
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.rs")),
            Some(CloneLanguage::Rust)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.py")),
            Some(CloneLanguage::Python)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.java")),
            Some(CloneLanguage::Java)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.js")),
            Some(CloneLanguage::JavaScript)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.mjs")),
            Some(CloneLanguage::JavaScript)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.cjs")),
            Some(CloneLanguage::JavaScript)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.jsx")),
            Some(CloneLanguage::JavaScript)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.ts")),
            Some(CloneLanguage::TypeScript)
        );
        assert_eq!(
            CloneLanguage::from_path(Path::new("a.tsx")),
            Some(CloneLanguage::Tsx)
        );
        assert_eq!(CloneLanguage::from_path(Path::new("a.txt")), None);
    }

    #[test]
    fn tsx_grammar_parses_jsx() {
        // Regression: previously `.tsx` mapped to LANGUAGE_TYPESCRIPT which
        // can't parse JSX tags. Real-world TSX components like
        // `const X = () => <div>{n}</div>` would yield an ERROR node and
        // skip clone detection silently.
        let lang = CloneLanguage::Tsx.language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("set TSX language");
        let src = "export const View = (n: number) => <div className=\"x\">{n}</div>;";
        let tree = parser.parse(src, None).expect("parse TSX");
        assert!(
            !tree.root_node().has_error(),
            "TSX grammar must accept JSX tags; got: {}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn jsx_grammar_parses_jsx() {
        // `.jsx` files route through the JavaScript variant. The plain
        // tree-sitter-javascript grammar already accepts JSX without any
        // additional grammar selection.
        let lang = CloneLanguage::JavaScript.language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("set JS language");
        let src = "export const View = ({n}) => <div className=\"x\">{n}</div>;";
        let tree = parser.parse(src, None).expect("parse JSX");
        assert!(
            !tree.root_node().has_error(),
            "JavaScript grammar must accept JSX; got: {}",
            tree.root_node().to_sexp()
        );
    }
}
