//! CSV emitters for the coupling and duplication family: logical
//! coupling, sum-of-coupling, clones, clone-coupling, and function coupling.

use super::quote_if_needed;
use crate::analyses::clone_coupling::CloneCouplingRow;
use crate::analyses::clones::ClonesRow;
use crate::analyses::coupling::CouplingRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

/// DEEP-1, DEEP-2, DEEP-3: under `--code-maat-compat`, emit code-maat's
/// verbose 7-column shape with truncated-integer `degree` and the
/// legacy column-name set (`entity`, `coupled`, `first-entity-revisions`,
/// `second-entity-revisions`, `shared-revisions`). The Fisher exact p-value
/// is a `CodeLore` extension with no code-maat equivalent and gets
/// dropped under compat — migrating tools wouldn't know how to interpret
/// it anyway. `CodeLore`'s modern default keeps the 8-column shape with
/// float `degree` and the always-verbose paired columns.
pub fn write_coupling_csv<W: Write>(
    rows: &[CouplingRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    if code_maat_compat {
        // Code-maat's verbose-results column order: entity, coupled,
        // degree, average-revs, first-entity-revisions,
        // second-entity-revisions, shared-revisions.
        writeln!(
            w,
            "entity,coupled,degree,average-revs,\
             first-entity-revisions,second-entity-revisions,shared-revisions"
        )
        .map_err(CodeLoreError::Io)?;
        for row in rows {
            // DEEP-2: degree formatted as truncated integer to match
            // code-maat's `(int coupling)`. `as u32` performs the
            // truncation; `f64::min(100.0)` guards against the rare
            // > 100 round-up case (boundary degenerate from the SQL
            // float division).
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let degree_int = row.degree.clamp(0.0, 100.0) as u32;
            writeln!(
                w,
                "{},{},{},{},{},{},{}",
                quote_if_needed(&row.entity_a),
                quote_if_needed(&row.entity_b),
                degree_int,
                row.average_revs,
                row.revs_a,
                row.revs_b,
                row.shared,
            )
            .map_err(CodeLoreError::Io)?;
        }
        return Ok(());
    }
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

pub fn write_soc_csv<W: Write>(rows: &[crate::analyses::soc::SocRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,soc").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.entity), row.soc).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_clone_coupling_csv<W: Write>(rows: &[CloneCouplingRow], w: &mut W) -> Result<()> {
    // 19 columns: 18 from CloneCouplingRow + `at_risk`.
    writeln!(
        w,
        "clone-group,fingerprint,file-a,file-b,entity-a,entity-b,\
         start-line-a,end-line-a,start-line-b,end-line-b,\
         node-count,similarity,shared-revs,support-a,support-b,\
         degree-pct,p-value,combined-score,at-risk"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{:.4},{},{},{},{:.4},{:.4},{:.4},{}",
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
            row.at_risk,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_function_coupling_csv<W: Write>(
    rows: &[crate::analyses::function_coupling::FunctionCouplingRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "a,b,co-changes,a-changes,b-changes,confidence,p-value")
        .map_err(CodeLoreError::Io)?;
    for row in rows {
        let p = row.p_value.map_or_else(String::new, |v| format!("{v:.4}"));
        writeln!(
            w,
            "{},{},{},{},{},{:.4},{}",
            quote_if_needed(&row.a),
            quote_if_needed(&row.b),
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
