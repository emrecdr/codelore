//! CSV emitters for VCS-history basics: revisions, churn, code age,
//! stale code, commit messages, and the code-maat repository summary.

use super::quote_if_needed;
use crate::analyses::churn::{AbsChurnRow, AuthorChurnRow, EntityChurnRow};
use crate::analyses::code_age::CodeAgeRow;
use crate::analyses::summary::SummaryRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

pub fn write_revisions_csv<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    writeln!(w, "entity,n-revs").map_err(CodeLoreError::Io)?;
    for (entity, n) in rows {
        writeln!(w, "{},{}", quote_if_needed(entity), n).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_age_csv<W: Write>(
    rows: &[CodeAgeRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    // Under `--code-maat-compat`, emit `entity,age-months` (hyphenated,
    // single metric column). CodeLore's modern default surfaces second-level
    // precision (`age_days`) and a triage context column (`last_modified`).
    if code_maat_compat {
        writeln!(w, "entity,age-months").map_err(CodeLoreError::Io)?;
        for row in rows {
            writeln!(w, "{},{}", quote_if_needed(&row.path), row.age_months)
                .map_err(CodeLoreError::Io)?;
        }
    } else {
        writeln!(w, "entity,age_months,age_days,last_modified").map_err(CodeLoreError::Io)?;
        for row in rows {
            writeln!(
                w,
                "{},{},{},{}",
                quote_if_needed(&row.path),
                row.age_months,
                row.age_days,
                row.last_modified
            )
            .map_err(CodeLoreError::Io)?;
        }
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

/// code-maat's `summary` emits `statistic,value`; `CodeLore` uses the
/// slightly clearer `metric,value`. Under `--code-maat-compat`, emit the
/// legacy header so downstream tooling (code-maat-targeted dashboards)
/// keeps parsing.
pub fn write_summary_csv<W: Write>(
    rows: &[SummaryRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    let header = if code_maat_compat {
        "statistic,value"
    } else {
        "metric,value"
    };
    writeln!(w, "{header}").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.metric), row.value).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `stale-code` CSV emitter.
pub fn write_stale_code_csv<W: Write>(
    rows: &[crate::analyses::stale_code::StaleCodeRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "path,last_touched,months_since_touched,max_cognitive")
        .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.2}",
            quote_if_needed(&row.path),
            row.last_touched,
            row.months_since_touched,
            row.max_cognitive,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_messages_csv<W: Write>(
    rows: &[crate::analyses::messages::MessagesRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity,matches").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.entity), row.matches)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
