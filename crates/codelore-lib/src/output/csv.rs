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
    // RFC 4180 §2.5: fields containing `,`, `"`, CR, or LF MUST be quoted.
    // Missing `\r` here would split a row in two if an author name or commit
    // metadata carried a bare carriage return (rare but legal in git's byte
    // stream).
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
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
    // `mi` is the SEI-variant Maintainability Index per Coleman 1994 / SEI 1997.
    // `mi-rank` is the file's repo-relative percentile in `[0,1]` (PERCENT_RANK
    // over the analyzed repo's MI distribution). `mi-band` is the
    // repo-relative Low/Moderate/High classification derived from the rank —
    // see `crates/codelore-lib/src/analyses/mi.rs` for the empirical
    // calibration that motivates relative bands over absolute Coleman/SEI
    // thresholds. Cells are empty when `mi` is unknown.
    //
    // `ai-pct` is the share of commits touching this file that carry an
    // AI-attribution signal (ai-assisted | ai-authored per identity::bots).
    // Range [0, 100]; absolute interpretation is meaningful across repos.
    writeln!(
        w,
        "entity,revisions,cognitive,code-health,hotspot-score,mi,mi-rank,mi-band,ai-pct"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        let mi_cell = match row.mi {
            Some(v) => format!("{v:.2}"),
            None => String::new(),
        };
        let (rank_cell, band_cell) = match row.mi_rank {
            Some(rank) if rank.is_finite() => (
                format!("{rank:.4}"),
                crate::analyses::mi::MiBand::from_rank(rank)
                    .as_str()
                    .to_owned(),
            ),
            _ => (String::new(), String::new()),
        };
        let ai_cell = match row.ai_pct {
            Some(v) if v.is_finite() => format!("{v:.2}"),
            _ => String::new(),
        };
        writeln!(
            w,
            "{},{},{:.2},{:.2},{:.4},{},{},{},{}",
            quote_if_needed(&row.path),
            row.revisions,
            row.cognitive,
            row.code_health,
            row.hotspot_score,
            mi_cell,
            rank_cell,
            band_cell,
            ai_cell
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

pub fn write_code_age_csv<W: Write>(
    rows: &[CodeAgeRow],
    w: &mut W,
    code_maat_compat: bool,
) -> Result<()> {
    // PAR-5: code-maat emits `entity,age-months` (hyphenated, single
    // metric column); CodeLore's modern default surfaces second-level
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

/// PAR-5: under `--code-maat-compat` the header uses code-maat's
/// `author,peer` column names (matching `communication.clj`'s output);
/// `CodeLore`'s modern default uses the symmetric `author-a,author-b`
/// pair which is clearer about the equality of roles.
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

/// PAR-5: under `--code-maat-compat`, emit code-maat's exact 3-column
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

/// PAR-5: code-maat's `summary` emits `statistic,value`; `CodeLore` uses
/// the slightly clearer `metric,value`. Under `--code-maat-compat`,
/// emit the legacy header so downstream tooling (`code-maat`-targeted
/// dashboards) keeps parsing.
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

pub fn write_soc_csv<W: Write>(rows: &[crate::analyses::soc::SocRow], w: &mut W) -> Result<()> {
    writeln!(w, "entity,soc").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "{},{}", quote_if_needed(&row.entity), row.soc).map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_centrality_csv<W: Write>(
    rows: &[crate::analyses::centrality::CentralityRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity,degree,weighted_degree,revs").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.6},{}",
            quote_if_needed(&row.entity),
            row.degree,
            row.weighted_degree,
            row.revs,
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

pub fn write_clone_coupling_csv<W: Write>(rows: &[CloneCouplingRow], w: &mut W) -> Result<()> {
    // 19 columns: 18 from CloneCouplingRow + T9 `at_risk`.
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

#[cfg(test)]
mod tests {
    use super::quote_if_needed;

    #[test]
    fn quotes_carriage_return() {
        assert_eq!(quote_if_needed("a\rb"), "\"a\rb\"");
    }

    #[test]
    fn quotes_comma_and_doubles_quotes() {
        assert_eq!(quote_if_needed("a,\"b\""), "\"a,\"\"b\"\"\"");
    }

    #[test]
    fn leaves_plain_string_alone() {
        assert_eq!(quote_if_needed("plain"), "plain");
    }
}
