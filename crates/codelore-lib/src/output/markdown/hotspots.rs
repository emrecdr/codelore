//! Markdown emitters for the hotspot and code-health family: hotspots,
//! velocity, code health, health trend, effort exposure, refactoring targets,
//! the per-function x-ray, finding overlap, and defect validation.

use super::{escape_md_cell, header};
use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::hotspots::HotspotRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

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
        "| Entity | Cognitive | Score | Structural risk | Percentile | Band | Corpus percentile | Corpus 95% CI |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
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
        // Wilson 95% interval on the corpus percentile, as an integer-percent
        // range. Present exactly when `corpus_percentile` is.
        let corpus_ci_cell = match (row.corpus_percentile_ci_low, row.corpus_percentile_ci_high) {
            (Some(lo), Some(hi)) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // Both bounds are in [0.0, 1.0]; *100+round fits u32.
                let (lo_pct, hi_pct) = ((lo * 100.0).round() as u32, (hi * 100.0).round() as u32);
                format!("{lo_pct}–{hi_pct}%")
            }
            _ => "—".to_owned(),
        };
        writeln!(
            w,
            "| `{}` | {:.2} | {:.2} | {:.4} | {:.4} | {} | {} | {} |",
            escape_md_cell(&row.path),
            row.cognitive,
            row.score,
            row.structural_risk,
            row.percentile,
            row.band,
            corpus_cell,
            corpus_ci_cell
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
        "| Band | Files | LOC share % | Commit share % | Churn share % | CI 95% low | CI 95% high | Improving churn % | Degrading churn % |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        // The improving/degrading split is populated only for the red band when
        // the decomposition ran (repo available); elsewhere it reads "—".
        let improving = row
            .churn_share_improving_pct
            .map_or_else(|| "—".to_owned(), |v| format!("{v:.1}"));
        let degrading = row
            .churn_share_degrading_pct
            .map_or_else(|| "—".to_owned(), |v| format!("{v:.1}"));
        writeln!(
            w,
            "| {} | {} | {:.1} | {:.1} | {:.1} | {:.3} | {:.3} | {} | {} |",
            escape_md_cell(&row.band),
            row.files,
            row.loc_share_pct,
            row.commit_share_pct,
            row.churn_share_pct,
            row.commit_share_ci_low,
            row.commit_share_ci_high,
            improving,
            degrading,
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
