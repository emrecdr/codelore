//! GitHub Actions workflow-command output emitter.
//!
//! Emits findings as `::warning file=...,line=...,title=...::message`
//! workflow commands directly to stdout. When run inside a GitHub
//! Actions job, the runner intercepts these and surfaces each finding
//! as an inline annotation on the pull-request diff — same surface
//! Code Scanning uses, but without uploading SARIF (no permissions,
//! no API call, no upload step).
//!
//! Spec: <https://docs.github.com/en/actions/reference/workflow-commands-for-github-actions>
//!
//! Three severity levels map to three commands:
//! - HIGH    → `::error::`
//! - MEDIUM  → `::warning::`
//! - LOW     → `::notice::`
//!
//! Mapping is deliberately tight — hotspot scores in `(7, 10]` are
//! error-level (top-decile churn × complexity), `(4, 7]` are warning,
//! everything else is notice. Threshold tuning happens behind the
//! `--workflow-command-floor` flag if user pull demands it.
//!
//! ## Escaping
//!
//! GitHub's parser is brittle around `:`, `,`, `\r`, `\n`, and `%` in
//! property values + messages. The official escape sequences are:
//!
//! | Char | Escape |
//! |------|--------|
//! | `%`  | `%25`  |
//! | `\r` | `%0D`  |
//! | `\n` | `%0A`  |
//! | `:`  | `%3A`  (in property values only)
//! | `,`  | `%2C`  (in property values only)
//!
//! Property values get the full set; the trailing message body only
//! needs `%` / `\r` / `\n` escaped (colons + commas pass through, which
//! is why the bare `message` is the last field).
//!
//! ## Why not gate this on `GITHUB_ACTIONS=true`?
//!
//! Users can pipe a non-GHA shell into `tee` + redirect to a runner —
//! defensively gating would surprise them. The emitter prints workflow
//! commands unconditionally; runners outside Actions see them as plain
//! text lines, which is the expected fallback.

use crate::analyses::hotspots::HotspotRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

/// Default hotspot-score thresholds for severity bucketing.
const ERROR_FLOOR: f64 = 7.0;
const WARNING_FLOOR: f64 = 4.0;

/// Escape a property-value or full message according to the GH workflow-
/// command grammar. `properties = true` engages the wider escape set
/// (`:` and `,`); messages need only `%` / `\r` / `\n`.
fn escape(s: &str, properties: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            ':' if properties => out.push_str("%3A"),
            ',' if properties => out.push_str("%2C"),
            c => out.push(c),
        }
    }
    out
}

/// Emit hotspot rows as GitHub Actions workflow commands.
///
/// Each row → one `::error file=...::message` or `::warning::` /
/// `::notice::` line on stdout, with severity bucketed by the
/// composite `hotspot_score` and the file path as the anchor. Line
/// number is omitted — hotspot is a whole-file signal, not a line
/// signal; the Actions UI files an annotation against the top of the
/// file in this case, which matches the analysis semantics.
///
/// # Errors
///
/// Returns [`CodeLoreError::Io`] on writer failure.
pub fn write_hotspots_gha<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    for row in rows {
        let level = if row.hotspot_score >= ERROR_FLOOR {
            "error"
        } else if row.hotspot_score >= WARNING_FLOOR {
            "warning"
        } else {
            "notice"
        };
        let title = format!("CodeLore hotspot — score {:.2}", row.hotspot_score);
        let message = format!(
            "Hotspot: {} revisions, cognitive {:.0}, code-health {:.1}, score {:.2}",
            row.revisions, row.cognitive, row.code_health, row.hotspot_score
        );
        writeln!(
            w,
            "::{level} file={file},title={title}::{message}",
            level = level,
            file = escape(&row.path, true),
            title = escape(&title, true),
            message = escape(&message, false),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, score: f64) -> HotspotRow {
        HotspotRow {
            path: path.into(),
            revisions: 12,
            cognitive: 25.0,
            code_health: 65.0,
            hotspot_score: score,
            mi: None,
            mi_rank: None,
            ai_pct: None,
        }
    }

    #[test]
    fn high_score_emits_error_level() {
        let mut buf = Vec::new();
        write_hotspots_gha(&[row("src/main.rs", 8.5)], &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.starts_with("::error file=src/main.rs"), "got: {out}");
        assert!(out.contains("score 8.50"));
    }

    #[test]
    fn medium_score_emits_warning_level() {
        let mut buf = Vec::new();
        write_hotspots_gha(&[row("src/lib.rs", 5.0)], &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.starts_with("::warning file=src/lib.rs"));
    }

    #[test]
    fn low_score_emits_notice_level() {
        let mut buf = Vec::new();
        write_hotspots_gha(&[row("src/util.rs", 1.0)], &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.starts_with("::notice file=src/util.rs"));
    }

    #[test]
    fn path_with_special_chars_is_escaped_in_properties() {
        let mut buf = Vec::new();
        write_hotspots_gha(&[row("path/with comma,colon:.rs", 5.0)], &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        // Comma + colon must be percent-encoded inside property values
        // so the runner's parser doesn't split the property list.
        assert!(out.contains("path/with comma%2Ccolon%3A.rs"), "got: {out}");
        // Plain message body keeps the colon (last-field semantics).
    }

    #[test]
    fn newline_in_message_is_escaped() {
        // Forge a row with a \n in the path — defensive, since rows
        // come from git output that's generally clean. The escape must
        // still fire so the runner doesn't see a literal newline mid-
        // command.
        let mut buf = Vec::new();
        let r = row("a\nb.rs", 5.0);
        write_hotspots_gha(&[r], &mut buf).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("a%0Ab.rs"));
        // And the line itself is single-line — exactly one `\n` from
        // writeln!, none from the escaped path.
        assert_eq!(out.matches('\n').count(), 1);
    }

    #[test]
    fn empty_input_emits_no_lines() {
        let mut buf = Vec::new();
        write_hotspots_gha(&[], &mut buf).unwrap();
        assert!(buf.is_empty());
    }
}
