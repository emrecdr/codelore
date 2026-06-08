//! CSV emitters. Headers match code-maat exactly for golden-test parity.

use std::io::Write;

use crate::analyses::authors::AuthorsRow;
use crate::analyses::churn::{AbsChurnRow, AuthorChurnRow, EntityChurnRow};
use crate::analyses::clone_coupling::CloneCouplingRow;
use crate::analyses::clones::ClonesRow;
use crate::analyses::code_age::CodeAgeRow;
use crate::analyses::code_health::CodeHealthRow;
use crate::analyses::communication::CommunicationRow;
use crate::analyses::coupling::CouplingRow;
use crate::analyses::hotspots::HotspotRow;
use crate::analyses::ownership::OwnershipRow;
use crate::analyses::summary::SummaryRow;
use crate::{CodeLoreError, Result};

fn quote_if_needed(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_owned()
    }
}

pub fn write_revisions_csv<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    writeln!(w, "entity,n-revs").map_err(CodeLoreError::Io)?;
    for (entity, n) in rows {
        writeln!(w, "{},{}", quote_if_needed(entity), n).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_hotspots_csv<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    writeln!(
        w,
        "entity,revisions,cognitive,code-health,hotspot-score"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.2},{:.2},{:.4}",
            quote_if_needed(&row.path),
            row.revisions,
            row.cognitive,
            row.code_health,
            row.hotspot_score
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_health_csv<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,cognitive,score").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{:.2},{:.2}",
            quote_if_needed(&row.path),
            row.cognitive,
            row.score
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_age_csv<W: Write>(rows: &[CodeAgeRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,age-months").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.path), row.age_months)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_abs_churn_csv<W: Write>(rows: &[AbsChurnRow], w: &mut W) -> Result<()> {
    writeln!(w, "date,added,deleted,commits").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{}",
            quote_if_needed(&row.date),
            row.added,
            row.deleted,
            row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_author_churn_csv<W: Write>(rows: &[AuthorChurnRow], w: &mut W) -> Result<()> {
    writeln!(w, "author,added,deleted,commits").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{}",
            quote_if_needed(&row.author),
            row.added,
            row.deleted,
            row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_entity_churn_csv<W: Write>(rows: &[EntityChurnRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,added,deleted,commits").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{}",
            quote_if_needed(&row.path),
            row.added,
            row.deleted,
            row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_communication_csv<W: Write>(rows: &[CommunicationRow], w: &mut W) -> Result<()> {
    writeln!(w, "author-a,author-b,shared,average,strength").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{:.2}",
            quote_if_needed(&row.author_a),
            quote_if_needed(&row.author_b),
            row.shared,
            row.average,
            row.strength
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_ownership_csv<W: Write>(rows: &[OwnershipRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,main-dev,total-revs,fractal-value").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.2}",
            quote_if_needed(&row.path),
            quote_if_needed(&row.main_author),
            row.total_revs,
            row.fractal_value
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_coupling_csv<W: Write>(rows: &[CouplingRow], w: &mut W) -> Result<()> {
    writeln!(
        w,
        "entity-a,entity-b,shared,revs-a,revs-b,average-revs,degree,fisher-p"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{:.2},{:.4}",
            quote_if_needed(&row.entity_a),
            quote_if_needed(&row.entity_b),
            row.shared,
            row.revs_a,
            row.revs_b,
            row.average_revs,
            row.degree,
            row.fisher_p
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_summary_csv<W: Write>(rows: &[SummaryRow], w: &mut W) -> Result<()> {
    writeln!(w, "metric,value").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.metric), row.value).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_clones_csv<W: Write>(rows: &[ClonesRow], w: &mut W) -> Result<()> {
    writeln!(
        w,
        "clone-group,fingerprint,entity,function,start-line,end-line,node-count,similarity,family-size"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{:.4},{}",
            row.clone_group_id,
            row.fingerprint,
            quote_if_needed(&row.entity),
            quote_if_needed(&row.function),
            row.start_line,
            row.end_line,
            row.node_count,
            row.similarity,
            row.family_size
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_authors_csv<W: Write>(rows: &[AuthorsRow], w: &mut W) -> Result<()> {
    // Header matches code-maat's `authors` analysis ("name,n-commits") for parity.
    writeln!(w, "name,n-commits").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.author), row.commits)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_clone_coupling_csv<W: Write>(rows: &[CloneCouplingRow], w: &mut W) -> Result<()> {
    // 18 columns mirroring the CloneCouplingRow struct.
    writeln!(
        w,
        "clone-group,fingerprint,file-a,file-b,entity-a,entity-b,\
         start-line-a,end-line-a,start-line-b,end-line-b,\
         node-count,similarity,shared-revs,support-a,support-b,\
         degree-pct,p-value,combined-score"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{:.4},{},{},{},{:.4},{:.4},{:.4}",
            row.clone_group_id,
            row.fingerprint,
            quote_if_needed(&row.file_a),
            quote_if_needed(&row.file_b),
            quote_if_needed(&row.entity_a),
            quote_if_needed(&row.entity_b),
            row.start_line_a,
            row.end_line_a,
            row.start_line_b,
            row.end_line_b,
            row.node_count,
            row.similarity,
            row.shared_revs,
            row.support_a,
            row.support_b,
            row.degree_pct,
            row.p_value,
            row.combined_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
