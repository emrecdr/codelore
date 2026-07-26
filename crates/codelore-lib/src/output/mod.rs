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

/// Map a `serde_json` write failure to a [`crate::CodeLoreError`], preserving
/// an underlying I/O failure's [`std::io::ErrorKind`].
///
/// The serde-based emitters (JSON, NDJSON, SARIF) serialise straight into the
/// output writer, so a failure is either a genuine serialisation fault or the
/// sink erroring mid-write — most importantly `BrokenPipe`, raised when a
/// reader closes the pipe early (`codelore … | head`). Stringifying the error
/// into an `Output` message would erase that kind and hide the broken pipe from
/// the CLI's central quiet-exit arm, so an I/O-category failure is rebuilt as
/// [`crate::CodeLoreError::Io`] carrying the same kind. Both variants share the
/// same exit code, so non-pipe failures are unaffected beyond their message;
/// only `BrokenPipe` gains the quiet-exit treatment. `context` labels genuine
/// serialisation faults (e.g. `"json"`, `"ndjson row"`, `"sarif"`).
pub(crate) fn serde_json_io_err(context: &str, e: &serde_json::Error) -> crate::CodeLoreError {
    match e.io_error_kind() {
        Some(kind) => crate::CodeLoreError::Io(std::io::Error::from(kind)),
        None => crate::CodeLoreError::Output(format!("{context}: {e}")),
    }
}
