//! `bus-factor` analysis.
//!
//! Computes per-module (per directory or per group-file group) bus
//! factor: the minimum number of authors whose departure would
//! leave the module unmaintained.
//!
//! ## Modes
//!
//! `--knowledge-model commits` (default): Filatov 2010 — the smallest
//! set of contributors whose combined commit count covers ≥ 80% of the
//! module's total commits.
//!
//! `--knowledge-model doe`: Cury & Avelino SBES'24 truck-factor procedure
//! — repeatedly remove the author who is DOE-expert on the most remaining
//! files, stopping when >50% of files have zero remaining experts. The
//! removed count is the bus factor. Requires `doe_scores` to be
//! materialized (calls `materialize_knowledge_shares` first).
//!
//! ## Where the module boundary comes from
//!
//! Default: the top-level directory of each file path (e.g.
//! `src/foo/bar.rs` → module `src`). When `--group-file` is set,
//! the ingest's `apply_grouping` pass has already rewritten the
//! `changes.path` column to group names — the analysis just rolls
//! up per `path` which is now group-shaped. This is the intended
//! interaction: `--group-file` defines architectural modules; the
//! bus-factor analysis answers "what's the risk per architectural
//! module?".
//!
//! ## `CodeScene` parity-and-better
//!
//! `CodeScene`'s "Key Personnel" widget computes file-level bus
//! factor. This analysis lifts it to module-level, which is what
//! tech-leads actually care about — per-file is too granular to act
//! on. Bus factor = 1 module = a clear "who else needs to learn
//! this?" answer.

use std::collections::{HashMap, HashSet};

use duckdb::params;

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BusFactorRow {
    pub module: String,
    /// Total commits touching files in this module across the
    /// analysis window.
    pub total_commits: u32,
    /// Bus factor: the minimum number of authors whose departure would
    /// leave the module unmaintained (semantics depend on `model`).
    pub bus_factor: u32,
    /// Top contributor's name (highest commit count in the module for
    /// `commits` mode; author expert on the most files in `doe` mode).
    pub top_contributor: String,
    /// Top contributor's share of the module's total commits, in
    /// `[0, 1]`. In `doe` mode this is the share of expert files.
    /// 1.0 means a single author owns 100% of the module.
    pub top_contributor_share: f64,
    /// Knowledge model used to compute this row: `"commits"` or `"doe"`.
    pub model: String,
}

// SQL: build per-(module, author) commit counts, sort within each
// module by count DESC, compute the cumulative share, and surface the
// threshold-crossing position as the bus factor.
//
// Module boundary: the top-level directory of each path. Repo-root
// files (no `/`) fall into a `<root>` bucket so they are counted rather
// than silently dropped — every commit contributes to some module's
// bus-factor risk.
const SQL_COMMITS: &str = "
        WITH per_module_author AS (
            SELECT
                CASE
                    WHEN c.path LIKE '%/%' THEN regexp_extract(c.path, '^[^/]+', 0)
                    ELSE '<root>'
                END AS module,
                co.canonical_author AS author,
                COUNT(DISTINCT c.rev) AS commits
            FROM changes c
            INNER JOIN commits co ON co.rev = c.rev
            WHERE co.is_merge = FALSE
            GROUP BY module, co.canonical_author
        ),
        per_module AS (
            SELECT
                module,
                SUM(commits) AS total_commits
            FROM per_module_author
            GROUP BY module
        ),
        ranked AS (
            SELECT
                pma.module,
                pma.author,
                pma.commits,
                pm.total_commits,
                ROW_NUMBER() OVER (PARTITION BY pma.module ORDER BY pma.commits DESC, pma.author ASC) AS rank,
                SUM(pma.commits) OVER (
                    PARTITION BY pma.module
                    ORDER BY pma.commits DESC, pma.author ASC
                    ROWS UNBOUNDED PRECEDING
                ) AS cum_commits
            FROM per_module_author pma
            INNER JOIN per_module pm ON pm.module = pma.module
        ),
        bus_factor_calc AS (
            SELECT
                module,
                MIN(rank) AS bus_factor
            FROM ranked
            WHERE cum_commits >= total_commits * 0.8
            GROUP BY module
        ),
        top AS (
            SELECT module, author AS top_author, commits AS top_commits
            FROM ranked
            WHERE rank = 1
        )
        SELECT
            pm.module,
            CAST(pm.total_commits AS UINTEGER) AS total_commits,
            CAST(COALESCE(bfc.bus_factor, 1) AS UINTEGER) AS bus_factor,
            t.top_author,
            (t.top_commits::DOUBLE / NULLIF(pm.total_commits, 0)::DOUBLE) AS top_share
        FROM per_module pm
        LEFT JOIN bus_factor_calc bfc ON bfc.module = pm.module
        INNER JOIN top t ON t.module = pm.module
        WHERE pm.module IS NOT NULL AND pm.module != ''
        ORDER BY bus_factor ASC, pm.total_commits DESC, pm.module ASC
        LIMIT ?
    ";

