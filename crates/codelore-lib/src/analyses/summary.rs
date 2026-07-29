//! Summary analysis per spec §1.1 — 4-row repo overview.
//!
//! Research basis: see `docs/research-foundations.md` entry "summary"
//! (diagnostic analysis; no single foundational citation — used for
//! ingest sanity-checking, repo-comparison overviews, and CI dashboard
//! "is the data healthy?" panels).

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SummaryRow {
    pub metric: String,
    pub value: i64,
}

#[tracing::instrument(name = "summary", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_summary(db: &FactsDb, opts: &Options) -> Result<Vec<SummaryRow>> {
    // Under `--code-maat-compat`, emit code-maat's exact statistic names
    // (hyphenated `number-of-X`) so downstream scripts parsing CSV
    // like `if statistic == "number-of-commits"` keep working. The
    // CodeLore modern default uses concise names (`commits`, `entities`)
    // because the row label is already self-explanatory in the modern
    // surface where the column header reads `metric`.
    let sql = if opts.code_maat_compat {
        "
        SELECT 'number-of-commits' AS metric, COUNT(*) AS value FROM commits
        UNION ALL
        SELECT 'number-of-entities', COUNT(DISTINCT path) FROM changes  -- code-maat counts distinct changed paths, not tree-sitter entities
        UNION ALL
        SELECT 'number-of-entities-changed', COUNT(*) FROM changes
        UNION ALL
        SELECT 'number-of-authors', COUNT(DISTINCT canonical_author) FROM commits;
    "
        .to_string()
    } else {
        // Pair-granular: joins on the exact (raw_name, raw_email) that made
        // the commit, so a human sharing a canonical with a bot keeps their
        // own commits counted while the bot pair's are dropped row-wise.
        let human_aliases = crate::analyses::query::HUMAN_ALIASES_CTE;
        format!(
            "
        WITH {human_aliases}
        SELECT 'commits' AS metric, COUNT(*) AS value FROM commits
        UNION ALL
        SELECT 'changes', COUNT(*) FROM changes
        UNION ALL
        SELECT 'entities', COUNT(*) FROM entities
        UNION ALL
        SELECT 'authors', COUNT(DISTINCT co.canonical_author)
        FROM commits co
        JOIN human_aliases ha
            ON ha.raw_name = co.author_name AND ha.raw_email = co.author_email;
    "
        )
    };
    crate::analyses::query::explain_if_requested(db, &sql, [], "summary", opts)?;
    crate::analyses::query::query_map_collect(db, &sql, [], "summary", |r| {
        Ok(SummaryRow {
            metric: r.get::<_, String>(0)?,
            value: r.get::<_, i64>(1)?,
        })
    })
}
