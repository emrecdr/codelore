//! GitHub-Flavored Markdown (GFM) summary emitter for
//! `$GITHUB_STEP_SUMMARY` and other CI surfaces with a strict size /
//! script-tag-sanitization budget.
//!
//! **Why a separate emitter and not a `--format spa --embed` HTML
//! fragment**: GitHub Step Summary has a documented 1 MB cap (oversize
//! summaries are silently dropped, not error). The full SPA HTML is
//! ~1.3 MB on a small repo — dominated by the embedded `ECharts` (~1.1
//! MB) + `d3-hierarchy` libraries. Even stripping the header / footer
//! chrome would save <10 KB, far short of the budget. And GitHub
//! sanitizes `<script>` tags from rendered Step Summaries, so even if
//! the bundle shrank under cap the charts wouldn't render. The right
//! adaptation is a different output: GFM with tables, emoji, unicode
//! bar charts, and `<details>` collapsibles — all of which GitHub
//! renders natively. Typical output is 5–15 KB; comfortably under cap.
//!
//! Output shape: title → repo + timestamp → KPI table → top-N
//! hotspots → MI band breakdown → coupling density line → knowledge
//! islands `<details>` block. All sections degrade gracefully on empty
//! inputs (small fixtures, analyses that returned no rows, etc).

use std::io::Write;

use crate::analyses::mi::MiBand;
use crate::output::markdown::escape_md_cell;
use crate::output::spa::SpaDashboard;
use crate::{CodeLoreError, Result};

/// Maximum number of hotspot rows shown in the top-N table. Keeps the
/// output well under GitHub's 1 MB Step Summary cap on extreme repos.
const TOP_HOTSPOTS_LIMIT: usize = 10;

/// Maximum number of knowledge-island rows surfaced inside the
/// `<details>` block. Larger lists collapse to a "+N more" footer.
const TOP_ISLANDS_LIMIT: usize = 10;

/// Render the dashboard data as a GFM Markdown summary and write it to
/// `w`. Signature mirrors [`crate::output::spa::write_spa`] so the
/// dispatch in `codelore-cli` can swap one writer for the other.
///
/// # Errors
///
/// Propagates I/O errors from `w` as [`CodeLoreError::Io`].
pub fn write_step_summary<W: Write>(
    dash: &SpaDashboard,
    title: &str,
    repo_path: &str,
    generated_at: &str,
    w: &mut W,
) -> Result<()> {
    let io = CodeLoreError::Io;
    writeln!(w, "# 🔍 {title}").map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(w, "_Repo: `{repo_path}` · Generated: {generated_at}_").map_err(io)?;
    writeln!(w).map_err(io)?;

    write_at_a_glance(dash, w)?;
    write_top_hotspots(dash, w)?;
    write_mi_bands(dash, w)?;
    write_coupling_density(dash, w)?;
    write_knowledge_islands(dash, w)?;

    writeln!(w).map_err(io)?;
    writeln!(
        w,
        "_See the full interactive dashboard (`--format spa`) for charts, drill-down drawers, and theme controls._"
    )
    .map_err(io)?;
    Ok(())
}

fn write_at_a_glance<W: Write>(dash: &SpaDashboard, w: &mut W) -> Result<()> {
    let io = CodeLoreError::Io;
    writeln!(w, "## 📊 At a glance").map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(w, "| Metric | Value |").map_err(io)?;
    writeln!(w, "|---|---|").map_err(io)?;
    for row in &dash.summary {
        writeln!(w, "| {} | {} |", titlecase(&row.metric), row.value).map_err(io)?;
    }
    let coupling_pairs = dash.coupling.len();
    if coupling_pairs > 0 {
        writeln!(w, "| Significant coupling pairs | {coupling_pairs} |").map_err(io)?;
    }
    if let Some(top) = dash.hotspots.first() {
        writeln!(
            w,
            "| Top hotspot | `{}` (score {:.2}) |",
            escape_md_cell(&top.path),
            top.hotspot_score
        )
        .map_err(io)?;
    }
    // Repo-wide AI authorship percentage: AVG of per-file AI percentages
    // across hotspot rows that have a known value. Skip when no AI signal
    // is present (e.g. tiny fixture with no commits classified).
    let ai_values: Vec<f64> = dash
        .hotspots
        .iter()
        .filter_map(|r| r.ai_pct.filter(|p| p.is_finite()))
        .collect();
    if !ai_values.is_empty() {
        #[allow(clippy::cast_precision_loss)]
        // file count in the hotspot list is far below 2^52; the usize→f64 cast is exact
        let avg = ai_values.iter().sum::<f64>() / (ai_values.len() as f64);
        writeln!(w, "| AI authorship (avg across files) | {avg:.1}% |").map_err(io)?;
    }
    let island_count = dash.knowledge_islands.len();
    if island_count > 0 {
        writeln!(w, "| Knowledge islands | {island_count} |").map_err(io)?;
    }
    writeln!(w).map_err(io)?;
    Ok(())
}

