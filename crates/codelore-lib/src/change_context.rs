//! Temporal pre-write briefing: what an agent should know about a set of
//! files *before* it edits them.
//!
//! [`build_change_context`] assembles a compact, deterministic text briefing
//! for 1–[`MAX_BRIEFING_PATHS`] repo-relative paths. Like
//! [`crate::enrichment::fact_sheet`], it never computes anything new: it runs
//! the existing analyses once for the whole batch (`run_code_health`,
//! `run_hotspots`, `run_coupling`, `owner_activity_for_paths`, plus a windowed
//! churn query), then picks each path's values and renders them. It only
//! reads the fact store — it never writes to it and never perturbs any
//! analysis output.
//!
//! ## Determinism
//!
//! Two builds over the same fact store produce byte-identical text: every
//! float renders through a fixed rounding, blocks appear in request order, and
//! nothing iterates a `HashMap` into the output (co-change partners are sorted
//! into a `Vec` during assembly; per-path owner / churn data is looked up by
//! key, never enumerated).
//!
//! ## Rendered format (the output contract)
//!
//! When the repository is partway through a merge / rebase / cherry-pick /
//! revert (`Repo::merge_or_rebase_in_progress`), one leading note precedes
//! everything, separated from the first block by a blank line:
//!
//! ```text
//! note: merge/rebase in progress — briefing reflects committed HEAD history
//! ```
//!
//! Each requested path renders one block; blocks are separated by a single
//! blank line. A path with recorded history renders five indented lines, plus
//! an optional sixth `→` next-action line:
//!
//! ```text
//! crates/codelore-lib/src/cache.rs
//!   health 67.3 (yellow) · risk 0.42 · calibrated defects-2026-07-15
//!   hotspot #12 (score 0.67, 23 revs)
//!   co-change: options.rs (68%, p=0.003) · facts/mod.rs (54%, p=0.011)
//!   owner: Emre Camdere 82% (sole owner, active 12d ago)
//!   recent: 4 commits, 310 lines churned in last 90d
//!   → historically co-changes with options.rs — consider the same edit there
//! ```
//!
//! Each line falls back to an honest-absence form when its data is missing:
//!
//! - `health` — score to one decimal + band, `risk` (`structural_risk`) to two
//!   decimals, and a calibration suffix (`calibrated <vintage>` from
//!   `active_vintage`, else `uncalibrated`). A path with no code-health row but
//!   some other history renders `health: no code-health row`.
//! - `hotspot #<rank> (score <score>, <revs> revs)` — `rank` is the 1-based
//!   position in the un-row-limited hotspot ranking; a path outside the ranking
//!   renders `not in the hotspot set`.
//! - `co-change:` — up to three partners with `shared >= DEFAULT_MIN_SHARED_REVS`
//!   and `fisher_p < DEFAULT_FISHER_SIGNIFICANCE`, sorted by `degree` descending
//!   then partner path ascending; when more partners clear the filter than are
//!   rendered, the line ends with ` (+n more)` disclosing the overflow count;
//!   none renders `co-change: none significant`.
//! - `owner:` — main author, ownership share, `sole owner` / `shared`
//!   concentration, and `departed <n>d` when `days_since_main_active` exceeds
//!   `departed_threshold_days` (else `active <n>d ago`). A path with no
//!   attributable ownership renders `owner: inconclusive`.
//! - `recent:` — commit count + churned lines over the last `window_days`; a
//!   path untouched in the window renders `recent: quiet in last <window_days>d`.
//! - `→` (optional final line) — a single next action derived from the picked
//!   values: co-change partners surface the top partner to edit alongside; with
//!   no partners, a main author past `departed_threshold_days` surfaces a
//!   knowledge-continuity flag. A block with neither carries no action line.
//!
//! A path absent from *every* feed — no code-health row, not in the hotspot
//! ranking, no significant co-change partners, no attributable ownership, and
//! untouched in the churn window (a brand-new, untracked, or mistyped path) —
//! renders a two-line block instead:
//!
//! ```text
//! brand/new.rs
//!   no history at HEAD (new or untracked file)
//! ```

use std::collections::HashMap;

