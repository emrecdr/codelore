//! `god-classes` analysis — files combining high cognitive complexity
//! with high coupling (both inbound + outbound). Classic god-class
//! symptom (Brown et al. 1998 *`AntiPatterns`* §3.1; Riel 1996
//! *Object-Oriented Design Heuristics*).
//!
//! Consumes the `imports` table joined with `complexity_metrics`.
//! fan-in counts files that import THIS file; fan-out counts
//! distinct imports THIS file makes. The composite
//! `god_score = (cognitive/100) × (fan_in + fan_out)` ranks files
//! where every dimension is pulling up — not just any one.
//!
//! ## Why all three dimensions?
//!
//! A file with high cognitive complexity but low coupling is just a
//! gnarly algorithm — refactor candidate, not a god class. A file
//! with high fan-out but low cognitive is a thin façade — bus-factor
//! risk, not god class. A file with high fan-in but low cognitive is
//! a utility — the codebase needs it. **A god class is all three at
//! once**: it's complex, depends on many things, and many things
//! depend on it. The composite surfaces that intersection.
//!
//! ## Calibration
//!
//! Defaults: cognitive ≥ 30 (Sonar threshold), `fan_in + fan_out` ≥ 10.
//! Fan-in accuracy follows the import resolver's language coverage —
//! languages whose resolver doesn't yet populate `target_path` bias
//! the analysis toward fan-out + cognitive.

use duckdb::params;

use crate::facts::FactsDb;
use crate::{Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GodClassRow {
    pub path: String,
    pub cognitive: f64,
    /// Distinct files importing this file (via the resolved
    /// `target_path` column). Counts only resolvable imports —
    /// language coverage follows the import resolver's capabilities.
    pub fan_in: u32,
    /// Distinct raw `target` strings this file imports. Captures both
    /// resolved (in-repo) and unresolved (npm / pypi / std) imports,
    /// so it reflects the full structural dependency surface.
    pub fan_out: u32,
    /// `(cognitive / 100.0) × (fan_in + fan_out)`. Bigger = more
    /// god-class-shaped. Use this for ranking; the components are
    /// what to report when explaining "why this file flagged."
    pub god_score: f64,
}

/// Lower bound on cognitive complexity for a file to surface as a
/// god-class candidate. Matches the Sonar threshold for "complex
/// function" but applied at the file level (max cognitive across
/// entities in the file).
const DEFAULT_MIN_COGNITIVE: f64 = 30.0;

/// Lower bound on `fan_in + fan_out`. A file isolated from the
/// dependency graph isn't a god class even if it's gnarly internally.
const DEFAULT_MIN_TOTAL_FAN: u32 = 10;

const SQL: &str = "
    WITH file_complexity AS (
        SELECT path, MAX(cognitive)::DOUBLE AS cognitive
        FROM complexity_metrics
        WHERE cognitive IS NOT NULL
        GROUP BY path
    ),
    fan_in AS (
        -- One row per `target_path`; counts distinct importers.
        -- Only resolved imports contribute.
        SELECT target_path AS path, COUNT(DISTINCT src_path)::UINTEGER AS fan_in
        FROM imports
        WHERE target_path IS NOT NULL
        GROUP BY target_path
    ),
    fan_out AS (
        -- One row per `src_path`; counts distinct raw targets. Includes
        -- both resolved + unresolved targets, so external (npm / pypi
        -- / std) imports count toward fan-out — reflects the full
        -- structural-dependency surface.
        SELECT src_path AS path, COUNT(DISTINCT target)::UINTEGER AS fan_out
        FROM imports
        GROUP BY src_path
    )
    SELECT
        fc.path,
        fc.cognitive,
        COALESCE(fi.fan_in, 0)::UINTEGER AS fan_in,
        COALESCE(fo.fan_out, 0)::UINTEGER AS fan_out,
        (fc.cognitive / 100.0) * (COALESCE(fi.fan_in, 0) + COALESCE(fo.fan_out, 0))::DOUBLE AS god_score
    FROM file_complexity fc
    LEFT JOIN fan_in fi ON fi.path = fc.path
    LEFT JOIN fan_out fo ON fo.path = fc.path
    WHERE fc.cognitive >= ?
      AND (COALESCE(fi.fan_in, 0) + COALESCE(fo.fan_out, 0)) >= ?
    ORDER BY god_score DESC, fc.path ASC
    LIMIT ?
";

/// Run the `god-classes` analysis. Returns rows ranked by composite
/// god-score (highest first).
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` prepare /
/// query / collect errors.
pub fn run_god_classes(db: &FactsDb, opts: &Options) -> Result<Vec<GodClassRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    super::query::explain_if_requested(
        db,
        SQL,
        params![DEFAULT_MIN_COGNITIVE, DEFAULT_MIN_TOTAL_FAN, row_limit],
        "god-classes",
        opts,
    )?;
    super::query::query_map_collect(
        db,
        SQL,
        params![DEFAULT_MIN_COGNITIVE, DEFAULT_MIN_TOTAL_FAN, row_limit],
        "god-classes",
        |r| {
            Ok(GodClassRow {
                path: r.get::<_, String>(0)?,
                cognitive: r.get::<_, f64>(1)?,
                fan_in: r.get::<_, u32>(2)?,
                fan_out: r.get::<_, u32>(3)?,
                god_score: r.get::<_, f64>(4)?,
            })
        },
    )
}
