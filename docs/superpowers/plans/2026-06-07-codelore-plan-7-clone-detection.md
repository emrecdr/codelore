# Plan 7 — Clone Detection (T1+T2 AST hashing + T3 MinHash + co-change intersection)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add code clone detection as a v1 feature. Detect Type 1 (exact) + Type 2 (renamed/parameterized) clones via AST structural hashing on tree-sitter `FuncSpace`s; optionally Type 3 (near-miss) via MinHash/LSH similarity. Cross-product with the existing change-coupling analysis to surface the differentiating signal: **clones that ALSO change together** ("live clones") — the CodeScene X-Ray approach without their opaque ML.

**Architecture:** Reuse the existing `codelore-rca` `FuncSpace` traversal that Plan 3 wired in for complexity extraction. For each function: emit a structural fingerprint (pre-order sequence of `(node_kind, arity)` tuples, identifiers + literals normalized away). Hash → `HashMap<u64, Vec<FuncId>>` for exact T1+T2. Optional Plan 7 stretch: shingle the fingerprint into k-grams, MinHash + LSH for T3 with Jaccard ≥ 0.8. New `clones` table in the DuckDB schema. New `clones` and `clone-coupling` analyses with CSV/JSON/Markdown emitters; SARIF rules `CODELORE-CLONE` + `CODELORE-LIVE-CLONE`.

**Tech Stack (deltas over Plans 1–6):**
- `sha2` already in tree (used for SARIF fingerprints) — same hash for structural fingerprints
- `smallvec` for arity tuples (most AST nodes have <8 children) — optional perf tweak; defer unless benches require
- Hand-rolled MinHash (one struct, ~80 LOC) — no new crate needed
- All other infrastructure (tree-sitter, FuncSpace, FactsDb schema, output emitters) is already in place from Plans 1–6

---

## §0 — Cold-start audit

```bash
PATH="$HOME/.rustup/toolchains/1.89.0-aarch64-apple-darwin/bin:$PATH" RUSTUP_HOME="$HOME/.rustup" cargo test --workspace --all-features 2>&1 | tail -5
git log --oneline -10
```

Expected baseline: 312 tests, 1 ignored (RCA upstream), latest commit on `main` per Plan 6 close-out.

---

## §1 — Fingerprint extraction (Phase 7.A)

### Task 1: AST structural fingerprint extractor

**Files:**
- Create: `crates/codelore-lib/src/clones/mod.rs`
- Create: `crates/codelore-lib/src/clones/fingerprint.rs`
- Modify: `crates/codelore-lib/src/lib.rs` — `pub mod clones;`

The fingerprint extracts a function's structural shape while erasing identifiers and literals. This is what makes it Type 2-aware: `fn add(a: i32, b: i32) -> i32 { a + b }` and `fn mul(x: u64, y: u64) -> u64 { x + y }` produce the same fingerprint because the `(node_kind, arity)` pre-order traversal is identical once names + literal values are normalized.