use crate::analyses::code_health::run_code_health;
use crate::analyses::coupling::{CouplingRow, run_coupling};
use crate::analyses::hotspots::run_hotspots;
use crate::analyses::knowledge_islands::{OwnerActivity, owner_activity_for_paths};
use crate::analyses::lineage;
use crate::constants::{DEFAULT_FISHER_SIGNIFICANCE, DEFAULT_MIN_SHARED_REVS};
use crate::defect_calibration::active_vintage;
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::{CodeLoreError, Options, Result};

/// Maximum number of paths a single briefing accepts. Keeps the assembled
/// text within the tool's token budget and bounds the per-batch query cost.
pub const MAX_BRIEFING_PATHS: usize = 20;

/// Number of co-change partners surfaced per path. Small enough to keep each
/// block scannable and within the token budget.
const MAX_PARTNERS: usize = 3;

/// One path's picked-and-formatted-ready values — the intermediate between
/// assembly (which runs the analyses) and rendering (which is pure). Every
/// field is `Option` / `Vec` so the renderer can emit each line's
/// honest-absence form without re-querying.
struct PathBriefing {
    /// Repo-relative path this block describes.
    path: String,
    /// `(score, band, structural_risk)` when the path has a code-health row.
    health: Option<(f64, String, f64)>,
    /// Active defect-calibration vintage (repo-wide); `None` when uncalibrated.
    calibration: Option<String>,
    /// `(1-based rank, hotspot_score, revisions)` when the path is ranked.
    hotspot: Option<(usize, f64, u32)>,
    /// `(partner, degree, fisher_p)`, already filtered, sorted, and truncated.
    partners: Vec<(String, f64, f64)>,
    /// Count of partners that cleared the significance filter *before*
    /// truncation; when it exceeds the rendered count the co-change line
    /// discloses the overflow as ` (+n more)`.
    partners_total: usize,
    /// Owner activity when the path has LoC-attributable ownership.
    owner: Option<OwnerActivity>,
    /// `(revisions, churned_lines)` over the recent window when the path was
    /// touched in it.
    recent: Option<(u32, i64)>,
}

/// Assemble and render the pre-write briefing for `paths`.
///
/// Runs each feed analysis once for the whole batch (forcing `min_revs = 1`
/// and clearing the row limit so a briefing finds its targets regardless of
/// `--rows`), picks per-path values, and renders the deterministic text
/// described in the module docs. Reads the fact store only.
///
/// # Errors
///
/// Returns [`CodeLoreError::InvalidOptions`] (naming the
/// [`MAX_BRIEFING_PATHS`] limit) when `paths` is empty or longer than the
/// limit, and propagates any analysis or fact-store error from the feeds.
pub fn build_change_context<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    paths: &[String],
) -> Result<String> {
    if paths.is_empty() {
        return Err(CodeLoreError::InvalidOptions(format!(
            "change_context needs at least 1 path (limit {MAX_BRIEFING_PATHS})"
        )));
    }
    if paths.len() > MAX_BRIEFING_PATHS {
        return Err(CodeLoreError::InvalidOptions(format!(
            "change_context accepts at most {MAX_BRIEFING_PATHS} paths, got {}",
            paths.len()
        )));
    }

    let briefings = assemble(db, opts, paths)?;
    let merge_note = repo.merge_or_rebase_in_progress();
    Ok(render(&briefings, merge_note, opts))
}

/// Run the feeds once and pick each path's values into a [`PathBriefing`].
fn assemble(db: &FactsDb, opts: &Options, paths: &[String]) -> Result<Vec<PathBriefing>> {
    // A briefing must find its targets regardless of the caller's cosmetic
    // row cap or revision floor — mirror the fact-sheet precedent.
    let opts = {
        let mut o = opts.with_no_row_limit();
        o.min_revs = 1;
        o
    };

    let health_rows = run_code_health(db, &opts)?;
    let hotspot_rows = run_hotspots(db, &opts)?;
    let coupling_rows = run_coupling(db, &opts)?;
    let owners = owner_activity_for_paths(db, &opts, paths)?;
    let churn = window_churn_for_paths(db, &opts, paths)?;
    // Repo-wide: read once, stamp the same vintage on every block.
    let vintage = active_vintage(&opts)?;

    let briefings = paths
        .iter()
        .map(|path| {
            let health = health_rows
                .iter()
                .find(|r| r.path == *path)
                .map(|r| (r.score, r.band.clone(), r.structural_risk));
            let hotspot = hotspot_rows
                .iter()
                .enumerate()
                .find(|(_, r)| r.path == *path)
                .map(|(idx, r)| (idx + 1, r.hotspot_score, r.revisions));
            let (partners, partners_total) = coupling_partners(&coupling_rows, path);
            PathBriefing {
                path: path.clone(),
                health,
                calibration: vintage.clone(),
                hotspot,
                partners,
                partners_total,
                owner: owners.get(path).cloned(),
                recent: churn.get(path).copied(),
            }
        })
        .collect();
    Ok(briefings)
}

