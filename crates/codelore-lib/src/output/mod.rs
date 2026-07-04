//! Output emitters: CSV, SARIF, JSON, Markdown, Parquet,
//! `SQLite`. The `banner` module renders the
//! stderr pre-flight banner shown at the start of every analyze run.

pub mod banner;
pub mod csv;
pub mod gha;
pub mod html;
pub mod json;
pub mod markdown;
pub mod ndjson;
pub mod parquet;
pub mod sarif;
#[cfg(feature = "spa")]
pub mod spa;
pub mod sqlite;
#[cfg(feature = "spa")]
pub mod step_summary;
pub(crate) mod template;
