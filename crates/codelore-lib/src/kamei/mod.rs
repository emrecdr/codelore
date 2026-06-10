//! Kamei 14-feature JIT-SDP canonical change vector enrichment.
//!
//! See spec §3.1 and Kamei et al. 2013 (TSE).
//!
//! Implemented as a series of SQL UPDATE passes over the commits + changes
//! tables. Run after the main commit/changes ingest + complexity pass.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Result};

/// Enrich all rows in `commits` table with the 14 Kamei features.
/// Idempotent; safe to call multiple times.
///
/// When `use_lineage` is true, history-aware features (ndev, nuc, age, sexp)
/// resolve renamed paths via `changes_lineage` so pre-rename history is
/// merged onto the canonical post-rename name. When false, the join uses
/// raw `changes` (code-maat parity).
pub fn enrich(db: &FactsDb, use_lineage: bool) -> Result<()> {
    let src = if use_lineage {
        crate::facts::ingest::materialize_changes_lineage(db)?;
        "changes_lineage"
    } else {
        "changes"
    };
    enrich_diffusion(db)?;
    enrich_size(db)?;
    enrich_fix(db)?;
    enrich_history(db, src)?;
    enrich_experience(db, src)?;
    Ok(())
}

/// Diffusion: NS, ND, NF, entropy
fn enrich_diffusion(db: &FactsDb) -> Result<()> {
    // NS = distinct top-level dirs touched
    // ND = distinct directory paths touched
    // NF = distinct files touched
    let sql_counts = "
        UPDATE commits SET
          nf = (SELECT COUNT(DISTINCT path) FROM changes WHERE changes.rev = commits.rev),
          ns = (SELECT COUNT(DISTINCT SPLIT_PART(path, '/', 1)) FROM changes WHERE changes.rev = commits.rev),
          nd = (SELECT COUNT(DISTINCT
                    CASE
                        WHEN STRPOS(path, '/') > 0
                        THEN SUBSTR(path, 1, LENGTH(path) - LENGTH(SPLIT_PART(path, '/', -1)) - 1)
                        ELSE ''
                    END
                ) FROM changes WHERE changes.rev = commits.rev);
    ";
    db.conn()
        .execute_batch(sql_counts)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei diffusion counts: {e}")))?;

    // entropy = -Σ p_i log2(p_i) over LOC distribution across files
    // Run as a separate statement so errors are easier to isolate.
    let sql_entropy = "
        UPDATE commits SET
          entropy = COALESCE((
              WITH dist AS (
                  SELECT CAST(loc_added AS DOUBLE) AS x
                  FROM changes
                  WHERE changes.rev = commits.rev AND loc_added > 0
              ),
              total AS (SELECT SUM(x) AS t FROM dist)
              SELECT -SUM((x / NULLIF(total.t, 0)) * LOG2(NULLIF(x / NULLIF(total.t, 0), 0)))
              FROM dist, total
          ), 0.0);
    ";
    db.conn()
        .execute_batch(sql_entropy)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei diffusion entropy: {e}")))?;

    Ok(())
}

