//! CSV emitters. Headers match code-maat exactly for golden-test parity.

use std::io::Write;

use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::hotspots::HotspotRow;
use crate::{BcaError, Result};

fn quote_if_needed(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_owned()
    }
}

pub fn write_revisions_csv<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    writeln!(w, "entity,n-revs").map_err(BcaError::Io)?;
    for (entity, n) in rows {
        writeln!(w, "{},{}", quote_if_needed(entity), n).map_err(BcaError::Io)?;
    }
    Ok(())
}

pub fn write_hotspots_csv<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    writeln!(
        w,
        "entity,name,revisions,cognitive,code-health,hotspot-score"
    )
    .map_err(BcaError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.2},{:.2},{:.4}",
            quote_if_needed(&row.path),
            quote_if_needed(&row.name),
            row.revisions,
            row.cognitive,
            row.code_health,
            row.hotspot_score
        )
        .map_err(BcaError::Io)?;
    }
    Ok(())
}

pub fn write_code_health_csv<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,name,cognitive,score").map_err(BcaError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.2},{:.2}",
            quote_if_needed(&row.path),
            quote_if_needed(&row.name),
            row.cognitive,
            row.score
        )
        .map_err(BcaError::Io)?;
    }
    Ok(())
}
