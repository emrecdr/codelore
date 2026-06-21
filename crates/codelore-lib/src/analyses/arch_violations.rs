//! `architecture-violations` analysis.
//!
//! Joins the `imports` table to the [`LayerRules`](crate::arch_rules::LayerRules)
//! config; reports every import edge that crosses a forbidden
//! layer boundary.
//!
//! ## Workflow
//!
//! 1. Discover `.codelore-arch-rules.toml` at the repo root (or
//!    explicit `--arch-rules-file`).
//! 2. Walk every `(src_path, target_path)` row in the `imports`
//!    table where both endpoints classify into a declared layer.
//! 3. Call [`LayerRules::validate`] for each; collect violations.
//!
//! ## Empty rule set
//!
//! When no `.codelore-arch-rules.toml` exists in the repo, the
//! analysis returns an empty Vec — users opt INTO architectural
//! validation rather than being forced to declare layers upfront.

use crate::arch_rules::LayerRules;
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchViolationRow {
    /// File that contains the offending import.
    pub src_path: String,
    /// Resolved target file of the offending import (only resolvable
    /// imports surface here — external imports skip).
    pub target_path: String,
    /// Layer the source belongs to (per the config's first-match
    /// path classification).
    pub src_layer: String,
    /// Layer the target belongs to.
    pub target_layer: String,
    /// Raw `target` string from the imports table — useful for
    /// jumping straight to the offending import line.
    pub raw_target: String,
}

/// Run the architecture-violations analysis. Returns a (possibly
/// empty) Vec of violations sorted by `(src_path, target_path)`.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` query errors or
/// arch-rules file I/O / parse errors.
#[tracing::instrument(name = "arch-violations", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_arch_violations(db: &FactsDb, opts: &Options) -> Result<Vec<ArchViolationRow>> {
    let rules = LayerRules::discover(&opts.repo_path)?;
    if rules.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = db
        .conn()
        .prepare(
            "SELECT src_path, target_path, target \
             FROM imports \
             WHERE target_path IS NOT NULL \
             ORDER BY src_path ASC, target_path ASC",
        )
        .map_err(|e| CodeLoreError::Analysis(format!("prepare arch-violations scan: {e}")))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query arch-violations scan: {e}")))?;
    let imports: Vec<(String, String, String)> =
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| CodeLoreError::Analysis(format!("collect arch-violations scan: {e}")))?;

    let mut out = Vec::new();
    for (src_path, target_path, raw_target) in imports {
        if let Some(v) = rules.validate(&src_path, &target_path) {
            out.push(ArchViolationRow {
                src_path,
                target_path,
                src_layer: v.src_layer,
                target_layer: v.target_layer,
                raw_target,
            });
        }
    }

    // Apply rows-limit if set.
    if let Some(limit) = opts.rows_limit {
        out.truncate(limit as usize);
    }
    Ok(out)
}
