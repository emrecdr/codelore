//! Markdown emitters for the knowledge and social family: ownership, authors,
//! main-dev, familiarity, coordination, pair programming, bus factor, knowledge
//! islands, entity effort/ownership, team composition, and marginal-owner risk.

use super::{escape_md_cell, header};
use crate::analyses::authors::AuthorsRow;
use crate::analyses::communication::CommunicationRow;
use crate::analyses::ownership::OwnershipRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

pub fn write_communication_markdown<W: Write>(rows: &[CommunicationRow], w: &mut W) -> Result<()> {
    header(w, "CodeLore communication")?;
    writeln!(w, "| Author A | Author B | Shared | Average | Strength |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {:.2} |",
            escape_md_cell(&row.author_a),
            escape_md_cell(&row.author_b),
            row.shared,
            row.average,
            row.strength
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
            escape_md_cell(&row.path),
            escape_md_cell(&row.main_author),
            row.total_revs,
            row.fractal_value
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
            escape_md_cell(&row.entity),
            row.n_authors,
            row.n_humans,
            row.n_bots,
            row.n_revs,
            escape_md_cell(&row.last_author),
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
            escape_md_cell(&row.author),
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

pub fn write_code_familiarity_markdown<W: Write>(
    rows: &[crate::analyses::code_familiarity::CodeFamiliarityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore code-familiarity")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No knowledge data — ensure commits exist and complexity metrics are available._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Scope | Familiarity % | Active Authors | Total Authors | Islands % | Verdict |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {:.1} | {} | {} | {:.1} | {} |",
            escape_md_cell(&row.scope),
            row.familiarity_pct,
            row.active_authors,
            row.total_authors,
            row.islands_pct,
            escape_md_cell(&row.verdict),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_coordination_needs_markdown<W: Write>(
    rows: &[crate::analyses::coordination_needs::CoordinationNeedsRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore coordination-needs")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No coordination data — ensure knowledge shares are available._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Authors | Fragmentation | Interleave | Co-change Entropy | Tier | Health Band |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {:.2} | {:.2} | {:.4} | {} | {} |",
            escape_md_cell(&row.path),
            row.authors,
            row.fragmentation,
            row.interleave,
            row.cochange_entropy,
            escape_md_cell(&row.tier),
            escape_md_cell(&row.health_band),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// pair-programming markdown emitter.
pub fn write_pair_programming_markdown<W: Write>(
    rows: &[crate::analyses::pair_programming::PairRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore pair-programming")?;
    writeln!(w, "| Author A | Author B | Pair commits |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} |",
            escape_md_cell(&row.author_a),
            escape_md_cell(&row.author_b),
            row.pair_commits,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// bus-factor markdown emitter.
pub fn write_bus_factor_markdown<W: Write>(
    rows: &[crate::analyses::bus_factor::BusFactorRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore bus-factor")?;
    writeln!(
        w,
        "| Module | Total commits | Bus factor | Top contributor | Top share | Model |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {} | {} | {:.1}% | {} |",
            escape_md_cell(&row.module),
            row.total_commits,
            row.bus_factor,
            escape_md_cell(&row.top_contributor),
            row.top_contributor_share * 100.0,
            row.model,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_knowledge_islands_markdown<W: Write>(
    rows: &[crate::analyses::knowledge_islands::KnowledgeIslandRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore knowledge-islands (bus-factor risk)")?;
    writeln!(
        w,
        "| Entity | Main author | Ownership % | Days since main active | Last main commit | Substantial others |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {:.2} | {} | {} | {} |",
            escape_md_cell(&row.entity),
            escape_md_cell(&row.main_author),
            row.ownership_pct,
            row.days_since_main_active,
            row.last_main_author_commit,
            row.n_substantial_others,
        )
        .map_err(CodeLoreError::Io)?;
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
            escape_md_cell(&row.entity),
            escape_md_cell(&row.main_dev),
            row.metric,
            row.total,
            row.ownership,
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
            escape_md_cell(&row.entity),
            escape_md_cell(&row.author),
            row.author_revs,
            row.total_revs,
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
            escape_md_cell(&row.entity),
            escape_md_cell(&row.author),
            row.added,
            row.deleted,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `team-composition` markdown emitter.
///
/// The `__summary__` carrier row (bucket-share percentages, not per-author
/// data) is not a data row and is skipped.
pub fn write_team_composition_markdown<W: Write>(
    rows: &[crate::analyses::team_composition::TeamCompositionRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore team-composition")?;
    if rows.is_empty() {
        writeln!(w, "_No commit history found._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Author | Tenure (d) | Bucket | Breadth OK | Active | Commits | Files | Onboarding (wk) |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        if row.author == "__summary__" {
            continue;
        }
        let ob = row
            .onboarding_weeks
            .map_or_else(|| "—".to_string(), |v| v.to_string());
        writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            escape_md_cell(&row.author),
            row.tenure_days,
            escape_md_cell(&row.bucket),
            row.veteran_breadth_ok,
            row.active,
            row.commits,
            row.files_touched,
            ob,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_marginal_owner_risk_markdown<W: Write>(
    rows: &[crate::analyses::marginal_owner_risk::MarginalOwnerRiskRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore marginal-owner-risk")?;
    if rows.is_empty() {
        writeln!(w, "_No marginal-owner risk detected._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Path | Band | Top Active Share | Risk |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | {:.4} | {} |",
            escape_md_cell(&row.path),
            escape_md_cell(&row.band),
            row.top_active_share,
            escape_md_cell(&row.risk),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod team_composition_tests {
    use crate::analyses::team_composition::TeamCompositionRow;

    #[test]
    fn team_composition_markdown_skips_summary_row() {
        let rows = vec![
            TeamCompositionRow {
                author: "alice".to_string(),
                tenure_days: 110,
                bucket: "experienced".to_string(),
                veteran_breadth_ok: false,
                active: true,
                commits: 12,
                files_touched: 3,
                onboarding_weeks: None,
            },
            TeamCompositionRow {
                author: "__summary__".to_string(),
                tenure_days: 0,
                bucket: "onboarded=33.3% experienced=66.7% veteran=0.0%".to_string(),
                veteran_breadth_ok: false,
                active: false,
                commits: 0,
                files_touched: 0,
                onboarding_weeks: None,
            },
        ];
        let mut buf = Vec::new();
        super::write_team_composition_markdown(&rows, &mut buf).expect("write markdown");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(
            !out.contains("__summary__"),
            "markdown output must not contain the __summary__ carrier row: {out}"
        );
        assert!(
            out.contains("alice"),
            "markdown output must contain real author rows: {out}"
        );
    }
}
