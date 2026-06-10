//! `authors` analysis — number of distinct authors per file (modernised).
//!
//! ## What this signal tells you
//!
//! For each file in the repository, the analysis emits how many distinct
//! authors have touched it. This is one of the most-cited findings in
//! software-engineering empirical research: **defect density correlates
//! strongly with the number of distinct contributors to a module**
//! (Bird, Nagappan, Murphy, Devanbu, Zeller — "Don't Touch My Code!
//! Examining the Effects of Ownership on Software Quality", FSE 2011).
//!
//! Files touched by many authors are at higher risk of defects than
//! files with a single dominant owner, even controlling for size,
//! complexity, and churn.
//!
//! ## Why this is richer than code-maat's version
//!
//! Code-maat's `-a authors` emits `[entity, n-authors, n-revs]` — three
//! columns. `CodeLore`'s identity layers know more about each contributor:
//! whose commits are `.mailmap`-canonicalised, who's a bot
//! (`.codelorebots` + heuristics), what fraction were AI-assisted vs
//! AI-authored vs human. That intelligence is dead weight if the
//! output schema doesn't surface it. So the modern default columns are:
//!
//! - `entity` — the file path (or canonical-lineage entity under
//!   `--use-canonical-lineage`).
//! - `n_authors` — total distinct canonical authors touching this file.
//! - `n_humans` — distinct authors whose commits to this file were
//!   classified as `human` AND who aren't flagged as bots.
//! - `n_bots` — distinct authors flagged as bots in `author_aliases`
//!   OR whose every commit to this file was classified as
//!   `ai-authored` (the "wholly machine-generated commit" tier).
//! - `n_revs` — distinct commits this file appears in.
//! - `last_author` — most recent canonical author to touch this file.
//! - `last_modified` — calendar date (`YYYY-MM-DD`) of the most recent
//!   commit affecting this file.
//!
//! Under `--code-maat-compat`, only `entity, n_authors, n_revs` are
//! emitted by the CSV writer so existing tooling that parses code-maat's
//! CSV continues to work — see PAR-5 in the deep-analysis report.
//!
//! ## Where the per-author leaderboard went
//!
//! The previous behaviour of this analysis (one row per author, with a
//! global commit count) is now exposed as the separate `top-committers`
//! analysis with additional columns (`LoC` added/deleted, first/last commit
//! dates). That's a different question — "who commits the most overall"
//! — and conflating it with code-maat's `authors` was a silent migration
//! trap (PAR-1 in the deep-analysis report).
//!
//! Research basis: see `docs/research-foundations.md` entry "authors".

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthorsRow {
    pub entity: String,
    pub n_authors: u32,
    pub n_humans: u32,
    pub n_bots: u32,
    pub n_revs: u32,
    pub last_author: String,
    pub last_modified: String,
}

/// Per-entity author breakdown. A two-CTE pipeline derives:
///
/// 1. `per_file_author` — for each `(path, author)`, the commit count
///    on that file, how many of those commits were `ai-authored`, and
///    the latest timestamp.
/// 2. `classified` — joins in `author_aliases.is_bot` and flags an
///    author as "bot for THIS entity" if either the alias is a bot
///    OR every commit by that author on this file was classified
///    as `ai-authored`. The `LEFT JOIN` tolerates authors that don't
///    have an alias row (legacy data or pattern-only classifications).
/// 3. The outer SELECT aggregates back to per-entity, splits authors
///    into human/bot via the classified flag, picks the latest author
///    using a window function, and renders the latest date as `YYYY-MM-DD`.
const SQL: &str = "
    WITH per_file_author AS (
        SELECT
            changes.path,
            commits.canonical_author AS author,
            COUNT(*) AS n_commits,
            SUM(CASE WHEN commits.ai_attribution = 'ai-authored'
                     THEN 1 ELSE 0 END) AS n_ai,
            MAX(commits.date) AS last_at
        FROM changes
        INNER JOIN commits ON commits.rev = changes.rev
        GROUP BY changes.path, commits.canonical_author
    ),
    last_author_per_path AS (
        SELECT
            path,
            author,
            ROW_NUMBER() OVER (
                PARTITION BY path
                ORDER BY last_at DESC, author ASC
            ) AS rn
        FROM per_file_author
    ),
    classified AS (
        SELECT
            pfa.path,
            pfa.author,
            pfa.n_commits,
            pfa.last_at,
            (
                COALESCE(aa.is_bot, FALSE)
                OR pfa.n_ai = pfa.n_commits
            ) AS is_bot_for_entity
        FROM per_file_author pfa
        LEFT JOIN (
            -- F31: dedupe by canonical (raw_email is the PK; canonical
            -- is N:1 for multi-email authors). Without this the join
            -- multiplied per_file_author rows by N, inflating COUNT and
            -- SUM aggregates downstream.
            SELECT canonical, BOOL_OR(is_bot) AS is_bot
            FROM author_aliases
            GROUP BY canonical
        ) aa ON aa.canonical = pfa.author
    )
    SELECT
        cls.path AS entity,
        CAST(COUNT(DISTINCT cls.author) AS UINTEGER) AS n_authors,
        CAST(COUNT(DISTINCT CASE WHEN NOT cls.is_bot_for_entity
                                 THEN cls.author END) AS UINTEGER) AS n_humans,
        CAST(COUNT(DISTINCT CASE WHEN cls.is_bot_for_entity
                                 THEN cls.author END) AS UINTEGER) AS n_bots,
        CAST(SUM(cls.n_commits) AS UINTEGER) AS n_revs,
        ANY_VALUE(la.author) AS last_author,
        CAST(CAST(MAX(cls.last_at) AS DATE) AS TEXT) AS last_modified
    FROM classified cls
    INNER JOIN last_author_per_path la
        ON la.path = cls.path AND la.rn = 1
    GROUP BY cls.path
    HAVING COUNT(DISTINCT cls.author) > 0
       AND SUM(cls.n_commits) >= ?
    ORDER BY n_authors DESC, n_revs DESC, entity ASC
    LIMIT ?
";

pub fn run_authors(db: &FactsDb, opts: &Options) -> Result<Vec<AuthorsRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    super::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "authors",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "authors",
        |r| {
            Ok(AuthorsRow {
                entity: r.get::<_, String>(0)?,
                n_authors: r.get::<_, u32>(1)?,
                n_humans: r.get::<_, u32>(2)?,
                n_bots: r.get::<_, u32>(3)?,
                n_revs: r.get::<_, u32>(4)?,
                last_author: r.get::<_, String>(5)?,
                last_modified: r.get::<_, String>(6)?,
            })
        },
    )
}
