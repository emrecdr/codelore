#![allow(clippy::doc_markdown)]
//! `knowledge-islands` analysis — automatic bus-factor / knowledge-loss
//! detection per file.
//!
//! ## What this signal tells you
//!
//! For each currently-live file, surfaces the highest-risk
//! knowledge-loss cases: files where the primary author (by LoC added)
//! has effectively departed (`--departed-threshold-days`, default 90)
//! AND no other contributor owns a substantial share (default 10%).
//! These are the files that would become unmaintainable if you needed
//! to ship a fix tomorrow — the people who could fix them have already
//! left.
//!
//! ## Why this is `CodeLore`'s strategic differentiator
//!
//! Industry behavioural-code-analysis tools have two of the three
//! ingredients for this signal:
//!
//! - `CodeScene`'s **Knowledge Distribution** + **Bus Factor**:
//!   identifies primary owners but requires you to **manually mark**
//!   each "Ex-Developer" in a list. Maintaining that list is
//!   organisational labour — and dashboards stay wrong until someone
//!   updates them after every offboard.
//! - `code-maat`: has none of the three (no departure detection, no
//!   bus-factor analysis).
//! - `GitHub Insights`: shows contributor counts but no risk modeling.
//!
//! `CodeLore` ships all three automatically:
//!
//! 1. **Primary-author detection** (existing `ownership` analysis logic
//!    — author with max LoC added per file).
//! 2. **Departed-author detection** (new — `commits.canonical_author`
//!    grouped by `MAX(commits.date)` falloff > `--departed-threshold-days`).
//! 3. **Substantial-other-owner check** (new — `n_substantial_others` =
//!    count of authors with ≥ 10% LoC share on the file, excluding the
//!    main author).
//!
//! ## Output columns (modern default)
//!
//! - `entity` — the file path (or canonical-lineage entity under
//!   `--use-canonical-lineage`).
//! - `main_author` — author with max LoC added on the file
//!   (alphabetical-first tiebreak; deterministic).
//! - `ownership_pct` — `main_author` LoC share of the file
//!   (`0`–`100`, two decimals).
//! - `days_since_main_active` — days since `main_author`'s most-recent
//!   commit anywhere in the repo (not just on this file).
//! - `last_main_author_commit` — calendar date (`YYYY-MM-DD`) of that
//!   most-recent commit.
//! - `n_substantial_others` — count of other authors with ≥ 10%
//!   LoC share on this file. Zero is the actionable signal.
//!
//! Sort: `ownership_pct DESC, days_since_main_active DESC, entity ASC`.
//! Highest-concentration-then-longest-departed first — exactly the
//! triage order a tech lead wants.
//!
//! ## Practitioner heuristics
//!
//! - **`ownership_pct > 80` + `n_substantial_others = 0` +
//!   `is_departed = true`**: red flag. Find someone to learn this file
//!   BEFORE you need to ship a fix.
//! - **`ownership_pct < 50`**: ownership is genuinely diffuse; bus
//!   factor is healthy regardless of any one departure.
//! - **`days_since_main_active` between 60 and 90**: gray zone. May be
//!   sabbatical / between projects rather than permanent departure.
//!   Lower `--departed-threshold-days` to see these flagged.
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "knowledge-islands" (Bird et al., FSE 2011 — original n-authors risk
//! indicator; Avelino et al., SANER 2016 — Truck Factor estimation;
//! Cosentino et al., CHASE 2015 — bus-factor measurement; `CodeScene`
//! — productisation reference with the modernise-don't-migrate
//! improvement of automatic departure detection).

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeIslandRow {
    pub entity: String,
    pub main_author: String,
    pub ownership_pct: f64,
    pub days_since_main_active: i32,
    pub last_main_author_commit: String,
    pub n_substantial_others: u32,
}

/// Un-thresholded per-path owner-activity snapshot — the same
/// ownership/activity primitive behind [`KnowledgeIslandRow`], but
/// returned for every requested path regardless of how recently the main
/// author committed. Callers (e.g. the agent-loop pre-write briefing)
/// decide what counts as "departed" by comparing `days_since_main_active`
/// against their own threshold.
#[derive(Debug, Clone)]
pub struct OwnerActivity {
    /// Author with the max LoC share on the path (alphabetical-first
    /// tiebreak; deterministic).
    pub main_author: String,
    /// `main_author`'s LoC share of the path (`0`-`100`, two decimals).
    pub ownership_pct: f64,
    /// Days since `main_author`'s most-recent commit anywhere in the
    /// repo, relative to `opts.age_time_now` (or now, if unset).
    pub days_since_main_active: i32,
    /// Calendar date (`YYYY-MM-DD`) of that most-recent commit.
    pub last_main_author_commit: String,
    /// Count of other authors with a LoC share on this path at or above
    /// the substantial-owner threshold.
    pub n_substantial_others: u32,
}

/// Anchor timestamp for "as of" temporal calculations (departure
/// detection, live-path liveness, days-since-active). Reuses
/// `--age-time-now` when set (so back-test mode works: "who had departed
/// as of June 2024?"), otherwise the current instant. Shared by
/// [`run_knowledge_islands`] and [`owner_activity_for_paths`].
fn anchor_timestamp(opts: &Options) -> String {
    if let Some(d) = opts.age_time_now {
        format!(
            "{:04}-{:02}-{:02} 23:59:59",
            d.year(),
            u8::from(d.month()),
            d.day()
        )
    } else {
        let n = time::OffsetDateTime::now_utc();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            n.year(),
            u8::from(n.month()),
            n.day(),
            n.hour(),
            n.minute(),
            n.second(),
        )
    }
}