/// The path's Fisher-significant co-change partners, sorted by `degree`
/// descending then partner path ascending, capped at [`MAX_PARTNERS`] —
/// paired with the pre-truncation partner count so the renderer can disclose
/// how many significant partners the cap hid.
fn coupling_partners(rows: &[CouplingRow], path: &str) -> (Vec<(String, f64, f64)>, usize) {
    let mut partners: Vec<(&str, f64, f64)> = rows
        .iter()
        .filter_map(|r| {
            let (partner, shared, degree, fisher_p) = if r.entity_a == path {
                (r.entity_b.as_str(), r.shared, r.degree, r.fisher_p)
            } else if r.entity_b == path {
                (r.entity_a.as_str(), r.shared, r.degree, r.fisher_p)
            } else {
                return None;
            };
            (shared >= DEFAULT_MIN_SHARED_REVS && fisher_p < DEFAULT_FISHER_SIGNIFICANCE)
                .then_some((partner, degree, fisher_p))
        })
        .collect();
    partners.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let total = partners.len();
    partners.truncate(MAX_PARTNERS);
    let rendered = partners
        .into_iter()
        .map(|(partner, degree, fisher_p)| (partner.to_string(), degree, fisher_p))
        .collect();
    (rendered, total)
}

/// Per-path commit count + churned lines over the last `opts.window_days`,
/// batched over `paths`. Uses the `cycle-health` window predicate anchored to
/// the repo's last commit date, with the path list bound (never interpolated).
///
/// Bind order is `window_days` first (the `INTERVAL` predicate), then the
/// paths (the `IN (VALUES …)` list) — the order the placeholders appear in the
/// SQL text.
fn window_churn_for_paths(
    db: &FactsDb,
    opts: &Options,
    paths: &[String],
) -> Result<HashMap<String, (u32, i64)>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }

    lineage::materialize_if_needed(db, opts)?;
    let src = lineage::source_table(opts);
    let placeholders = std::iter::repeat_n("(?)", paths.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT ch.path,
                CAST(COUNT(ch.rev) AS BIGINT) AS revs,
                CAST(COALESCE(SUM(ch.loc_added + ch.loc_deleted), 0) AS BIGINT) AS churn
         FROM {src} ch
         JOIN commits co ON co.rev = ch.rev
         WHERE co.date >= (SELECT MAX(date) FROM commits) - INTERVAL (?) DAY
           AND ch.path IN (VALUES {placeholders})
         GROUP BY ch.path"
    );

    let window_days = i64::from(opts.window_days);
    let mut binds: Vec<&dyn duckdb::ToSql> = Vec::with_capacity(paths.len() + 1);
    binds.push(&window_days);
    for path in paths {
        binds.push(path as &dyn duckdb::ToSql);
    }

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("change_context churn prepare: {e}")))?;
    let rows = stmt
        .query_map(binds.as_slice(), |r| {
            let path: String = r.get(0)?;
            let revs: i64 = r.get(1)?;
            let churn: i64 = r.get(2)?;
            Ok((path, (u32::try_from(revs).unwrap_or(u32::MAX), churn)))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("change_context churn query: {e}")))?;
    rows.collect::<std::result::Result<HashMap<_, _>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("change_context churn collect: {e}")))
}

/// Render the briefings deterministically. Pure over `briefings` / `opts` — no
/// I/O, no fact-store access; two calls with equal inputs produce equal text.
fn render(briefings: &[PathBriefing], merge_note: bool, opts: &Options) -> String {
    let mut chunks: Vec<String> = Vec::new();
    if merge_note {
        chunks.push(
            "note: merge/rebase in progress — briefing reflects committed HEAD history".to_string(),
        );
    }
    for briefing in briefings {
        chunks.push(render_block(briefing, opts));
    }
    chunks.join("\n\n")
}

