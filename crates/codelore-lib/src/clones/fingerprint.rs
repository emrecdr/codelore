//! Structural AST fingerprinting for clone detection.
//!
//! Walks a tree-sitter parse tree in pre-order, emitting `(kind_name,
//! child_count)` pairs while skipping identifier, literal, and comment
//! nodes. The resulting byte sequence is SHA-256-hashed to yield a 256-bit
//! `Fingerprint::digest` that:
//!   - is identical for Type 1 (exact) clones
//!   - is identical for Type 2 (renamed/parameterized) clones — names and
//!     literals are normalized away
//!   - diverges for structurally different code

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use tree_sitter::{Node, Parser, TreeCursor};

use crate::clones::language::CloneLanguage;
use crate::{CodeLoreError, Result};

/// One function-or-file's structural fingerprint.
#[derive(Debug, Clone)]
pub struct Fingerprint {
    /// 256-bit SHA-256 of the pre-order `(kind_name, child_count)` byte
    /// sequence (identifiers, literals, and comments omitted).
    pub digest: [u8; 32],
    /// Number of AST nodes that contributed to the fingerprint (i.e. the
    /// nodes that survived the identifier/literal skip filter). Use this as
    /// the minimum-fragment-size knob to drop trivial getters/setters.
    pub node_count: u32,
}

impl Fingerprint {
    /// Render the digest as lowercase hex for CSV/JSON output.
    #[must_use]
    pub fn hex(&self) -> String {
        hex::encode(self.digest)
    }

    /// Build a `Fingerprint` directly from a pre-computed `(kind_name, arity)`
    /// sequence. Used by the function extractor in `clones::extractor` so it
    /// can run the same walk over a subtree (the function body) instead of
    /// the full file.
    #[must_use]
    pub fn from_sequence(sequence: &[(&str, u16)]) -> Self {
        let mut hasher = Sha256::new();
        for (kind, arity) in sequence {
            hasher.update(kind.as_bytes());
            // Names are variable-length; a NUL delimiter keeps the byte
            // stream unambiguous so ("ab", _) can't collide with the
            // concatenation of ("a", _) and ("b", _).
            hasher.update(b"\x00");
            hasher.update(arity.to_le_bytes());
        }
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&hasher.finalize());
        let node_count = u32::try_from(sequence.len()).unwrap_or(u32::MAX);
        // Only `digest` and `node_count` are kept; the pre-order sequence
        // is borrowed for hashing + length and then dropped by the caller.
        Self { digest, node_count }
    }
}

/// Compute a structural fingerprint over the full source of `code` for
/// language `lang`. Returns `Err` if tree-sitter fails to load the language
/// (should never happen for the pinned grammars) or to parse (returns an
/// empty-tree fingerprint, not an error, since tree-sitter is permissive).
pub fn fingerprint_source(code: &[u8], lang: CloneLanguage) -> Result<Fingerprint> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.language())
        .map_err(|e| CodeLoreError::Analysis(format!("clone-fingerprint: set_language: {e}")))?;
    let tree = parser
        .parse(code, None)
        .ok_or_else(|| CodeLoreError::Analysis("clone-fingerprint: parse returned None".into()))?;

    let skip: HashSet<&'static str> = lang.skip_kinds().iter().copied().collect();
    let comment: HashSet<&'static str> = lang.comment_kinds().iter().copied().collect();

    let mut sequence: Vec<(&str, u16)> = Vec::new();
    let root = tree.root_node();
    walk_preorder_internal(root, &skip, &comment, &mut sequence);
    Ok(Fingerprint::from_sequence(&sequence))
}

