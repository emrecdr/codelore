//! CSV emitters for the knowledge and social family: ownership, authors,
//! main-dev, familiarity, coordination, pair programming, bus factor, knowledge
//! islands, entity effort/ownership, team composition, and marginal-owner risk.

use super::quote_if_needed;
use crate::analyses::authors::AuthorsRow;
use crate::analyses::communication::CommunicationRow;
use crate::analyses::ownership::OwnershipRow;
use crate::{CodeLoreError, Result};
use std::io::Write;

/// Under `--code-maat-compat` the header uses code-maat's `author,peer`
/// column names (matching `communication.clj`'s output); `CodeLore`'s
/// modern default uses the symmetric `author-a,author-b` pair which is
/// clearer about the equality of roles.
pub fn write_communication_csv<W: Write>(
    rows: &[CommunicationRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    let header = if code_maat_compat {
        "author,peer,shared,average,strength"
    } else {
        "author-a,author-b,shared,average,strength"
    };
    writeln!(w, "{header}").map_err(CodeLoreError::Io)?;
    for row in rows {
        // code-maat emits strength as a truncated integer (`(int …)`); the
        // compat SQL already floors it, so `as i64` reproduces code-maat's cell
        // verbatim. Modern mode keeps two-decimal precision.
        #[allow(clippy::cast_possible_truncation)]
        let strength_cell = if code_maat_compat {
            format!("{}", row.strength as i64)
        } else {
            format!("{:.2}", row.strength)
        };
        writeln!(
            w,
            "{},{},{},{},{}",
            quote_if_needed(&row.author_a),
            quote_if_needed(&row.author_b),
            row.shared,
            row.average,
            strength_cell
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// Under `--code-maat-compat`, emit code-maat's exact 3-column
/// fragmentation output (`entity,fractal-value,total-revs` — note the
/// column order; code-maat sorts the columns differently from `CodeLore`'s
/// natural `path / main_author / total_revs / fractal_value` shape).
/// `CodeLore`'s modern default surfaces `main_author` as a context column
/// for triage (single-author file? long-tail file? the operator sees
/// without re-running ownership).
pub fn write_ownership_csv<W: Write>(
    rows: &[OwnershipRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    if code_maat_compat {
        writeln!(w, "entity,fractal-value,total-revs").map_err(CodeLoreError::Io)?;
        for row in rows {
            writeln!(
                w,
                "{},{:.2},{}",
                quote_if_needed(&row.path),
                row.fractal_value,
                row.total_revs,
            )
            .map_err(CodeLoreError::Io)?;
        }
    } else {
        writeln!(w, "entity,main-author,total-revs,fractal-value").map_err(CodeLoreError::Io)?;
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
    }
    Ok(())
}

/// Modern columns for the `authors` analysis (per-entity author breakdown
/// — Bird et al. 2011 risk indicator). Code-maat's parity columns
/// `entity,n-authors,n-revs` are emitted under `--code-maat-compat`.
pub fn write_authors_csv<W: Write>(
    rows: &[AuthorsRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    if code_maat_compat {
        writeln!(w, "entity,n-authors,n-revs").map_err(CodeLoreError::Io)?;
        for row in rows {
            writeln!(
                w,
                "{},{},{}",
                quote_if_needed(&row.entity),
                row.n_authors,
                row.n_revs
            )
            .map_err(CodeLoreError::Io)?;
        }
    } else {
        writeln!(
            w,
            "entity,n_authors,n_humans,n_bots,n_revs,last_author,last_modified"
        )
        .map_err(CodeLoreError::Io)?;
        for row in rows {
            writeln!(
                w,
                "{},{},{},{},{},{},{}",
                quote_if_needed(&row.entity),
                row.n_authors,
                row.n_humans,
                row.n_bots,
                row.n_revs,
                quote_if_needed(&row.last_author),
                row.last_modified,
            )
            .map_err(CodeLoreError::Io)?;
        }
    }
    Ok(())
}

/// Per-author commit leaderboard — the previous behaviour of the
/// `authors` analysis, now exposed as a distinct first-class analysis.
pub fn write_top_committers_csv<W: Write>(
    rows: &[crate::analyses::top_committers::TopCommittersRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "author,commits,loc_added,loc_deleted,first_commit,last_commit,is_bot"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{}",
            quote_if_needed(&row.author),
            row.commits,
            row.loc_added,
            row.loc_deleted,
            row.first_commit,
            row.last_commit,
            row.is_bot,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_code_familiarity_csv<W: Write>(
    rows: &[crate::analyses::code_familiarity::CodeFamiliarityRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "scope,familiarity-pct,active-authors,total-authors,islands-pct,verdict"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{:.2},{},{},{:.2},{}",
            quote_if_needed(&row.scope),
            row.familiarity_pct,
            row.active_authors,
            row.total_authors,
            row.islands_pct,
            quote_if_needed(&row.verdict),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_coordination_needs_csv<W: Write>(
    rows: &[crate::analyses::coordination_needs::CoordinationNeedsRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "path,authors,fragmentation,interleave,cochange-entropy,tier,health-band"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.4},{:.4},{:.4},{},{}",
            quote_if_needed(&row.path),
            row.authors,
            row.fragmentation,
            row.interleave,
            row.cochange_entropy,
            quote_if_needed(&row.tier),
            quote_if_needed(&row.health_band),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `pair-programming` CSV emitter.
pub fn write_pair_programming_csv<W: Write>(
    rows: &[crate::analyses::pair_programming::PairRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "author_a,author_b,pair_commits").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{}",
            quote_if_needed(&row.author_a),
            quote_if_needed(&row.author_b),
            row.pair_commits,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `bus-factor` CSV emitter.
pub fn write_bus_factor_csv<W: Write>(
    rows: &[crate::analyses::bus_factor::BusFactorRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "module,total_commits,bus_factor,top_contributor,top_contributor_share,model"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{:.4},{}",
            quote_if_needed(&row.module),
            row.total_commits,
            row.bus_factor,
            quote_if_needed(&row.top_contributor),
            row.top_contributor_share,
            row.model,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// T8: per-file knowledge-loss risk (`knowledge-islands` analysis).
/// No code-maat equivalent — strict `CodeLore` extension; no compat-mode
/// header variant needed.
pub fn write_knowledge_islands_csv<W: Write>(
    rows: &[crate::analyses::knowledge_islands::KnowledgeIslandRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "entity,main_author,ownership_pct,days_since_main_active,last_main_author_commit,n_substantial_others"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.2},{},{},{}",
            quote_if_needed(&row.entity),
            quote_if_needed(&row.main_author),
            row.ownership_pct,
            row.days_since_main_active,
            row.last_main_author_commit,
            row.n_substantial_others,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// Helper for the three main-dev variants — only the column-2 header (and
/// the value's column-3 / column-4 names) differ. Modern-default headers
/// (revisions/total-revisions for rev-count variant); code-maat-compat
/// flag preserves the legacy lying-headers (`added`/`total-added` for revs)
/// for migration-tooling parity.
fn write_main_dev_csv_with_headers<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
    metric_name: &str,
    total_name: &str,
) -> Result<()> {
    writeln!(w, "entity,main-dev,{metric_name},{total_name},ownership")
        .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{:.2}",
            quote_if_needed(&row.entity),
            quote_if_needed(&row.main_dev),
            row.metric,
            row.total,
            row.ownership,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_main_dev_csv<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_csv_with_headers(rows, w, "added", "total-added")
}

pub fn write_main_dev_by_revs_csv<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    if code_maat_compat {
        write_main_dev_csv_with_headers(rows, w, "added", "total-added")
    } else {
        write_main_dev_csv_with_headers(rows, w, "revisions", "total-revisions")
    }
}

pub fn write_main_dev_by_deletions_csv<W: Write>(
    rows: &[crate::analyses::main_dev::MainDevRow],
    w: &mut W,
) -> Result<()> {
    write_main_dev_csv_with_headers(rows, w, "removed", "total-removed")
}

pub fn write_entity_effort_csv<W: Write>(
    rows: &[crate::analyses::entity_effort::EntityEffortRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity,author,author-revs,total-revs").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{}",
            quote_if_needed(&row.entity),
            quote_if_needed(&row.author),
            row.author_revs,
            row.total_revs,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_entity_ownership_csv<W: Write>(
    rows: &[crate::analyses::entity_ownership::EntityOwnershipRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity,author,added,deleted").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{}",
            quote_if_needed(&row.entity),
            quote_if_needed(&row.author),
            row.added,
            row.deleted,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `team-composition` CSV emitter.
///
/// CSV header: `author,tenure-days,bucket,veteran-breadth-ok,active,commits,files-touched,onboarding-weeks`
///
/// The `onboarding-weeks` column is empty for `NULL` (founders and authors who
/// never reached the weekly 80%-core set). The `__summary__` carrier row
/// (bucket-share percentages, not per-author data) is not a data row and is
/// skipped.
pub fn write_team_composition_csv<W: Write>(
    rows: &[crate::analyses::team_composition::TeamCompositionRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "author,tenure-days,bucket,veteran-breadth-ok,active,commits,files-touched,onboarding-weeks"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        if row.author == "__summary__" {
            continue;
        }
        let ob = row
            .onboarding_weeks
            .map_or_else(String::new, |v| v.to_string());
        writeln!(
            w,
            "{},{},{},{},{},{},{},{}",
            quote_if_needed(&row.author),
            row.tenure_days,
            quote_if_needed(&row.bucket),
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

pub fn write_marginal_owner_risk_csv<W: Write>(
    rows: &[crate::analyses::marginal_owner_risk::MarginalOwnerRiskRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "path,band,top-active-share,risk").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.4},{}",
            quote_if_needed(&row.path),
            quote_if_needed(&row.band),
            row.top_active_share,
            quote_if_needed(&row.risk),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn team_composition_csv_skips_summary_row() {
        use crate::analyses::team_composition::TeamCompositionRow;

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
        super::write_team_composition_csv(&rows, &mut buf).expect("write csv");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(
            !out.contains("__summary__"),
            "csv output must not contain the __summary__ carrier row: {out}"
        );
        assert!(
            out.contains("alice"),
            "csv output must contain real author rows: {out}"
        );
    }
}
