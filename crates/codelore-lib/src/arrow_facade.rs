//! Single source of truth for Arrow types throughout the workspace.
//!
//! Re-exports the version of arrow-rs that the `duckdb` crate currently
//! depends on. When `duckdb` bumps `arrow`, we bump here in lockstep
//! and the rest of the workspace stays unchanged.
//!
//! **Discipline:** never `use arrow::*` directly anywhere else in the
//! workspace. Always `use codelore_lib::arrow_facade::*` or `use crate::arrow_facade::*`.
//! See spec §2.6.

pub use arrow::array::{
    Array, ArrayBuilder, ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Float64Builder,
    Int32Builder, Int64Builder, LargeBinaryBuilder, StringBuilder, UInt32Builder, UInt64Builder,
};
pub use arrow::buffer::Buffer;
pub use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
pub use arrow::error::ArrowError;
pub use arrow::record_batch::RecordBatch;

/// Version reported by the runtime (for provenance manifests in Plan 5).
pub const ARROW_RUNTIME_VERSION: &str = "58.3.0";
