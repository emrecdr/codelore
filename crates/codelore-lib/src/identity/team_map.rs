//! Team-map projection: optional `author,team` CSV that aliases author
//! identities to logical teams at ingest time. Mirrors code-maat's
//! `-p / --team-map-file` flag. Applied AFTER mailmap normalization and
//! AFTER bot filtering — the input is the already-canonical author
//! email; the output is either the matched team name or the email
//! unchanged.
//!
//! Format: a simple two-column CSV with a required header row.
//!
//! ```text
//! author,team
//! alice@example.com,Backend
//! bob@example.com,Frontend
//! ```
//!
//! No quoting support today — the parser splits on the first comma per
//! line. Author identities with commas in them are not supported (none
//! exist in any real-world repo we know of). This matches code-maat's
//! own implementation, which uses a similarly naive parser.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{CodeLoreError, Result};

/// In-memory team-map: maps the canonical author email to the team name.
pub type TeamMap = HashMap<String, String>;

/// Load a team-map from disk. Returns an empty map if `path` is `None`;
/// callers can then call [`apply`] unconditionally without branching.
///
/// # Errors
///
/// Returns [`CodeLoreError::MalformedTeamMap`] for parse problems and
/// [`CodeLoreError::Io`] for I/O failures.
pub fn load(path: Option<&Path>) -> Result<TeamMap> {
    let Some(path) = path else {
        return Ok(TeamMap::new());
    };
    let raw = fs::read_to_string(path).map_err(|e| {
        CodeLoreError::Io(std::io::Error::new(
            e.kind(),
            format!("read team-map {}: {e}", path.display()),
        ))
    })?;
    parse(&raw, path)
}

fn malformed(path: &Path, line: usize, reason: impl Into<String>) -> CodeLoreError {
    CodeLoreError::MalformedTeamMap {
        path: PathBuf::from(path),
        line,
        reason: reason.into(),
    }
}

/// Parse team-map CSV text. Separated from [`load`] for round-trip tests.
fn parse(raw: &str, source: &Path) -> Result<TeamMap> {
    let mut lines = raw.lines().enumerate();
    let (_, header_line) = lines
        .next()
        .ok_or_else(|| malformed(source, 0, "file is empty (expected `author,team` header)"))?;
    let header = header_line.trim().to_lowercase();
    if header != "author,team" {
        return Err(malformed(
            source,
            1,
            format!("malformed header — got {header_line:?}, expected `author,team`"),
        ));
    }

    let mut map = TeamMap::new();
    for (idx, line) in lines {
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let (author, team) = line.split_once(',').ok_or_else(|| {
            malformed(
                source,
                idx + 1,
                format!("missing `,` separator — got {line:?}"),
            )
        })?;
        let author = author.trim().to_string();
        let team = team.trim().to_string();
        if author.is_empty() || team.is_empty() {
            return Err(malformed(
                source,
                idx + 1,
                format!("blank author or team in {line:?}"),
            ));
        }
        if let Some(existing) = map.insert(author.clone(), team.clone()) {
            return Err(malformed(
                source,
                idx + 1,
                format!("duplicate author {author:?} (was {existing:?}, now {team:?})"),
            ));
        }
    }
    Ok(map)
}

/// Apply the team-map to a single author. Unmatched authors pass through
/// unchanged (matches code-maat's `(get team-lookup author author)` fallback).
#[must_use]
pub fn apply<'a>(map: &'a TeamMap, author: &'a str) -> &'a str {
    map.get(author).map_or(author, String::as_str)
}

/// Auto-discovery: if `--team-map-file` isn't passed, look for
/// `<repo>/.codelore-teams`. Returns `None` if the file doesn't exist.
/// Mirrors the `.codelorebots` discovery pattern.
#[must_use]
pub fn discover(repo_root: &Path) -> Option<std::path::PathBuf> {
    let candidate = repo_root.join(".codelore-teams");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(raw: &str) -> Result<TeamMap> {
        parse(raw, Path::new("test"))
    }

    #[test]
    fn loads_a_minimal_team_map() {
        let raw = "author,team\nalice@example.com,Backend\nbob@example.com,Frontend\n";
        let map = parse_str(raw).expect("parse");
        assert_eq!(map.get("alice@example.com").unwrap(), "Backend");
        assert_eq!(map.get("bob@example.com").unwrap(), "Frontend");
    }

    #[test]
    fn applies_with_passthrough_for_unmatched() {
        let mut map = TeamMap::new();
        map.insert("alice@example.com".to_string(), "Backend".to_string());
        assert_eq!(apply(&map, "alice@example.com"), "Backend");
        assert_eq!(apply(&map, "carol@example.com"), "carol@example.com");
    }

    #[test]
    fn rejects_missing_header() {
        let raw = "alice@example.com,Backend\n";
        let err = parse_str(raw).expect_err("must fail");
        let s = format!("{err}");
        assert!(s.contains("malformed header") || s.contains("header"), "{s}");
        // Typed-variant invariant: this is a MalformedTeamMap, not a Provenance bag.
        assert!(matches!(err, CodeLoreError::MalformedTeamMap { .. }));
    }

    #[test]
    fn rejects_duplicate_author() {
        let raw = "author,team\nalice@example.com,Backend\nalice@example.com,Frontend\n";
        let err = parse_str(raw).expect_err("must fail");
        assert!(format!("{err}").contains("duplicate"), "{err}");
        if let CodeLoreError::MalformedTeamMap { line, .. } = err {
            assert_eq!(line, 3, "duplicate is on line 3 of the input (1-indexed)");
        } else {
            panic!("expected MalformedTeamMap, got {err:?}");
        }
    }

    #[test]
    fn rejects_blank_field() {
        let raw = "author,team\n,Backend\n";
        let err = parse_str(raw).expect_err("must fail");
        assert!(format!("{err}").contains("blank"), "{err}");
    }

    #[test]
    fn skips_blank_data_lines() {
        let raw = "author,team\n\nalice@example.com,Backend\n\n";
        let map = parse_str(raw).expect("parse");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let raw = "author,team\r\nalice@example.com,Backend\r\n";
        let map = parse_str(raw).expect("parse");
        assert_eq!(map.get("alice@example.com").unwrap(), "Backend");
    }
}
