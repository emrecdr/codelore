//! Markdown output emitter for `$GITHUB_STEP_SUMMARY` and human-readable
//! CI artifacts.

use crate::analyses::{
    authors::AuthorsRow,
    churn::{AbsChurnRow, AuthorChurnRow, EntityChurnRow},
    clone_coupling::CloneCouplingRow,
    clones::ClonesRow,
    code_age::CodeAgeRow,
    code_health::CodeHealthRow,
    communication::CommunicationRow,
    coupling::CouplingRow,
    hotspots::HotspotRow,
    ownership::OwnershipRow,
    summary::SummaryRow,
};
use crate::{CodeLoreError, Result};
use std::borrow::Cow;
use std::io::Write;

fn header<W: Write>(w: &mut W, title: &str) -> Result<()> {
    writeln!(w, "# {title}").map_err(CodeLoreError::Io)?;
    writeln!(w).map_err(CodeLoreError::Io)?;
    Ok(())
}

/// Escape a string for safe inclusion inside a GFM table cell.
///
/// Per the GFM spec, an unescaped `|` inside a cell ends the cell —
/// every subsequent character on the line lands in the next column,
/// so a single stray pipe in a path / author name / commit message
/// silently corrupts the entire row's column alignment. The escape is
/// a leading backslash on the pipe character.
///
/// Newlines and carriage returns inside a cell are GFM-unsupported
/// entirely (the table row terminates at the line break), so they get
/// replaced with the visual `↵` glyph to preserve the cell boundary
/// rather than break the row.
///
/// Returns `Cow::Borrowed` for the common case (no escape needed) so
/// the happy path is allocation-free. The borrow checker thread-of-
/// life this preserves matters here — every analysis emits hundreds
/// to thousands of rows and the markdown emitter is a hot path for
/// the `$GITHUB_STEP_SUMMARY` flow.
#[must_use]
pub fn escape_md_cell(s: &str) -> Cow<'_, str> {
    if !s.contains('|') && !s.contains('\n') && !s.contains('\r') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '|' => out.push_str("\\|"),
            '\n' | '\r' => out.push('↵'),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