fn write_top_hotspots<W: Write>(dash: &SpaDashboard, w: &mut W) -> Result<()> {
    let io = CodeLoreError::Io;
    let rows: Vec<_> = dash.hotspots.iter().take(TOP_HOTSPOTS_LIMIT).collect();
    if rows.is_empty() {
        return Ok(());
    }
    writeln!(w, "## 🔥 Top hotspots").map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(
        w,
        "| # | File | Score | Cognitive Health | Cognitive | MI |"
    )
    .map_err(io)?;
    writeln!(w, "|---|---|---|---|---|---|").map_err(io)?;
    for (i, row) in rows.iter().enumerate() {
        let mi_cell = match (row.mi, row.mi_rank) {
            (Some(v), Some(rank)) if rank.is_finite() => {
                format!("{v:.1} {}", band_emoji(MiBand::from_rank(rank)))
            }
            (Some(v), _) => format!("{v:.1}"),
            _ => "—".to_owned(),
        };
        writeln!(
            w,
            "| {n} | `{path}` | {score:.2} | {health:.1} | {cog:.0} | {mi} |",
            n = i + 1,
            path = escape_md_cell(&row.path),
            score = row.hotspot_score,
            health = row.cognitive_health,
            cog = row.cognitive,
            mi = mi_cell,
        )
        .map_err(io)?;
    }
    if dash.hotspots.len() > TOP_HOTSPOTS_LIMIT {
        writeln!(
            w,
            "\n_…and {} more in the full output._",
            dash.hotspots.len() - TOP_HOTSPOTS_LIMIT
        )
        .map_err(io)?;
    }
    writeln!(w).map_err(io)?;
    Ok(())
}

fn write_mi_bands<W: Write>(dash: &SpaDashboard, w: &mut W) -> Result<()> {
    let Some(rollup) = dash.mi_rollup else {
        return Ok(());
    };
    let known = rollup.known();
    if known == 0 && rollup.unknown == 0 {
        return Ok(());
    }
    let io = CodeLoreError::Io;
    writeln!(w, "## 🎯 Maintainability bands").map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(w, "| Band | Files | |").map_err(io)?;
    writeln!(w, "|---|---|---|").map_err(io)?;
    let max = rollup
        .low
        .max(rollup.moderate)
        .max(rollup.high)
        .max(rollup.unknown)
        .max(1);
    let bar = |n: usize| -> String { "▇".repeat((n * 20 + max / 2) / max) };
    writeln!(w, "| 🟢 High | {} | `{}` |", rollup.high, bar(rollup.high)).map_err(io)?;
    writeln!(
        w,
        "| 🟡 Moderate | {} | `{}` |",
        rollup.moderate,
        bar(rollup.moderate)
    )
    .map_err(io)?;
    writeln!(w, "| 🔴 Low | {} | `{}` |", rollup.low, bar(rollup.low)).map_err(io)?;
    if rollup.unknown > 0 {
        writeln!(
            w,
            "| ⚪ Unknown | {} | `{}` |",
            rollup.unknown,
            bar(rollup.unknown)
        )
        .map_err(io)?;
    }
    writeln!(w).map_err(io)?;
    writeln!(
        w,
        "_Bands are **repo-relative** percentile ranks within the analyzed repo's MI distribution — not the literature's absolute Coleman/SEI thresholds. See `docs/research-foundations.md`._"
    )
    .map_err(io)?;
    writeln!(w).map_err(io)?;
    Ok(())
}

fn write_coupling_density<W: Write>(dash: &SpaDashboard, w: &mut W) -> Result<()> {
    let Some(density) = dash.coupling_density else {
        return Ok(());
    };
    let io = CodeLoreError::Io;
    let label = density_label(density);
    writeln!(w, "## 🕸️ Behavioral coupling density").map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(w, "**{density:.4}** — _{label}_").map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(
        w,
        "_Ratio of Fisher-significant coupling pairs to the maximum possible pairs in the candidate node set. `<0.01` = sparsely coupled / well-modularised; `0.01–0.10` = typical; `>0.10` = tightly coupled or small cohesive codebase._"
    )
    .map_err(io)?;
    writeln!(w).map_err(io)?;
    Ok(())
}

