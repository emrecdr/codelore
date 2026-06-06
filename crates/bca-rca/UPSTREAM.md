# bca-rca: Vendored Mozilla rust-code-analysis

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

## Known residual references (to be resolved in Task 5/6)

Dropping the above files leaves broken `use` statements in several files. These are
catalogued here for Task 5's implementer:

- `src/spaces.rs` — imports `crate::abc`, `crate::npa`, `crate::npm`, `crate::wmc`;
  references `metrics.abc`, `metrics.wmc`, `metrics.npm`, `metrics.npa` fields
- `src/traits.rs` — imports `crate::abc::Abc`, `crate::npa::Npa`, `crate::npm::Npm`, `crate::wmc::Wmc`
- `src/parser.rs` — imports `crate::abc::Abc`, `crate::npa::Npa`, `crate::npm::Npm`, `crate::wmc::Wmc`
- `src/output/dump_metrics.rs` — imports `crate::abc`, `crate::npa`, `crate::npm`, `crate::wmc`;
  calls `dump_abc`, `dump_wmc`, `dump_npm`, `dump_npa`
- `src/macros.rs` — `implement_metric_trait!(Abc, ...)` and `implement_metric_trait!(Wmc, ...)`
  macro arms reference dropped trait types (Abc, Wmc)
- `src/langs.rs` — `Mozjs` / `MozjsCode` / `MozjsParser` / `tree_sitter_mozjs` referenced
  via the `mk_langs!` macro; depends on both `language_mozjs.rs` (dropped) and the
  `tree_sitter-mozjs` external crate
- `src/macros.rs` — `get_language!(tree_sitter_cpp)` arm references `tree_sitter_mozcpp::LANGUAGE`;
  requires the `tree-sitter-mozcpp` external crate in Cargo.toml
- Multiple metric files (`cognitive.rs`, `loc.rs`, `exit.rs`, `halstead.rs`, `cyclomatic.rs`,
  `nargs.rs`, `nom.rs`, `mi.rs`) reference `MozjsCode`/`MozjsParser`/`Mozjs` — these come from
  `language_mozjs.rs` (dropped) via `langs.rs`

Task 5 options for resolution:
  a) Re-add `tree-sitter-mozjs` as external dep + stub out `language_mozjs.rs` with just the enum
  b) Fully excise Mozjs from all metric impls and `langs.rs` (larger surgery, out of scope for Task 4)
  c) Feature-gate Mozjs behind a `mozjs` cargo feature

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
4. Run `cargo test -p bca-rca` to verify.

Year-1 maintenance budget: ~8 days (see spec §4.1).
