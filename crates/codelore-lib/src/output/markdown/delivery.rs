//! Markdown emitters for the delivery-flow family: lead time, delivery
//! friction, delivery metrics, and release cadence.

use super::{escape_md_cell, header};
use crate::{CodeLoreError, Result};
use std::io::Write;

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
