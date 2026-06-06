//! Output emitters. Plan 1 ships CSV; SARIF, JSON, Markdown, Parquet,
//! `SQLite`, etc. land in Plan 5.

pub mod csv;
pub mod json;
pub mod markdown;
pub mod parquet;
pub mod sarif;
pub mod sqlite;
