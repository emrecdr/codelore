//! JSON output emitter for all analyses.
//!
//! Most analyses emit their rows directly via [`write_json`] — a single
//! generic over any `serde::Serialize` slice. The two exceptions are
//! `revisions` (whose `(String, u32)` tuple needs a struct wrapping for
//! named-field JSON output) and `communities` (whose output is a
//! wrapper struct carrying the row array plus partition-level summary
//! statistics, not the rows alone).
//!
//! CLI dispatch calls `write_json::<HotspotRow>(&rows, &mut out)`
//! directly with a turbofished row type — the same pattern the
//! `output::ndjson::write_ndjson` emitter uses. Per-analysis wrapper
//! shims (`write_hotspots_json`, `write_code_health_json`, etc.) were
//! retired in commit b2efe20's successor: 27 shims removed, 33 call
//! sites updated to use the generic.

use crate::{CodeLoreError, Result};
use std::io::Write;

/// Generic JSON emitter. Serialises any `serde::Serialize` slice as a
/// pretty-printed JSON array. The pretty form matches the historic
/// shape from the pre-shim era; downstream consumers that expect
/// stream-parseable single-row-per-line semantics should use
/// `--format ndjson` instead.
///
/// # Errors
///
/// Returns [`CodeLoreError::Output`] on serialisation or I/O failure.
pub fn write_json<W: Write, T: serde::Serialize>(rows: &[T], w: &mut W) -> Result<()> {
    serde_json::to_writer_pretty(w, rows).map_err(|e| CodeLoreError::Output(format!("json: {e}")))
}

/// `revisions` emitter — the tuple `(path, n_revs)` is serialised as a
/// named-field struct so JSON consumers see `{"entity": "...",
/// "n_revs": N}` rather than `["...", N]` (which would be the default
/// tuple serialisation).
///
/// # Errors
///
/// Returns [`CodeLoreError::Output`] on serialisation or I/O failure.
pub fn write_revisions_json<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    #[derive(serde::Serialize)]
    struct R<'a> {
        entity: &'a str,
        n_revs: u32,
    }
    let typed: Vec<R> = rows
        .iter()
        .map(|(p, n)| R {
            entity: p,
            n_revs: *n,
        })
        .collect();
    write_json(&typed, w)
}

/// `communities` emitter — emits the wrapper struct (`rows` +
/// `modularity` + `community_count`), not just the row array — the
/// partition-level summary is the headline metric and JSON consumers
/// expect it alongside the per-file mapping.
///
/// # Errors
///
/// Returns [`CodeLoreError::Output`] on serialisation or I/O failure.
pub fn write_communities_json<W: Write>(
    result: &crate::analyses::communities::CommunitiesResult,
    w: &mut W,
) -> Result<()> {
    serde_json::to_writer_pretty(w, result).map_err(|e| CodeLoreError::Output(format!("json: {e}")))
}
