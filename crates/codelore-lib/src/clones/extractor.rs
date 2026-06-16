//! Per-file function extraction + clone grouping.
//!
//! Given the source of one Tier-1 file, find every function-shaped subtree,
//! fingerprint each, and return `FunctionFingerprint` records. The grouper
//! (next layer up) consumes these across all files and emits clone families
//! by `fingerprint.digest` equality.

use std::collections::HashSet;
use tree_sitter::{Node, Parser};

use crate::clones::fingerprint::{Fingerprint, walk_preorder_internal};
use crate::clones::language::CloneLanguage;
use crate::{CodeLoreError, Result};

/// One function found in one file at one rev, with its structural fingerprint.
#[derive(Debug, Clone)]
pub struct FunctionFingerprint {
    pub path: String,
    /// Heuristic name extracted from the function's first identifier child.
    /// Empty for closures / arrow functions / anonymous methods.
    pub function_name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub fingerprint: Fingerprint,
}

/// Parse `code` for `lang` and return one `FunctionFingerprint` per function-
/// shaped node. The min-fragment-size filter is applied by the caller against
/// `fingerprint.node_count` so callers can tune it per-run.
pub fn extract_functions(
    path: &str,
    code: &[u8],
    lang: CloneLanguage,
) -> Result<Vec<FunctionFingerprint>> {
    let mut parser = Parser::new();
    parser
        .set_language(&lang.language())
        .map_err(|e| CodeLoreError::Analysis(format!("clones::extract: set_language: {e}")))?;
    let tree = parser
        .parse(code, None)
        .ok_or_else(|| CodeLoreError::Analysis("clones::extract: parse returned None".into()))?;

    let skip: HashSet<&'static str> = lang.skip_kinds().iter().copied().collect();
    let func_kinds: HashSet<&'static str> = lang.function_kinds().iter().copied().collect();

    let mut out: Vec<FunctionFingerprint> = Vec::new();
    visit(tree.root_node(), code, path, &skip, &func_kinds, &mut out);
    Ok(out)
}

fn visit(
    node: Node,
    code: &[u8],
    path: &str,
    skip: &HashSet<&'static str>,
    func_kinds: &HashSet<&'static str>,
    out: &mut Vec<FunctionFingerprint>,
) {
    // Iterative pre-order traversal with a SINGLE outer TreeCursor
    // allocation regardless of subtree size. The previous recursive
    // form allocated one cursor per AST node via `node.walk()`. The
    // inner per-function fingerprint walk (`walk_preorder_internal`)
    // also uses a single cursor each — so total cursors for a file
    // drop from O(AST nodes) to O(functions + 1).
    //
    // Behaviour preserved: every function node is fingerprinted and
    // recursion descends INTO function bodies too, so nested helpers
    // (Python `def` inside `def`, JS closures, Rust `fn outer() {
    // fn helper() {} }`) are emitted alongside their enclosing
    // function's fingerprint.
    let root = node;
    let mut cursor = root.walk();
    loop {
        let current = cursor.node();
        if func_kinds.contains(current.kind()) {
            let mut sequence: Vec<(u16, u16)> = Vec::new();
            walk_preorder_internal(current, skip, &mut sequence);
            let fingerprint = Fingerprint::from_sequence(sequence);
            let function_name = extract_function_name(current, code).unwrap_or_default();
            let start_line = u32::try_from(current.start_position().row + 1).unwrap_or(u32::MAX);
            let end_line = u32::try_from(current.end_position().row + 1).unwrap_or(u32::MAX);
            out.push(FunctionFingerprint {
                path: path.to_string(),
                function_name,
                start_line,
                end_line,
                fingerprint,
            });
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() || cursor.node().id() == root.id() {
                return;
            }
        }
    }
}