// Shared CTE fragments between `run_knowledge_islands` (restricted to
// currently-live paths, filtered to authors past `departed_threshold_days`)
// and `owner_activity_for_paths` (restricted to a caller-supplied path
// list, unthresholded) — both need "who owns this file, and when did they
// last commit anywhere", just gated differently at the edges.

// Per-author `MAX(commits.date)` anywhere in the repo, as of the anchor.
const AUTHOR_LAST_COMMIT_CTE: &str = "author_last_commit AS (
        SELECT canonical_author AS author, MAX(date) AS last_at
        FROM commits
        WHERE date <= CAST(? AS TIMESTAMP)
        GROUP BY canonical_author
    )";

// Per-path total LoC across all (non-bot) authors. `HAVING SUM(loc) > 0`
// keeps binary-only and fully-bot-owned files from propagating downstream
// as NULL-ownership rows.
const TOTALS_CTE: &str = "totals AS (
        SELECT path, SUM(loc) AS total_loc
        FROM per_path_author
        GROUP BY path
        HAVING SUM(loc) > 0
    )";

// Per-path dominant author by LoC (deterministic alphabetical tiebreak).
const MAIN_PER_PATH_CTE: &str = "main_per_path AS (
        SELECT
            path,
            first(author ORDER BY loc DESC, author ASC) AS author,
            MAX(loc) AS loc
        FROM per_path_author
        GROUP BY path
    )";

// Per-path count of non-main authors holding at least the
// substantial-owner LoC share threshold.
const SUBSTANTIAL_OTHERS_CTE: &str = "substantial_others AS (
        SELECT
            ppa.path,
            CAST(SUM(CASE
                WHEN ppa.author != m.author
                 AND t.total_loc > 0
                 AND CAST(ppa.loc AS DOUBLE) / t.total_loc >= ?
                THEN 1 ELSE 0
            END) AS UINTEGER) AS n_others
        FROM per_path_author ppa
        INNER JOIN totals t ON ppa.path = t.path
        INNER JOIN main_per_path m ON m.path = ppa.path
        GROUP BY ppa.path
    )";

// The shared projection: main author, ownership share, days-since-active,
// last-active date, substantial-others count. `run_knowledge_islands`
// appends a departed-threshold `WHERE` + `ORDER BY` + `LIMIT`;
// `owner_activity_for_paths` appends nothing (unthresholded, unlimited).
const OWNER_ACTIVITY_SELECT_CORE: &str = "SELECT
        m.path AS entity,
        m.author AS main_author,
        100.0 * m.loc / NULLIF(t.total_loc, 0) AS ownership_pct,
        DATE_DIFF('day', alc.last_at, CAST(? AS TIMESTAMP)) AS days_since_main_active,
        CAST(CAST(alc.last_at AS DATE) AS TEXT) AS last_main_author_commit,
        so.n_others AS n_substantial_others
    FROM main_per_path m
    INNER JOIN totals t ON t.path = m.path
    INNER JOIN author_last_commit alc ON alc.author = m.author
    INNER JOIN substantial_others so ON so.path = m.path";