fn write_knowledge_islands<W: Write>(dash: &SpaDashboard, w: &mut W) -> Result<()> {
    let io = CodeLoreError::Io;
    let total = dash.knowledge_islands.len();
    writeln!(w, "## 🧠 Knowledge islands").map_err(io)?;
    writeln!(w).map_err(io)?;
    if total == 0 {
        writeln!(
            w,
            "_None detected — no files are critically dependent on departed contributors._"
        )
        .map_err(io)?;
        writeln!(w).map_err(io)?;
        return Ok(());
    }
    writeln!(w, "<details>").map_err(io)?;
    writeln!(
        w,
        "<summary><strong>{total} island{plural} detected</strong> — files at risk from departed contributors</summary>",
        plural = if total == 1 { "" } else { "s" }
    )
    .map_err(io)?;
    writeln!(w).map_err(io)?;
    writeln!(
        w,
        "| File | Main author | Ownership | Days since active | Others ≥ threshold |"
    )
    .map_err(io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(io)?;
    for row in dash.knowledge_islands.iter().take(TOP_ISLANDS_LIMIT) {
        writeln!(
            w,
            "| `{}` | {} | {:.1}% | {} | {} |",
            escape_md_cell(&row.entity),
            escape_md_cell(&row.main_author),
            row.ownership_pct,
            row.days_since_main_active,
            row.n_substantial_others,
        )
        .map_err(io)?;
    }
    if total > TOP_ISLANDS_LIMIT {
        writeln!(w, "\n_…and {} more._", total - TOP_ISLANDS_LIMIT).map_err(io)?;
    }
    writeln!(w).map_err(io)?;
    writeln!(w, "</details>").map_err(io)?;
    writeln!(w).map_err(io)?;
    Ok(())
}

/// Pick the human-facing emoji for an MI band. Kept in sync with the
/// band → emoji mapping used in `write_mi_bands`.
fn band_emoji(band: MiBand) -> &'static str {
    match band {
        MiBand::High => "🟢",
        MiBand::Moderate => "🟡",
        MiBand::Low => "🔴",
    }
}

/// Map a coupling-graph density value to a short human-facing label.
/// Matches the band guidance in `coupling::density` docs.
fn density_label(d: f64) -> &'static str {
    if d < 0.01 {
        "sparsely coupled — well-modularised"
    } else if d < 0.10 {
        "modular — typical production codebase"
    } else {
        "tightly coupled — small/cohesive codebase or refactor candidate"
    }
}