/// One path's block: the two-line no-history form when the path is absent
/// from every feed, else the five indented feed lines plus an optional final
/// `→` action line ([`action_line`]). A path known to any feed — even just
/// ownership or a hotspot rank — renders the full block, with each missing feed
/// line in its honest-absence form.
fn render_block(briefing: &PathBriefing, opts: &Options) -> String {
    if briefing.health.is_none()
        && briefing.hotspot.is_none()
        && briefing.partners.is_empty()
        && briefing.owner.is_none()
        && briefing.recent.is_none()
    {
        return format!(
            "{}\n  no history at HEAD (new or untracked file)",
            briefing.path
        );
    }
    let mut lines = vec![
        briefing.path.clone(),
        format!("  {}", health_line(briefing)),
        format!("  {}", hotspot_line(briefing)),
        format!("  {}", cochange_line(briefing)),
        format!("  {}", owner_line(briefing, opts)),
        format!("  {}", recent_line(briefing, opts)),
    ];
    if let Some(action) = action_line(briefing, opts) {
        lines.push(format!("  {action}"));
    }
    lines.join("\n")
}

/// One optional next-action line for a block, derived only from data already
/// picked into the briefing — no new queries. Co-change partners are the most
/// actionable signal (edit the partner too), so they take priority; absent
/// those, a departed main author is the knowledge-continuity risk worth
/// flagging. A block with neither gets no line, so the briefing carries no
/// filler.
fn action_line(briefing: &PathBriefing, opts: &Options) -> Option<String> {
    if let Some((partner, _, _)) = briefing.partners.first() {
        return Some(format!(
            "\u{2192} historically co-changes with {partner} — consider the same edit there"
        ));
    }
    let owner = briefing.owner.as_ref()?;
    let threshold = i32::try_from(opts.departed_threshold_days).unwrap_or(i32::MAX);
    if owner.days_since_main_active > threshold {
        return Some(format!(
            "\u{2192} knowledge risk: main author {} has departed — line up a second reviewer",
            owner.main_author
        ));
    }
    None
}

fn health_line(briefing: &PathBriefing) -> String {
    match &briefing.health {
        Some((score, band, risk)) => {
            let calibration = match &briefing.calibration {
                Some(vintage) => format!("calibrated {vintage}"),
                None => "uncalibrated".to_string(),
            };
            format!("health {score:.1} ({band}) · risk {risk:.2} · {calibration}")
        }
        None => "health: no code-health row".to_string(),
    }
}

fn hotspot_line(briefing: &PathBriefing) -> String {
    match briefing.hotspot {
        Some((rank, score, revs)) => format!("hotspot #{rank} (score {score:.2}, {revs} revs)"),
        None => "not in the hotspot set".to_string(),
    }
}

fn cochange_line(briefing: &PathBriefing) -> String {
    if briefing.partners.is_empty() {
        return "co-change: none significant".to_string();
    }
    let parts: Vec<String> = briefing
        .partners
        .iter()
        .map(|(partner, degree, fisher_p)| format!("{partner} ({degree:.0}%, p={fisher_p:.3})"))
        .collect();
    let hidden = briefing.partners_total.saturating_sub(MAX_PARTNERS);
    if hidden > 0 {
        format!("co-change: {} (+{hidden} more)", parts.join(" · "))
    } else {
        format!("co-change: {}", parts.join(" · "))
    }
}

fn owner_line(briefing: &PathBriefing, opts: &Options) -> String {
    let Some(owner) = &briefing.owner else {
        return "owner: inconclusive".to_string();
    };
    let concentration = if owner.n_substantial_others == 0 {
        "sole owner"
    } else {
        "shared"
    };
    let threshold = i32::try_from(opts.departed_threshold_days).unwrap_or(i32::MAX);
    let activity = if owner.days_since_main_active > threshold {
        format!("departed {}d", owner.days_since_main_active)
    } else {
        format!("active {}d ago", owner.days_since_main_active)
    };
    format!(
        "owner: {} {:.0}% ({concentration}, {activity})",
        owner.main_author, owner.ownership_pct
    )
}

