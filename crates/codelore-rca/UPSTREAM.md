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
Upstream reaches the mozcpp grammar from the `tree-sitter-mozcpp/` top-level crate,
via `macros.rs` referencing `tree_sitter_mozcpp::LANGUAGE`. That reference is gone
from this fork — the C++ grammar was excised along with the other three the product
could not reach, so `src/macros.rs` names only the grammars still declared.

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

## Mozjs and dropped-metrics excision (2026-06-06)

**Option B chosen: fully excise Mozjs and dropped metrics.**

The dangling references left by the file drops above were resolved by:

### Mozjs excision
- `src/langs.rs` — removed Mozjs entry from `mk_langs!`; moved js/jsm/mjs/jsx extensions to Javascript
- `src/macros.rs` — restored mozcpp special-case for `get_language!(tree_sitter_cpp)`;
  removed `implement_metric_trait!(Abc, ...)` and `implement_metric_trait!(Wmc, ...)` macro arms
- `src/checker.rs` — removed `impl Checker for MozjsCode` block
- `src/getter.rs` — removed `impl Getter for MozjsCode` block; fixed `Mozjs::Pair` /
  `Mozjs::VariableDeclarator` references in JS/TS/TSX impl blocks to use correct enum namespaces
- `src/alterator.rs` — removed `impl Alterator for MozjsCode` block
- `src/metrics/cognitive.rs` — removed `impl Cognitive for MozjsCode`; updated MozjsParser tests
  to use JavascriptParser. The re-pointed mozjs else-if test read sum 16 instead of 11 not
  because of a grammar difference but because the JavaScript `is_else_if` helper compared the
  wrong parent node kind and never matched, so `else if` branches were over-counted. The
  cognitive-complexity correctness fix (see the "Cognitive-complexity correctness" section)
  restores 11.
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

## Unreachable-grammar excision

**Option B again: fully excise Cpp, Kotlin, Ccomment and Preproc.**

The product dispatches six parsers (`Rust`, `Python`, `Java`, `Javascript`,
`Typescript`, `Tsx`) and resolves only their extensions. The other four
grammars were compiled into every build and reachable from nothing:
`bca-tree-sitter-mozcpp` and `tree-sitter-kotlin-ng` contributed ~46 MB of
generated C, and `bca-tree-sitter-ccomment` / `bca-tree-sitter-preproc` were
the crate's only source of C++ (two `scanner.cc` files, 4,839 B), which is
what required a `musl-g++` cross-toolchain and cost the
`x86_64-unknown-linux-musl` release target.

The four could not be removed independently. `preproc.rs::preprocess` takes a
`&PreprocParser`, and its `PreprocResults` was a parameter of
`ParserTrait::new` consumed only by the `LANG::Cpp` arm of
`get_fake_code` — so dropping the preprocessor while keeping C++ would have
gutted C++ macro expansion, and dropping C++ while keeping the preprocessor
would have left machinery with no consumer.

- `src/langs.rs` — removed the four `mk_langs!` entries and the
  `PreprocResults` import
- `src/macros.rs` — removed the `get_language!(tree_sitter_cpp)` mozcpp
  special-case; dropped the `pr` parameter from `action`, `get_function_spaces`
  and `get_ops`, and the now-unused `path` parameter from `action`
- `src/traits.rs`, `src/parser.rs` — `ParserTrait::new(code)` no longer takes
  `path` or `Option<Arc<PreprocResults>>`; `get_fake_code` deleted
- `src/checker.rs`, `src/getter.rs`, `src/alterator.rs` — removed the
  `CppCode` / `KotlinCode` / `PreprocCode` / `CcommentCode` impls
- `src/metrics/{cognitive,cyclomatic,exit,halstead,loc,mi,nargs,nom}.rs` —
  removed the `CppCode` impls, the dead entries from `implement_metric_trait!`,
  and the C++-only tests
- `src/tools.rs` — `test_guess_language` re-pointed at Python/Rust
- `src/spaces.rs` — deleted `c_scope_resolution_operator` (C++-only)
- `src/comment_rm.rs` — `ccomment_remove_comments` re-pointed at `RustParser`
  as `rust_remove_comments`; `rm_comments` itself is generic and unchanged
- `src/ops.rs` — deleted the C++ operator tests
- Deleted: `src/preproc.rs`, `src/c_macro.rs`, `src/c_langs_macros/`,
  `src/languages/language_{cpp,kotlin,preproc,ccomment}.rs`
- Doc examples across `spaces.rs`, `ops.rs` and `output/dump*.rs` rewritten
  from C++ to Rust

`petgraph` left with the excision: `preproc.rs` was its only consumer in the
workspace, so its `dependabot.yml` ignore rule is retired. What that unblocks
elsewhere in the workspace is recorded beside the dependency it affects.

Removing public API (`LANG` variants, parser types, and the `ParserTrait::new`
signature) is a breaking change for this crate.

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

These five are the whole set. Kotlin, C++ and the two Mozilla helper grammars
(`ccomment`, `preproc`) were removed once it was established that nothing in the
product could reach them; see "Unreachable-grammar excision" above. `language_cpp.rs`
and the `get_language!(tree_sitter_cpp)` macro arm went with them, so no grammar
here is aliased and none is reached through a special case except TypeScript, which
serves both `LANGUAGE_TYPESCRIPT` and `LANGUAGE_TSX` from one crate.

When upgrading grammar versions in future, verify node IDs by running:
```
let root = tree.root_node();
println!("{} id: {}", root.kind(), root.kind_id());
```
and comparing against the enum constants in `src/languages/language_*.rs`.

## Cognitive-complexity correctness

The cognitive-complexity visitors in `src/metrics/cognitive.rs` and their
`is_else_if` helpers in `src/checker.rs` diverge from upstream to correct
miscounts against the Campbell/SonarSource cognitive-complexity specification.
Current behavior:

- **`else if` detection.** `is_else_if` for JavaScript and TSX compares the
  parent node kind against `else_clause`. Upstream compared it against
  `if_statement`, which never matches because tree-sitter wraps the else branch
  in an `else_clause`; the result was that `else if` branches were counted as
  freshly-nested `if`s. Java has no `else_clause` node, so its `is_else_if`
  matches an `if_statement` whose parent `if_statement` holds it as the
  `alternative` field.

- **Missing nesting constructs.** The increase-nesting arm also covers Rust
  `loop`, Java enhanced-`for` and ternary, and Python `match`. `case`/`switch`
  arms add nothing on their own, matching upstream's treatment of `switch`.

- **`finally` is not a branch.** Python `finally` does not increment; only
  `else`, `elif`, and `except` do. `except`/`catch` still counts via its own arm.

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

Year-1 maintenance budget: ~8 days.