/// Pre-order walk: emit `(kind_name, arity)` for nodes whose kind is not in
/// the skip set. Recurse into skipped identifier/literal nodes so any real
/// structure they wrap still contributes, but PRUNE comment subtrees
/// entirely — a comment emits nothing, is excluded from its parent's arity
/// (see [`effective_arity`]), and is not descended into, so its anonymous
/// marker tokens (`//`, the `///` doc markers, `/* */` delimiters) never
/// leak into the digest. Identifier/literal children still count toward
/// arity, preserving shape-sensitivity — only comments are made invisible.
///
/// Exposed at crate visibility so `clones::extractor` can run the same walk
/// over a function-body subtree.
pub(crate) fn walk_preorder_internal(
    node: Node,
    skip: &HashSet<&'static str>,
    comment: &HashSet<&'static str>,
    out: &mut Vec<(&'static str, u16)>,
) {
    // Iterative pre-order traversal with a SINGLE TreeCursor
    // allocation for the walk, regardless of subtree size. The previous
    // recursive form called `node.walk()` at every node, allocating one
    // cursor per AST node (deep ASTs → tens of thousands of cursor allocs +
    // drops on a hot path). The iterative form descends via
    // `goto_first_child`, traverses siblings via `goto_next_sibling`,
    // and backtracks via `goto_parent` — emitting each node exactly
    // once in pre-order. A second cursor is allocated once here and reused
    // (via `reset`, which allocates nothing) to count each emitted node's
    // non-comment children, keeping allocations at O(1) per walk.
    let root = node;
    let mut cursor: TreeCursor<'_> = root.walk();
    let mut child_cursor: TreeCursor<'_> = root.walk();
    loop {
        let current = cursor.node();
        let kind = current.kind();
        // Comments are pruned outright: they emit nothing, are excluded from
        // their parent's arity, and — critically — we do not descend into
        // them, so their anonymous marker tokens stay out of the digest.
        let is_comment = comment.contains(kind);
        if !skip.contains(kind) {
            let arity = effective_arity(current, comment, &mut child_cursor);
            out.push((kind, arity));
        }
        // Descend if possible, but never into a comment subtree.
        if !is_comment && cursor.goto_first_child() {
            continue;
        }
        // Otherwise advance to the next sibling, climbing as needed.
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            // No sibling; bubble up. Stop once we'd ascend past the
            // subtree root the caller asked us to walk.
            if !cursor.goto_parent() || cursor.node().id() == root.id() {
                return;
            }
        }
    }
}