/// Extract a heuristic function name: the first child node whose kind is
/// `identifier` or `property_identifier`. Returns `None` for anonymous
/// functions (closures, arrow functions, IIFEs).
fn extract_function_name(node: Node, code: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        let child = cursor.node();
        let kind = child.kind();
        if kind == "identifier" || kind == "property_identifier" {
            let start = child.start_byte();
            let end = child.end_byte();
            if let Ok(s) = std::str::from_utf8(&code[start..end]) {
                return Some(s.to_string());
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    None
}

/// One clone family: a fingerprint shared by ≥ 2 members.
#[derive(Debug, Clone)]
pub struct CloneGroup {
    pub clone_group_id: u32,
    pub members: Vec<FunctionFingerprint>,
}

/// Group function-fingerprints by digest, return families of size ≥ 2.
/// `min_node_count` filters out trivial functions (default 30 ≈ 5-8 statements
/// of structural shape after identifier/literal normalization).
#[must_use]
pub fn group_clones(
    fingerprints: Vec<FunctionFingerprint>,
    min_node_count: u32,
) -> Vec<CloneGroup> {
    // `BTreeMap` (not `HashMap`) so iteration order is digest-sorted.
    // `clone_group_id` is assigned in iteration order via `enumerate()`;
    // `HashMap` iteration depends on std's `RandomState`, which
    // randomises per process. Two back-to-back runs over the same
    // fingerprint set could swap clone_group_id assignments, breaking
    // diff-mode comparisons and any downstream tooling that joins on
    // the ID. Digest-sorted iteration makes IDs deterministic across runs.
    use std::collections::BTreeMap;
    let mut bucket: BTreeMap<[u8; 32], Vec<FunctionFingerprint>> = BTreeMap::new();
    for f in fingerprints {
        if f.fingerprint.node_count < min_node_count {
            continue;
        }
        bucket.entry(f.fingerprint.digest).or_default().push(f);
    }
    let mut groups: Vec<CloneGroup> = bucket
        .into_values()
        .filter(|g| g.len() >= 2)
        .enumerate()
        .map(|(i, members)| CloneGroup {
            clone_group_id: u32::try_from(i + 1).unwrap_or(u32::MAX),
            members,
        })
        .collect();
    // Stable order: by group id then by (path, start_line) within group.
    groups.sort_by_key(|g| g.clone_group_id);
    for g in &mut groups {
        g.members
            .sort_by(|a, b| (&a.path, a.start_line).cmp(&(&b.path, b.start_line)));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_each_top_level_rust_function() {
        let code = "fn a() {}\nfn b() { let x = 1; }\nfn c(x: i32) -> i32 { x + 1 }\n";
        let fns = extract_functions("test.rs", code.as_bytes(), CloneLanguage::Rust).unwrap();
        let names: Vec<_> = fns.iter().map(|f| f.function_name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn groups_type2_clones_across_files() {
        let f1 = extract_functions(
            "a.rs",
            b"fn add(a: i32, b: i32) -> i32 { let x = 1; a + b + x }",
            CloneLanguage::Rust,
        )
        .unwrap();
        let f2 = extract_functions(
            "b.rs",
            b"fn mul(p: u64, q: u64) -> u64 { let z = 2; p + q + z }",
            CloneLanguage::Rust,
        )
        .unwrap();
        let all: Vec<_> = f1.into_iter().chain(f2).collect();
        let groups = group_clones(all, 0); // min=0 to include the tiny fn
        assert_eq!(
            groups.len(),
            1,
            "should find exactly 1 clone family across 2 files"
        );
        assert_eq!(groups[0].members.len(), 2);
        let paths: Vec<_> = groups[0].members.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn min_node_count_drops_trivial_functions() {
        let code = "fn tiny() {}\nfn also_tiny() {}\n";
        let fns = extract_functions("t.rs", code.as_bytes(), CloneLanguage::Rust).unwrap();
        let groups = group_clones(fns, 30);
        assert_eq!(
            groups.len(),
            0,
            "trivial functions should be filtered by min_node_count"
        );
    }

    #[test]
    fn clone_group_id_is_deterministic_across_runs() {
        // Two families with distinguishable shapes; if `group_clones`
        // iterates a `HashMap` keyed on a 32-byte digest, the two
        // family→ID assignments would swap roughly half the time
        // across process restarts because std's `RandomState` is
        // randomised per process. `BTreeMap` iteration is digest-sorted
        // → the same input always produces the same id assignment.
        // We invoke `group_clones` twice in the same process and rely
        // on the fact that BTreeMap iteration order is stable.
        let mk = |c: &str| -> Vec<FunctionFingerprint> {
            extract_functions("x.rs", c.as_bytes(), CloneLanguage::Rust).unwrap()
        };
        let pair_a = mk("fn add(a: i32, b: i32) -> i32 { let x = 1; a + b + x } \
             fn sum(p: i32, q: i32) -> i32 { let z = 2; p + q + z }");
        let pair_b = mk(
            "fn mul(a: i64, b: i64) -> i64 { let r = a * b; let q = r + 1; q } \
             fn prod(p: u64, q: u64) -> u64 { let s = p * q; let t = s + 2; t }",
        );
        let all: Vec<_> = pair_a.into_iter().chain(pair_b).collect();
        let g1 = group_clones(all.clone(), 0);
        let g2 = group_clones(all, 0);
        assert_eq!(g1.len(), 2, "expected 2 clone families");
        assert_eq!(g2.len(), g1.len());
        // ID-to-digest mapping must be identical between runs.
        for (a, b) in g1.iter().zip(g2.iter()) {
            assert_eq!(a.clone_group_id, b.clone_group_id);
            assert_eq!(
                a.members[0].fingerprint.digest,
                b.members[0].fingerprint.digest
            );
        }
    }

    #[test]
    fn extracts_python_methods_inside_class() {
        let code = "class C:\n    def a(self): pass\n    def b(self, x): return x\n";
        let fns = extract_functions("c.py", code.as_bytes(), CloneLanguage::Python).unwrap();
        assert_eq!(fns.len(), 2, "should find 2 methods inside the class");
    }
}
