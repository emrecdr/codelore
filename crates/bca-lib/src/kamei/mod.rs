//! Kamei 14-feature JIT-SDP canonical change vector enrichment.
//!
//! See spec §3.1 and Kamei et al. 2013 (TSE).
//!
//! Implemented as a series of SQL UPDATE passes over the commits + changes
//! tables. Run after the main commit/changes ingest + complexity pass.

use crate::facts::FactsDb;
use crate::{BcaError, Result};

/// Enrich all rows in `commits` table with the 14 Kamei features.
/// Idempotent; safe to call multiple times.
pub fn enrich(db: &FactsDb) -> Result<()> {
    enrich_diffusion(db)?;
    enrich_size(db)?;
    enrich_fix(db)?;
    enrich_history(db)?;
    enrich_experience(db)?;
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
        .map_err(|e| BcaError::Analysis(format!("kamei diffusion counts: {e}")))?;

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
        .map_err(|e| BcaError::Analysis(format!("kamei diffusion entropy: {e}")))?;

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
        .map_err(|e| BcaError::Analysis(format!("kamei size: {e}")))?;
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
        .map_err(|e| BcaError::Analysis(format!("kamei fix: {e}")))?;
    Ok(())
}

/// History: NDEV, AGE, NUC
fn enrich_history(db: &FactsDb) -> Result<()> {
    // Use `date <= c.date AND prev.rev != c.rev` to handle same-day commits
    // (e.g. test fixtures) without losing history.
    let sql = "
        UPDATE commits AS c SET
          ndev = COALESCE((
              SELECT COUNT(DISTINCT prev.canonical_author)
              FROM commits prev
              INNER JOIN changes pchg ON pchg.rev = prev.rev
              INNER JOIN changes cchg ON cchg.rev = c.rev AND cchg.path = pchg.path
              WHERE prev.rev != c.rev AND prev.date <= c.date
          ), 0),
          nuc = COALESCE((
              SELECT COUNT(DISTINCT prev.rev)
              FROM commits prev
              INNER JOIN changes pchg ON pchg.rev = prev.rev
              INNER JOIN changes cchg ON cchg.rev = c.rev AND cchg.path = pchg.path
              WHERE prev.rev != c.rev AND prev.date <= c.date
          ), 0),
          age = COALESCE((
              SELECT AVG(DATE_DIFF('day', last_date, c.date))
              FROM (
                  SELECT MAX(prev.date) AS last_date
                  FROM commits prev
                  INNER JOIN changes pchg ON pchg.rev = prev.rev
                  INNER JOIN changes cchg ON cchg.rev = c.rev AND cchg.path = pchg.path
                  WHERE prev.rev != c.rev AND prev.date <= c.date
                  GROUP BY cchg.path
              )
          ), 0.0);
    ";
    db.conn()
        .execute_batch(sql)
        .map_err(|e| BcaError::Analysis(format!("kamei history: {e}")))?;
    Ok(())
}

/// Experience: EXP, REXP, SEXP
fn enrich_experience(db: &FactsDb) -> Result<()> {
    // Use `date <= c.date AND prev.rev != c.rev` to handle repos where many
    // commits share the same calendar date (e.g. test fixtures, bulk imports).
    // Strictly-before (`<`) would give EXP=0 for all same-day commits.
    let sql = "
        UPDATE commits AS c SET
          exp = COALESCE((
              SELECT COUNT(*)
              FROM commits prev
              WHERE prev.canonical_author = c.canonical_author
                AND prev.rev != c.rev
                AND prev.date <= c.date
          ), 0),
          rexp = COALESCE((
              SELECT SUM(1.0 / (1.0 + CAST(DATE_DIFF('year', prev.date, c.date) AS DOUBLE)))
              FROM commits prev
              WHERE prev.canonical_author = c.canonical_author
                AND prev.rev != c.rev
                AND prev.date <= c.date
          ), 0.0),
          sexp = COALESCE((
              SELECT COUNT(DISTINCT prev.rev)
              FROM commits prev
              INNER JOIN changes pchg ON pchg.rev = prev.rev
              INNER JOIN changes cchg ON cchg.rev = c.rev
              WHERE prev.canonical_author = c.canonical_author
                AND prev.rev != c.rev
                AND prev.date <= c.date
                AND SPLIT_PART(pchg.path, '/', 1) = SPLIT_PART(cchg.path, '/', 1)
          ), 0);
    ";
    db.conn()
        .execute_batch(sql)
        .map_err(|e| BcaError::Analysis(format!("kamei experience: {e}")))?;
    Ok(())
}