/// Child count of `node` with comment children excluded, so comments are
/// fully transparent to the structural shape: a comment neither emits a
/// node nor inflates its parent's arity. Non-comment children (identifiers,
/// literals, operator tokens) still count, so arity keeps its
/// shape-sensitivity. `cursor` is reused across calls (`reset` allocates
/// nothing); leaf nodes short-circuit without touching it.
fn effective_arity<'tree>(
    node: Node<'tree>,
    comment: &HashSet<&'static str>,
    cursor: &mut TreeCursor<'tree>,
) -> u16 {
    if node.child_count() == 0 {
        return 0;
    }
    let mut count: usize = 0;
    cursor.reset(node);
    if cursor.goto_first_child() {
        loop {
            if !comment.contains(cursor.node().kind()) {
                count += 1;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    u16::try_from(count).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(lang: CloneLanguage, code: &str) -> Fingerprint {
        fingerprint_source(code.as_bytes(), lang).expect("fingerprint")
    }

    #[test]
    fn identical_rust_functions_share_fingerprint() {
        let a = fp(
            CloneLanguage::Rust,
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        );
        let b = fp(
            CloneLanguage::Rust,
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        );
        assert_eq!(a.digest, b.digest);
    }

    #[test]
    fn type2_renamed_rust_functions_share_fingerprint() {
        // Same shape, different names + types + literals — Type 2 clone.
        let a = fp(
            CloneLanguage::Rust,
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        );
        let b = fp(
            CloneLanguage::Rust,
            "fn mul(x: u64, y: u64) -> u64 { x + y }",
        );
        assert_eq!(a.digest, b.digest, "Type 2 clones should share fingerprint");
    }

    #[test]
    fn structurally_different_rust_functions_diverge() {
        let a = fp(CloneLanguage::Rust, "fn id(x: i32) -> i32 { x }");
        let b = fp(CloneLanguage::Rust, "fn id(x: i32) -> i32 { x + 1 }");
        assert_ne!(
            a.digest, b.digest,
            "different shape ⇒ different fingerprint"
        );
    }

    #[test]
    fn identical_python_functions_share_fingerprint() {
        let a = fp(CloneLanguage::Python, "def add(a, b):\n    return a + b\n");
        let b = fp(CloneLanguage::Python, "def mul(x, y):\n    return x + y\n");
        // Different identifiers — should match (Type 2).
        assert_eq!(a.digest, b.digest);
    }

    #[test]
    fn fingerprint_carries_node_count_and_hex_digest() {
        let f = fp(
            CloneLanguage::Rust,
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        );
        assert!(f.node_count > 0, "node_count should be positive");
        assert_eq!(f.hex().len(), 64, "hex digest is 64 chars");
    }

    #[test]
    fn line_comment_does_not_change_fingerprint_rust() {
        // A single in-body line comment must not perturb the structural
        // fingerprint: comments carry no program structure, so a `// TODO`
        // dropped into one of two otherwise-identical bodies would defeat
        // Type 1/Type 2 matching. `node_count` must also be unchanged — the
        // comment node is fully skipped, not merely zero-weighted.
        let plain = fp(CloneLanguage::Rust, "fn f() -> i32 { 1 + 2 }");
        let with_comment = fp(CloneLanguage::Rust, "fn f() -> i32 { // TODO\n1 + 2 }");
        assert_eq!(plain.digest, with_comment.digest);
        assert_eq!(plain.node_count, with_comment.node_count);
    }

    #[test]
    fn doc_comment_children_are_skipped_rust() {
        // `///` doc comments parse as a `line_comment` wrapping
        // `outer_doc_comment_marker` + `doc_comment` children. Because the
        // walk descends into a skipped node's children, skipping only the
        // outer comment kind would leak those markers into the digest.
        // Guard that the whole doc-comment subtree drops out.
        let plain = fp(CloneLanguage::Rust, "fn f() -> i32 { 1 + 2 }");
        let with_doc = fp(CloneLanguage::Rust, "/// docs\nfn f() -> i32 { 1 + 2 }");
        assert_eq!(plain.digest, with_doc.digest);
    }

    #[test]
    fn comment_does_not_change_fingerprint_python() {
        let plain = fp(CloneLanguage::Python, "def f():\n    return 1 + 2\n");
        let with_comment = fp(
            CloneLanguage::Python,
            "def f():\n    # note\n    return 1 + 2\n",
        );
        assert_eq!(plain.digest, with_comment.digest);
    }

    #[test]
    fn identical_function_matches_across_ts_and_tsx() {
        // The same non-JSX source parses to the same node-kind NAMES under
        // both the TypeScript and TSX grammars, but to different numeric
        // kind ids (TSX's extra JSX kinds shift the id table). Hashing kind
        // names — not ids — makes ordinary TypeScript clones comparable
        // across the `.ts`/`.tsx` dialect split.
        let src = "function add(a: number, b: number): number { return a + b; }";
        let ts = fp(CloneLanguage::TypeScript, src);
        let tsx = fp(CloneLanguage::Tsx, src);
        assert_eq!(ts.digest, tsx.digest);
    }

    #[test]
    fn parameterless_function_matches_across_js_and_ts() {
        // TypeScript wraps each parameter in a `required_parameter` node
        // absent from JavaScript, so only parameterless functions share a
        // structure across the two grammars. With kind-name hashing their
        // digests match.
        let src = "function f() { return 1 + 2; }";
        let js = fp(CloneLanguage::JavaScript, src);
        let ts = fp(CloneLanguage::TypeScript, src);
        assert_eq!(js.digest, ts.digest);
    }

    #[test]
    fn jsx_component_does_not_match_plain_ts() {
        // JSX introduces real `jsx_element` nodes with no analogue in plain
        // TypeScript, so a JSX-returning component must NOT collide with a
        // plain expression body. Guards against over-matching once digests
        // become dialect-comparable.
        let jsx = fp(CloneLanguage::Tsx, "const C = () => <div>{x}</div>;");
        let plain = fp(CloneLanguage::TypeScript, "const C = () => y;");
        assert_ne!(jsx.digest, plain.digest);
    }
}
