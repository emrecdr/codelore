//! Decayed-knowledge materialization: `knowledge_shares` and `doe_scores`
//! temporary tables consumed by every WS-B analysis.
//!
//! ## Knowledge share model
//!
//! Each developer's knowledge of a file decays exponentially with time since
//! their last contribution. The decay constant (220 days, halving ≈ 5 months)
//! comes from Jabrayilzade et al., ICSE-SEIP 2022 (arXiv 2202.01523 §3.1).
//!
//! Contribution weight is scaled by AI attribution:
//! - Human commit → weight 1.0
//! - AI-assisted commit → weight 0.7 (heuristic motivated by arXiv 2507.08160,
//!   which shows GenAI-heavy commits distort expertise models)
//! - AI-authored commit → weight 0.3 (same source)
//!
//! Reviewer credit: commits carrying `Co-Authored-By:` or `Reviewed-By:`
//! trailers award each reviewer `W_REVIEWER` (0.5) of the author weight per
//! Jabrayilzade 2022 (reviewers = ½ author weight) and Rigby & Bird,
//! ESEC/FSE 2013 (review transfers 66–150% of authorship knowledge). Commits
//! touching more than 10 files are excluded from reviewer credit (Rigby &
//! Bird's >10-file exclusion rule: large sweeping commits do not meaningfully
//! transfer file-level knowledge to reviewers).
//!
//! ## DOE (Degree of Expertise) model
//!
//! DOE per author×file is computed from the linear model by Cury & Avelino,
//! SBES'24 (arXiv 2408.08733):
//!
//! ```text
//! doe = 5.28223
//!     + 0.23173 × ln(1 + adds)
//!     + 0.36151 × fa
//!     − 0.19421 × ln(1 + num_days)
//!     − 0.28761 × ln(size.max(1))
//! ```
//!
//! Where:
//! - `adds` = lifetime `SUM(loc_added)` by this author for this path
//! - `fa` = 1.0 if this author created the file (`change_type = 'added'`),
//!   0.0 otherwise
//! - `num_days` = days since this author's last touch of the file, measured
//!   against the repo's newest commit (recency, per the DOE definition)
//! - `size` = HEAD `SUM(sloc)` from `complexity_metrics` (clamped ≥ 1;
//!   the formula has `ln(size)` without a +1 guard, so clamping prevents ln(0))
//!
//! Expert threshold: `doe >= 1.0 AND doe >= 0.75 × max_doe_for_file`.
//! This normalization convention is adopted from Avelino's DOA work; the
//! DOE paper itself leaves the threshold unstated.

use std::collections::HashMap;

use crate::analyses::knowledge::trailers;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Exponential decay constant in days.
/// Source: Jabrayilzade et al., ICSE-SEIP 2022 (arXiv 2202.01523 §3.1).
pub const DECAY_DAYS: f64 = 220.0;

/// AI-authored commit knowledge weight.
/// Heuristic motivated by arXiv 2507.08160 (`GenAI` distorts expertise models).
/// See also [`W_AI_ASSISTED`].
pub const W_AI_AUTHORED: f64 = 0.3;

/// AI-assisted commit knowledge weight (same source as `W_AI_AUTHORED`).
pub const W_AI_ASSISTED: f64 = 0.7;

/// Reviewer knowledge weight relative to author.
/// Sources: Jabrayilzade et al. 2022 (reviewers = ½ author weight);
/// Rigby & Bird, ESEC/FSE 2013 (review transfers 66–150% of authorship).
pub const W_REVIEWER: f64 = 0.5;

