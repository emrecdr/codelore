//! Architectural grouping for `--group-file` — code-maat parity (PAR-7).
//!
//! Reads a text file where each non-blank line maps a path-prefix or regex
//! to a logical group name:
//!
//! ```text
//! src/auth                     => Auth
//! ^src\/.*Tests\.cs$           => CS Tests
//! ^src\/((?!.*Test.*).).*$     => Production
//! ```
//!
//! - **Plain text on the LHS** (no leading `^`): rewritten internally to
//!   `^<escape(path)>/` so it matches everything under that prefix without
//!   accidentally matching `src/foobar` when the user wrote `src/foo`.
//! - **Regex on the LHS** (starts with `^`): used as-is. Lookaround supported
//!   via fancy-regex (code-maat's own fixtures rely on it).
//! - **First-match-wins**: rules are checked in file order; the first match
//!   is the entity's group. Order in the file matters.
//!
//! ## Strict vs non-strict
//!
//! - **Strict** (`--strict-grouping`, code-maat default): unmatched entities
//!   are dropped from analysis output silently.
//! - **Non-strict** (`CodeLore` default): unmatched entities keep their raw
//!   path. Less surprise; users opt into silent drop explicitly.
//!
//! Code-maat's behavior is always-strict; we deliberately diverge to the
//! safer default. Under `--code-maat-compat` the strict default is flipped
//! back on.

use std::path::Path;

use fancy_regex::Regex;

/// One mapping rule from the group file.
#[derive(Debug)]
pub struct GroupRule {
    pub pattern: Regex,
    pub name: String,
    /// Original LHS string for error messages. The compiled regex may have
    /// been rewritten (prefix-anchored + trailing-slashed) if the LHS was
    /// plain text.
    pub raw: String,
}

/// Compiled set of grouping rules. Built once per run from `Options.group_file`.
#[derive(Debug, Default)]
pub struct GroupMap {
    pub rules: Vec<GroupRule>,
    pub strict: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum GroupParseError {
    #[error("group file line {line}: missing `=>` separator")]
    MissingSeparator { line: usize },
    #[error("group file line {line}: empty pattern")]
    EmptyPattern { line: usize },
    #[error("group file line {line}: empty group name")]
    EmptyName { line: usize },
    // Boxed because fancy_regex::Error is large (~200 bytes) and would bloat
    // every Result<T, GroupParseError> return on the hot non-error path.
    #[error("group file line {line}: invalid regex {pattern:?}: {source}")]
    InvalidRegex {
        line: usize,
        pattern: String,
        #[source]
        source: Box<fancy_regex::Error>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl GroupMap {
    /// Read `<repo_root>/<file>` (or any absolute path) and parse it.
    /// Strict mode is supplied separately from the `Options.strict_grouping`
    /// field; the file format doesn't carry it.
    ///
    /// # Errors
    ///
    /// Returns [`GroupParseError`] on I/O failure, missing separator, empty
    /// pattern/name, or invalid regex.
    pub fn from_file(path: &Path, strict: bool) -> Result<Self, GroupParseError> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text, strict)
    }

    /// Parse the group-file text format.
    ///
    /// Lines:
    /// - Blank lines are skipped.
    /// - `#`-prefix lines are treated as comments and skipped.
    /// - Other lines must contain `=>` separating LHS from RHS.
    /// - LHS leading `^` → treat as full regex (with lookaround support).
    /// - LHS without `^` → escape as literal, prefix-anchor + trailing slash:
    ///   `src/foo` → `^src/foo/` so it matches `src/foo/bar.rs` but not
    ///   `src/foobar.rs`.
    ///
    /// # Errors
    ///
    /// See [`GroupParseError`].
    pub fn parse(text: &str, strict: bool) -> Result<Self, GroupParseError> {
        let mut rules = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let line_no = i + 1;
            let (lhs, rhs) = trimmed
                .split_once("=>")
                .ok_or(GroupParseError::MissingSeparator { line: line_no })?;
            let path = lhs.trim();
            let name = rhs.trim();
            if path.is_empty() {
                return Err(GroupParseError::EmptyPattern { line: line_no });
            }
            if name.is_empty() {
                return Err(GroupParseError::EmptyName { line: line_no });
            }
            // Code-maat semantics: `^...$` literal regex; otherwise prefix-
            // anchor and slash-bound.
            //
            // CodeLore divergence: code-maat does NOT escape the plain-text
            // path before prefixing. We do (`regex::escape`) — it prevents
            // accidental wildcard interpretation of `.` in literal paths.
            // Documented in advanced-usage as a deliberate safety improvement.
            let pattern_str = if path.starts_with('^') {
                path.to_string()
            } else {
                format!("^{}/", regex::escape(path))
            };
            let pattern = Regex::new(&pattern_str).map_err(|e| GroupParseError::InvalidRegex {
                line: line_no,
                pattern: path.to_string(),
                source: Box::new(e),
            })?;
            rules.push(GroupRule {
                pattern,
                name: name.to_string(),
                raw: path.to_string(),
            });
        }
        Ok(Self { rules, strict })
    }

