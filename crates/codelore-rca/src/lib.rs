// Vendored MPL-2.0 fork of Mozilla's rust-code-analysis. Don't refactor
// upstream code to satisfy newer clippy lints — keep the divergence from
// upstream minimal. Crate-level allows below cover lints introduced in
// Rust toolchain bumps that the upstream code happens to trigger.
#![allow(clippy::collapsible_match)]

//! `codelore-rca` analyzes source code and extracts complexity metrics and
//! structural information. It is a maintained fork of Mozilla's
//! <a href="https://github.com/mozilla/rust-code-analysis/" target="_blank">rust-code-analysis</a>,
//! reduced to the languages the CodeLore product dispatches. `UPSTREAM.md`
//! records the vendoring baseline and every divergence from upstream.
//!
//! Source, issues and feature requests for this fork belong on
//! <a href="https://github.com/emrecdr/codelore" target="_blank">its own GitHub repository</a>,
//! not upstream's.
//!
//! ## Supported Languages
//!
//! - Java
//! - JavaScript
//! - Python
//! - Rust
//! - TypeScript, including TSX
//!
//! Upstream additionally lists C++, C#, CSS, Go, HTML and a Firefox-internal
//! JavaScript dialect. None of those applies here: C# / CSS / Go / HTML were
//! never vendored into this fork, and the C++ and Mozilla-JavaScript grammars
//! were removed once it was established the product could not reach them.
//!
//! ## Supported Metrics
//!
//! - CC: it calculates the code complexity examining the
//!   control flow of a program.
//! - SLOC: it counts the number of lines in a source file.
//! - PLOC: it counts the number of physical lines (instructions)
//!   contained in a source file.
//! - LLOC: it counts the number of logical lines (statements)
//!   contained in a source file.
//! - CLOC: it counts the number of comments in a source file.
//! - BLANK: it counts the number of blank lines in a source file.
//! - HALSTEAD: it is a suite that provides a series of information,
//!   such as the effort required to maintain the analyzed code,
//!   the size in bits to store the program, the difficulty to understand
//!   the code, an estimate of the number of bugs present in the codebase,
//!   and an estimate of the time needed to implement the software.
//! - MI: it is a suite that allows to evaluate the maintainability
//!   of a software.
//! - NOM: it counts the number of functions and closures
//!   in a file/trait/class.
//! - NEXITS: it counts the number of possible exit points
//!   from a method/function.
//! - NARGS: it counts the number of arguments of a function/method.

#![allow(clippy::upper_case_acronyms)]

mod getter;
mod macros;

mod alterator;
pub use alterator::*;

mod node;
pub use crate::node::*;

mod metrics;
pub use metrics::*;

mod languages;
pub(crate) use languages::*;

mod checker;
pub(crate) use checker::*;

mod output;
pub use output::*;

mod spaces;
pub use crate::spaces::*;

mod ops;
pub use crate::ops::*;

mod find;
pub use crate::find::*;

mod function;
pub use crate::function::*;

mod ast;
pub use crate::ast::*;

mod count;
pub use crate::count::*;

mod langs;
pub use crate::langs::*;

mod tools;
pub use crate::tools::*;

mod concurrent_files;
pub use crate::concurrent_files::*;

mod traits;
pub use crate::traits::*;

mod parser;
pub use crate::parser::*;

mod comment_rm;
pub use crate::comment_rm::*;
