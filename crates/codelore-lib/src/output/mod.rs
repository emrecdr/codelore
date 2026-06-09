//! Output emitters. Plan 1 ships CSV; SARIF, JSON, Markdown, Parquet,
//! `SQLite`, etc. land in Plan 5. The `banner` module renders the
//! stderr pre-flight banner shown at the start of every analyze run.

pub mod banner;
pub mod csv;
pub mod json;
pub mod markdown;
pub mod parquet;
pub mod sarif;
pub mod sqlite;