pub fn write_revisions_markdown<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    header(w, "CodeLore revisions")?;
    writeln!(w, "| Entity | Revisions |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|").map_err(CodeLoreError::Io)?;
    for (path, n) in rows {
        writeln!(w, "| `{}` | {n} |", escape_md_cell(path)).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `hotspot-velocity` markdown emitter — files ranked by change
/// acceleration (heating up first).
pub fn write_hotspot_velocity_markdown<W: Write>(
    rows: &[crate::analyses::hotspot_velocity::HotspotVelocityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore hotspot velocity")?;
    if rows.is_empty() {
        writeln!(w, "_No files changed in the recent window._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Trend | Revs (recent) | Revs (baseline) | Recent/wk | Baseline/wk | Acceleration |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|:---:|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let trend = if row.acceleration > 0.0 {
            "▲ heating"
        } else if row.acceleration < 0.0 {
            "▼ cooling"
        } else {
            "– steady"
        };
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {:.2} | {:.2} | {:+.2} |",
            escape_md_cell(&row.path),
            trend,
            row.revs_recent,
            row.revs_baseline,
            row.recent_per_week,
            row.baseline_per_week,
            row.acceleration,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_hotspots_markdown<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore hotspots")?;
    // The MI cell renders `value (band, rank%)` when the file has a known
    // file-level MI; `—` otherwise. Bands are repo-relative — see
    // `crates/codelore-lib/src/analyses/mi.rs` for why we don't use the
    // literature's absolute Coleman/SEI thresholds.
    //
    // The AI cell is the share of commits with AI-attribution signal
    // (ai-assisted | ai-authored), rendered as `X.X%` or `—`.
    writeln!(
        w,
        "| Entity | Revisions | Cognitive | Cognitive Health | Score | MI | AI % |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let mi_cell = match (row.mi, row.mi_rank) {
            (Some(v), Some(rank)) if rank.is_finite() => {
                let band = crate::analyses::mi::MiBand::from_rank(rank);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // rank is in [0.0,1.0] and guarded is_finite(); after *100+round the value is in [0,100] — fits u32 exactly
                let rank_pct = (rank * 100.0).round() as u32;
                format!("{v:.2} ({}, {rank_pct}%)", band.as_str())
            }
            (Some(v), _) => format!("{v:.2}"),
            (None, _) => "—".to_owned(),
        };
        let ai_cell = match row.ai_pct {
            Some(v) if v.is_finite() => format!("{v:.1}%"),
            _ => "—".to_owned(),
        };
        writeln!(
            w,
            "| `{}` | {} | {:.2} | {:.2} | {:.4} | {} | {} |",
            escape_md_cell(&row.path),
            row.revisions,
            row.cognitive,
            row.cognitive_health,
            row.hotspot_score,
            mi_cell,
            ai_cell
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_health_markdown<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore code-health")?;
    writeln!(
        w,
        "| Entity | Cognitive | Score | Structural risk | Percentile | Band | Corpus percentile |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let corpus_cell = match row.corpus_percentile {
            Some(v) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // v is in [0.0, 1.0] after saturation; *100+round fits u32.
                let pct = (v * 100.0).round() as u32;
                if row.beyond_corpus {
                    format!("{pct}%+")
                } else {
                    format!("{pct}%")
                }
            }
            None => "—".to_owned(),
        };
        writeln!(
            w,
            "| `{}` | {:.2} | {:.2} | {:.4} | {:.4} | {} | {} |",
            escape_md_cell(&row.path),
            row.cognitive,
            row.score,
            row.structural_risk,
            row.percentile,
            row.band,
            corpus_cell
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_age_markdown<W: Write>(rows: &[CodeAgeRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore code-age")?;
    writeln!(w, "| Entity | Age (months) | Age (days) | Last modified |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} |",
            escape_md_cell(&row.path),
            row.age_months,
            row.age_days,
            row.last_modified
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_abs_churn_markdown<W: Write>(rows: &[AbsChurnRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore abs-churn")?;
    writeln!(w, "| Date | Added | Deleted | Commits |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} |",
            row.date, row.added, row.deleted, row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_author_churn_markdown<W: Write>(rows: &[AuthorChurnRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore author-churn")?;
    writeln!(w, "| Author | Added | Deleted | Commits |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} |",
            escape_md_cell(&row.author),
            row.added,
            row.deleted,
            row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_entity_churn_markdown<W: Write>(rows: &[EntityChurnRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore entity-churn")?;
    writeln!(w, "| Entity | Added | Deleted | Commits |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} |",
            escape_md_cell(&row.path),
            row.added,
            row.deleted,
            row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_communication_markdown<W: Write>(rows: &[CommunicationRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore communication")?;
    writeln!(w, "| Author A | Author B | Shared | Average | Strength |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {:.2} |",
            escape_md_cell(&row.author_a),
            escape_md_cell(&row.author_b),
            row.shared,
            row.average,
            row.strength
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_ownership_markdown<W: Write>(rows: &[OwnershipRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore code-ownership")?;
    writeln!(w, "| Entity | Main Author | Total Revs | Fractal Value |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {:.4} |",
            escape_md_cell(&row.path),
            escape_md_cell(&row.main_author),
            row.total_revs,
            row.fractal_value
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_coupling_markdown<W: Write>(rows: &[CouplingRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore change-coupling")?;
    writeln!(w, "| Entity A | Entity B | Shared | Degree | Fisher p |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | `{}` | {} | {:.2}% | {:.4} |",
            escape_md_cell(&row.entity_a),
            escape_md_cell(&row.entity_b),
            row.shared,
            row.degree,
            row.fisher_p
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_summary_markdown<W: Write>(rows: &[SummaryRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore summary")?;
    writeln!(w, "| Metric | Value |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "| {} | {} |", escape_md_cell(&row.metric), row.value)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_clones_markdown<W: Write>(rows: &[ClonesRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore clones")?;
    writeln!(
        w,
        "| Clone group | Entity | Function | Lines | Nodes | Similarity | Family size |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | `{}` | {}-{} | {} | {:.4} | {} |",
            row.clone_group_id,
            escape_md_cell(&row.entity),
            escape_md_cell(&row.function),
            row.start_line,
            row.end_line,
            row.node_count,
            row.similarity,
            row.family_size,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_authors_markdown<W: Write>(rows: &[AuthorsRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore authors (per-entity author breakdown)")?;
    writeln!(
        w,
        "| Entity | Authors | Humans | Bots | Revs | Last author | Last modified |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.entity),
            row.n_authors,
            row.n_humans,
            row.n_bots,
            row.n_revs,
            escape_md_cell(&row.last_author),
            row.last_modified,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_top_committers_markdown<W: Write>(
    rows: &[crate::analyses::top_committers::TopCommittersRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore top-committers")?;
    writeln!(
        w,
        "| Author | Commits | LoC added | LoC deleted | First commit | Last commit | Bot |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.author),
            row.commits,
            row.loc_added,
            row.loc_deleted,
            row.first_commit,
            row.last_commit,
            if row.is_bot { "yes" } else { "no" },
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// god-classes markdown emitter — cognitive × `fan_in` × `fan_out`
/// intersection. Top-of-list files have all three pulling up.
pub fn write_god_classes_markdown<W: Write>(
    rows: &[crate::analyses::god_classes::GodClassRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore god-classes")?;
    writeln!(w, "| Path | Cognitive | Fan-in | Fan-out | God score |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {:.0} | {} | {} | {:.3} |",
            escape_md_cell(&row.path),
            row.cognitive,
            row.fan_in,
            row.fan_out,
            row.god_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// instability markdown emitter — Martin Ca/Ce/Instability per file.
pub fn write_instability_markdown<W: Write>(
    rows: &[crate::analyses::instability::InstabilityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore instability")?;
    if rows.is_empty() {
        writeln!(w, "_No resolved import graph — nothing to score._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Path | Ca | Ce | Instability |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {:.2} |",
            escape_md_cell(&row.path),
            row.ca,
            row.ce,
            row.instability,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// architecture-metrics markdown emitter — repo-level `(metric, value)`.
/// `cycle-origins` markdown emitter — when each HEAD cycle first formed.
pub fn write_cycle_origins_markdown<W: Write>(
    rows: &[crate::analyses::cycle_origins::CycleOriginRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore cycle origins")?;
    if rows.is_empty() {
        writeln!(w, "_No dependency cycles at HEAD — nothing to trace._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Size | Formed at | Date | Members |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | {} | {} |",
            row.size,
            escape_md_cell(&row.formed_at_rev),
            escape_md_cell(&row.formed_at_date),
            escape_md_cell(&row.members),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `architecture-trend` markdown emitter — structural decay over time.
pub fn write_architecture_trend_markdown<W: Write>(
    rows: &[crate::analyses::architecture_trend::ArchitectureTrendRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture trend")?;
    if rows.is_empty() {
        writeln!(w, "_No commit history to sample._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Date | Rev | Files | Propagation cost | Cycles | Largest cycle |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | {} | {:.1}% | {} | {} |",
            escape_md_cell(&row.date),
            escape_md_cell(&row.rev),
            row.files,
            row.propagation_cost * 100.0,
            row.cycle_count,
            row.largest_cycle,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `health-trend` markdown emitter — repo health timeline across sampled revs.
pub fn write_health_trend_markdown<W: Write>(
    rows: &[crate::analyses::health_trend::HealthTrendRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore health trend")?;
    if rows.is_empty() {
        writeln!(w, "_No commit history to sample._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Date | Rev | Files | Arch | Code | Combined |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | {} | {:.1} ({}) | {:.1} ({}) | {:.1} ({}) |",
            escape_md_cell(&row.date),
            escape_md_cell(&row.rev),
            row.files,
            row.arch_health,
            row.arch_band,
            row.code_health,
            row.code_band,
            row.combined_health,
            row.combined_band,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_effort_exposure_markdown<W: Write>(
    rows: &[crate::analyses::effort_exposure::EffortExposureRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore effort-exposure")?;
    if rows.is_empty() {
        writeln!(w, "_No code-health data — run with `--min-revs 1` or ensure complexity metrics are available._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Band | Files | LOC share % | Commit share % | Churn share % | CI 95% low | CI 95% high |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {:.1} | {:.1} | {:.1} | {:.3} | {:.3} |",
            escape_md_cell(&row.band),
            row.files,
            row.loc_share_pct,
            row.commit_share_pct,
            row.churn_share_pct,
            row.commit_share_ci_low,
            row.commit_share_ci_high,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_familiarity_markdown<W: Write>(
    rows: &[crate::analyses::code_familiarity::CodeFamiliarityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore code-familiarity")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No knowledge data — ensure commits exist and complexity metrics are available._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Scope | Familiarity % | Active Authors | Total Authors | Islands % | Verdict |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {:.1} | {} | {} | {:.1} | {} |",
            escape_md_cell(&row.scope),
            row.familiarity_pct,
            row.active_authors,
            row.total_authors,
            row.islands_pct,
            escape_md_cell(&row.verdict),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_coordination_needs_markdown<W: Write>(
    rows: &[crate::analyses::coordination_needs::CoordinationNeedsRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore coordination-needs")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No coordination data — ensure knowledge shares are available._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Authors | Fragmentation | Interleave | Co-change Entropy | Tier | Health Band |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {:.2} | {:.2} | {:.4} | {} | {} |",
            escape_md_cell(&row.path),
            row.authors,
            row.fragmentation,
            row.interleave,
            row.cochange_entropy,
            escape_md_cell(&row.tier),
            escape_md_cell(&row.health_band),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `cycle-health` markdown emitter — per-SCC heat, verdict, extraction
/// candidate, and predicted propagation-cost drop.
pub fn write_cycle_health_markdown<W: Write>(
    rows: &[crate::analyses::cycle_health::CycleHealthRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore cycle-health")?;
    if rows.is_empty() {
        writeln!(w, "_No import cycles detected._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Cycle | Size | Members | Heat % | Verdict | Extract candidate | Predicted PC drop |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---:|---|---:|---|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let drop_cell = match row.predicted_pc_drop {
            Some(d) => Cow::Owned(format!("{d:.2}")),
            None => Cow::Borrowed("—"),
        };
        writeln!(
            w,
            "| {} | {} | {} | {:.2} | {} | {} | {} |",
            row.cycle_id,
            row.size,
            escape_md_cell(&row.members_preview),
            row.heat_pct,
            row.verdict,
            escape_md_cell(&row.extract_candidate),
            drop_cell,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_architecture_metrics_markdown<W: Write>(
    rows: &[crate::analyses::architecture_metrics::ArchitectureMetricRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture-metrics")?;
    if rows.is_empty() {
        writeln!(w, "_No resolved import graph — no metrics to compute._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Metric | Value |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} |",
            escape_md_cell(&row.metric),
            escape_md_cell(&row.value),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `defect-validation` markdown emitter — flat `(metric, value)` evidence
/// rows from a defect-calibration artifact. Empty (no artifact configured)
/// prints an honest-absence note pointing at `codelore calibrate-defects`.
pub fn write_defect_validation_markdown<W: Write>(
    rows: &[crate::analyses::defect_validation::DefectValidationRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore defect-validation")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No defect-calibration artifact configured — run `codelore calibrate-defects` and pass it with `--defect-calibration`._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Metric | Value |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} |",
            escape_md_cell(&row.metric),
            escape_md_cell(&row.value),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// architecture-roles markdown emitter — per-file role + visibility reach.
pub fn write_architecture_roles_markdown<W: Write>(
    rows: &[crate::analyses::architecture_roles::ArchitectureRoleRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture-roles")?;
    if rows.is_empty() {
        writeln!(w, "_No resolved import graph — nothing to classify._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Role | VFI | VFO | In cycle | Level | Reach % |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|:--:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {} | {} | {:.1} |",
            escape_md_cell(&row.path),
            row.role,
            row.vfi,
            row.vfo,
            row.in_cycle,
            row.level,
            row.reach_pct,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// dependency-cycles markdown emitter — one row per (cycle, member).
pub fn write_dependency_cycles_markdown<W: Write>(
    rows: &[crate::analyses::dependency_cycles::DependencyCycleRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore dependency-cycles")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No dependency cycles — the resolved import graph is acyclic (or no imports resolved)._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Cycle | Size | Path |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | `{}` |",
            row.cycle_id,
            row.size,
            escape_md_cell(&row.path),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// modularity-violations markdown emitter — Fisher-significant
/// co-change pairs with no structural import edge.
pub fn write_modularity_violations_markdown<W: Write>(
    rows: &[crate::analyses::modularity_violations::ModularityViolationRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore modularity-violations")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No modularity violations — every Fisher-significant co-change pair also has a structural import edge (or no co-change pairs were found)._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Entity A | Entity B | Shared | Degree | Fisher p |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | `{}` | {} | {:.2} | {:.4} |",
            escape_md_cell(&row.entity_a),
            escape_md_cell(&row.entity_b),
            row.shared,
            row.degree,
            row.fisher_p,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// crossing markdown emitter — structural "X" files coupling upstream
/// and downstream through themselves (DV8 Crossing).
pub fn write_crossing_markdown<W: Write>(
    rows: &[crate::analyses::crossing::CrossingRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore crossing")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No crossings — no file is both a wide hub and a wide sink that co-changes in both directions._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Fan-in | Fan-out | Coupled upstream | Coupled downstream | Crossing score |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {} | {:.1} |",
            escape_md_cell(&row.path),
            row.fan_in,
            row.fan_out,
            row.coupled_upstream,
            row.coupled_downstream,
            row.crossing_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// unstable-interface markdown emitter — interfaces whose instability
/// propagates to their dependents.
pub fn write_unstable_interface_markdown<W: Write>(
    rows: &[crate::analyses::unstable_interface::UnstableInterfaceRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore unstable-interface")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No unstable interfaces — no widely-imported file changes often enough to drag its dependents (or the import graph is empty)._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Fan-in | Revisions | Coupled dependents | Instability score |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {:.1} |",
            escape_md_cell(&row.path),
            row.fan_in,
            row.revisions,
            row.coupled_dependents,
            row.instability_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// architecture-violations markdown emitter — one row per import
/// edge that crosses a forbidden layer boundary.
pub fn write_arch_violations_markdown<W: Write>(
    rows: &[crate::analyses::arch_violations::ArchViolationRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture-violations")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No violations detected — either the rule set is empty (no `.codelore-arch-rules.toml` at the repo root) or every import edge respects the declared layer boundaries._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Source file | Source layer | Target file | Target layer | Raw target |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | `{}` | {} | `{}` |",
            escape_md_cell(&row.src_path),
            escape_md_cell(&row.src_layer),
            escape_md_cell(&row.target_path),
            escape_md_cell(&row.target_layer),
            escape_md_cell(&row.raw_target),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// stale-code markdown emitter.
pub fn write_stale_code_markdown<W: Write>(
    rows: &[crate::analyses::stale_code::StaleCodeRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore stale-code")?;
    writeln!(w, "| Path | Last touched | Months since | Max cognitive |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {:.0} |",
            escape_md_cell(&row.path),
            row.last_touched,
            row.months_since_touched,
            row.max_cognitive,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// pair-programming markdown emitter.
pub fn write_pair_programming_markdown<W: Write>(
    rows: &[crate::analyses::pair_programming::PairRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore pair-programming")?;
    writeln!(w, "| Author A | Author B | Pair commits |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} |",
            escape_md_cell(&row.author_a),
            escape_md_cell(&row.author_b),
            row.pair_commits,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// lead-time markdown emitter.
pub fn write_lead_time_markdown<W: Write>(
    rows: &[crate::analyses::lead_time::LeadTimeRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore lead-time")?;
    writeln!(
        w,
        "| Rev | Author | Authored | Committed | Lead time (days) |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let short_rev = if row.rev.len() >= 8 {
            &row.rev[..8]
        } else {
            &row.rev
        };
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {:.2} |",
            short_rev,
            escape_md_cell(&row.canonical_author),
            row.author_date,
            row.committer_date,
            row.lead_time_days,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// delivery-friction markdown emitter.
pub fn write_delivery_friction_markdown<W: Write>(
    rows: &[crate::analyses::delivery_friction::DeliveryFrictionRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore delivery-friction")?;
    writeln!(
        w,
        "| Entity | Revisions | Cognitive | Median lead-time (days) | p95 lead-time (days) | WIP age (days) | Friction score |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {:.1} | {:.2} | {:.2} | {:.1} | {:.2} |",
            escape_md_cell(&row.path),
            row.revisions,
            row.cognitive,
            row.median_lead_time_days,
            row.p95_lead_time_days,
            row.wip_age_days,
            row.friction_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// bus-factor markdown emitter.
pub fn write_bus_factor_markdown<W: Write>(
    rows: &[crate::analyses::bus_factor::BusFactorRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore bus-factor")?;
    writeln!(
        w,
        "| Module | Total commits | Bus factor | Top contributor | Top share | Model |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {:.1}% | {} |",
            escape_md_cell(&row.module),
            row.total_commits,
            row.bus_factor,
            escape_md_cell(&row.top_contributor),
            row.top_contributor_share * 100.0,
            row.model,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_knowledge_islands_markdown<W: Write>(
    rows: &[crate::analyses::knowledge_islands::KnowledgeIslandRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore knowledge-islands (bus-factor risk)")?;
    writeln!(
        w,
        "| Entity | Main author | Ownership % | Days since main active | Last main commit | Substantial others |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {:.2} | {} | {} | {} |",
            escape_md_cell(&row.entity),
            escape_md_cell(&row.main_author),
            row.ownership_pct,
            row.days_since_main_active,
            row.last_main_author_commit,
            row.n_substantial_others,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_soc_markdown<W: Write>(
    rows: &[crate::analyses::soc::SocRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore sum-of-coupling")?;
    writeln!(w, "| Entity | SoC |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "| `{}` | {} |", escape_md_cell(&row.entity), row.soc)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_messages_markdown<W: Write>(
    rows: &[crate::analyses::messages::MessagesRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore messages")?;
    writeln!(w, "| Entity | Matches |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "| `{}` | {} |", escape_md_cell(&row.entity), row.matches)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

fn write_main_dev_markdown_with_headers<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
    title: &str,
    metric_label: &str,
    total_label: &str,
) -> Result<()> {
    header(w, title)?;
    writeln!(
        w,
        "| Entity | Main Dev | {metric_label} | {total_label} | Ownership |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {:.2} |",
            escape_md_cell(&row.entity),
            escape_md_cell(&row.main_dev),
            row.metric,
            row.total,
            row.ownership,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_main_dev_markdown<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_markdown_with_headers(
        rows,
        w,
        "CodeLore main-dev (by lines added)",
        "Added",
        "Total Added",
    )
}

pub fn write_main_dev_by_revs_markdown<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_markdown_with_headers(
        rows,
        w,
        "CodeLore main-dev (by revision count)",
        "Revisions",
        "Total Revisions",
    )
}

pub fn write_main_dev_by_deletions_markdown<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_markdown_with_headers(
        rows,
        w,
        "CodeLore main-dev-by-deletions (alias: refactoring-main-dev)",
        "Removed",
        "Total Removed",
    )
}

pub fn write_entity_effort_markdown<W: Write>(
    rows: &[crate::analyses::entity_effort::EntityEffortRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore entity-effort")?;
    writeln!(w, "| Entity | Author | Author Revs | Total Revs |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} |",
            escape_md_cell(&row.entity),
            escape_md_cell(&row.author),
            row.author_revs,
            row.total_revs,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_entity_ownership_markdown<W: Write>(
    rows: &[crate::analyses::entity_ownership::EntityOwnershipRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore entity-ownership")?;
    writeln!(w, "| Entity | Author | Added | Deleted |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} |",
            escape_md_cell(&row.entity),
            escape_md_cell(&row.author),
            row.added,
            row.deleted,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_clone_coupling_markdown<W: Write>(rows: &[CloneCouplingRow], w: &mut W) -> Result<()> {
    header(
        w,
        "CodeLore live clones (clone × Fisher-significant co-change)",
    )?;
    writeln!(
        w,
        "| At-risk | Group | File A | File B | Shared | Degree | Combined |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|:---:|---|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | `{}` | `{}` | {} | {:.2}% | {:.4} |",
            if row.at_risk { "**⚠**" } else { "" },
            row.clone_group_id,
            escape_md_cell(&row.file_a),
            escape_md_cell(&row.file_b),
            row.shared_revs,
            row.degree_pct * 100.0,
            row.combined_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_centrality_markdown<W: Write>(
    rows: &[crate::analyses::centrality::CentralityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore behavioural-coupling centrality")?;
    writeln!(w, "| Entity | Degree | Weighted | PageRank | Eigenvector |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {:.2} | {:.4} | {:.4} |",
            escape_md_cell(&row.path),
            row.degree,
            row.weighted_degree,
            row.pagerank,
            row.eigenvector,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_communities_markdown<W: Write>(
    result: &crate::analyses::communities::CommunitiesResult,
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore behavioural communities (Leiden partition)")?;
    writeln!(
        w,
        "**Modularity Q = {:.4}** across **{} communities**.\n",
        result.modularity, result.community_count,
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "| Community | Size | Entity |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in &result.rows {
        writeln!(
            w,
            "| {} | {} | `{}` |",
            row.community_id,
            row.community_size,
            escape_md_cell(&row.path),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// Markdown table emitter for the `refactoring-targets` analysis.
///
/// # Errors
/// Propagates any write error from `w`.
pub fn write_refactoring_targets_markdown<W: Write>(
    rows: &[crate::analyses::refactoring_targets::RefactoringTargetRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "| Entity | Priority | Combined risk | Structural risk | Hotspot | Revisions | LOC | Type | Band | ManualUp |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {:.6} | {:.6} | {:.4} | {:.4} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.path),
            row.priority,
            row.combined_risk,
            row.structural_risk,
            row.hotspot_score,
            row.revisions,
            row.loc,
            escape_md_cell(&row.dominant_type),
            row.band,
            row.manual_up_rank,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `team-composition` markdown emitter.
///
/// The `__summary__` carrier row (bucket-share percentages, not per-author
/// data) is not a data row and is skipped.
pub fn write_team_composition_markdown<W: Write>(
    rows: &[crate::analyses::team_composition::TeamCompositionRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore team-composition")?;
    if rows.is_empty() {
        writeln!(w, "_No commit history found._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Author | Tenure (d) | Bucket | Breadth OK | Active | Commits | Files | Onboarding (wk) |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        if row.author == "__summary__" {
            continue;
        }
        let ob = row
            .onboarding_weeks
            .map_or_else(|| "—".to_string(), |v| v.to_string());
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.author),
            row.tenure_days,
            escape_md_cell(&row.bucket),
            row.veteran_breadth_ok,
            row.active,
            row.commits,
            row.files_touched,
            ob,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_marginal_owner_risk_markdown<W: Write>(
    rows: &[crate::analyses::marginal_owner_risk::MarginalOwnerRiskRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore marginal-owner-risk")?;
    if rows.is_empty() {
        writeln!(w, "_No marginal-owner risk detected._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Path | Band | Top Active Share | Risk |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {:.4} | {} |",
            escape_md_cell(&row.path),
            escape_md_cell(&row.band),
            row.top_active_share,
            escape_md_cell(&row.risk),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_release_cadence_markdown<W: Write>(
    rows: &[crate::analyses::release_cadence::ReleaseCadenceRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore release-cadence")?;
    let tag_rows: Vec<_> = rows.iter().filter(|r| r.tag != "__summary__").collect();
    if tag_rows.is_empty() {
        writeln!(w, "_No release tags matched the glob._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Tag | Date | Days Since Prev |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|-----|------|-----------------|").map_err(CodeLoreError::Io)?;
    for row in &tag_rows {
        let gap = match row.days_since_prev {
            Some(d) => format!("{d:.1}"),
            None => "—".to_string(),
        };
        writeln!(w, "| {} | {} | {} |", row.tag, row.date, gap).map_err(CodeLoreError::Io)?;
    }
    if let Some(summary) = rows.iter().find(|r| r.tag == "__summary__") {
        writeln!(w).map_err(CodeLoreError::Io)?;
        writeln!(
            w,
            "**Summary** — median gap: {:.1} d | {} | trend: {}",
            summary.days_since_prev.unwrap_or(0.0),
            summary.date,
            summary.trend,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_delivery_metrics_markdown<W: Write>(
    rows: &[crate::analyses::delivery_metrics::DeliveryMetricsRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore delivery-metrics")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No delivery metrics computed — no merge commits ingested._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Metric | p50 | p75 | p90 | n | Caveat |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {:.2} | {:.2} | {:.2} | {} | {} |",
            escape_md_cell(&row.metric),
            row.p50,
            row.p75,
            row.p90,
            row.n,
            escape_md_cell(&row.caveat),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_function_xray_markdown<W: Write>(
    rows: &[crate::analyses::function_xray::FunctionXrayRow],
    target: &str,
    w: &mut W,
) -> Result<()> {
    header(w, &format!("CodeLore function-xray — {target}"))?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No HEAD-alive functions found in `{target}` or no changes recorded._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Function | Change Freq | LOC | Cyclomatic | Cognitive | Last Changed |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let cyc = row
            .cyclomatic
            .map_or_else(|| "—".to_string(), |v| v.to_string());
        let cog = row
            .cognitive
            .map_or_else(|| "—".to_string(), |v| v.to_string());
        let last = if row.last_changed.is_empty() {
            "—".to_string()
        } else {
            row.last_changed.clone()
        };
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.function),
            row.change_freq,
            row.loc,
            cyc,
            cog,
            last,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_function_coupling_markdown<W: Write>(
    rows: &[crate::analyses::function_coupling::FunctionCouplingRow],
    target: &str,
    w: &mut W,
) -> Result<()> {
    header(w, &format!("CodeLore function-coupling — {target}"))?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No coupled function pairs (co-changes ≥ 2) found in `{target}`._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| A | B | Co-Changes | A Changes | B Changes | Confidence | p-value |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let p = row
            .p_value
            .map_or_else(|| "—".to_string(), |v| format!("{v:.4}"));
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {:.4} | {} |",
            escape_md_cell(&row.a),
            escape_md_cell(&row.b),
            row.co_changes,
            row.a_changes,
            row.b_changes,
            row.confidence,
            p,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_finding_hotspot_overlap_markdown<W: Write>(
    rows: &[crate::analyses::finding_hotspot_overlap::FindingHotspotOverlapRow],
    w: &mut W,
) -> crate::Result<()> {
    writeln!(
        w,
        "| Path | Findings | Engines | Worst Level | Hotspot Score | Revs Percentile | Health Band | Priority |"
    )
    .map_err(crate::CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---|---|---:|---:|---|---|").map_err(crate::CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {:.4} | {:.4} | {} | {} |",
            escape_md_cell(&row.path),
            row.findings,
            escape_md_cell(&row.engines),
            escape_md_cell(&row.worst_level),
            row.hotspot_score,
            row.revs_percentile,
            escape_md_cell(&row.health_band),
            escape_md_cell(&row.priority),
        )
        .map_err(crate::CodeLoreError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod escape_tests {
    use super::escape_md_cell;
    use std::borrow::Cow;

    /// Common case: no special characters → borrow the input.
    #[test]
    fn plain_string_is_borrowed() {
        let s = "src/main.rs";
        match escape_md_cell(s) {
            Cow::Borrowed(out) => assert_eq!(out, s),
            Cow::Owned(_) => panic!("expected borrow, got owned allocation"),
        }
    }

    /// A literal `|` in a path / author name / commit message gets
    /// backslash-escaped so GFM renders the row with the correct column
    /// count instead of treating the pipe as a cell terminator.
    #[test]
    fn pipe_is_backslash_escaped() {
        let out = escape_md_cell("path/with|pipe.rs");
        assert_eq!(out, "path/with\\|pipe.rs");
    }

    /// Multiple pipes within a single cell — each escaped.
    #[test]
    fn multiple_pipes_each_escaped() {
        let out = escape_md_cell("a|b|c");
        assert_eq!(out, "a\\|b\\|c");
    }

    /// Newlines inside a cell terminate the GFM row entirely. Substitute
    /// the visual `↵` glyph so the row stays intact.
    #[test]
    fn newline_becomes_arrow_glyph() {
        let out = escape_md_cell("line1\nline2");
        assert_eq!(out, "line1↵line2");
    }

    /// Mixed payload — verifies the loop handles each special character
    /// independently and preserves all other characters verbatim.
    #[test]
    fn mixed_special_chars_pipe_and_newline() {
        let out = escape_md_cell("a|b\nc");
        assert_eq!(out, "a\\|b↵c");
    }
}

#[cfg(test)]
mod team_composition_tests {
    use crate::analyses::team_composition::TeamCompositionRow;

    #[test]
    fn team_composition_markdown_skips_summary_row() {
        let rows = vec![
            TeamCompositionRow {
                author: "alice".to_string(),
                tenure_days: 110,
                bucket: "experienced".to_string(),
                veteran_breadth_ok: false,
                active: true,
                commits: 12,
                files_touched: 3,
                onboarding_weeks: None,
            },
            TeamCompositionRow {
                author: "__summary__".to_string(),
                tenure_days: 0,
                bucket: "onboarded=33.3% experienced=66.7% veteran=0.0%".to_string(),
                veteran_breadth_ok: false,
                active: false,
                commits: 0,
                files_touched: 0,
                onboarding_weeks: None,
            },
        ];
        let mut buf = Vec::new();
        super::write_team_composition_markdown(&rows, &mut buf).expect("write markdown");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(
            !out.contains("__summary__"),
            "markdown output must not contain the __summary__ carrier row: {out}"
        );
        assert!(
            out.contains("alice"),
            "markdown output must contain real author rows: {out}"
        );
    }
}
