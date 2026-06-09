//! Markdown output emitter for `$GITHUB_STEP_SUMMARY` and human-readable
//! CI artifacts.

use crate::analyses::{
    authors::AuthorsRow,
    churn::{AbsChurnRow, AuthorChurnRow, EntityChurnRow},
    clone_coupling::CloneCouplingRow,
    clones::ClonesRow,
    code_age::CodeAgeRow,
    code_health::CodeHealthRow,
    communication::CommunicationRow,
    coupling::CouplingRow,
    hotspots::HotspotRow,
    ownership::OwnershipRow,
    summary::SummaryRow,
};
use crate::{CodeLoreError, Result};
use std::io::Write;

fn header<W: Write>(w: &mut W, title: &str) -> Result<()> {
    writeln!(w, "# {title}").map_err(CodeLoreError::Io)?;
    writeln!(w).map_err(CodeLoreError::Io)?;
    Ok(())
}

pub fn write_revisions_markdown<W: Write>(rows: &[(String, u32)], w: &mut W) -> Result<()> {
    header(w, "CodeLore revisions")?;
    writeln!(w, "| Entity | Revisions |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|").map_err(CodeLoreError::Io)?;
    for (path, n) in rows {
        writeln!(w, "| `{path}` | {n} |").map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_hotspots_markdown<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore hotspots")?;
    writeln!(
        w,
        "| Entity | Revisions | Cognitive | Code Health | Score |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {:.2} | {:.2} | {:.4} |",
            row.path, row.revisions, row.cognitive, row.code_health, row.hotspot_score
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_health_markdown<W: Write>(rows: &[CodeHealthRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore code-health")?;
    writeln!(w, "| Entity | Cognitive | Score |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {:.2} | {:.2} |",
            row.path, row.cognitive, row.score
        )
        .map_err(CodeLoreError::Io)?;
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
            row.path, row.age_months, row.age_days, row.last_modified
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
            row.author, row.added, row.deleted, row.commits
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
            row.path, row.added, row.deleted, row.commits
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_communication_markdown<W: Write>(rows: &[CommunicationRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore communication")?;
    writeln!(w, "| Author A | Author B | Shared | Average | Strength |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {:.2} |",
            row.author_a, row.author_b, row.shared, row.average, row.strength
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_ownership_markdown<W: Write>(rows: &[OwnershipRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore code-ownership")?;
    writeln!(w, "| Entity | Main Author | Total Revs | Fractal Value |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {:.4} |",
            row.path, row.main_author, row.total_revs, row.fractal_value
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_coupling_markdown<W: Write>(rows: &[CouplingRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore change-coupling")?;
    writeln!(w, "| Entity A | Entity B | Shared | Degree | Fisher p |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | `{}` | {} | {:.2}% | {:.4} |",
            row.entity_a, row.entity_b, row.shared, row.degree, row.fisher_p
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
        writeln!(w, "| {} | {} |", row.metric, row.value).map_err(CodeLoreError::Io)?;
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
            row.entity,
            row.function,
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

pub fn write_authors_markdown<W: Write>(rows: &[AuthorsRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore authors (per-entity author breakdown)")?;
    writeln!(
        w,
        "| Entity | Authors | Humans | Bots | Revs | Last author | Last modified |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            row.entity,
            row.n_authors,
            row.n_humans,
            row.n_bots,
            row.n_revs,
            row.last_author,
            row.last_modified,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_top_committers_markdown<W: Write>(
    rows: &[crate::analyses::top_committers::TopCommittersRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore top-committers")?;
    writeln!(
        w,
        "| Author | Commits | LoC added | LoC deleted | First commit | Last commit | Bot |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} |",
            row.author,
            row.commits,
            row.loc_added,
            row.loc_deleted,
            row.first_commit,
            row.last_commit,
            if row.is_bot { "yes" } else { "no" },
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
        writeln!(w, "| `{}` | {} |", row.entity, row.soc).map_err(CodeLoreError::Io)?;
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
        writeln!(w, "| `{}` | {} |", row.entity, row.matches).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

fn write_main_dev_markdown_with_headers<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
    title: &str,
    metric_label: &str,
    total_label: &str,
) -> Result<()> {
    header(w, title)?;
    writeln!(
        w,
        "| Entity | Main Dev | {metric_label} | {total_label} | Ownership |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {:.2} |",
            row.entity, row.main_dev, row.metric, row.total, row.ownership,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_main_dev_markdown<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_markdown_with_headers(
        rows,
        w,
        "CodeLore main-dev (by lines added)",
        "Added",
        "Total Added",
    )
}

pub fn write_main_dev_by_revs_markdown<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_markdown_with_headers(
        rows,
        w,
        "CodeLore main-dev (by revision count)",
        "Revisions",
        "Total Revisions",
    )
}

pub fn write_main_dev_by_deletions_markdown<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_markdown_with_headers(
        rows,
        w,
        "CodeLore main-dev-by-deletions (alias: refactoring-main-dev)",
        "Removed",
        "Total Removed",
    )
}

pub fn write_entity_effort_markdown<W: Write>(
    rows: &[crate::analyses::entity_effort::EntityEffortRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore entity-effort")?;
    writeln!(w, "| Entity | Author | Author Revs | Total Revs |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} |",
            row.entity, row.author, row.author_revs, row.total_revs,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_entity_ownership_markdown<W: Write>(
    rows: &[crate::analyses::entity_ownership::EntityOwnershipRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore entity-ownership")?;
    writeln!(w, "| Entity | Author | Added | Deleted |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} |",
            row.entity, row.author, row.added, row.deleted,
        )
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
        "| Group | File A | File B | Shared | Degree | Combined |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | `{}` | {} | {:.2}% | {:.4} |",
            row.clone_group_id,
            row.file_a,
            row.file_b,
            row.shared_revs,
            row.degree_pct * 100.0,
            row.combined_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