/// Per-`(path, author)` LoC-added, bot-filtered, restricted to
/// `restrict_to` — a CTE providing the candidate `path` set (`live_paths`
/// for the departed-threshold analysis, `requested_paths` for the
/// arbitrary-path helper).
fn per_path_author_cte(restrict_to: &str) -> String {
    format!(
        "per_path_author AS (
        -- Filter bots BEFORE aggregating ownership: bots (dependabot,
        -- renovate, etc.) don't have knowledge to lose — flagging a
        -- dependabot-dominated lockfile as a 'knowledge island' is
        -- exactly the false positive that destroys signal credibility.
        SELECT
            changes.path,
            commits.canonical_author AS author,
            SUM(COALESCE(changes.loc_added, 0)) AS loc
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        INNER JOIN {restrict_to} USING (path)
        LEFT JOIN (
            -- Dedupe by canonical: author_aliases is keyed on
            -- (raw_name, raw_email); canonical is N:1 (one person owns
            -- several name+email identities). The LEFT JOIN tolerates
            -- authors without an author_aliases row (legacy / pattern-only
            -- classifications).
            SELECT canonical, BOOL_OR(is_bot) AS is_bot
            FROM author_aliases
            GROUP BY canonical
        ) aa ON aa.canonical = commits.canonical_author
        WHERE COALESCE(aa.is_bot, FALSE) = FALSE
          AND commits.date <= CAST(? AS TIMESTAMP)
        GROUP BY changes.path, commits.canonical_author
    )"
    )
}

/// Assemble `run_knowledge_islands`'s query text: the shared CTEs plus the
/// `live_paths` liveness + `--min-revs` gate, the departed-threshold
/// `WHERE`, and the deterministic sort + row limit.
///
/// The anchor (`?` placeholder) is applied to EVERY CTE that touches
/// `commits.date` so back-test mode (`--age-time-now <past>`) gets a
/// temporally-isolated view: `author_last_commit`, `live_paths`, and
/// `per_path_author` must all share the same `commits.date <= anchor`
/// filter, or authors who commit after the anchor leak in (negative
/// `days_since_main_active`), files deleted after the anchor look
/// "still deleted" instead of live-as-of-anchor, and LoC ownership counts
/// post-anchor contributions.
///
/// Bind order: `[anchor, anchor, min_revs, anchor, substantial_threshold,
/// anchor, anchor, departed_threshold_days, row_limit]`.
fn knowledge_islands_sql() -> String {
    let per_path_author = per_path_author_cte("live_paths");
    format!(
        "WITH {AUTHOR_LAST_COMMIT_CTE},
        live_paths AS (
            -- (rev, path) is the changes PK, so COUNT(c.rev) ==
            -- COUNT(DISTINCT c.rev) per path — the same `--min-revs`
            -- floor every other per-path analysis honors (see
            -- hotspots.rs / revisions.rs), applied here so single-commit
            -- files don't escape the floor just because this CTE's
            -- primary job is liveness detection.
            SELECT path FROM (
                SELECT c.path,
                       arg_max(
                           c.change_type,
                           ROW(commits.date, -commits.rowid)
                       ) AS change_type,
                       COUNT(c.rev) AS revs
                FROM changes c
                INNER JOIN commits ON commits.rev = c.rev
                WHERE commits.date <= CAST(? AS TIMESTAMP)
                GROUP BY c.path
                HAVING revs >= ?
            ) WHERE change_type != 'deleted'
        ),
        {per_path_author},
        {TOTALS_CTE},
        {MAIN_PER_PATH_CTE},
        {SUBSTANTIAL_OTHERS_CTE}
        {OWNER_ACTIVITY_SELECT_CORE}
        WHERE DATE_DIFF('day', alc.last_at, CAST(? AS TIMESTAMP)) > ?
        ORDER BY ownership_pct DESC, days_since_main_active DESC, entity ASC
        LIMIT ?"
    )
}

#[tracing::instrument(name = "knowledge-islands", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_knowledge_islands(db: &FactsDb, opts: &Options) -> Result<Vec<KnowledgeIslandRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let anchor_str = anchor_timestamp(opts);
    let substantial_threshold = crate::constants::DEFAULT_SUBSTANTIAL_OWNER_THRESHOLD;
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(&knowledge_islands_sql(), opts);
    super::query::explain_if_requested(
        db,
        &sql,
        params![
            anchor_str,                   // [1] author_last_commit CTE WHERE
            anchor_str,                   // [2] live_paths inner SELECT WHERE
            opts.min_revs,                // [3] live_paths HAVING revs >= ?
            anchor_str,                   // [4] per_path_author WHERE
            substantial_threshold,        // [5] substantial_others CASE
            anchor_str,                   // [6] main SELECT DATE_DIFF for days_since
            anchor_str,                   // [7] WHERE DATE_DIFF (departure filter)
            opts.departed_threshold_days, // [8]
            row_limit,                    // [9]
        ],
        "knowledge-islands",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        &sql,
        params![
            anchor_str,                   // [1] author_last_commit CTE WHERE
            anchor_str,                   // [2] live_paths inner SELECT WHERE
            opts.min_revs,                // [3] live_paths HAVING revs >= ?
            anchor_str,                   // [4] per_path_author WHERE
            substantial_threshold,        // [5] substantial_others CASE
            anchor_str,                   // [6] main SELECT DATE_DIFF for days_since
            anchor_str,                   // [7] WHERE DATE_DIFF (departure filter)
            opts.departed_threshold_days, // [8]
            row_limit,                    // [9]
        ],
        "knowledge-islands",
        |r| {
            Ok(KnowledgeIslandRow {
                entity: r.get::<_, String>(0)?,
                main_author: r.get::<_, String>(1)?,
                ownership_pct: r.get::<_, f64>(2)?,
                days_since_main_active: i32::try_from(r.get::<_, i64>(3)?).unwrap_or(i32::MAX),
                last_main_author_commit: r.get::<_, String>(4)?,
                n_substantial_others: r.get::<_, u32>(5)?,
            })
        },
    )
}

