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
/// Panics if the embedded regex fails to compile — unreachable since the
/// pattern is a compile-time literal, validated by the unit tests in this
/// module.
#[must_use]
pub fn rewrite(sql: &str, opts: &Options) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();

    let src = source_table(opts);
    if src == "changes" {
        return sql.to_string();
    }

    // Regex is case-INSENSITIVE (`(?i)`) so lowercase SQL (`from changes
    // group by ...`) gets rewritten the same as the uppercase canonical
    // form. The next-word capture `\w*` grabs the entire identifier after
    // `changes` so we can disambiguate "user-supplied alias" from "SQL
    // keyword like WHERE/GROUP/ORDER" by checking against an explicit
    // case-insensitive keyword whitelist — the prior case-based heuristic
    // (lowercase = alias, uppercase = keyword) silently failed on
    // lowercase SQL by treating `group` as an alias and emitting
    // `FROM changes_lineage group BY ...` (parse error).
    let re =
        RE.get_or_init(|| regex::Regex::new(r"(?i)\b(FROM|JOIN)\s+changes\b(\s*)(\w*)").unwrap());

    re.replace_all(sql, |caps: &regex::Captures<'_>| {
        let kw = &caps[1];
        let ws = &caps[2];
        let next = &caps[3];
        // No next word → end of input / punctuation → no alias present →
        // need to add one so qualified `changes.col` references remain
        // valid. Next word is a SQL keyword → no alias present → need
        // to add one. Otherwise the next word IS the user's alias and
        // we leave it alone.
        let needs_alias = next.is_empty() || is_sql_keyword(next);
        if needs_alias {
            format!("{kw} {src} AS changes{ws}{next}")
        } else {
            format!("{kw} {src}{ws}{next}")
        }
    })
    .into_owned()
}

/// SQL keyword whitelist used to distinguish "this token after `changes` is
/// a keyword that ends the FROM clause" from "this token is the user's
/// per-query alias". The set covers SQL-92 / SQL-2008 clauses commonly
/// seen in codelore's analyses plus DuckDB-specific extensions (`QUALIFY`,
/// `SAMPLE`, `USING`, `TABLESAMPLE`). Matching is case-insensitive.
///
/// Conservatism note: erring on the side of `keyword` produces correct SQL
/// — at worst the rewriter adds an `AS changes` alias the user didn't
/// need; that's harmless. Erring on the side of `alias` silently produces
/// broken SQL (the prior bug). So the set is intentionally over-broad:
/// every plausible token following `FROM <table>` in `DuckDB`'s SQL grammar
/// is included.
fn is_sql_keyword(token: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "WHERE",
        "GROUP",
        "HAVING",
        "ORDER",
        "LIMIT",
        "OFFSET",
        "JOIN",
        "INNER",
        "LEFT",
        "RIGHT",
        "FULL",
        "OUTER",
        "CROSS",
        "NATURAL",
        "ON",
        "USING",
        "UNION",
        "INTERSECT",
        "EXCEPT",
        "WINDOW",
        "QUALIFY",
        "FETCH",
        "SAMPLE",
        "TABLESAMPLE",
        "AS",
        "WITH",
        "ANTI",
        "SEMI",
        "ASOF",
    ];
    let upper = token.to_ascii_uppercase();
    KEYWORDS.contains(&upper.as_str())
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

    /// NEW-C regression: prior to v0.1.4 the rewriter used a case-based
    /// alias-vs-keyword discriminator (next char uppercase → keyword,
    /// lowercase → alias). Lowercase SQL therefore parsed `group` as if it
    /// were a user-supplied alias and produced
    /// `FROM changes_lineage group BY path` — a parse error. The
    /// case-insensitive keyword-whitelist replacement handles both
    /// canonical-case and lowercase variants the same way.
    #[test]
    fn rewrite_handles_lowercase_sql_keywords() {
        let sql = "select path from changes\ngroup by path";
        let out = rewrite(sql, &opts_with(true));
        let lower = out.to_lowercase();
        // Correct behaviour: rewriter sees lowercase `group` as a keyword,
        // adds `AS changes` alias, leaves `group by` intact afterwards.
        // Final shape: `from changes_lineage as changes\ngroup by path`.
        assert!(
            lower.contains("from changes_lineage as changes"),
            "lowercase `from changes` must rewrite + add alias: {out}"
        );
        assert!(
            lower.contains("group by path"),
            "`group by` keyword sequence must survive verbatim: {out}"
        );
        // Guard the original NEW-C bug: `from changes_lineage group` would
        // mean the rewriter treated `group` as an alias and DIDN'T add
        // `AS changes` — the resulting SQL would be a DuckDB parse error.
        assert!(
            !lower.contains("from changes_lineage group"),
            "lowercase `group` must NOT be treated as an alias: {out}"
        );
    }

    /// Companion regression: lowercase aliases should still be preserved
    /// even when the SQL is otherwise lowercase (`from changes c group by c.path`).
    #[test]
    fn rewrite_lowercase_alias_preserved_in_lowercase_sql() {
        let sql = "select c.path from changes c group by c.path";
        let out = rewrite(sql, &opts_with(true));
        assert!(
            out.to_lowercase().contains("from changes_lineage c"),
            "lowercase alias `c` must survive: {out}"
        );
        assert!(!out.contains("AS changes c"));
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