/// Size: LA, LD, LT
fn enrich_size(db: &FactsDb) -> Result<()> {
    // LA = total loc added in commit
    // LD = total loc deleted in commit
    // LT = mean total LOC of touched files (pre-change).
    //      Plan 4 approximation: stub to 0 since pre-change LOC requires
    //      reading historical blobs; Plan 5+ may improve.
    let sql = "
        UPDATE commits SET
          la = COALESCE((SELECT SUM(loc_added) FROM changes WHERE changes.rev = commits.rev), 0),
          ld = COALESCE((SELECT SUM(loc_deleted) FROM changes WHERE changes.rev = commits.rev), 0),
          lt = 0.0;
    ";
    db.conn()
        .execute_batch(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei size: {e}")))?;
    Ok(())
}

/// Purpose: FIX (commit message matches bug/fix regex)
fn enrich_fix(db: &FactsDb) -> Result<()> {
    let sql = "
        UPDATE commits SET
          fix = REGEXP_MATCHES(LOWER(message), '\\b(bug|fix|fixes|fixed|defect|patch|hotfix|issue|error)\\b');
    ";
    db.conn()
        .execute_batch(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei fix: {e}")))?;
    Ok(())
}

/// History: NDEV, AGE, NUC.
///
/// Rewritten from N correlated subqueries (one per commit) to two hash-joined
/// UPDATE…FROM passes. The original was O(N²) on commit count because every
/// commit ran an independent `SELECT … FROM commits prev … WHERE prev.date <= c.date`.
/// The new shape lets `DuckDB` resolve the cross-commit join once via a hash
/// pass, then projects the aggregates back via a join key — orders of
/// magnitude faster on >10k-commit repos.
///
/// Two passes because `age` needs a different grouping (per-file MAX of
/// `prev.date`, then AVG across files) than `ndev`/`nuc` (distinct counts).
/// Initial zero-pass ensures commits with no prior history retain `0` /
/// `0.0` (semantically equivalent to the old `COALESCE(…, 0)` wrap).
fn enrich_history(db: &FactsDb, src: &str) -> Result<()> {
    // Pass 1: zero out — commits with no history retain these defaults.
    db.conn()
        .execute_batch("UPDATE commits SET ndev = 0, nuc = 0, age = 0.0;")
        .map_err(|e| CodeLoreError::Analysis(format!("kamei history reset: {e}")))?;

    // Pass 2: ndev + nuc + age via WINDOWED per-path running aggregation
    // followed by per-commit cross-path union.
    //
    // The previous shape was a path-self-join:
    //   {src} cchg ON cchg.rev = c.rev
    //   {src} pchg ON pchg.path = cchg.path
    // which for a path touched K times produced K×K rows per commit
    // touching that path. On monorepos with hot files (lockfiles,
    // top-level manifests, vendored config), the row blow-up dominated
    // ingest wall-clock and could OOM larger fact stores.
    //
    // The windowed shape walks each path's touch sequence once,
    // accumulating prior authors / revs / dates as DuckDB LIST values
    // via a `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW EXCLUDE
    // CURRENT ROW` frame. RANGE (not ROWS) preserves the Kamei
    // same-second semantic: peers (rows sharing the current row's
    // ORDER BY date) are included in the prior set; only the current
    // row itself is excluded. This matches the legacy `prev.date <=
    // c.date AND prev.rev != c.rev` predicate exactly.
    //
    // Per-commit aggregation across paths uses FLATTEN(LIST(...)) +
    // LIST_DISTINCT to union the per-path priors and dedupe. age is
    // AVG of (curr_date − prior_last_date_at_path) across paths that
    // have a prior touch.
    let sql_history = format!(
        "UPDATE commits SET
             ndev = COALESCE(h.ndev, 0),
             nuc = COALESCE(h.nuc, 0),
             age = COALESCE(h.age, 0.0)
        FROM (
            WITH path_prior_state AS (
                SELECT
                    ch.path,
                    cm.rev,
                    cm.date AS curr_date,
                    LIST(cm.canonical_author) OVER w AS prior_authors_at_path,
                    LIST(cm.rev) OVER w AS prior_revs_at_path,
                    MAX(cm.date) OVER w AS prior_last_date_at_path
                FROM commits cm
                INNER JOIN {src} ch ON ch.rev = cm.rev
                WINDOW w AS (
                    PARTITION BY ch.path
                    ORDER BY cm.date
                    RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                    EXCLUDE CURRENT ROW
                )
            )
            SELECT
                rev AS curr_rev,
                LENGTH(LIST_DISTINCT(FLATTEN(LIST(prior_authors_at_path)))) AS ndev,
                LENGTH(LIST_DISTINCT(FLATTEN(LIST(prior_revs_at_path)))) AS nuc,
                AVG(DATE_DIFF('day', prior_last_date_at_path, curr_date))
                    FILTER (WHERE prior_last_date_at_path IS NOT NULL) AS age
            FROM path_prior_state
            GROUP BY rev
        ) AS h
        WHERE commits.rev = h.curr_rev;"
    );
    db.conn()
        .execute_batch(&sql_history)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei history ndev/nuc/age: {e}")))?;

    Ok(())
}

/// Experience: EXP, REXP, SEXP.
///
/// Rewritten from N correlated subqueries to two hash-joined UPDATE…FROM
/// passes (same motivation + shape as `enrich_history` above). `prev.date <=
/// c.date AND prev.rev != c.rev` preserves the same-day-commit semantics —
/// strictly-before would give EXP=0 for repos with many same-date commits
/// (test fixtures, bulk imports, ingest-time clusters).
fn enrich_experience(db: &FactsDb, src: &str) -> Result<()> {
    db.conn()
        .execute_batch("UPDATE commits SET exp = 0, rexp = 0.0, sexp = 0;")
        .map_err(|e| CodeLoreError::Analysis(format!("kamei experience reset: {e}")))?;

    // Pass 1: EXP + REXP via single per-author aggregation.
    let sql_exp_rexp = "
        UPDATE commits SET exp = ae.exp, rexp = ae.rexp
        FROM (
            SELECT
                c.rev AS curr_rev,
                COUNT(*) AS exp,
                SUM(1.0 / (1.0 + CAST(DATE_DIFF('year', prev.date, c.date) AS DOUBLE))) AS rexp
            FROM commits c
            INNER JOIN commits prev
                ON prev.canonical_author = c.canonical_author
                AND prev.rev != c.rev
                AND prev.date <= c.date
            GROUP BY c.rev
        ) AS ae
        WHERE commits.rev = ae.curr_rev;
    ";
    db.conn()
        .execute_batch(sql_exp_rexp)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei exp/rexp: {e}")))?;

    // Pass 2: SEXP (subsystem experience) — distinct prior commits by the
    // same author that touched the same top-level dir as the current commit.
    //
    // Windowed replacement of the legacy dir × author self-join: for a
    // top-level dir touched K times by the same author, the join used to
    // produce K×K rows per current commit. The windowed form walks each
    // (dir, author) sequence once, accumulating prior revs as a DuckDB
    // LIST via a RANGE … EXCLUDE CURRENT ROW frame — preserving the
    // Kamei same-second semantic (peers included, current row excluded).
    //
    // Distinct revs per current commit come from FLATTEN(LIST(...)) +
    // LIST_DISTINCT across the commit's touched dirs.
    let sql_sexp = format!(
        "UPDATE commits SET sexp = COALESCE(asx.sexp, 0)
        FROM (
            WITH dir_author_prior AS (
                SELECT
                    SPLIT_PART(ch.path, '/', 1) AS dir,
                    cm.canonical_author,
                    cm.rev,
                    cm.date,
                    LIST(cm.rev) OVER w AS prior_revs
                FROM commits cm
                INNER JOIN {src} ch ON ch.rev = cm.rev
                WINDOW w AS (
                    PARTITION BY SPLIT_PART(ch.path, '/', 1), cm.canonical_author
                    ORDER BY cm.date
                    RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                    EXCLUDE CURRENT ROW
                )
            )
            SELECT
                rev AS curr_rev,
                LENGTH(LIST_DISTINCT(FLATTEN(LIST(prior_revs)))) AS sexp
            FROM dir_author_prior
            GROUP BY rev
        ) AS asx
        WHERE commits.rev = asx.curr_rev;"
    );
    db.conn()
        .execute_batch(&sql_sexp)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei sexp: {e}")))?;

    Ok(())
}
