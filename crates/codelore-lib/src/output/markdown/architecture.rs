//! Markdown emitters for the architecture / import-graph family: god classes,
//! instability, cycles, roles, metrics, trends, boundary crossings, centrality,
//! and communities.

use super::{escape_md_cell, header};
use crate::{CodeLoreError, Result};
use std::borrow::Cow;
use std::io::Write;

/// god-classes markdown emitter — cognitive × `fan_in` × `fan_out`
/// intersection. Top-of-list files have all three pulling up.
pub fn write_god_classes_markdown<W: Write>(
    rows: &[crate::analyses::god_classes::GodClassRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore god-classes")?;
    writeln!(w, "| Path | Cognitive | Fan-in | Fan-out | God score |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {:.0} | {} | {} | {:.3} |",
            escape_md_cell(&row.path),
            row.cognitive,
            row.fan_in,
            row.fan_out,
            row.god_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// instability markdown emitter — Martin Ca/Ce/Instability per file.
pub fn write_instability_markdown<W: Write>(
    rows: &[crate::analyses::instability::InstabilityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore instability")?;
    if rows.is_empty() {
        writeln!(w, "_No resolved import graph — nothing to score._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Path | Ca | Ce | Instability |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {:.2} |",
            escape_md_cell(&row.path),
            row.ca,
            row.ce,
            row.instability,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// architecture-metrics markdown emitter — repo-level `(metric, value)`.
/// `cycle-origins` markdown emitter — when each HEAD cycle first formed.
pub fn write_cycle_origins_markdown<W: Write>(
    rows: &[crate::analyses::cycle_origins::CycleOriginRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore cycle origins")?;
    if rows.is_empty() {
        writeln!(w, "_No dependency cycles at HEAD — nothing to trace._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Size | Formed at | Date | Members |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | {} | {} |",
            row.size,
            escape_md_cell(&row.formed_at_rev),
            escape_md_cell(&row.formed_at_date),
            escape_md_cell(&row.members),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `architecture-trend` markdown emitter — structural decay over time.
pub fn write_architecture_trend_markdown<W: Write>(
    rows: &[crate::analyses::architecture_trend::ArchitectureTrendRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture trend")?;
    if rows.is_empty() {
        writeln!(w, "_No commit history to sample._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Date | Rev | Files | Propagation cost | Cycles | Largest cycle |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | `{}` | {} | {:.1}% | {} | {} |",
            escape_md_cell(&row.date),
            escape_md_cell(&row.rev),
            row.files,
            row.propagation_cost * 100.0,
            row.cycle_count,
            row.largest_cycle,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `cycle-health` markdown emitter — per-SCC heat, verdict, extraction
/// candidate, and predicted propagation-cost drop.
pub fn write_cycle_health_markdown<W: Write>(
    rows: &[crate::analyses::cycle_health::CycleHealthRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore cycle-health")?;
    if rows.is_empty() {
        writeln!(w, "_No import cycles detected._").map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Cycle | Size | Members | Heat % | Verdict | Extract candidate | Predicted PC drop |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---:|---|---:|---|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        let drop_cell = match row.predicted_pc_drop {
            Some(d) => Cow::Owned(format!("{d:.2}")),
            None => Cow::Borrowed("—"),
        };
        writeln!(
            w,
            "| {} | {} | {} | {:.2} | {} | {} | {} |",
            row.cycle_id,
            row.size,
            escape_md_cell(&row.members_preview),
            row.heat_pct,
            row.verdict,
            escape_md_cell(&row.extract_candidate),
            drop_cell,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_architecture_metrics_markdown<W: Write>(
    rows: &[crate::analyses::architecture_metrics::ArchitectureMetricRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture-metrics")?;
    if rows.is_empty() {
        writeln!(w, "_No resolved import graph — no metrics to compute._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Metric | Value |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} |",
            escape_md_cell(&row.metric),
            escape_md_cell(&row.value),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// architecture-roles markdown emitter — per-file role + visibility reach.
pub fn write_architecture_roles_markdown<W: Write>(
    rows: &[crate::analyses::architecture_roles::ArchitectureRoleRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture-roles")?;
    if rows.is_empty() {
        writeln!(w, "_No resolved import graph — nothing to classify._")
            .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Role | VFI | VFO | In cycle | Level | Reach % |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|:--:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {} | {} | {:.1} |",
            escape_md_cell(&row.path),
            row.role,
            row.vfi,
            row.vfo,
            row.in_cycle,
            row.level,
            row.reach_pct,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// dependency-cycles markdown emitter — one row per (cycle, member).
pub fn write_dependency_cycles_markdown<W: Write>(
    rows: &[crate::analyses::dependency_cycles::DependencyCycleRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore dependency-cycles")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No dependency cycles — the resolved import graph is acyclic (or no imports resolved)._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Cycle | Size | Path |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| {} | {} | `{}` |",
            row.cycle_id,
            row.size,
            escape_md_cell(&row.path),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// modularity-violations markdown emitter — Fisher-significant
/// co-change pairs with no structural import edge.
pub fn write_modularity_violations_markdown<W: Write>(
    rows: &[crate::analyses::modularity_violations::ModularityViolationRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore modularity-violations")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No modularity violations — every Fisher-significant co-change pair also has a structural import edge (or no co-change pairs were found)._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(w, "| Entity A | Entity B | Shared | Degree | Fisher p |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | `{}` | {} | {:.2} | {:.4} |",
            escape_md_cell(&row.entity_a),
            escape_md_cell(&row.entity_b),
            row.shared,
            row.degree,
            row.fisher_p,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// crossing markdown emitter — structural "X" files coupling upstream
/// and downstream through themselves (DV8 Crossing).
pub fn write_crossing_markdown<W: Write>(
    rows: &[crate::analyses::crossing::CrossingRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore crossing")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No crossings — no file is both a wide hub and a wide sink that co-changes in both directions._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Fan-in | Fan-out | Coupled upstream | Coupled downstream | Crossing score |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {} | {:.1} |",
            escape_md_cell(&row.path),
            row.fan_in,
            row.fan_out,
            row.coupled_upstream,
            row.coupled_downstream,
            row.crossing_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// unstable-interface markdown emitter — interfaces whose instability
/// propagates to their dependents.
pub fn write_unstable_interface_markdown<W: Write>(
    rows: &[crate::analyses::unstable_interface::UnstableInterfaceRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore unstable-interface")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No unstable interfaces — no widely-imported file changes often enough to drag its dependents (or the import graph is empty)._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Path | Fan-in | Revisions | Coupled dependents | Instability score |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {} | {} | {:.1} |",
            escape_md_cell(&row.path),
            row.fan_in,
            row.revisions,
            row.coupled_dependents,
            row.instability_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// architecture-violations markdown emitter — one row per import
/// edge that crosses a forbidden layer boundary.
pub fn write_arch_violations_markdown<W: Write>(
    rows: &[crate::analyses::arch_violations::ArchViolationRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore architecture-violations")?;
    if rows.is_empty() {
        writeln!(
            w,
            "_No violations detected — either the rule set is empty (no `.codelore-arch-rules.toml` at the repo root) or every import edge respects the declared layer boundaries._"
        )
        .map_err(CodeLoreError::Io)?;
        return Ok(());
    }
    writeln!(
        w,
        "| Source file | Source layer | Target file | Target layer | Raw target |"
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | `{}` | {} | `{}` |",
            escape_md_cell(&row.src_path),
            escape_md_cell(&row.src_layer),
            escape_md_cell(&row.target_path),
            escape_md_cell(&row.target_layer),
            escape_md_cell(&row.raw_target),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_centrality_markdown<W: Write>(
    rows: &[crate::analyses::centrality::CentralityRow],
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore behavioural-coupling centrality")?;
    writeln!(w, "| Entity | Degree | Weighted | PageRank | Eigenvector |")
        .map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---:|---:|---:|---:|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "| `{}` | {} | {:.2} | {:.4} | {:.4} |",
            escape_md_cell(&row.path),
            row.degree,
            row.weighted_degree,
            row.pagerank,
            row.eigenvector,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_communities_markdown<W: Write>(
    result: &crate::analyses::communities::CommunitiesResult,
    w: &mut W,
) -> Result<()> {
    header(w, "CodeLore behavioural communities (Leiden partition)")?;
    writeln!(
        w,
        "**Modularity Q = {:.4}** across **{} communities**.\n",
        result.modularity, result.community_count,
    )
    .map_err(CodeLoreError::Io)?;
    writeln!(w, "| Community | Size | Entity |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---:|---:|---|").map_err(CodeLoreError::Io)?;
    for row in &result.rows {
        writeln!(
            w,
            "| {} | {} | `{}` |",
            row.community_id,
            row.community_size,
            escape_md_cell(&row.path),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