// SQL for DOE mode: fetch all expert (path, author) pairs from doe_scores,
// mapping each path to its top-level module bucket. Also fetch module-level
// commit totals (for `total_commits`) and the most-expert author per module
// (for `top_contributor` and `top_contributor_share` in terms of expert files).
const SQL_DOE_EXPERTS: &str = "
    SELECT
        CASE
            WHEN path LIKE '%/%' THEN regexp_extract(path, '^[^/]+', 0)
            ELSE '<root>'
        END AS module,
        author,
        path
    FROM doe_scores
    WHERE is_expert = TRUE
    ORDER BY module, author, path
";

const SQL_DOE_MODULE_COMMITS: &str = "
    WITH per_module AS (
        SELECT
            CASE
                WHEN c.path LIKE '%/%' THEN regexp_extract(c.path, '^[^/]+', 0)
                ELSE '<root>'
            END AS module,
            COUNT(DISTINCT c.rev) AS total_commits
        FROM changes c
        INNER JOIN commits co ON co.rev = c.rev
        WHERE co.is_merge = FALSE
        GROUP BY module
    )
    SELECT module, CAST(total_commits AS UINTEGER)
    FROM per_module
    WHERE module IS NOT NULL AND module != ''
    ORDER BY module
";

/// Run the `bus-factor` analysis.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on `DuckDB` errors.
#[tracing::instrument(name = "bus-factor", skip_all, fields(min_revs = opts.min_revs, knowledge_model = %opts.knowledge_model))]
pub fn run_bus_factor(db: &FactsDb, opts: &Options) -> Result<Vec<BusFactorRow>> {
    // Path-aggregating analysis: route `FROM changes` through the
    // rename-aware lineage view when `--use-canonical-lineage` is set so
    // a renamed file's history attributes to one module, not two.
    crate::analyses::lineage::materialize_if_needed(db, opts)?;

    if opts.knowledge_model == "doe" {
        run_bus_factor_doe(db, opts)
    } else {
        run_bus_factor_commits(db, opts)
    }
}

