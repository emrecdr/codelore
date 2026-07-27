//! CSV emitters for the delivery-flow family: lead time, delivery
//! friction, delivery metrics, and release cadence.

use super::quote_if_needed;
use crate::{CodeLoreError, Result};
use std::io::Write;

/// `lead-time` CSV emitter.
pub fn write_lead_time_csv<W: Write>(
    rows: &[crate::analyses::lead_time::LeadTimeRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "rev,canonical_author,author_date,committer_date,lead_time_seconds,lead_time_days"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{:.3}",
            quote_if_needed(&row.rev),
            quote_if_needed(&row.canonical_author),
            quote_if_needed(&row.author_date),
            quote_if_needed(&row.committer_date),
            row.lead_time_seconds,
            row.lead_time_days,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `delivery-friction` CSV emitter.
pub fn write_delivery_friction_csv<W: Write>(
    rows: &[crate::analyses::delivery_friction::DeliveryFrictionRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "entity,revisions,cognitive,median_lead_time_days,p95_lead_time_days,wip_age_days,friction_score"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.1},{:.2},{:.2},{:.2},{:.2}",
            quote_if_needed(&row.path),
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

pub fn write_release_cadence_csv<W: Write>(
    rows: &[crate::analyses::release_cadence::ReleaseCadenceRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "tag,date,days-since-prev,trend").map_err(CodeLoreError::Io)?;
    for row in rows {
        let gap = match row.days_since_prev {
            Some(d) => format!("{d:.2}"),
            None => String::new(),
        };
        writeln!(
            w,
            "{},{},{},{}",
            quote_if_needed(&row.tag),
            quote_if_needed(&row.date),
            gap,
            quote_if_needed(&row.trend),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_delivery_metrics_csv<W: Write>(
    rows: &[crate::analyses::delivery_metrics::DeliveryMetricsRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "metric,p50,p75,p90,n,caveat").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{:.2},{:.2},{:.2},{},{}",
            quote_if_needed(&row.metric),
            row.p50,
            row.p75,
            row.p90,
            row.n,
            quote_if_needed(&row.caveat),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
