//! `crossing` analysis — the "X" of Kazman & Cai's DV8 hotspot patterns
//! (Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns*).
//!
//! A **Crossing** is a file that is simultaneously a hub *and* a sink:
//! high fan-in (many files import it) **and** high fan-out (it imports
//! many), whose changes co-occur with **both** the files that depend on
//! it (upstream) **and** the files it depends on (downstream). It is the
//! point where two change-flows cross, coupling its upstream and
//! downstream together *through itself* — the hardest shape to change
//! safely, because edits ripple in both directions at once.
//!
//! ## Fusion — two graphs `CodeLore` already builds
//!
//! - **Structural:** the `imports` table gives, per file, both its
//!   importers (in-edges → fan-in) and its imports (out-edges →
//!   fan-out).
//! - **Temporal:** [`coupling::run_coupling`](crate::analyses::coupling::run_coupling)
//!   Fisher-significant co-change partners.
//!
//! A crossing co-changes with ≥ 1 importer AND ≥ 1 import — both flows
//! are live, not just structurally present.
//!
//! ## Calibration
//!
//! Both fan-in and fan-out must reach [`DEFAULT_MIN_FAN`] (the "X" needs
//! genuine breadth on both axes), and at least one co-change partner must
//! sit on each side. `crossing_score = coupled_upstream +
//! coupled_downstream` ranks by how much change actually flows through
//! the crossing in both directions. Accuracy follows the import
//! resolver's language coverage, same caveat as `god_classes` fan-in.

use std::collections::{HashMap, HashSet};

use crate::facts::FactsDb;
use crate::{Options, Result};

/// Minimum fan-in and fan-out for a file to count as a structural "X".
pub const DEFAULT_MIN_FAN: u32 = 3;

/// One crossing finding.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CrossingRow {
    /// The crossing file.
    pub path: String,
    /// Distinct files importing this one (fan-in / afferent).
    pub fan_in: u32,
    /// Distinct files this one imports (fan-out / efferent, resolved).
    pub fan_out: u32,
    /// Importers that also co-change with this file (upstream flow).
    pub coupled_upstream: u32,
    /// Imports that also co-change with this file (downstream flow).
    pub coupled_downstream: u32,
    /// `coupled_upstream + coupled_downstream` — total change flowing
    /// through the crossing in both directions. Higher = worse.
    pub crossing_score: f64,
}

/// Run the `crossing` analysis. Returns crossings ranked by composite
/// crossing score (highest first).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` query errors
/// (propagated from the imports scan or the inner coupling run).
#[tracing::instrument(name = "crossing", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_crossing(db: &FactsDb, opts: &Options) -> Result<Vec<CrossingRow>> {
    // Resolved import edges. One pass builds both directions:
    //   importers[target] = files importing target (in-edges)
    //   imports[src]      = files src imports        (out-edges)
    let import_pairs: Vec<(String, String)> = crate::analyses::query::query_map_collect(
        db,
        "SELECT src_path, target_path FROM imports WHERE target_path IS NOT NULL",
        [],
        "crossing imports",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    if import_pairs.is_empty() {
        return Ok(Vec::new());
    }
    let mut importers: HashMap<String, HashSet<String>> = HashMap::new();
    let mut imports: HashMap<String, HashSet<String>> = HashMap::new();
    for (src, target) in import_pairs {
        importers
            .entry(target.clone())
            .or_default()
            .insert(src.clone());
        imports.entry(src).or_default().insert(target);
    }

    // Fisher-significant co-change partners per file, both directions.
    // Memoized inner call; `--rows N` must not cap the partner pool.
    let coupling_rows = crate::analyses::coupling::run_coupling(db, &opts.with_no_row_limit())?;
    let partners = crate::analyses::coupling::partner_index(&coupling_rows);

    let mut out: Vec<CrossingRow> = Vec::new();
    for (path, in_set) in &importers {
        let fan_in = u32::try_from(in_set.len()).unwrap_or(u32::MAX);
        if fan_in < DEFAULT_MIN_FAN {
            continue;
        }
        // Must also import a comparable breadth — the other arm of the X.
        let Some(out_set) = imports.get(path) else {
            continue;
        };
        let fan_out = u32::try_from(out_set.len()).unwrap_or(u32::MAX);
        if fan_out < DEFAULT_MIN_FAN {
            continue;
        }
        // Change must actually flow on BOTH arms (DV8 requires both).
        let Some(pt) = partners.get(path) else {
            continue;
        };
        let coupled_upstream = u32::try_from(in_set.intersection(pt).count()).unwrap_or(u32::MAX);
        let coupled_downstream =
            u32::try_from(out_set.intersection(pt).count()).unwrap_or(u32::MAX);
        if coupled_upstream == 0 || coupled_downstream == 0 {
            continue;
        }
        out.push(CrossingRow {
            path: path.clone(),
            fan_in,
            fan_out,
            coupled_upstream,
            coupled_downstream,
            crossing_score: f64::from(coupled_upstream) + f64::from(coupled_downstream),
        });
    }

    out.sort_by(|a, b| {
        b.crossing_score
            .partial_cmp(&a.crossing_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