```rust
// crates/codelore-lib/src/clones/fingerprint.rs

use codelore_rca::*;   // FuncSpace, Node, traits — exact API per the existing complexity module
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// The 256-bit structural hash. Stored as bytes; rendered as hex for CSV/JSON.
    pub digest: [u8; 32],
    /// Pre-order (node_kind, arity) sequence — kept so MinHash (Task 3) can shingle it.
    pub sequence: Vec<(u16, u16)>,
    /// Number of AST nodes — gates the min-fragment-size filter.
    pub node_count: u32,
}

pub fn fingerprint_function(func_space: &FuncSpace, source: &[u8]) -> Fingerprint {
    let mut seq: Vec<(u16, u16)> = Vec::new();
    walk_preorder(func_space.root_node(), source, &mut seq);
    let mut hasher = Sha256::new();
    for (kind, arity) in &seq {
        hasher.update(kind.to_le_bytes());
        hasher.update(arity.to_le_bytes());
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hasher.finalize());
    Fingerprint {
        digest,
        node_count: seq.len() as u32,
        sequence: seq,
    }
}

fn walk_preorder(node: Node, source: &[u8], out: &mut Vec<(u16, u16)>) {
    let kind_id: u16 = node.kind_id();    // tree-sitter exposes a numeric kind id per language
    let arity = node.child_count() as u16;
    // Skip identifier and literal node kinds — these are the Type 2 normalization.
    if !is_identifier_or_literal(kind_id) {
        out.push((kind_id, arity));
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk_preorder(cursor.node(), source, out);
            if !cursor.goto_next_sibling() { break; }
        }
    }
}

fn is_identifier_or_literal(kind_id: u16) -> bool {
    // tree-sitter's kind_id is language-specific; the actual implementation
    // looks up the kind *string* via Node::kind() once per node (cheap, interned)
    // and matches against a per-language skip set. Plan 7 ships skip sets for
    // Tier-1 languages: identifier, type_identifier, integer_literal,
    // string_literal, char_literal, float_literal, boolean_literal.
    // The exact set lives in clones::language.
    crate::clones::language::is_skipped(kind_id)
}
```

The skip set lives in a sibling `crates/codelore-lib/src/clones/language.rs` that mirrors the Tier-1 language registry from `crates/codelore-lib/src/complexity/language.rs` (the parser-dispatch pattern is already in place).

- [ ] **Step 1: Read existing complexity/language.rs to mirror the dispatch pattern**

```bash
cat crates/codelore-lib/src/complexity/language.rs
cat crates/codelore-lib/src/complexity/mod.rs | head -30
```

- [ ] **Step 2: Write fingerprint module + skip-set helper**

(code per template above)

- [ ] **Step 3: Write unit tests**

```rust
// crates/codelore-lib/src/clones/fingerprint.rs (tests at bottom)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_functions_share_fingerprint() {
        let f1 = parse_rust("fn add(a: i32, b: i32) -> i32 { a + b }");
        let f2 = parse_rust("fn mul(x: u64, y: u64) -> u64 { x + y }");
        // Same structure, different names + types → same fingerprint (Type 2).
        assert_eq!(fingerprint_function(&f1, src1.as_bytes()).digest,
                   fingerprint_function(&f2, src2.as_bytes()).digest);
    }

    #[test]
    fn structurally_different_functions_diverge() {
        let f1 = parse_rust("fn id(x: i32) -> i32 { x }");
        let f2 = parse_rust("fn id(x: i32) -> i32 { x + 1 }");
        // Different statement shape → different fingerprint.
        assert_ne!(fingerprint_function(&f1, src1).digest,
                   fingerprint_function(&f2, src2).digest);
    }
}
```

- [ ] **Step 4: Run, commit**

Commit: `feat(lib): AST structural fingerprint extractor (Type 1 + Type 2 clone basis)`.

---

### Task 2: Clones table in FactsDb schema

**Files:**
- Modify: `crates/codelore-lib/src/facts/schema_v1.sql`
- Modify: `crates/codelore-lib/src/facts/schema.rs`

Add the `clones` table:

```sql
CREATE TABLE IF NOT EXISTS clones (
    clone_group_id  INTEGER NOT NULL,    -- groups identical fingerprints
    fingerprint     BLOB NOT NULL,       -- 32-byte SHA-256 of the AST shape
    rev             VARCHAR NOT NULL,    -- HEAD SHA at the time of analysis
    path            VARCHAR NOT NULL,
    function        VARCHAR NOT NULL,    -- qualified function name from FuncSpace
    start_line      INTEGER NOT NULL,
    end_line        INTEGER NOT NULL,
    node_count      INTEGER NOT NULL,    -- structural size (filter knob)
    similarity      DOUBLE NOT NULL,     -- 1.0 for exact (T1/T2); < 1.0 for T3
    PRIMARY KEY (clone_group_id, path, function, start_line)
);
CREATE INDEX IF NOT EXISTS idx_clones_group ON clones(clone_group_id);
CREATE INDEX IF NOT EXISTS idx_clones_fp ON clones(fingerprint);
```

