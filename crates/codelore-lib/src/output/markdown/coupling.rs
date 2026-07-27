//! Markdown emitters for the coupling and duplication family: logical
//! coupling, sum-of-coupling, clones, clone-coupling, and function coupling.

use super::{escape_md_cell, header};
use crate::analyses::clone_coupling::CloneCouplingRow;
use crate::analyses::clones::ClonesRow;
use crate::analyses::coupling::CouplingRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

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