/// Materialises two temporary tables into `db`:
///
/// - **`knowledge_shares(path TEXT, author TEXT, k DOUBLE, k_norm DOUBLE)`** —
///   per-author raw knowledge score `k` and its within-path normalised share
///   `k_norm` (summing to ~1.0 per path after reviewer credit).
/// - **`doe_scores(path TEXT, author TEXT, doe DOUBLE, is_expert BOOLEAN)`** —
///   DOE scores and expert flags per Cury & Avelino, SBES'24 (arXiv 2408.08733).
///
/// Both tables are idempotent: calling this function more than once on the
/// same `FactsDb` is a no-op (guarded by [`FactsDb::is_knowledge_shares_built`]).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "knowledge-shares", skip_all)]
pub fn materialize_knowledge_shares(db: &FactsDb, opts: &Options) -> Result<()> {
    if db.is_knowledge_shares_built() {
        return Ok(());
    }

    let src = crate::analyses::lineage::source_table(opts);
    crate::analyses::lineage::materialize_if_needed(db, opts)?;

    // ── Step 1: base knowledge_shares from author contributions ──────────────
    // Decay = exp(−Δdays / 220): Jabrayilzade et al., ICSE-SEIP 2022 §3.1.
    // AI weight: W_AI_AUTHORED=0.3, W_AI_ASSISTED=0.7, human=1.0 per
    // arXiv 2507.08160 (AI commits distort expertise models).
    // Bot rows are excluded pair-granularly via the JOIN on human_aliases:
    // a human sharing a canonical with a bot keeps their own commits'
    // knowledge weight counted while the bot pair's is dropped row-wise.
    // Deleted paths are excluded (change_type != 'deleted').
    let now_anchor = crate::analyses::query::clamped_now_anchor("date");
    let human_aliases = crate::analyses::query::HUMAN_ALIASES_CTE;
    let base_sql = format!(
        "CREATE OR REPLACE TEMP TABLE knowledge_shares AS
         WITH anchor AS (SELECT {now_anchor} AS max_d FROM commits),
         {human_aliases},
         contrib AS (
           SELECT c.path,
                  co.canonical_author AS author,
                  SUM(
                    c.loc_added
                    * CASE co.ai_attribution
                        WHEN 'ai-authored'  THEN {W_AI_AUTHORED}
                        WHEN 'ai-assisted'  THEN {W_AI_ASSISTED}
                        ELSE 1.0
                      END
                    -- GREATEST floors the age at 0: a commit dated after the
                    -- clamped anchor (clock skew, a future date) reads as the
                    -- present, never a negative age that would invert the decay.
                    * EXP(-GREATEST(date_diff('day', co.date, (SELECT max_d FROM anchor)), 0)
                          / {DECAY_DAYS})
                  ) AS k
           FROM {src} c
           JOIN commits co USING (rev)
           JOIN human_aliases ha ON ha.raw_name = co.author_name AND ha.raw_email = co.author_email
           WHERE c.change_type != 'deleted'
           GROUP BY c.path, co.canonical_author
         )
         SELECT path,
                author,
                k,
                k / NULLIF(SUM(k) OVER (PARTITION BY path), 0) AS k_norm
         FROM contrib",
    );
    db.execute_batch(&base_sql)?;

    // ── Step 2: reviewer credit ───────────────────────────────────────────────
    // Pull commits with nf ≤ 10 (Rigby & Bird: >10-file commits excluded from
    // reviewer credit) that have non-empty messages. Then add W_REVIEWER×k rows
    // for each trailer identity found.
    let reviewer_rows = collect_reviewer_rows(db, src)?;
    if !reviewer_rows.is_empty() {
        insert_reviewer_rows(db, &reviewer_rows)?;
        // Re-normalize k_norm after the reviewer rows are added. DuckDB
        // rejects window functions inside UPDATE ("Binder Error: window
        // functions are not allowed in UPDATE"), so rebuild the temp table
        // with the recomputed shares instead.
        //
        // An author can appear twice at this point: once from the base
        // contributor row (Step 1) and once from a reviewer-trailer row
        // (this step), when they both wrote code on a path AND are named
        // in a Co-Authored-By/Reviewed-By trailer on a commit touching that
        // same path. GROUP BY path, author merges those into one row before
        // computing k_norm — otherwise the same person keeps two un-merged
        // shares of the same path, which lets consumers like
        // `code_familiarity`'s per-path ROW_NUMBER() rank the same person
        // #1 and #2, and inflates `coordination_needs` fragmentation
        // (1 − Σk_norm²) since a² + b² < (a + b)².
        db.execute_batch(
            "CREATE OR REPLACE TEMP TABLE knowledge_shares AS
             WITH merged AS (
               SELECT path, author, SUM(k) AS k
               FROM knowledge_shares
               GROUP BY path, author
             )
             SELECT path,
                    author,
                    k,
                    k / NULLIF(SUM(k) OVER (PARTITION BY path), 0) AS k_norm
             FROM merged",
        )?;
    }

    // ── Step 3: DOE scores ────────────────────────────────────────────────────
    materialize_doe_scores(db, src)?;

    db.mark_knowledge_shares_built();
    Ok(())
}

// ---------------------------------------------------------------------------
// Reviewer credit helpers
// ---------------------------------------------------------------------------

/// One reviewer-credit row to INSERT into `knowledge_shares`.
struct ReviewerRow {
    path: String,
    author: String,
    k: f64,
}

