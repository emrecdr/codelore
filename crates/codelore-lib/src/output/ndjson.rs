//! Newline-delimited JSON (NDJSON / JSON Lines) output emitter.
//!
//! Each row is serialized as its own line of compact JSON, terminated by
//! `\n`. Consumers stream-parse line-by-line — LSP integrations, CI log
//! pipelines, and `jq -c` filters can react as analyses complete rather
//! than waiting for the closing `]` of a JSON array.
//!
//! Spec: <https://github.com/ndjson/ndjson-spec>
//!
//! Each line is a complete JSON object on its own; lines are independent.
//! No trailing newline after the final row (also valid NDJSON; many
//! consumers strip it anyway). UTF-8 throughout. Numbers are emitted via
//! `serde_json`'s default `f64`/`i64` encoding — same shape as the
//! batch `json` emitter.
//!
//! Existing `json.rs` writers all expand to `serde_json::to_writer_pretty(w,
//! rows)` over a `&[T: Serialize]` slice. The NDJSON shape iterates the
//! same slice and emits one compact object per row. No new row types —
//! the same `HotspotRow` / `CodeHealthRow` / etc serialize identically;
//! only the framing changes.

use crate::{CodeLoreError, Result};
use std::io::Write;

/// Generic NDJSON writer. Each `row` is emitted via
/// `serde_json::to_writer` (compact form) followed by `\n`.
///
/// # Errors
///
/// Returns [`CodeLoreError::Io`] when the output sink fails mid-write (e.g. a
/// reader closed the pipe early) and [`CodeLoreError::Output`] on a genuine
/// serialisation fault.
pub fn write_ndjson<W: Write, T: serde::Serialize>(rows: &[T], w: &mut W) -> Result<()> {
    for row in rows {
        serde_json::to_writer(&mut *w, row)
            .map_err(|e| super::serde_json_io_err("ndjson row", &e))?;
        w.write_all(b"\n").map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct Row {
        path: String,
        score: f64,
    }

    #[test]
    fn emits_one_object_per_line_no_trailing_array() {
        let rows = vec![
            Row {
                path: "src/a.rs".into(),
                score: 1.5,
            },
            Row {
                path: "src/b.rs".into(),
                score: 2.0,
            },
        ];
        let mut buf = Vec::new();
        write_ndjson(&rows, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        // Two lines, each a self-contained JSON object. No `[` or `]`
        // anywhere — that's the framing contract.
        assert!(
            !out.contains('['),
            "ndjson must not wrap in an array: {out}"
        );
        assert!(!out.contains(']'), "ndjson must not close an array: {out}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with('{') && lines[0].ends_with('}'));
        assert!(lines[1].starts_with('{') && lines[1].ends_with('}'));
        let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(r0["path"], "src/a.rs");
    }

    #[test]
    fn empty_input_emits_no_output() {
        let rows: Vec<Row> = vec![];
        let mut buf = Vec::new();
        write_ndjson(&rows, &mut buf).unwrap();
        assert!(buf.is_empty(), "empty slice emits zero bytes");
    }

    #[test]
    fn writer_uses_compact_form_not_pretty() {
        let rows = vec![Row {
            path: "p".into(),
            score: 1.0,
        }];
        let mut buf = Vec::new();
        write_ndjson(&rows, &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        // Compact form has no spaces after `:` or `,`; pretty form does.
        assert!(out.contains("\"path\":\"p\""));
        assert!(out.contains("\"score\":1"));
    }
}
