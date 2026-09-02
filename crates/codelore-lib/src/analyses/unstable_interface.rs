//! `unstable-interface` analysis — heavily-depended-on files that
//! change often AND drag their dependents along when they do.
//!
//! The "Unstable Interface" of Kazman & Cai's DV8 hotspot patterns
//! (Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns*): an anchor file
//! that (i) many files import (high afferent coupling / fan-in), (ii)
//! changes frequently (it is itself unstable), and (iii) co-changes
//! with the very files that depend on it — so its instability
//! propagates outward. A stable, widely-imported interface is healthy;
//! an *unstable* one is a structural debt amplifier.
//!
//! ## Fusion — three facts `CodeLore` already holds
//!
//! - **Structural fan-in:** distinct importers per file, from the
//!   `imports` table (the afferent-coupling term `god_classes` already
//!   computes).
//! - **Instability:** per-file revision count, from
//!   [`revisions::run_revisions`](crate::analyses::revisions::run_revisions).
//! - **Propagation:** importers that are also Fisher-significant
//!   co-change partners, from
//!   [`coupling::run_coupling`](crate::analyses::coupling::run_coupling).
//!
//! ## Calibration
//!
//! A file qualifies when it has at least [`DEFAULT_MIN_FAN_IN`]
//! importers, has been revised at least `opts.min_revs` times, and
//! co-changes with at least one of its importers (the DV8 definition
//! requires the instability to actually reach a dependent). The
//! composite `instability_score = revisions × coupled_dependents`
//! ranks files where high own-churn meets wide dependent propagation.
//! Thresholds follow the import resolver's language coverage, exactly
//! as `god_classes` fan-in does.

use std::collections::{HashMap, HashSet};

use crate::facts::FactsDb;
use crate::{Options, Result};

/// Minimum distinct importers for a file to count as an "interface".
/// A file imported by one or two others is just a dependency, not an
/// interface whose instability is worth surfacing.
pub const DEFAULT_MIN_FAN_IN: u32 = 3;

/// One unstable-interface finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UnstableInterfaceRow {
    /// The interface file.
    pub path: String,
    /// Distinct files importing this file (afferent coupling, resolved
    /// imports only — language coverage follows the resolver).
    pub fan_in: u32,
    /// Commits touching this file — its own change frequency. High =
    /// unstable.
    pub revisions: u32,
    /// Importers of this file that are ALSO Fisher-significant
    /// co-change partners — the dependents the instability propagates
    /// to.
    pub coupled_dependents: u32,
    /// `revisions × coupled_dependents`. Bigger = an unstable interface
    /// dragging more of its dependents. Use for ranking; report the
    /// components when explaining why a file flagged.
    pub instability_score: f64,
}

/// Run the `unstable-interface` analysis. Returns interfaces ranked by
/// composite instability score (highest first).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the imports scan, the revisions run, or the inner
/// coupling run).
#[tracing::instrument(name = "unstable-interface", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_unstable_interface(db: &FactsDb, opts: &Options) -> Result<Vec<UnstableInterfaceRow>> {
    // Importers per file (resolved edges only). fan-in = set size.
    let import_pairs: Vec<(String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT target_path, src_path FROM imports WHERE target_path IS NOT NULL",
        [],
        "unstable-interface imports",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    if import_pairs.is_empty() {
        return Ok(Vec::new());
    }
    let mut importers: HashMap<String, HashSet<String>> = HashMap::new();
    for (target, src) in import_pairs {
        importers.entry(target).or_default().insert(src);
    }

    // Per-file revision counts (own instability). Strip the row limit so
    // every interface candidate is covered.
    let revisions: HashMap<String, u32> =
        crate::analyses::revisions::run_revisions(db, &opts.with_no_row_limit())?
            .into_iter()
            .collect();

    // Fisher-significant co-change partners per file, both directions.
    // Memoized inner call; `--rows N` must not cap the partner pool.
    let coupling_rows = crate::analyses::coupling::run_coupling(db, &opts.with_no_row_limit())?;
    let partners = crate::analyses::coupling::partner_index(&coupling_rows);

    let mut out: Vec<UnstableInterfaceRow> = Vec::new();
    for (path, deps) in &importers {
        let fan_in = u32::try_from(deps.len()).unwrap_or(u32::MAX);
        if fan_in < DEFAULT_MIN_FAN_IN {
            continue;
        }
        let revs = revisions.get(path).copied().unwrap_or(0);
        if revs < opts.min_revs {
            continue;
        }
        // Importers that also co-change with this file — the propagated
        // dependents. The DV8 definition requires ≥ 1.
        let coupled_dependents = partners.get(path).map_or(0, |pt| {
            u32::try_from(deps.intersection(pt).count()).unwrap_or(u32::MAX)
        });
        if coupled_dependents == 0 {
            continue;
        }
        out.push(UnstableInterfaceRow {
            path: path.clone(),
            fan_in,
            revisions: revs,
            coupled_dependents,
            instability_score: f64::from(revs) * f64::from(coupled_dependents),
        });
    }

    // Rank by composite score; tie-break on path for determinism.
    out.sort_by(|a, b| {
        b.instability_score
            .total_cmp(&a.instability_score)
            .then_with(|| a.path.cmp(&b.path))
    });
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
