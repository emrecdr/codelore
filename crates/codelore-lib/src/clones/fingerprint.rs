//! Structural AST fingerprinting for clone detection.
//!
//! Walks a tree-sitter parse tree in pre-order, emitting `(node_kind_id,
//! child_count)` pairs while skipping identifier + literal nodes. The
//! resulting byte sequence is SHA-256-hashed to yield a 256-bit
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
    /// 256-bit SHA-256 of the pre-order `(kind_id, child_count)` byte
    /// sequence (identifiers + literals omitted).
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

    /// Build a `Fingerprint` directly from a pre-computed `(kind_id, arity)`
    /// sequence. Used by the function extractor in `clones::extractor` so it
    /// can run the same walk over a subtree (the function body) instead of
    /// the full file.
    #[must_use]
    pub fn from_sequence(sequence: &[(u16, u16)]) -> Self {
        let mut hasher = Sha256::new();
        for (kind, arity) in sequence {
            hasher.update(kind.to_le_bytes());
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

    let mut sequence: Vec<(u16, u16)> = Vec::new();
    let root = tree.root_node();
    walk_preorder_internal(root, &skip, &mut sequence);
    Ok(Fingerprint::from_sequence(&sequence))
}

/// Pre-order walk: emit `(kind_id, child_count)` for nodes whose kind is
/// not in the skip set; always recurse so children of a skipped node still
/// contribute (a literal node is a leaf so this has no effect, but a
/// future skip-set might include non-leaf kinds).
///
/// Exposed at crate visibility so `clones::extractor` can run the same walk
/// over a function-body subtree.
pub(crate) fn walk_preorder_internal(
    node: Node,
    skip: &HashSet<&'static str>,
    out: &mut Vec<(u16, u16)>,
) {
    // Iterative pre-order traversal with a SINGLE TreeCursor
    // allocation, regardless of subtree size. The previous recursive
    // form called `node.walk()` at every node, allocating one cursor
    // per AST node (deep ASTs → tens of thousands of cursor allocs +
    // drops on a hot path). The iterative form descends via
    // `goto_first_child`, traverses siblings via `goto_next_sibling`,
    // and backtracks via `goto_parent` — emitting each node exactly
    // once in pre-order.
    let root = node;
    let mut cursor: TreeCursor<'_> = root.walk();
    loop {
        let current = cursor.node();
        let kind = current.kind();
        let arity = u16::try_from(current.child_count()).unwrap_or(u16::MAX);
        if !skip.contains(kind) {
            out.push((current.kind_id(), arity));
        }
        // Descend if possible.
        if cursor.goto_first_child() {
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
}