/// Title-case the first character of a metric key (e.g. `commits` →
/// `Commits`) for display in the KPI table.
fn titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyses::coupling::CouplingRow;
    use crate::analyses::hotspots::HotspotRow;
    use crate::analyses::knowledge_islands::KnowledgeIslandRow;
    use crate::analyses::mi::MiRollup;
    use crate::analyses::summary::SummaryRow;
    use crate::output::spa::SpaDashboard;

    fn sample_dash() -> SpaDashboard {
        SpaDashboard {
            hotspots: vec![
                HotspotRow {
                    path: "src/main.rs".into(),
                    revisions: 35,
                    cognitive: 30.0,
                    cognitive_health: 60.0,
                    hotspot_score: 9.7,
                    mi: Some(-137.0),
                    mi_rank: Some(0.03),
                    ai_pct: None,
                },
                HotspotRow {
                    path: "src/lib.rs".into(),
                    revisions: 24,
                    cognitive: 14.0,
                    cognitive_health: 81.3,
                    hotspot_score: 3.9,
                    mi: Some(50.0),
                    mi_rank: Some(0.7),
                    ai_pct: None,
                },
            ],
            summary: vec![
                SummaryRow {
                    metric: "commits".into(),
                    value: 285,
                },
                SummaryRow {
                    metric: "authors".into(),
                    value: 1,
                },
            ],
            coupling: vec![CouplingRow {
                entity_a: "src/a.rs".into(),
                entity_b: "src/b.rs".into(),
                shared: 5,
                revs_a: 6,
                revs_b: 5,
                average_revs: 5,
                degree: 90.91,
                fisher_p: 0.0,
            }],
            mi_rollup: Some(MiRollup {
                low: 13,
                moderate: 23,
                high: 6,
                unknown: 25,
            }),
            coupling_density: Some(0.0275),
            knowledge_islands: vec![KnowledgeIslandRow {
                entity: "src/lonely.rs".into(),
                main_author: "Departed Dev".into(),
                ownership_pct: 92.0,
                days_since_main_active: 412,
                last_main_author_commit: "2025-01-15".into(),
                n_substantial_others: 0,
            }],
            ..SpaDashboard::default()
        }
    }

    fn render(dash: &SpaDashboard) -> String {
        let mut buf = Vec::new();
        write_step_summary(
            dash,
            "CodeLore Analysis",
            "/tmp/repo",
            "2026-06-13",
            &mut buf,
        )
        .expect("render");
        String::from_utf8(buf).expect("utf8")
    }

    #[test]
    fn emits_well_under_github_step_summary_cap() {
        let md = render(&sample_dash());
        assert!(
            md.len() < 50_000,
            "step summary should be < 50 KB; got {}",
            md.len()
        );
    }

    #[test]
    fn emits_title_and_metadata() {
        let md = render(&sample_dash());
        assert!(md.contains("# 🔍 CodeLore Analysis"));
        assert!(md.contains("Repo: `/tmp/repo`"));
        assert!(md.contains("Generated: 2026-06-13"));
    }

    #[test]
    fn emits_top_hotspots_with_mi_band_emoji() {
        let md = render(&sample_dash());
        assert!(md.contains("Top hotspots"));
        assert!(md.contains("`src/main.rs`"));
        // mi_rank 0.03 → bottom quartile → Low band → 🔴
        assert!(
            md.contains("🔴"),
            "main.rs (rank 0.03) should display Low-band 🔴"
        );
        // mi_rank 0.7 → Moderate → 🟡
        assert!(md.contains("🟡"));
    }

    #[test]
    fn emits_mi_band_breakdown_with_unicode_bars() {
        let md = render(&sample_dash());
        assert!(md.contains("Maintainability bands"));
        assert!(md.contains("🟢 High"));
        assert!(md.contains("🟡 Moderate"));
        assert!(md.contains("🔴 Low"));
        assert!(md.contains("▇"));
        assert!(md.contains("repo-relative"));
    }

    #[test]
    fn emits_coupling_density_with_band_label() {
        let md = render(&sample_dash());
        assert!(md.contains("0.0275"));
        // 0.0275 is in [0.01, 0.10) → "modular" label
        assert!(md.contains("modular"));
    }

    #[test]
    fn knowledge_islands_render_inside_details_collapsible() {
        let md = render(&sample_dash());
        assert!(md.contains("<details>"));
        assert!(md.contains("1 island detected"));
        assert!(md.contains("`src/lonely.rs`"));
    }

    #[test]
    fn knowledge_islands_empty_case() {
        let dash = SpaDashboard {
            knowledge_islands: vec![],
            ..sample_dash()
        };
        let md = render(&dash);
        assert!(md.contains("None detected"));
        // No <details> block when empty.
        assert!(!md.contains("<details>"));
    }

    #[test]
    fn omits_sections_when_data_absent() {
        let bare = SpaDashboard::default();
        let md = render(&bare);
        // KPI table renders (just empty body). Other sections skip.
        assert!(md.contains("# 🔍"));
        assert!(!md.contains("Top hotspots"));
        assert!(!md.contains("Maintainability bands"));
        assert!(!md.contains("Behavioral coupling density"));
        // Knowledge islands always render (with "none detected" message).
        assert!(md.contains("Knowledge islands"));
    }

    #[test]
    fn pipe_in_path_is_escaped_for_table_safety() {
        let mut dash = sample_dash();
        dash.hotspots[0].path = "src/a|b.rs".into();
        let md = render(&dash);
        assert!(md.contains(r"src/a\|b.rs"));
    }

    #[test]
    fn newline_in_author_cannot_forge_markdown_rows() {
        // An author name carrying a newline (crafted, or via a corrupt
        // mailmap) must not break out of its table cell and inject a heading
        // into the rendered $GITHUB_STEP_SUMMARY. escape_md_cell folds \n/\r
        // to the visual ↵ glyph so the row stays on one line.
        let mut dash = sample_dash();
        dash.knowledge_islands[0].main_author = "Real Dev\n# INJECTED HEADING".into();
        let md = render(&dash);
        assert!(
            !md.contains("\n# INJECTED HEADING"),
            "author newline forged a heading:\n{md}"
        );
        assert!(md.contains("Real Dev↵# INJECTED HEADING"));
    }

    #[test]
    fn density_label_thresholds() {
        assert_eq!(density_label(0.0), "sparsely coupled — well-modularised");
        assert_eq!(density_label(0.005), "sparsely coupled — well-modularised");
        assert_eq!(density_label(0.01), "modular — typical production codebase");
        assert_eq!(density_label(0.05), "modular — typical production codebase");
        assert_eq!(
            density_label(0.10),
            "tightly coupled — small/cohesive codebase or refactor candidate"
        );
    }
}
