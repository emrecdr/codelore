//! CSV emitters for the architecture / import-graph family: god classes,
//! instability, cycles, roles, metrics, trends, boundary crossings, centrality,
//! and communities.

use super::quote_if_needed;
use crate::{CodeLoreError, Result};
use std::io::Write;

/// `god-classes` CSV emitter — `fan_in` / `fan_out` / cognitive
/// intersection ranking per file.
pub fn write_god_classes_csv<W: Write>(
    rows: &[crate::analyses::god_classes::GodClassRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "path,cognitive,fan_in,fan_out,god_score").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{:.2},{},{},{:.4}",
            quote_if_needed(&row.path),
            row.cognitive,
            row.fan_in,
            row.fan_out,
            row.god_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `instability` CSV emitter — Martin Ca/Ce/Instability per file.
pub fn write_instability_csv<W: Write>(
    rows: &[crate::analyses::instability::InstabilityRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "path,ca,ce,instability").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.4}",
            quote_if_needed(&row.path),
            row.ca,
            row.ce,
            row.instability,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `architecture-metrics` CSV emitter — repo-level `(metric, value)` rows.
/// `cycle-origins` CSV emitter — the commit each HEAD dependency cycle
/// first formed at.
pub fn write_cycle_origins_csv<W: Write>(
    rows: &[crate::analyses::cycle_origins::CycleOriginRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "size,formed-at-rev,formed-at-date,members").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{}",
            row.size,
            quote_if_needed(&row.formed_at_rev),
            quote_if_needed(&row.formed_at_date),
            quote_if_needed(&row.members),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `architecture-trend` CSV emitter — structural-health metrics at
/// sampled historical revs.
pub fn write_architecture_trend_csv<W: Write>(
    rows: &[crate::analyses::architecture_trend::ArchitectureTrendRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "date,rev,files,propagation-cost,cycle-count,largest-cycle"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.4},{},{}",
            quote_if_needed(&row.date),
            quote_if_needed(&row.rev),
            row.files,
            row.propagation_cost,
            row.cycle_count,
            row.largest_cycle,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `cycle-health` CSV emitter — per-SCC heat, verdict, extraction candidate,
/// and predicted propagation-cost drop.
///
/// Header: `cycle-id,size,members,heat-pct,verdict,extract-candidate,predicted-pc-drop`
/// Floats are formatted `{:.2}`; an absent `predicted_pc_drop` (cycles above the
/// trial-removal bound) emits an empty cell.
pub fn write_cycle_health_csv<W: Write>(
    rows: &[crate::analyses::cycle_health::CycleHealthRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "cycle-id,size,members,heat-pct,verdict,extract-candidate,predicted-pc-drop"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        let drop_cell = match row.predicted_pc_drop {
            Some(d) => format!("{d:.2}"),
            None => String::new(),
        };
        writeln!(
            w,
            "{},{},{},{:.2},{},{},{}",
            row.cycle_id,
            row.size,
            quote_if_needed(&row.members_preview),
            row.heat_pct,
            row.verdict,
            quote_if_needed(&row.extract_candidate),
            drop_cell,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_architecture_metrics_csv<W: Write>(
    rows: &[crate::analyses::architecture_metrics::ArchitectureMetricRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "metric,value").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{}",
            quote_if_needed(&row.metric),
            quote_if_needed(&row.value),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `architecture-roles` CSV emitter — per-file Core/Shared/Control/
/// Periphery role + visibility fan-in/out.
pub fn write_architecture_roles_csv<W: Write>(
    rows: &[crate::analyses::architecture_roles::ArchitectureRoleRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "path,role,vfi,vfo,in_cycle,level,reach_pct").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{},{:.2}",
            quote_if_needed(&row.path),
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

/// `dependency-cycles` CSV emitter — one row per (cycle, member file);
/// rows sharing a `cycle_id` form one tangle.
pub fn write_dependency_cycles_csv<W: Write>(
    rows: &[crate::analyses::dependency_cycles::DependencyCycleRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "cycle_id,size,path").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{}",
            row.cycle_id,
            row.size,
            quote_if_needed(&row.path),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `modularity-violations` CSV emitter — Fisher-significant co-change
/// pairs with no structural import edge (the structure×history fusion).
pub fn write_modularity_violations_csv<W: Write>(
    rows: &[crate::analyses::modularity_violations::ModularityViolationRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity_a,entity_b,shared,degree,fisher_p").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{:.4},{:.6}",
            quote_if_needed(&row.entity_a),
            quote_if_needed(&row.entity_b),
            row.shared,
            row.degree,
            row.fisher_p,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `crossing` CSV emitter — structural "X" files that co-change in both
/// directions (DV8 Crossing).
pub fn write_crossing_csv<W: Write>(
    rows: &[crate::analyses::crossing::CrossingRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "path,fan_in,fan_out,coupled_upstream,coupled_downstream,crossing_score"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{},{:.1}",
            quote_if_needed(&row.path),
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

/// `unstable-interface` CSV emitter — heavily-imported files that
/// change often and co-change with their dependents.
pub fn write_unstable_interface_csv<W: Write>(
    rows: &[crate::analyses::unstable_interface::UnstableInterfaceRow],
    w: &mut W,
) -> Result<()> {
    writeln!(
        w,
        "path,fan_in,revisions,coupled_dependents,instability_score"
    )
    .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{:.2}",
            quote_if_needed(&row.path),
            row.fan_in,
            row.revisions,
            row.coupled_dependents,
            row.instability_score,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

/// `architecture-violations` CSV emitter — one row per import edge
/// that crosses a forbidden layer boundary per
/// `.codelore-arch-rules.toml`.
pub fn write_arch_violations_csv<W: Write>(
    rows: &[crate::analyses::arch_violations::ArchViolationRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "src_path,target_path,src_layer,target_layer,raw_target")
        .map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{},{},{}",
            quote_if_needed(&row.src_path),
            quote_if_needed(&row.target_path),
            quote_if_needed(&row.src_layer),
            quote_if_needed(&row.target_layer),
            quote_if_needed(&row.raw_target),
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_centrality_csv<W: Write>(
    rows: &[crate::analyses::centrality::CentralityRow],
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity,degree,weighted_degree,pagerank,eigenvector").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(
            w,
            "{},{},{:.6},{:.8},{:.8}",
            quote_if_needed(&row.path),
            row.degree,
            row.weighted_degree,
            row.pagerank,
            row.eigenvector,
        )
        .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}

pub fn write_communities_csv<W: Write>(
    result: &crate::analyses::communities::CommunitiesResult,
    w: &mut W,
) -> Result<()> {
    writeln!(w, "entity,community_id,community_size").map_err(CodeLoreError::Io)?;
    for row in &result.rows {
        writeln!(
            w,
            "{},{},{}",
            quote_if_needed(&row.path),
            row.community_id,
            row.community_size,
        )
        .map_err(CodeLoreError::Io)?;
    }
    // Trailing comment line carries the partition-level summary
    // (modularity, community count). CSV consumers can ignore comments;
    // the field is needed for the markdown / json variants and humans
    // running the csv variant will appreciate seeing the score.
    writeln!(
        w,
        "# partition_modularity={:.6} community_count={}",
        result.modularity, result.community_count,
    )
    .map_err(CodeLoreError::Io)?;
    Ok(())
}
