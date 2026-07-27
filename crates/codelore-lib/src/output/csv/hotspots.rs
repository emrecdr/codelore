//! CSV emitters for the hotspot and code-health family: hotspots,
//! velocity, code health, health trend, effort exposure, refactoring targets,
//! the per-function x-ray, finding overlap, and defect validation.

use super::quote_if_needed;
use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::hotspots::HotspotRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

/// `hotspot-velocity` CSV emitter — recent vs baseline churn rate +
/// acceleration (heating up / cooling down).
pub fn write_hotspot_velocity_csv<W: Write>(
    rows: &[crate::analyses::hotspot_velocity::HotspotVelocityRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "entity,revs-recent,revs-baseline,recent-per-week,baseline-per-week,acceleration"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.2},{:.2},{:.2}",
            quote_if_needed(&row.path),
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

pub fn write_hotspots_csv<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    // `mi` is the SEI-variant Maintainability Index per Coleman 1994 / SEI 1997.
    // `mi-rank` is the file's repo-relative percentile in `[0,1]` (PERCENT_RANK
    // over the analyzed repo's MI distribution). `mi-band` is the
    // repo-relative Low/Moderate/High classification derived from the rank —
    // see `crates/codelore-lib/src/analyses/mi.rs` for the empirical
    // calibration that motivates relative bands over absolute Coleman/SEI
    // thresholds. Cells are empty when `mi` is unknown.
    //
    // `ai-pct` is the share of commits touching this file that carry an
    // AI-attribution signal (ai-assisted | ai-authored per identity::bots).
    // Range [0, 100]; absolute interpretation is meaningful across repos.
    writeln!(
        w,
        "entity,revisions,cognitive,cognitive-health,hotspot-score,mi,mi-rank,mi-band,ai-pct"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        let mi_cell = match row.mi {
            Some(v) => format!("{v:.2}"),
            None => String::new(),
        };
        let (rank_cell, band_cell) = match row.mi_rank {
            Some(rank) if rank.is_finite() => (
                format!("{rank:.4}"),
                crate::analyses::mi::MiBand::from_rank(rank)
                    .as_str()
                    .to_owned(),
            ),
            _ => (String::new(), String::new()),
        };
        let ai_cell = match row.ai_pct {
            Some(v) if v.is_finite() => format!("{v:.2}"),
            _ => String::new(),
        };
        writeln!(
            w,
            "{},{},{:.2},{:.2},{:.4},{},{},{},{}",
            quote_if_needed(&row.path),
            row.revisions,
            row.cognitive,
            row.cognitive_health,
            row.hotspot_score,
            mi_cell,
            rank_cell,
            band_cell,
            ai_cell
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_health_csv<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    writeln!(
        w,
        "entity,cognitive,score,structural_risk,percentile,band,corpus-pct,corpus-pct-ci-low,corpus-pct-ci-high"
    )
    .map_err(CodeLoreError::Io)?;
    // A cell holding a `{:.2}` fraction when present, else empty — the shared
    // shape of the corpus percentile and its two Wilson bounds.
    let cell = |v: Option<f64>| v.map_or_else(String::new, |v| format!("{v:.2}"));
    for row in rows {
        writeln!(
            w,
            "{},{:.2},{:.2},{:.4},{:.4},{},{},{},{}",
            quote_if_needed(&row.path),
            row.cognitive,
            row.score,
            row.structural_risk,
            row.percentile,
            row.band,
            cell(row.corpus_percentile),
            cell(row.corpus_percentile_ci_low),
            cell(row.corpus_percentile_ci_high),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `health-trend` CSV emitter — repo health timeline across sampled revs.
pub fn write_health_trend_csv<W: Write>(
    rows: &[crate::analyses::health_trend::HealthTrendRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "date,rev,files,arch-health,code-health,combined-health,arch-band,code-band,combined-band"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.2},{:.2},{:.2},{},{},{}",
            quote_if_needed(&row.date),
            quote_if_needed(&row.rev),
            row.files,
            row.arch_health,
            row.code_health,
            row.combined_health,
            row.arch_band,
            row.code_band,
            row.combined_band,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_effort_exposure_csv<W: Write>(
    rows: &[crate::analyses::effort_exposure::EffortExposureRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "band,files,loc-share-pct,commit-share-pct,churn-share-pct,commit-share-ci-low,commit-share-ci-high"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.2},{:.2},{:.2},{:.4},{:.4}",
            quote_if_needed(&row.band),
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

/// `defect-validation` CSV emitter — flat `(metric, value)` evidence rows from
/// a defect-calibration artifact. Header `metric,value`; zero rows (no
/// artifact configured) still emit the header.
pub fn write_defect_validation_csv<W: Write>(
    rows: &[crate::analyses::defect_validation::DefectValidationRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "metric,value").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{}",
            quote_if_needed(&row.metric),
            quote_if_needed(&row.value),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// CSV emitter for the `refactoring-targets` analysis.
///
/// # Errors
/// Propagates any write error from `w`.
pub fn write_refactoring_targets_csv<W: Write>(
    rows: &[crate::analyses::refactoring_targets::RefactoringTargetRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "entity,priority,combined_risk,structural_risk,hotspot_score,revisions,loc,dominant_type,band,manual_up_rank"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{:.6},{:.6},{:.4},{:.4},{},{},{},{},{}",
            quote_if_needed(&row.path),
            row.priority,
            row.combined_risk,
            row.structural_risk,
            row.hotspot_score,
            row.revisions,
            row.loc,
            quote_if_needed(&row.dominant_type),
            row.band,
            row.manual_up_rank,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_function_xray_csv<W: Write>(
    rows: &[crate::analyses::function_xray::FunctionXrayRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "function,change-freq,loc,cyclomatic,cognitive,last-changed"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{}",
            quote_if_needed(&row.function),
            row.change_freq,
            row.loc,
            row.cyclomatic.map_or_else(String::new, |v| v.to_string()),
            row.cognitive.map_or_else(String::new, |v| v.to_string()),
            quote_if_needed(&row.last_changed),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_finding_hotspot_overlap_csv<W: Write>(
    rows: &[crate::analyses::finding_hotspot_overlap::FindingHotspotOverlapRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "path,findings,engines,worst-level,hotspot-score,revs-percentile,health-band,priority"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{:.4},{:.4},{},{}",
            quote_if_needed(&row.path),
            row.findings,
            quote_if_needed(&row.engines),
            quote_if_needed(&row.worst_level),
            row.hotspot_score,
            row.revs_percentile,
            quote_if_needed(&row.health_band),
            quote_if_needed(&row.priority),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