fn run_bus_factor_commits(db: &FactsDb, opts: &Options) -> Result<Vec<BusFactorRow>> {
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    let sql = crate::analyses::lineage::rewrite(SQL_COMMITS, opts);

    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare bus-factor: {e}")))?;
    let rows = stmt
        .query_map(params![row_limit], |r| {
            Ok(BusFactorRow {
                module: r.get(0)?,
                total_commits: r.get(1)?,
                bus_factor: r.get(2)?,
                top_contributor: r.get(3)?,
                top_contributor_share: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                model: "commits".to_string(),
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query bus-factor: {e}")))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect bus-factor: {e}")))
}

/// DOE-based truck-factor computation (Cury & Avelino SBES'24).
///
/// Algorithm (per module):
/// 1. Collect all (path, author) pairs where `is_expert = TRUE`.
/// 2. Greedy loop: repeatedly remove the author expert on the most
///    remaining files (ties broken alphabetically for determinism).
/// 3. Stop when >50% of the module's files have zero remaining experts.
/// 4. `bus_factor = count of authors removed`.
///
/// **Edge case — `removed_count.max(1)`**: the stop condition is checked
/// *before* the first removal. If all files are already uncovered before
/// any author is removed (e.g. a module where no author qualifies as
/// DOE-expert at all), the greedy loop never fires and `removed_count = 0`.
/// The result is clamped to `1` so that `bus_factor` is always ≥ 1,
/// matching the semantics of "at least one key person exists."
///
/// `top_contributor` is the first author removed (expert on the most
/// files before any removal). `top_contributor_share` is their share
/// of expert-file assignments across the module.
#[allow(clippy::too_many_lines)]
fn run_bus_factor_doe(db: &FactsDb, opts: &Options) -> Result<Vec<BusFactorRow>> {
    // Materialize doe_scores (idempotent — Cell<bool> guard inside).
    crate::analyses::knowledge::shares::materialize_knowledge_shares(db, opts)?;

    // Fetch module-level commit totals for the `total_commits` field.
    let commit_sql = crate::analyses::lineage::rewrite(SQL_DOE_MODULE_COMMITS, opts);
    let mut commit_stmt = db
        .conn()
        .prepare(&commit_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare doe module-commits: {e}")))?;
    let mut module_commits: HashMap<String, u32> = HashMap::new();
    let commit_rows = commit_stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))
        .map_err(|e| CodeLoreError::Analysis(format!("query doe module-commits: {e}")))?;
    for pair in commit_rows {
        let (module, total) =
            pair.map_err(|e| CodeLoreError::Analysis(format!("collect doe module-commits: {e}")))?;
        module_commits.insert(module, total);
    }

    // Fetch all expert (module, author, path) rows.
    let mut expert_stmt = db
        .conn()
        .prepare(SQL_DOE_EXPERTS)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare doe experts: {e}")))?;

    // module → { author → set of files they're expert on }
    let mut module_author_files: HashMap<String, HashMap<String, HashSet<String>>> = HashMap::new();
    // module → total number of distinct files (with at least one expert)
    let mut module_file_count: HashMap<String, HashSet<String>> = HashMap::new();

    let expert_rows = expert_stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query doe experts: {e}")))?;

    for row in expert_rows {
        let (module, author, path) =
            row.map_err(|e| CodeLoreError::Analysis(format!("collect doe experts: {e}")))?;
        module_file_count
            .entry(module.clone())
            .or_default()
            .insert(path.clone());
        module_author_files
            .entry(module)
            .or_default()
            .entry(author)
            .or_default()
            .insert(path);
    }

    let row_limit: usize = opts.rows_limit.map_or(usize::MAX, |l| l as usize);

    let mut results: Vec<BusFactorRow> = Vec::new();

    for (module, mut author_files) in module_author_files {
        let total_files = module_file_count.get(&module).map_or(0, HashSet::len);

        if total_files == 0 {
            continue;
        }

        // Track which files still have at least one expert remaining.
        // Initially all files that have any expert are covered.
        let mut covered: HashSet<String> =
            module_file_count.get(&module).cloned().unwrap_or_default();

        // Pre-compute the initial top author (most expert files) for the
        // `top_contributor` field. We record this before the greedy loop
        // mutates `author_files`, so the fallback branch is never needed.
        let (initial_top_author, initial_top_count) = author_files
            .iter()
            .map(|(a, files)| (a.clone(), files.len()))
            .max_by(|(_, ca), (_, cb)| ca.cmp(cb))
            .unwrap_or_else(|| (String::new(), 0));

        let mut first_author: Option<String> = None;
        let mut first_author_file_count: usize = 0;
        let mut removed_count: u32 = 0;

        // Greedy loop: remove the author expert on the most remaining files.
        loop {
            // Stop condition: >50% of files have no expert.
            let uncovered = total_files.saturating_sub(covered.len());
            if uncovered * 2 > total_files {
                break;
            }
            if author_files.is_empty() {
                break;
            }

            // Pick the author with the most expert-file assignments among
            // still-covered files; break ties alphabetically.
            let best = author_files
                .iter()
                .map(|(a, files)| {
                    let coverage: usize = files.iter().filter(|f| covered.contains(*f)).count();
                    (coverage, a.clone())
                })
                .max_by(|(ca, aa), (cb, ab)| ca.cmp(cb).then(aa.cmp(ab).reverse()))
                .map(|(count, a)| (a, count));

            let Some((author, count)) = best else {
                break;
            };

            if count == 0 {
                // No author covers any still-covered files; we're done.
                break;
            }

            // Record first removal for top_contributor.
            if first_author.is_none() {
                first_author = Some(author.clone());
                first_author_file_count = count;
            }

            // Remove this author and update covered set.
            let removed_files = author_files.remove(&author).unwrap_or_default();
            removed_count += 1;

            // For each file this author was expert on, check if any
            // remaining author is still expert on it.
            for file in &removed_files {
                let still_covered = author_files.values().any(|fs| fs.contains(file));
                if !still_covered {
                    covered.remove(file);
                }
            }
        }

        // Use the first greedy-removal author as top_contributor; fall back to
        // the pre-computed initial top author if the loop never ran.
        let (top_contributor, top_file_count) = first_author.map_or_else(
            || (initial_top_author, initial_top_count),
            |a| (a, first_author_file_count),
        );
        #[allow(clippy::cast_precision_loss)]
        let top_contributor_share = if total_files > 0 {
            top_file_count as f64 / total_files as f64
        } else {
            0.0
        };
        let total_commits = module_commits.get(&module).copied().unwrap_or(0);

        results.push(BusFactorRow {
            module,
            total_commits,
            bus_factor: removed_count.max(1),
            top_contributor,
            top_contributor_share,
            model: "doe".to_string(),
        });
    }

    // Sort: bus_factor ASC, total_commits DESC, module ASC — matches commits mode.
    results.sort_by(|a, b| {
        a.bus_factor
            .cmp(&b.bus_factor)
            .then(b.total_commits.cmp(&a.total_commits))
            .then(a.module.cmp(&b.module))
    });
    results.truncate(row_limit);

    Ok(results)
}
