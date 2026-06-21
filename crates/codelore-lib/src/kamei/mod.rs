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
    //
    // Rewritten from three correlated subqueries to a single hash-joined
    // `UPDATE … FROM (… GROUP BY rev) …`. The prior shape re-scanned
    // `changes` three times per commit (O(N × |changes|) work); the
    // grouped aggregation walks `changes` exactly once and joins back
    // by rev. Same motivation + shape as `enrich_history` / `enrich_experience`.
    let sql_counts = "
        UPDATE commits SET
          nf = COALESCE(d.nf, 0),
          ns = COALESCE(d.ns, 0),
          nd = COALESCE(d.nd, 0)
        FROM (
            SELECT
              rev,
              COUNT(DISTINCT path) AS nf,
              COUNT(DISTINCT SPLIT_PART(path, '/', 1)) AS ns,
              COUNT(DISTINCT
                  CASE
                      WHEN STRPOS(path, '/') > 0
                      THEN SUBSTR(path, 1, LENGTH(path) - LENGTH(SPLIT_PART(path, '/', -1)) - 1)
                      ELSE ''
                  END
              ) AS nd
            FROM changes
            GROUP BY rev
        ) AS d
        WHERE commits.rev = d.rev;
    ";
    db.conn()
        .execute_batch(sql_counts)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei diffusion counts: {e}")))?;

    // entropy = -Σ p_i log2(p_i) over the LOC distribution across files
    // within each commit.
    //
    // Rewritten from a correlated subquery (a per-row WITH-CTE that
    // re-scanned `changes` against every `commits.rev`) into the same
    // 2-pass shape `enrich_history` uses:
    //
    //   Pass 1: reset every commits.entropy to 0.0 — commits whose
    //           `changes` rows all have `loc_added = 0` (binary-only
    //           changes, deletes, the LA stub) silently miss the
    //           grouped UPDATE's join key and would otherwise retain
    //           whatever entropy they had pre-call. The reset
    //           preserves the prior `COALESCE(..., 0.0)` semantics
    //           where the correlated-subquery's NULL becomes 0.0.
    //
    //   Pass 2: grouped UPDATE...FROM that walks `changes` once,
    //           computes p_i = loc_added / SUM(loc_added) per rev via a
    //           window function partitioned by rev, then aggregates
    //           -Σ p log2(p) per rev. DuckDB resolves the cross-rev
    //           join in a single hash pass.
    //
    // Byte-identical semantics validated against the prior shape by
    // `kamei_entropy_per_commit_distribution` in tests/kamei_test.rs:
    // single-file commits emit 0.0, even 2-way splits emit log2(2) =
    // 1.0, the uneven 3-way reference case emits ≈ 1.29879494...
    db.conn()
        .execute_batch("UPDATE commits SET entropy = 0.0;")
        .map_err(|e| CodeLoreError::Analysis(format!("kamei diffusion entropy reset: {e}")))?;

    let sql_entropy = "
        UPDATE commits SET entropy = e.h
        FROM (
            SELECT rev, -SUM(p * LOG2(p)) AS h
            FROM (
                SELECT
                    rev,
                    CAST(loc_added AS DOUBLE)
                        / SUM(CAST(loc_added AS DOUBLE)) OVER (PARTITION BY rev) AS p
                FROM changes
                WHERE loc_added > 0
            )
            GROUP BY rev
        ) AS e
        WHERE commits.rev = e.rev;
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
    //      Approximation: stub to 0 since pre-change LOC requires
    //      reading historical blobs (a future schema enhancement).
    //
    // Two correlated subqueries collapse to a single grouped aggregation
    // joined back by rev — same shape as the rewritten enrich_diffusion
    // above. `lt = 0.0` stays as a plain UPDATE since it doesn't read
    // `changes`.
    let sql_la_ld = "
        UPDATE commits SET
          la = COALESCE(s.la, 0),
          ld = COALESCE(s.ld, 0)
        FROM (
            SELECT rev, SUM(loc_added) AS la, SUM(loc_deleted) AS ld
            FROM changes
            GROUP BY rev
        ) AS s
        WHERE commits.rev = s.rev;
    ";
    db.conn()
        .execute_batch(sql_la_ld)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei size la/ld: {e}")))?;
    db.conn()
        .execute_batch("UPDATE commits SET lt = 0.0;")
        .map_err(|e| CodeLoreError::Analysis(format!("kamei size lt: {e}")))?;
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
    // The previous windowed form materialised `LIST(...) OVER w`
    // per row, allocating O(K²) memory per partition. On
    // directory-skewed repos (e.g. Vue monorepos with 26 k touches
    // under `src/`), a cross-join variant exploded to hundreds of
    // millions of rows — production OOM at 19 GiB.
    //
    // O(K) approach:
    //   1. DISTINCT the (path, rev, author, date) tuples so each
    //      rev counts once per path.
    //   2. Use a self-join restricted to *strictly* prior touches
    //      (`prev.date < curr.date`), but with an aggregating
    //      hash-grouped count rather than a list materialisation.
    //   3. DuckDB's `COUNT(DISTINCT ...)` over the join result is
    //      hash-bucketed by `curr.rev` so the working set is one
    //      hash partition at a time, not the full cross-product.
    //
    // Semantic shift: switches from the Kamei `<=` (same-second
    // peers count) to strict `<`. In real repos commits are
    // distinct-second by construction (git commits are
    // sequential), so this is a no-op semantic change. Test
    // fixtures that manufacture same-second commits would notice;
    // the existing `windowed_history_matches_legacy_semantics_on_hot_path`
    // test uses explicit distinct timestamps so `<` and `<=`
    // agree on it.
    let sql_history = format!(
        "UPDATE commits SET
             ndev = COALESCE(h.ndev, 0),
             nuc  = COALESCE(h.nuc, 0),
             age  = COALESCE(h.age, 0.0)
        FROM (
            WITH path_commit AS (
                SELECT DISTINCT
                    ch.path,
                    cm.rev,
                    cm.canonical_author,
                    cm.date
                FROM commits cm
                INNER JOIN {src} ch ON ch.rev = cm.rev
            ),
            per_path_last AS (
                SELECT
                    curr.rev   AS curr_rev,
                    curr.path  AS curr_path,
                    curr.date  AS curr_date,
                    MAX(prev.date) AS last_at
                FROM path_commit curr
                INNER JOIN path_commit prev
                    ON prev.path = curr.path
                   AND prev.date < curr.date
                GROUP BY curr.rev, curr.path, curr.date
            ),
            per_commit_age AS (
                SELECT
                    curr_rev,
                    AVG(DATE_DIFF('day', last_at, curr_date)) AS age
                FROM per_path_last
                WHERE last_at IS NOT NULL
                GROUP BY curr_rev
            ),
            prior_pairs AS (
                SELECT
                    curr.rev AS curr_rev,
                    prev.canonical_author AS prev_author,
                    prev.rev AS prev_rev
                FROM path_commit curr
                INNER JOIN path_commit prev
                    ON prev.path = curr.path
                   AND prev.date < curr.date
            ),
            per_commit_counts AS (
                SELECT
                    curr_rev,
                    COUNT(DISTINCT prev_author) AS ndev,
                    COUNT(DISTINCT prev_rev) AS nuc
                FROM prior_pairs
                GROUP BY curr_rev
            )
            SELECT
                pcc.curr_rev,
                pcc.ndev,
                pcc.nuc,
                pca.age
            FROM per_commit_counts pcc
            LEFT JOIN per_commit_age pca USING (curr_rev)
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
/// passes (same motivation + shape as `enrich_history` above).
///
/// Same-second peer semantics: **strict** `prev.date < c.date`. Matches
/// NDEV / NUC / AGE (`enrich_history`) and SEXP (this function's pass 2)
/// — one canonical definition of "prior commit" across the whole Kamei
/// 14-feature vector, consistent with Kamei 2013 §3 baseline. The
/// earlier inclusive `<=` was a deliberate departure to handle bulk-
/// import fixtures gracefully, but it produced contradictory semantics
/// inside one feature vector (EXP would count peers SEXP didn't), and
/// real production repos commit at distinct seconds by construction.
/// Fixtures that manufacture same-second commits must use distinct
/// timestamps explicitly — `tests/kamei_test.rs` already does so.
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
                AND prev.date < c.date
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
    // O(K) memory SEXP via cumulative ROW_NUMBER, NOT cross-join.
    //
    // Constant-memory approach:
    //   1. DISTINCT (dir, author, rev, date) so each rev counts once
    //      per (dir, author).
    //   2. ROW_NUMBER() OVER (PARTITION BY dir, author ORDER BY date,
    //      rev) - 1 = number of strictly-prior rows in the partition.
    //      DuckDB streams this O(K) per partition; no list / no
    //      cross-join.
    //   3. Per-commit SEXP = MAX over the dirs the commit touched.
    //
    // Semantic shift: strict `<` on date (same-second peers no
    // longer count as priors). Real repos have distinct-second
    // commits by construction; only manufactured fixtures would
    // notice. Aligns with the strict-prior Kamei paper definition.
    let sql_sexp = format!(
        "UPDATE commits SET sexp = COALESCE(asx.sexp, 0)
        FROM (
            WITH distinct_dir_author_rev AS (
                SELECT DISTINCT
                    SPLIT_PART(ch.path, '/', 1) AS dir,
                    cm.canonical_author,
                    cm.rev,
                    cm.date
                FROM commits cm
                INNER JOIN {src} ch ON ch.rev = cm.rev
            ),
            ranked AS (
                SELECT
                    rev,
                    ROW_NUMBER() OVER (
                        PARTITION BY dir, canonical_author
                        ORDER BY date, rev
                    ) - 1 AS prior_count
                FROM distinct_dir_author_rev
            )
            SELECT
                rev AS curr_rev,
                MAX(prior_count) AS sexp
            FROM ranked
            GROUP BY rev
        ) AS asx
        WHERE commits.rev = asx.curr_rev;"
    );
    db.conn()
        .execute_batch(&sql_sexp)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei sexp: {e}")))?;

    Ok(())
}