- [ ] **Step 1: Update schema_v1.sql + schema.rs**
- [ ] **Step 2: Add roundtrip test in tests/facts_test.rs**
- [ ] **Step 3: Commit**

Commit: `feat(lib): add clones table to FactsDb schema`.

---

## §2 — Clone grouping (Phase 7.B)

### Task 3: Exact clone grouper (T1+T2)

**Files:**
- Create: `crates/codelore-lib/src/clones/grouper.rs`
- Create: `crates/codelore-lib/tests/clones_grouper_test.rs`

Walk the working tree at HEAD, fingerprint every function via Task 1, group by digest into clone families. Insert into the `clones` table via DuckDB Appender. Filter out fragments below `min_node_count` (default 30 — about 5-8 statements; keeps trivial getters/setters out).

```rust
pub fn extract_clones_at_head(
    db: &FactsDb,
    repo: &impl Repo,
    opts: &Options,
) -> Result<usize> {
    let head_sha = /* via gix repo */;
    // For each Tier-1 file at HEAD:
    //   - read blob
    //   - parse via codelore-rca to get FuncSpace tree
    //   - for each function FuncSpace: compute fingerprint
    //   - skip if fingerprint.node_count < opts.min_clone_node_count
    //   - collect (path, function, fingerprint, start_line, end_line)
    //
    // Group by fingerprint.digest → families of size ≥ 2 are clones.
    // Assign monotonic clone_group_id per family. Insert into clones table.
    // Return number of clone-rows inserted.
}
```

`opts` gains `min_clone_node_count: u32` (default 30) and `clones_enabled: bool` (default true).

- [ ] **Step 1: Wire grouper into FactsDb::ingest as a final pass**
- [ ] **Step 2: Write tests using a fixture with 3 deliberately cloned functions**
- [ ] **Step 3: Commit**

Commit: `feat(lib): exact clone grouper (T1+T2) via fingerprint hash`.

---

### Task 4 (optional, defer to v1.x if scope blows): MinHash for Type 3 near-miss

**Files:**
- Create: `crates/codelore-lib/src/clones/minhash.rs`
- Modify: `crates/codelore-lib/src/clones/grouper.rs`

Shingle the fingerprint sequence into k-grams (k=4 by default), compute MinHash signature (128 permutations), bucket via LSH (32 bands × 4 rows). Pairs in the same LSH bucket get Jaccard similarity computed; pairs with Jaccard ≥ `opts.clone_similarity_threshold` (default 0.8) become T3 near-miss clones with `similarity = jaccard`.

This adds ~100 LOC + 1 dep (none, we hand-roll MinHash on `u64::wrapping_mul` LCG hashes).

Decision point: ship T3 in Plan 7 only if the T1+T2 implementation lands in <2 task-days. Otherwise defer to Plan 8 (or v1.x post-tag) so the v1 release isn't blocked.

Commit (if shipped): `feat(lib): MinHash + LSH for Type 3 near-miss clones`.

---

## §3 — Clones × Coupling intersection (Phase 7.C) — the differentiator

### Task 5: `clone-coupling` analysis

**Files:**
- Create: `crates/codelore-lib/src/analyses/clone_coupling.rs`
- Modify: `crates/codelore-lib/src/analyses/mod.rs`
- Create: `crates/codelore-lib/tests/clone_coupling_test.rs`

A clone group is "live" if its members **co-change at Fisher-significant rates** (`coupling.p_value < opts.fisher_significance`). Dead clones (low or zero co-change) are noise — code that happens to look alike but evolves independently. This is the structural signal the spec calls out as the methodologically-honest counterpart to CodeScene's X-Ray.

