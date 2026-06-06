#![allow(clippy::enum_variant_names)]

pub mod language_ccomment;
pub use language_ccomment::*;

pub mod language_cpp;
pub use language_cpp::*;

pub mod language_java;
pub use language_java::*;

pub mod language_kotlin;
pub use language_kotlin::*;

// language_mozjs removed: Mozilla-specific tree-sitter-js fork (bca vendor drop)

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

pub mod language_preproc;
pub use language_preproc::*;
