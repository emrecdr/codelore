#![allow(clippy::enum_variant_names)]

pub mod language_java;
pub use language_java::*;

// language_mozjs removed: Mozilla-specific tree-sitter-js fork (bca vendor drop)
// language_cpp / language_kotlin removed: grammars the product cannot reach (bca vendor drop)

pub mod language_javascript;
pub use language_javascript::*;

pub mod language_python;
pub use language_python::*;

pub mod language_rust;
pub use language_rust::*;

pub mod language_tsx;
pub use language_tsx::*;

pub mod language_typescript;
pub use language_typescript::*;