/// Scan commits with `nf <= 10` for trailer identities, map them to
/// `canonical_author` via `author_aliases`, and produce the reviewer rows.
///
/// Returns one row per `(path, reviewer_canonical)` with the aggregated
/// reviewer k-weight across all commits that touch that path. Trailer emails
/// are matched against `author_aliases.raw_email` (case-insensitively); rows
/// with no alias match are skipped — unregistered emails cannot be attributed.
fn collect_reviewer_rows(db: &FactsDb, src: &str) -> Result<Vec<ReviewerRow>> {
    // Query returns one row per (rev, path) so no ARRAY_AGG is needed.
    // The k_weight column is per-path (loc_added × ai_weight × decay),
    // matching the granularity of the base knowledge_shares CTE.
    let now_anchor = crate::analyses::query::clamped_now_anchor("date");
    let human_aliases = crate::analyses::query::HUMAN_ALIASES_CTE;
    let query = format!(
        "WITH {human_aliases}
         SELECT c.path,
                co.message,
                c.loc_added
                * CASE co.ai_attribution
                    WHEN 'ai-authored'  THEN {W_AI_AUTHORED}
                    WHEN 'ai-assisted'  THEN {W_AI_ASSISTED}
                    ELSE 1.0
                  END
                * EXP(-GREATEST(date_diff('day', co.date,
                      (SELECT {now_anchor} FROM commits)), 0)
                      / {DECAY_DAYS})
                AS k_path
         FROM {src} c
         JOIN commits co USING (rev)
         -- Pair-granular: a human sharing a canonical with a bot keeps
         -- their own reviewer-credit contribution counted (see contrib
         -- above in materialize_knowledge_shares).
         JOIN human_aliases ha ON ha.raw_name = co.author_name AND ha.raw_email = co.author_email
         WHERE c.change_type != 'deleted'
           AND co.nf <= 10",
    );

    // Raw row: (path, message, k_path).
    let mut flat_rows: Vec<(String, String, f64)> = Vec::new();
    {
        let mut stmt = db
            .conn()
            .prepare(&query)
            .map_err(|e| CodeLoreError::Analysis(format!("prepare reviewer scan: {e}")))?;
        let rows = stmt
            .query_map([], |r| {
                let path: String = r.get(0)?;
                let message: String = r.get(1)?;
                let k_path: f64 = r.get(2)?;
                Ok((path, message, k_path))
            })
            .map_err(|e| CodeLoreError::Analysis(format!("query reviewer scan: {e}")))?;
        for row in rows {
            let (path, message, k_path) =
                row.map_err(|e| CodeLoreError::Analysis(format!("row reviewer scan: {e}")))?;
            flat_rows.push((path, message, k_path));
        }
    }

    if flat_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Build a lowercase-email → canonical_author lookup from author_aliases.
    let alias_map = build_alias_map(db)?;

    // For each (path, message, k_path): extract trailer emails, look them up,
    // emit a ReviewerRow for each known reviewer.
    let mut agg: HashMap<(String, String), f64> = HashMap::new();
    for (path, message, k_path) in &flat_rows {
        let trailer_emails: Vec<String> = {
            let mut v = trailers::extract_coauthors(message);
            v.extend(trailers::extract_reviewers(message));
            v.sort();
            v.dedup();
            v
        };
        for email in &trailer_emails {
            let Some(canonical) = alias_map.get(email.as_str()) else {
                continue; // unregistered trailer email — skip
            };
            *agg.entry((path.clone(), canonical.clone())).or_insert(0.0) += k_path * W_REVIEWER;
        }
    }

    Ok(agg
        .into_iter()
        .map(|((path, author), k)| ReviewerRow { path, author, k })
        .collect())
}

/// Build a `raw_email_lowercase → canonical_author` map from `author_aliases`.
///
/// This is a deliberately email-keyed lookup: trailer extraction
/// ([`trailers`]) yields only the `<email>` of a `Co-Authored-By:` /
/// `Reviewed-By:` line — the display name is discarded there because the
/// email is the more identity-stable half — so there is no name to resolve
/// on. For every author except the shared-commit-email case each email maps
/// to one canonical, so the map is unaffected by the `(raw_name, raw_email)`
/// re-key. When two identities do share a commit email a bare trailer email
/// cannot disambiguate them; `ORDER BY` makes the collapse deterministic
/// (highest canonical wins) rather than dependent on scan order.
fn build_alias_map(db: &FactsDb) -> Result<HashMap<String, String>> {
    let query = format!(
        "WITH {human_aliases} \
         SELECT raw_email, canonical FROM human_aliases ORDER BY raw_email, canonical",
        human_aliases = crate::analyses::query::HUMAN_ALIASES_CTE
    );
    let mut stmt = db
        .conn()
        .prepare(&query)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare alias_map: {e}")))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query alias_map: {e}")))?;
    let mut map = HashMap::new();
    for row in rows {
        let (raw, canonical) =
            row.map_err(|e| CodeLoreError::Analysis(format!("row alias_map: {e}")))?;
        map.insert(raw.to_lowercase(), canonical);
    }
    Ok(map)
}