    /// First-match-wins lookup. Returns the logical group name for the path
    /// or `None` if no rule matches.
    #[must_use]
    pub fn map_entity(&self, path: &str) -> Option<&str> {
        for rule in &self.rules {
            // fancy-regex's `is_match` returns Result because lookaround can
            // backtrack-out (rare); we treat backtrack-failure as a non-match
            // and continue.
            if rule.pattern.is_match(path).unwrap_or(false) {
                return Some(&rule.name);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_plain_text_mapping() {
        let g = GroupMap::parse("src/auth => Auth\n", false).expect("parse");
        assert_eq!(g.rules.len(), 1);
        assert_eq!(g.map_entity("src/auth/login.rs"), Some("Auth"));
        // Prefix-anchor + slash-bound — must NOT match foobar-style false
        // prefixes.
        assert_eq!(g.map_entity("src/authbar/foo.rs"), None);
        assert_eq!(g.map_entity("unrelated/path.rs"), None);
    }

    #[test]
    fn parse_regex_with_anchors() {
        let g = GroupMap::parse("^src\\/.*Tests\\.cs$ => CS Tests\n", false).expect("parse");
        assert_eq!(g.map_entity("src/foo/FooTests.cs"), Some("CS Tests"));
        assert_eq!(g.map_entity("src/foo/Foo.cs"), None);
    }

    #[test]
    fn parse_regex_with_lookaround_via_fancy_regex() {
        // Code-maat's regex-layers-definition.txt uses this exact pattern.
        let g = GroupMap::parse("^src\\/((?!.*Test.*).).*$ => Production\n", false).expect("parse");
        assert_eq!(g.map_entity("src/lib.rs"), Some("Production"));
        assert_eq!(g.map_entity("src/foo/bar.rs"), Some("Production"));
        assert_eq!(
            g.map_entity("src/foo/Test.rs"),
            None,
            "lookaround excludes Test paths"
        );
    }

    #[test]
    fn parse_first_match_wins() {
        let g = GroupMap::parse(
            "src/auth/login => LoginSpecial\n\
             src/auth        => Auth\n",
            false,
        )
        .expect("parse");
        // First rule (more specific) wins for login.rs
        assert_eq!(
            g.map_entity("src/auth/login/handler.rs"),
            Some("LoginSpecial")
        );
        // Second rule (broader) catches everything else under src/auth/
        assert_eq!(g.map_entity("src/auth/session.rs"), Some("Auth"));
    }

    #[test]
    fn parse_skips_blank_and_comment_lines() {
        let g = GroupMap::parse(
            "# comment 1\n\
             \n\
             src/a => A\n\
             \n\
             # comment 2\n\
             src/b => B\n",
            false,
        )
        .expect("parse");
        assert_eq!(g.rules.len(), 2);
        assert_eq!(g.map_entity("src/a/foo.rs"), Some("A"));
        assert_eq!(g.map_entity("src/b/foo.rs"), Some("B"));
    }

    #[test]
    fn parse_errors_on_missing_separator() {
        let err = GroupMap::parse("just a line\n", false).expect_err("must fail");
        match err {
            GroupParseError::MissingSeparator { line } => assert_eq!(line, 1),
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn parse_errors_on_empty_pattern() {
        let err = GroupMap::parse(" => Name\n", false).expect_err("must fail");
        assert!(matches!(err, GroupParseError::EmptyPattern { .. }));
    }

    #[test]
    fn parse_errors_on_empty_name() {
        let err = GroupMap::parse("src/foo => \n", false).expect_err("must fail");
        assert!(matches!(err, GroupParseError::EmptyName { .. }));
    }

    #[test]
    fn from_file_reads_repo_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("groups.txt"), "src/auth => Auth\n").expect("write");
        let g = GroupMap::from_file(&tmp.path().join("groups.txt"), false).expect("read");
        assert_eq!(g.map_entity("src/auth/x.rs"), Some("Auth"));
    }
}