fn recent_line(briefing: &PathBriefing, opts: &Options) -> String {
    match briefing.recent {
        Some((revs, churn)) => {
            format!(
                "recent: {revs} commits, {churn} lines churned in last {}d",
                opts.window_days
            )
        }
        None => format!("recent: quiet in last {}d", opts.window_days),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PARTNERS, PathBriefing, cochange_line, render};
    use crate::Options;
    use crate::analyses::knowledge_islands::OwnerActivity;

    fn opts() -> Options {
        Options {
            window_days: 90,
            departed_threshold_days: 90,
            ..Options::default()
        }
    }

    fn owner(main_author: &str, ownership_pct: f64, days: i32, n_others: u32) -> OwnerActivity {
        OwnerActivity {
            main_author: main_author.to_string(),
            ownership_pct,
            days_since_main_active: days,
            last_main_author_commit: "2026-07-02".to_string(),
            n_substantial_others: n_others,
        }
    }

    fn populated() -> PathBriefing {
        PathBriefing {
            path: "crates/codelore-lib/src/cache.rs".to_string(),
            health: Some((67.3, "yellow".to_string(), 0.42)),
            calibration: Some("defects-2026-07-15".to_string()),
            hotspot: Some((12, 0.67, 23)),
            partners: vec![
                ("options.rs".to_string(), 68.0, 0.003),
                ("facts/mod.rs".to_string(), 54.0, 0.011),
            ],
            partners_total: 2,
            owner: Some(owner("Emre Camdere", 82.0, 12, 0)),
            recent: Some((4, 310)),
        }
    }

    #[test]
    fn renders_the_exact_block() {
        // The five feed lines plus the co-change next-action line (populated()
        // has significant partners, so the action line names the top one).
        let expected = "crates/codelore-lib/src/cache.rs\n  \
             health 67.3 (yellow) · risk 0.42 · calibrated defects-2026-07-15\n  \
             hotspot #12 (score 0.67, 23 revs)\n  \
             co-change: options.rs (68%, p=0.003) · facts/mod.rs (54%, p=0.011)\n  \
             owner: Emre Camdere 82% (sole owner, active 12d ago)\n  \
             recent: 4 commits, 310 lines churned in last 90d\n  \
             \u{2192} historically co-changes with options.rs — consider the same edit there";
        assert_eq!(render(&[populated()], false, &opts()), expected);
    }

    #[test]
    fn action_line_names_the_top_cochange_partner() {
        // options.rs has the higher degree, so it is the partner surfaced.
        let out = render(&[populated()], false, &opts());
        assert!(
            out.contains("\u{2192} historically co-changes with options.rs"),
            "the action line surfaces the top co-change partner: {out}"
        );
    }

    #[test]
    fn action_line_flags_departed_owner_when_no_partners() {
        // No significant partners, but the main author is past the departed
        // threshold → the knowledge-risk action line, not co-change.
        let mut briefing = populated();
        briefing.partners = Vec::new();
        briefing.partners_total = 0;
        briefing.owner = Some(owner("Ada Departed", 55.0, 400, 0));
        let out = render(&[briefing], false, &opts());
        assert!(
            out.contains("\u{2192} knowledge risk: main author Ada Departed has departed"),
            "a departed sole author with no partners triggers the knowledge-risk line: {out}"
        );
    }

    #[test]
    fn no_action_line_without_partners_or_departed_owner() {
        // Present but active owner, no partners → nothing actionable, no filler.
        let mut briefing = populated();
        briefing.partners = Vec::new();
        briefing.partners_total = 0;
        briefing.owner = Some(owner("Present Owner", 70.0, 5, 0));
        let out = render(&[briefing], false, &opts());
        assert!(
            !out.contains('\u{2192}'),
            "no partners and an active owner means no action line: {out}"
        );
    }

    #[test]
    fn uncalibrated_suffix_when_no_vintage() {
        let mut briefing = populated();
        briefing.calibration = None;
        let out = render(&[briefing], false, &opts());
        assert!(
            out.contains("· uncalibrated"),
            "health line must end with the uncalibrated marker: {out}"
        );
        assert!(
            !out.contains("calibrated defects"),
            "no vintage leaked: {out}"
        );
    }

    #[test]
    fn no_history_block_is_two_lines() {
        let briefing = PathBriefing {
            path: "brand/new.rs".to_string(),
            health: None,
            calibration: None,
            hotspot: None,
            partners: Vec::new(),
            partners_total: 0,
            owner: None,
            recent: None,
        };
        assert_eq!(
            render(&[briefing], false, &opts()),
            "brand/new.rs\n  no history at HEAD (new or untracked file)"
        );
    }

    #[test]
    fn owner_only_path_renders_the_five_line_block() {
        let briefing = PathBriefing {
            path: "docs/advanced-usage.md".to_string(),
            health: None,
            calibration: None,
            hotspot: None,
            partners: Vec::new(),
            partners_total: 0,
            owner: Some(owner("Emre Camdere", 91.0, 30, 0)),
            recent: None,
        };
        let out = render(&[briefing], false, &opts());
        assert!(
            !out.contains("no history at HEAD"),
            "a path with ownership is not unknown: {out}"
        );
        assert_eq!(
            out.lines().count(),
            6,
            "path line plus the five feed lines: {out}"
        );
        assert!(
            out.contains("health: no code-health row"),
            "health falls back to its honest-absence form: {out}"
        );
        assert!(
            out.contains("owner: Emre Camdere 91% (sole owner, active 30d ago)"),
            "the assembled owner row must survive: {out}"
        );
    }

    #[test]
    fn inconclusive_owner_when_ownership_absent() {
        let mut briefing = populated();
        briefing.owner = None;
        let out = render(&[briefing], false, &opts());
        assert!(
            out.contains("owner: inconclusive"),
            "missing ownership must render inconclusive: {out}"
        );
    }

    #[test]
    fn departed_owner_flag_past_threshold() {
        let mut briefing = populated();
        briefing.owner = Some(owner("Ada Departed", 50.0, 400, 1));
        let out = render(&[briefing], false, &opts());
        assert!(
            out.contains("owner: Ada Departed 50% (shared, departed 400d)"),
            "past-threshold main author must flag departed + shared: {out}"
        );
    }

    #[test]
    fn merge_note_leads_the_briefing() {
        let out = render(&[populated()], true, &opts());
        assert!(
            out.starts_with(
                "note: merge/rebase in progress — briefing reflects committed HEAD history\n\n"
            ),
            "merge note must lead, separated by a blank line: {out}"
        );
        assert!(
            out.contains("crates/codelore-lib/src/cache.rs"),
            "the block still follows the note: {out}"
        );
    }

    #[test]
    fn token_budget_holds_for_three_maximal_paths() {
        let make = |path: &str| PathBriefing {
            path: path.to_string(),
            health: Some((12.3, "red".to_string(), 0.98)),
            calibration: Some("defects-2026-07-15".to_string()),
            hotspot: Some((7, 0.91, 128)),
            partners: vec![
                (
                    "crates/codelore-lib/src/analyses/coupling.rs".to_string(),
                    88.0,
                    0.001,
                ),
                (
                    "crates/codelore-lib/src/analyses/hotspots.rs".to_string(),
                    71.0,
                    0.004,
                ),
                (
                    "crates/codelore-lib/src/enrichment/fact_sheet.rs".to_string(),
                    63.0,
                    0.009,
                ),
            ],
            // Five significant partners, three rendered — the maximal
            // co-change line carries the ` (+2 more)` disclosure suffix.
            partners_total: 5,
            owner: Some(owner("Some Long Author Name", 74.0, 365, 2)),
            recent: Some((41, 9310)),
        };
        assert_eq!(make("a").partners.len(), MAX_PARTNERS);
        let briefings = [
            make("crates/codelore-lib/src/change_context.rs"),
            make("crates/codelore-cli/src/mcp.rs"),
            make("crates/codelore-lib/src/analyses/knowledge_islands.rs"),
        ];
        let out = render(&briefings, true, &opts());
        assert!(
            out.contains("(+2 more)"),
            "the disclosure suffix must render and be counted in the budget: {out}"
        );
        let tokens = out.split_whitespace().count();
        assert!(
            tokens <= 150 * briefings.len(),
            "budget is 150 tokens/path; got {tokens} for {} paths",
            briefings.len()
        );
    }

    #[test]
    fn cochange_line_discloses_truncated_partners() {
        let mut briefing = populated();
        briefing.partners = vec![
            ("a.rs".to_string(), 90.0, 0.001),
            ("b.rs".to_string(), 80.0, 0.002),
            ("c.rs".to_string(), 70.0, 0.003),
        ];
        briefing.partners_total = 6;
        let line = cochange_line(&briefing);
        assert!(
            line.contains("(+3 more)"),
            "three of six significant partners rendered must disclose the other three: {line}"
        );
    }
}