/// INSERT the aggregated reviewer rows into `knowledge_shares`.
/// Uses prepared INSERT (not Appender) so it works on read-only connections
/// (temp tables are in-memory, separate from the file catalog).
fn insert_reviewer_rows(db: &FactsDb, rows: &[ReviewerRow]) -> Result<()> {
    let mut stmt = db
        .conn()
        .prepare("INSERT INTO knowledge_shares (path, author, k, k_norm) VALUES (?, ?, ?, 0.0)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare insert reviewer rows: {e}")))?;
    for row in rows {
        stmt.execute(duckdb::params![row.path, row.author, row.k])
            .map_err(|e| CodeLoreError::Analysis(format!("insert reviewer row: {e}")))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DOE scores
// ---------------------------------------------------------------------------

/// Materialise `doe_scores(path TEXT, author TEXT, doe DOUBLE, is_expert BOOLEAN)`.
///
/// DOE formula from Cury & Avelino, SBES'24 (arXiv 2408.08733):
/// ```text
/// doe = 5.28223
///     + 0.23173 × ln(1 + adds)
///     + 0.36151 × fa
///     − 0.19421 × ln(1 + num_days)
///     − 0.28761 × ln(size.max(1))
/// ```
/// Expert threshold: doe >= 1.0 AND doe >= 0.75 × `max_doe_for_file`
/// (Avelino DOA normalization convention; threshold unstated in DOE paper —
/// documented here as adopted convention).
fn materialize_doe_scores(db: &FactsDb, src: &str) -> Result<()> {
    let now_anchor = crate::analyses::query::clamped_now_anchor("date");
    let human_aliases = crate::analyses::query::HUMAN_ALIASES_CTE;
    let sql = format!(
        "CREATE OR REPLACE TEMP TABLE doe_scores AS
         WITH anchor AS (SELECT {now_anchor} AS max_d FROM commits),
         {human_aliases},
         -- Who first added each file (fa = 1 if this author created it).
         -- Bots excluded pair-granularly: a human sharing a canonical with
         -- a bot keeps their own 'added' commits counted (see agg below).
         -- DISTINCT: a path deleted and re-added by the same author would
         -- otherwise yield duplicate rows here, and the LEFT JOIN below
         -- would then duplicate that author's doe_scores row.
         first_adders AS (
           SELECT DISTINCT c.path,
                  co.canonical_author AS author
           FROM {src} c
           JOIN commits co USING (rev)
           JOIN human_aliases ha ON ha.raw_name = co.author_name AND ha.raw_email = co.author_email
           WHERE c.change_type = 'added'
         ),
         -- HEAD SLOC per path. `complexity_metrics` holds only the HEAD-time
         -- scan (its `rev` column carries the actual head SHA, never the
         -- literal 'HEAD'), so no rev filter is needed — filtering on
         -- rev = 'HEAD' would match zero rows and silently zero the size term.
         head_sloc AS (
           SELECT path, GREATEST(SUM(sloc), 1) AS size
           FROM complexity_metrics
           GROUP BY path
         ),
         -- Per author×path aggregates.
         agg AS (
           SELECT c.path,
                  co.canonical_author AS author,
                  SUM(c.loc_added) AS adds,
                  -- GREATEST floors the age at 0 so a commit dated after the
                  -- clamped anchor cannot drive LN(1 + num_days) negative.
                  GREATEST(date_diff('day',
                    MAX(co.date),
                    (SELECT max_d FROM anchor)
                  ), 0) AS num_days
           FROM {src} c
           JOIN commits co USING (rev)
           JOIN human_aliases ha ON ha.raw_name = co.author_name AND ha.raw_email = co.author_email
           WHERE c.change_type != 'deleted'
           GROUP BY c.path, co.canonical_author
         ),
         doe_raw AS (
           SELECT agg.path,
                  agg.author,
                  5.28223
                  + 0.23173 * LN(1.0 + agg.adds)
                  + 0.36151 * CASE WHEN fa.author IS NOT NULL THEN 1.0 ELSE 0.0 END
                  - 0.19421 * LN(1.0 + agg.num_days)
                  - 0.28761 * LN(COALESCE(hs.size, 1))
                  AS doe
           FROM agg
           LEFT JOIN first_adders fa
             ON fa.path = agg.path AND fa.author = agg.author
           LEFT JOIN head_sloc hs ON hs.path = agg.path
         )
         SELECT path,
                author,
                doe,
                doe >= 1.0
                AND doe >= 0.75 * MAX(doe) OVER (PARTITION BY path)
                AS is_expert
         FROM doe_raw",
    );
    db.execute_batch(&sql)
}