```rust
pub struct CloneCouplingRow {
    pub clone_group_id: i64,
    pub entity_a: String,
    pub entity_b: String,
    pub similarity: f64,        // from clones table
    pub degree_pct: f64,        // from coupling analysis
    pub p_value: f64,           // from coupling Fisher test
    pub shared_revs: u32,
    pub combined_score: f64,    // similarity × (1 − p_value) for ranking
}

pub fn run_clone_coupling(db: &FactsDb, opts: &Options) -> Result<Vec<CloneCouplingRow>>
```

SQL: JOIN `clones` (file-pairs in the same clone_group) against the coupling analysis (file-pairs with Fisher p < threshold). Sort by `combined_score` desc.

- [ ] **Step 1: Write SQL + Rust orchestrator**
- [ ] **Step 2: Test on a fixture with 2 clone families: one co-changes, one doesn't. Assert only the co-changing one appears.**
- [ ] **Step 3: Commit**

Commit: `feat(lib): clone-coupling analysis (live clones — the CodeScene X-Ray pattern)`.

---

## §4 — CLI + outputs (Phase 7.D)

### Task 6: CLI dispatch for `--analysis clones` and `--analysis clone-coupling`

**Files:**
- Modify: `crates/codelore-lib/src/analysis.rs` (add Clones + CloneCoupling enum variants)
- Modify: `crates/codelore-cli/src/main.rs` (dispatch arms for 4 formats × 2 analyses)
- Modify: `crates/codelore-lib/src/output/csv.rs` + json.rs + markdown.rs
- Modify: `crates/codelore-lib/src/output/sarif.rs` — add `CODELORE-CLONE` + `CODELORE-LIVE-CLONE` rules
- Modify: `crates/codelore-cli/tests/cli_test.rs` — 2 new smoke tests

Pattern matches the existing 11 analyses × 4 formats. The SARIF rules ship with the live-clone variant carrying a high `security-severity` (because the methodology says dead clones are noise; live clones are debt).

Commit: `feat(cli): wire clones + clone-coupling analyses across 4 output formats`.

---

### Task 7: CHANGELOG + README + spec §1 update

**Files:**
- Modify: `CHANGELOG.md` — Plan 7 entry
- Modify: `README.md` — bump analysis count from 11 → 13, add clones to the list
- Modify: `docs/superpowers/specs/2026-06-06-codelore-design.md` — move "Clone detection × co-change" out of Feature Registry §8 (deferred) into §1 (in scope); add a clones row to §3.2 (analyses)

Commit: `docs: Plan 7 — clone detection + live-clone analysis`.

---

## Plan 7 Definition of Done

- [ ] `Fingerprint::fingerprint_function` produces identical digests for Type 1+Type 2 clone pairs, divergent digests for structurally different functions
- [ ] `clones` table in FactsDb schema, roundtripped
- [ ] `extract_clones_at_head` populates the table on every `FactsDb::ingest`
- [ ] `--analysis clones --format csv` emits clone families (≥2 members)
- [ ] `--analysis clone-coupling --format csv` emits the Fisher-significant intersection
- [ ] SARIF rules `CODELORE-CLONE` + `CODELORE-LIVE-CLONE` registered
- [ ] All previous tests pass + Plan 7 tests pass
- [ ] clippy/fmt/deny clean
- [ ] CHANGELOG + README updated; spec §1 + §3.2 updated
- [ ] Optional: MinHash T3 shipped (or explicitly deferred to v1.x with rationale)

---

*End of Plan 7. After Plan 7: v1 surface = 13 analyses (revisions, hotspots, code-health, code-age, abs-churn, author-churn, entity-churn, communication, code-ownership, change-coupling, summary, **clones**, **clone-coupling**) × 6 output formats, plus provenance + SARIF + identity resolution + Kamei vector. Code-maat parity baseline locked. Spine v1 ships.*
