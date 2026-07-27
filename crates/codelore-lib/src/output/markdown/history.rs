//! Markdown emitters for VCS-history basics: revisions, churn, code age,
//! stale code, commit messages, and the code-maat repository summary.

use super::{escape_md_cell, header};
use crate::analyses::churn::{AbsChurnRow, AuthorChurnRow, EntityChurnRow};
use crate::analyses::code_age::CodeAgeRow;
use crate::analyses::summary::SummaryRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

pub fn write_revisions_markdown<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    header(w, "CodeLore revisions")?;
    writeln!(w, "| Entity | Revisions |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|").map_err(CodeLoreError::Io)?;
    for (path, n) in rows {
        writeln!(w, "| `{}` | {n} |", escape_md_cell(path)).map_err(CodeLoreError::Io)?;
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