/// Count files present at HEAD — the live-tree prevalence denominator.
///
/// Mirrors the `live_paths` liveness rule (`arg_max` of the latest
/// `change_type` per path, keeping non-`deleted`), but *without* the
/// `--min-revs` gate `run_knowledge_islands` applies to its island rows.
/// The Knowledge factor's fallback tile divides departed islands (already
/// `--min-revs`-filtered) by this full live-file count, so applying the gate
/// here too would compare unlike populations and skew the ratio.
///
/// Returns `0` on an empty store (no `changes` rows).
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` prepare, query, or
/// row-mapping failure.
pub fn count_live_files(db: &FactsDb) -> Result<u64> {
    db.query_row(
        "SELECT COUNT(*) FROM (
            SELECT c.path,
                   arg_max(
                       c.change_type,
                       ROW(commits.date, -commits.rowid)
                   ) AS change_type
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
            GROUP BY c.path
        ) WHERE change_type != 'deleted'",
        [],
        |r| r.get::<_, u64>(0),
    )
}

/// Batched, un-thresholded owner-activity lookup for an arbitrary set of
/// paths: the same ownership/activity primitive [`run_knowledge_islands`]
/// computes internally, but returned for every path with LoC-attributable
/// ownership — not just the ones already past `departed_threshold_days`.
/// The caller (e.g. a pre-write briefing) compares
/// `days_since_main_active` to its own threshold.
///
/// A path with no LoC-attributable ownership (never touched, binary-only,
/// or fully bot-owned) has no entry in the returned map — treat a missing
/// entry as **Inconclusive**, never as "not departed".
///
/// Bound via a `VALUES (?), (?), ...` clause rather than one query per
/// path or string-interpolated path literals (the `marginal_owner_risk`
/// batching precedent). Empty `paths` short-circuits to an empty map
/// without querying.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` prepare, query, or
/// row-mapping failure.
pub fn owner_activity_for_paths(
    db: &FactsDb,
    opts: &Options,
    paths: &[String],
) -> Result<HashMap<String, OwnerActivity>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }

    crate::analyses::lineage::materialize_if_needed(db, opts)?;

    let anchor_str = anchor_timestamp(opts);
    let substantial_threshold = crate::constants::DEFAULT_SUBSTANTIAL_OWNER_THRESHOLD;
    let placeholders = std::iter::repeat_n("(?)", paths.len())
        .collect::<Vec<_>>()
        .join(",");
    let per_path_author = per_path_author_cte("requested_paths");
    let sql = format!(
        "WITH requested_paths(path) AS (VALUES {placeholders}),
        {AUTHOR_LAST_COMMIT_CTE},
        {per_path_author},
        {TOTALS_CTE},
        {MAIN_PER_PATH_CTE},
        {SUBSTANTIAL_OTHERS_CTE}
        {OWNER_ACTIVITY_SELECT_CORE}"
    );
    let sql = crate::analyses::lineage::rewrite(&sql, opts);

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("owner_activity_for_paths prepare: {e}")))?;

    let mut bind_params: Vec<&dyn duckdb::ToSql> =
        paths.iter().map(|p| p as &dyn duckdb::ToSql).collect();
    bind_params.push(&anchor_str); // author_last_commit CTE WHERE
    bind_params.push(&anchor_str); // per_path_author WHERE
    bind_params.push(&substantial_threshold); // substantial_others CASE
    bind_params.push(&anchor_str); // SELECT DATE_DIFF for days_since

    let rows = stmt
        .query_map(bind_params.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                OwnerActivity {
                    main_author: r.get::<_, String>(1)?,
                    ownership_pct: r.get::<_, f64>(2)?,
                    days_since_main_active: i32::try_from(r.get::<_, i64>(3)?).unwrap_or(i32::MAX),
                    last_main_author_commit: r.get::<_, String>(4)?,
                    n_substantial_others: r.get::<_, u32>(5)?,
                },
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("owner_activity_for_paths query: {e}")))?;

    rows.collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("owner_activity_for_paths collect: {e}")))
}
