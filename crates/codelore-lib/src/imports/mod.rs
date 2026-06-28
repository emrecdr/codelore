//! Architecture import-graph extraction.
//!
//! Walks each Tier-1 source file at HEAD via tree-sitter, captures
//! every import statement as a raw `(src_path, target, kind)` row,
//! and bulk-inserts into the `imports` `DuckDB` table. Mirrors the
//! shape of `crate::clones` (per-language dispatch + iterative
//! cursor walker), so the next behavioural-signal addition can
//! follow the same module template.
//!
//! The extractor leaves every row with `resolved = false` /
//! `target_path = NULL`. The companion per-language resolver maps
//! `target → tracked repo path` and upgrades those columns in
//! place. Downstream analyses (layered-architecture rules,
//! god-class detector, architecture force graph, module chord)
//! all read against this schema.

mod extractor;
mod language;
mod resolver;

pub use extractor::{ImportKind, RawImport, extract_imports};
pub use language::ImportLanguage;
pub use resolver::{
    resolve_by_extension, resolve_js_relative, resolve_python_relative, resolve_rust_path,
};
