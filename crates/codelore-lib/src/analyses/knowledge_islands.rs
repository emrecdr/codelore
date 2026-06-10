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
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeIslandRow {
    pub entity: String,
    pub main_author: String,
    pub ownership_pct: f64,
    pub days_since_main_active: i32,
    pub last_main_author_commit: String,
    pub n_substantial_others: u32,
}

/// SQL pipeline:
///
/// 1. `author_last_commit` — per author's MAX(commit date) ANYWHERE.
/// 2. `live_paths` — files whose most-recent change isn't `'deleted'`
///    (same F16 pattern used in `code_age` / `entity-churn`).
/// 3. `per_path_author` — per `(path, author)`: SUM(loc_added).
/// 4. `totals` — per path: SUM of all authors' LoC.
/// 5. `main_per_path` — per path: pick the author with max LoC
///    (deterministic alphabetical tiebreak via `ROW_NUMBER`).
/// 6. `substantial_others` — per path: count of non-main authors with
///    LoC share ≥ the substantial threshold.
/// 7. Final SELECT: join + project + filter by
///    `days_since_main_active > ?`.
///
/// Bind values: `[substantial_threshold, anchor, anchor,
/// departed_threshold_days, row_limit]`.
const SQL: &str = "
    WITH author_last_commit AS (
        SELECT canonical_author AS author, MAX(date) AS last_at
        FROM commits
        GROUP BY canonical_author
    ),
    live_paths AS (
        SELECT path FROM (
            SELECT c.path, c.change_type,
                   ROW_NUMBER() OVER (
                       PARTITION BY c.path
                       ORDER BY commits.date DESC, commits.rowid ASC
                   ) AS rn
            FROM changes c
            INNER JOIN commits ON commits.rev = c.rev
        ) WHERE rn = 1 AND change_type != 'deleted'
    ),
    per_path_author AS (
        -- Improvement: filter bots BEFORE aggregating ownership. Bots
        -- (dependabot, renovate, etc.) don't have knowledge to lose —
        -- flagging dependabot-dominated lockfiles as 'knowledge islands'
        -- is exactly the kind of false positive that destroys signal
        -- credibility. The LEFT JOIN tolerates authors without an
        -- author_aliases row (legacy / pattern-only classifications).
        SELECT
            changes.path,
            commits.canonical_author AS author,
            SUM(COALESCE(changes.loc_added, 0)) AS loc
        FROM changes
        INNER JOIN commits ON changes.rev = commits.rev
        INNER JOIN live_paths USING (path)
        LEFT JOIN author_aliases aa ON aa.canonical = commits.canonical_author
        WHERE COALESCE(aa.is_bot, FALSE) = FALSE
        GROUP BY changes.path, commits.canonical_author
    ),
    totals AS (
        -- Improvement: filter total_loc > 0 here so binary-only files
        -- (no LoC tracking) AND fully-bot-owned files (everyone filtered
        -- out above) don't propagate downstream as NULL-ownership rows.
        SELECT path, SUM(loc) AS total_loc
        FROM per_path_author
        GROUP BY path
        HAVING SUM(loc) > 0
    ),
    main_per_path AS (
        SELECT path, author, loc
        FROM (
            SELECT
                path, author, loc,
                ROW_NUMBER() OVER (
                    PARTITION BY path
                    ORDER BY loc DESC, author ASC
                ) AS rn
            FROM per_path_author
        ) WHERE rn = 1
    ),
    substantial_others AS (
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
    )
    SELECT
        m.path AS entity,
        m.author AS main_author,
        100.0 * m.loc / NULLIF(t.total_loc, 0) AS ownership_pct,
        DATE_DIFF('day', alc.last_at, CAST(? AS TIMESTAMP)) AS days_since_main_active,
        CAST(CAST(alc.last_at AS DATE) AS TEXT) AS last_main_author_commit,
        so.n_others AS n_substantial_others
    FROM main_per_path m
    INNER JOIN totals t ON t.path = m.path
    INNER JOIN author_last_commit alc ON alc.author = m.author
    INNER JOIN substantial_others so ON so.path = m.path
    WHERE DATE_DIFF('day', alc.last_at, CAST(? AS TIMESTAMP)) > ?
    ORDER BY ownership_pct DESC, days_since_main_active DESC, entity ASC
    LIMIT ?
";

pub fn run_knowledge_islands(db: &FactsDb, opts: &Options) -> Result<Vec<KnowledgeIslandRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    // Anchor for "departed" calculation. Re-uses `--age-time-now` when set
    // (so back-test pattern works: "who had-departed as of June 2024?"),
    // otherwise the current instant.
    let anchor_str = if let Some(d) = opts.age_time_now {
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
    };
    let substantial_threshold = crate::constants::DEFAULT_SUBSTANTIAL_OWNER_THRESHOLD;
    crate::analyses::lineage::materialize_if_needed(db, opts)?;
    let sql = crate::analyses::lineage::rewrite(SQL, opts);
    super::query::explain_if_requested(
        db,
        &sql,
        params![
            substantial_threshold,
            anchor_str,
            anchor_str,
            opts.departed_threshold_days,
            row_limit,
        ],
        "knowledge-islands",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        &sql,
        params![
            substantial_threshold,
            anchor_str,
            anchor_str,
            opts.departed_threshold_days,
            row_limit,
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
