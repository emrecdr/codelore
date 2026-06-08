//! Shared helper for routing analysis SQL through `changes_lineage` when
//! `opts.use_canonical_lineage` is on. Each path-aggregating analysis calls
//! [`materialize_if_needed`] once at the top of its `run_*` function, then
//! wraps its SQL with [`rewrite`] (or uses `source_table` if it builds the
//! SQL via `format!()`).
//!
//! Centralised here so the 12 path-aggregating analyses all share one
//! dispatch — no per-analysis `if opts.use_canonical_lineage` ladders to
//! drift out of sync.

use crate::facts::FactsDb;
use crate::{Options, Result};

/// Returns the table name to use as the change source for `opts`.
///
/// Precedence: `--time-bucket` wins (the 4-way `changes_bucketed_lineage`
/// matrix is a v0.1.2 follow-up); canonical lineage second; raw `changes`
/// otherwise.
#[must_use]
pub fn source_table(opts: &Options) -> &'static str {
    if opts.time_bucket.is_some() {
        "changes_bucketed"
    } else if opts.use_canonical_lineage {
        "changes_lineage"
    } else {
        "changes"
    }
}

/// Materialise `changes_lineage` if the flag is on. Idempotent; safe to call
/// from every analysis.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on materialise failure.
pub fn materialize_if_needed(db: &FactsDb, opts: &Options) -> Result<()> {
    if opts.use_canonical_lineage && opts.time_bucket.is_none() {
        crate::facts::ingest::materialize_changes_lineage(db)?;
    }
    Ok(())
}

/// Unified source-table materialiser that honours BOTH `--time-bucket` AND
/// `--use-canonical-lineage`. Call this once at the top of an analysis;
/// follow up with [`source_table`] for the FROM clause.
///
/// Composition: when both flags are set, the lineage view is materialised
/// first and bucketing happens on top, so rename ancestry survives the
/// temporal collapse.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on materialise failure.
pub fn materialize_source(db: &FactsDb, opts: &Options) -> Result<()> {
    if let Some(bucket) = opts.time_bucket {
        crate::facts::ingest::materialize_changes_bucketed(db, bucket, opts.use_canonical_lineage)?;
    } else if opts.use_canonical_lineage {
        crate::facts::ingest::materialize_changes_lineage(db)?;
    }
    Ok(())
}

/// Substitute every standalone `FROM changes` / `JOIN changes` in `sql` with
/// the lineage-resolved source table when `opts.use_canonical_lineage` is on.
///
/// Handles two SQL conventions:
/// - `FROM changes ... changes.col` (no per-query alias) → adds `AS changes`
///   so qualified column references continue to resolve.
/// - `FROM changes c ... c.col` (existing per-query alias like `c`, `cchg`) →
///   replaces only the table name; the existing alias is preserved.
///
/// Word-boundary anchoring (regex `\b`) prevents touching `changes_bucketed`,
/// `changes_lineage`, or any column named `changes_*`.
///
/// # Panics
///
/// Panics if the embedded `\b(FROM|JOIN)\s+changes\b(\s*)([A-Za-z_]?)` regex
/// fails to compile — unreachable since the pattern is a compile-time
/// literal, validated by the unit tests in this module.
#[must_use]
pub fn rewrite(sql: &str, opts: &Options) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();

    let src = source_table(opts);
    if src == "changes" {
        return sql.to_string();
    }

    // Regex captures the next non-whitespace character after the table name
    // so we can tell a lowercase-letter alias (`c`, `cchg`) from a SQL
    // keyword or newline.
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\b(FROM|JOIN)\s+changes\b(\s*)([A-Za-z_]?)").unwrap()
    });

    re.replace_all(sql, |caps: &regex::Captures<'_>| {
        let kw = &caps[1];
        let ws = &caps[2];
        let next = &caps[3];
        // Heuristic: a single LOWERCASE letter immediately after the
        // table name is the start of a per-query alias (e.g. `c`, `cchg`,
        // `pchg`). An UPPERCASE letter or no letter signals a keyword
        // (`WHERE`, `ON`, `GROUP`, `LIMIT`, `INNER`, etc.) or a newline.
        let needs_alias =
            next.is_empty() || next.chars().next().is_some_and(char::is_uppercase);
        if needs_alias {
            format!("{kw} {src} AS changes{ws}{next}")
        } else {
            format!("{kw} {src}{ws}{next}")
        }
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_with(use_lineage: bool) -> Options {
        Options {
            use_canonical_lineage: use_lineage,
            ..Options::default()
        }
    }

    #[test]
    fn rewrite_adds_alias_when_no_existing_alias() {
        let sql = "SELECT path FROM changes\nGROUP BY path";
        let out = rewrite(sql, &opts_with(true));
        assert!(out.contains("FROM changes_lineage AS changes"));
    }

    #[test]
    fn rewrite_preserves_existing_alias() {
        let sql = "SELECT c.path FROM changes c GROUP BY c.path";
        let out = rewrite(sql, &opts_with(true));
        assert!(
            out.contains("FROM changes_lineage c"),
            "existing alias `c` must survive: {out}"
        );
        assert!(!out.contains("AS changes c"));
    }

    #[test]
    fn rewrite_join_with_qualified_refs() {
        let sql = "SELECT a FROM commits INNER JOIN changes ON changes.rev = commits.rev";
        let out = rewrite(sql, &opts_with(true));
        assert!(out.contains("INNER JOIN changes_lineage AS changes ON"));
        assert!(out.contains("changes.rev = commits.rev"));
    }

    #[test]
    fn rewrite_leaves_changes_bucketed_alone() {
        let sql = "SELECT path FROM changes_bucketed GROUP BY path";
        let out = rewrite(sql, &opts_with(true));
        assert_eq!(out, sql, "must not touch identifiers like changes_bucketed");
    }

    #[test]
    fn rewrite_noop_when_lineage_off() {
        let sql = "SELECT path FROM changes\nGROUP BY path";
        let out = rewrite(sql, &opts_with(false));
        assert_eq!(out, sql);
    }
}
