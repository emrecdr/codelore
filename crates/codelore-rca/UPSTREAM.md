# codelore-rca: Vendored Mozilla rust-code-analysis

This crate is a maintained fork of [mozilla/rust-code-analysis](https://github.com/mozilla/rust-code-analysis).

## Upstream baseline

- **Source commit:** `37e5d83c056c8cbf827223d5814a93c5218df1a9`
- **Source date:** `2026-01-20`
- **Vendored on:** 2026-06-06

## Upstream layout notes

The upstream repository is structured as a Cargo workspace:
- `src/` — the main library crate (what we vendor)
- `rust-code-analysis-cli/` — CLI binary (NOT vendored)
- `rust-code-analysis-web/` — actix-web HTTP frontend (NOT vendored; this is the "web" drop)
- `tree-sitter-mozcpp/` — Mozilla tree-sitter-cpp fork (NOT vendored; external dep)
- `tree-sitter-mozjs/` — Mozilla tree-sitter-js fork (NOT vendored; external dep)
- `tree-sitter-ccomment/` / `tree-sitter-preproc/` — custom grammars (NOT vendored; external deps)

Note: `src/languages/language_mozcpp.rs` does NOT exist in this upstream revision.
The mozcpp grammar lives in the `tree-sitter-mozcpp/` top-level crate and is accessed
via `macros.rs` referencing `tree_sitter_mozcpp::LANGUAGE`. That external dep reference
in `src/macros.rs` is kept as-is and will be resolved in Task 5 (Cargo.toml).

## Modifications from upstream

The following were REMOVED from the vendored tree:

- `src/languages/language_mozjs.rs` — Mozilla-specific tree-sitter-js fork (generated code,
  ~3000 lines of `Mozjs` enum variants). The `pub mod language_mozjs` and `pub use language_mozjs::*`
  declarations in `src/languages/mod.rs` were removed.

- `src/metrics/abc.rs` — ABC metric (Assignments/Branches/Conditions). Java-only specialization.
- `src/metrics/wmc.rs` — WMC metric (Weighted Methods per Class). Java-only specialization.
- `src/metrics/npa.rs` — NPA metric (Number of Public Attributes). Java-only specialization.
- `src/metrics/npm.rs` — NPM metric (Number of Public Methods). Java-only specialization.
  The `pub mod abc`, `pub mod wmc`, `pub mod npa`, `pub mod npm` declarations in
  `src/metrics/mod.rs` were removed.

## Task 5 excision (completed 2026-06-06)

**Option B chosen: fully excise Mozjs and dropped metrics.**

The dangling references from Task 4's file drops were resolved by:

### Mozjs excision
- `src/langs.rs` — removed Mozjs entry from `mk_langs!`; moved js/jsm/mjs/jsx extensions to Javascript
- `src/macros.rs` — restored mozcpp special-case for `get_language!(tree_sitter_cpp)`;
  removed `implement_metric_trait!(Abc, ...)` and `implement_metric_trait!(Wmc, ...)` macro arms
- `src/checker.rs` — removed `impl Checker for MozjsCode` block
- `src/getter.rs` — removed `impl Getter for MozjsCode` block; fixed `Mozjs::Pair` /
  `Mozjs::VariableDeclarator` references in JS/TS/TSX impl blocks to use correct enum namespaces
- `src/alterator.rs` — removed `impl Alterator for MozjsCode` block
- `src/metrics/cognitive.rs` — removed `impl Cognitive for MozjsCode`; updated MozjsParser tests
  to use JavascriptParser (snapshot adjusted for JS grammar difference: sum 11→16)
- `src/metrics/cyclomatic.rs` — removed `impl Cyclomatic for MozjsCode`
- `src/metrics/exit.rs` — removed `impl Exit for MozjsCode`
- `src/metrics/halstead.rs` — removed `impl Halstead for MozjsCode`; updated MozjsParser test refs
- `src/metrics/loc.rs` — removed `impl Loc for MozjsCode`; updated MozjsParser test refs
- `src/metrics/mi.rs` — removed MozjsCode from `implement_metric_trait!([Mi], ...)`
- `src/metrics/nargs.rs` — removed MozjsCode from `implement_metric_trait!([NArgs], ...)`
- `src/metrics/nom.rs` — removed MozjsCode from `implement_metric_trait!([Nom], ...)`
- `src/ops.rs` — removed `mozjs_ops` and `mozjs_function_ops` tests

### Abc/Wmc/Npa/Npm excision
- `src/spaces.rs` — removed imports, struct fields, merge/compute calls
- `src/traits.rs` — removed Abc/Wmc/Npa/Npm from ParserTrait associated types
- `src/parser.rs` — removed trait bounds and associated type assignments
- `src/output/dump_metrics.rs` — removed imports and dump_abc/wmc/npm/npa functions

**Files modified: 15 source files + Cargo.toml + UPSTREAM.md**

### Tree-sitter grammar version notes

The `language_*.rs` enum files were generated from specific grammar versions. Using mismatched
grammar versions causes node-ID mismatches and silent metric failures. Pinned versions verified
by checking root node kind_id against enum constants:

| Grammar | Version | Key node |
|---------|---------|----------|
| tree-sitter-javascript | =0.23.1 | program id=133 |
| tree-sitter-typescript | =0.23.1 | program id=166/172 |
| tree-sitter-python | =0.23.6 | module id=108 |
| tree-sitter-java | =0.23.5 | program id=138 |
| tree-sitter-rust | =0.23.2 | source_file id=155 |
| tree-sitter-kotlin-ng | 1.1.0 | source_file id=142 |
| bca-tree-sitter-mozcpp | 1.1.0 | translation_unit id=308 |
| bca-tree-sitter-ccomment | 1.1.0 | (aliased as tree-sitter-ccomment) |
| bca-tree-sitter-preproc | 1.1.0 | (aliased as tree-sitter-preproc) |

C++ uses `bca-tree-sitter-mozcpp` (aliased as `tree-sitter-mozcpp`) because `language_cpp.rs`
was generated from the mozcpp grammar. The `get_language!(tree_sitter_cpp)` macro arm routes
to `tree_sitter_mozcpp::LANGUAGE`.

When upgrading grammar versions in future, verify node IDs by running:
```
let root = tree.root_node();
println!("{} id: {}", root.kind(), root.kind_id());
```
and comparing against the enum constants in `src/languages/language_*.rs`.

## License

The upstream `rust-code-analysis` package declares `license = "MPL-2.0"` in its Cargo.toml.
No LICENSE file was present at the repository root at the vendored commit; the `LICENSE-MPL`
file in this directory contains the canonical MPL-2.0 text.

Original files retain their MPL-2.0 license as declared by upstream.
New files added by bca contributors carry GPL-3.0-only headers.

SPDX (crate-level): `MPL-2.0 AND GPL-3.0-only`

See also: Mozilla's [MPL combining guide](https://www.mozilla.org/en-US/MPL/2.0/combining-mpl-and-gpl/).

## Sync procedure

To pull upstream fixes:
1. Fetch upstream commits since the SHA above.
2. Cherry-pick correctness fixes and grammar bumps (avoid Mozilla-specific features).
3. Update this file with the new SHA + date.
4. Run `cargo test -p codelore-rca` to verify.

Year-1 maintenance budget: ~8 days (see spec §4.1).
