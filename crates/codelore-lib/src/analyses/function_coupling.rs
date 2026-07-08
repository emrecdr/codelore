//! `function-coupling` analysis.
//!
//! For a single target file, identifies pairs of functions that co-change
//! (appear together in the same revision's hunk-attributed change set) and
//! ranks them by Fisher exact significance. Only pairs with `co_changes ≥ 2`
//! are emitted.
//!
//! **Algorithm**: for each revision that touched the target file, the set of
//! HEAD-alive functions whose line spans overlapped any hunk is computed via
//! the same hunk-overlap logic used by `function-xray`. Co-change counts are
//! then accumulated over all revisions. For each pair `(a, b)` the Fisher
//! 2×2 contingency table is:
//!
//! ```text
//!           b touched   b not touched
//! a touched     co          a_only
//! a not         b_only      neither
//! ```
//!
//! where `n = total revisions touching the file` and
//! `neither = n − co − a_only − b_only`.
//!
//! **Rename limitation**: hunk attribution uses `WHERE h.path = ?` (current
//! HEAD-relative path). Pre-rename history is not attributed — see
//! `function_xray` module doc.
//!
//! **Output**: sorted by `p_value` ASC (`None` first — degenerate marginal
//! implies p → 0, i.e. perfectly coupled) then `confidence` DESC for
//! determinism. `confidence = co_changes / min(a_changes, b_changes)`.
//!
//! Research basis: Adams et al., ICSM 2006 "The Co-Change Rule"; Fisher
//! significance adapts the coupling analysis in Tornhill, "Your Code as a
//! Crime Scene" (2015) to function granularity.

use std::collections::HashSet;

use crate::analyses::function_xray::rev_to_function_sets;
use crate::facts::FactsDb;
use crate::repo::Repo;
use crate::stats::fisher_two_tail_pvalue;
use crate::{Options, Result};

/// One row per function pair with `co_changes ≥ 2`, sorted by `p_value` ASC.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionCouplingRow {
    /// First function name (deduped as `name@start-end`).
    pub a: String,
    /// Second function name (deduped as `name@start-end`).
    pub b: String,
    /// Number of revisions where both `a` and `b` were touched.
    pub co_changes: u32,
    /// Number of revisions where `a` was touched (including co-changes).
    pub a_changes: u32,
    /// Number of revisions where `b` was touched (including co-changes).
    pub b_changes: u32,
    /// `co_changes / min(a_changes, b_changes)`. 1.0 means perfect coupling.
    pub confidence: f64,
    /// Two-tailed Fisher exact p-value. `None` serialises as `null` in JSON
    /// and as an empty string in CSV.
    pub p_value: Option<f64>,
}

/// Run the `function-coupling` analysis.
///
/// `target` is a repo-relative path (e.g. `src/foo.rs`). Returns pairs of
/// HEAD-alive functions that co-changed in ≥ 2 revisions, sorted by
/// `p_value` ASC then `confidence` DESC. Pairs where Fisher returns `None`
/// (degenerate marginal — zero row or column sum, implying p → 0) sort
/// first as the strongest coupling signal.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on database errors or if the
/// target file is not a supported Tier-1 language.
#[tracing::instrument(name = "function-coupling", skip_all, fields(target = target))]
pub fn run_function_coupling<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    target: &str,
) -> Result<Vec<FunctionCouplingRow>> {
    // --- 1. Build per-rev function-change sets ----------------------------
    let rev_sets = rev_to_function_sets(db, repo, target)?;
    if rev_sets.is_empty() {
        return Ok(Vec::new());
    }

    // n = total revisions that touched the file (regardless of which fns).
    let n = u32::try_from(rev_sets.len()).unwrap_or(u32::MAX);

    // Collect all HEAD-alive function names that appear in any rev set.
    let all_fns: HashSet<&str> = rev_sets
        .values()
        .flat_map(|s| s.iter().map(String::as_str))
        .collect();
    let mut all_fns: Vec<&str> = all_fns.into_iter().collect();
    all_fns.sort_unstable();

    // --- 2. Accumulate per-function change counts and co-change counts ----
    // fn_changes[i] = number of revisions that touched all_fns[i]
    let fn_count = all_fns.len();
    let mut fn_changes: Vec<u32> = vec![0u32; fn_count];
    // co_matrix[i][j] (i < j) = co-change count for pair (i, j)
    // Stored as a flat upper-triangle: index(i,j) = i*fn_count + j
    let mut co_matrix: Vec<u32> = vec![0u32; fn_count * fn_count];

    for set in rev_sets.values() {
        // Find indices of functions touched in this rev.
        let touched: Vec<usize> = all_fns
            .iter()
            .enumerate()
            .filter_map(|(i, &name)| if set.contains(name) { Some(i) } else { None })
            .collect();
        for &i in &touched {
            fn_changes[i] += 1;
        }
        // Accumulate co-changes for every pair touched in this rev.
        for (pos_a, &i) in touched.iter().enumerate() {
            for &j in &touched[pos_a + 1..] {
                co_matrix[i * fn_count + j] += 1;
            }
        }
    }

    // --- 3. Build output rows for pairs with co_changes ≥ 2 ---------------
    let mut rows: Vec<FunctionCouplingRow> = Vec::new();
    for i in 0..fn_count {
        for j in i + 1..fn_count {
            let co = co_matrix[i * fn_count + j];
            if co < 2 {
                continue;
            }
            let a_ch = fn_changes[i];
            let b_ch = fn_changes[j];
            // Fisher 2×2 contingency:
            //   a=co, b=a_only, c=b_only, d=neither
            // where a_only = a_changes - co, b_only = b_changes - co,
            //       neither = n - co - a_only - b_only
            let a_only = a_ch.saturating_sub(co);
            let b_only = b_ch.saturating_sub(co);
            let both_or_neither = n
                .saturating_sub(co)
                .saturating_sub(a_only)
                .saturating_sub(b_only);
            let p_value = fisher_two_tail_pvalue(co, a_only, b_only, both_or_neither);
            let confidence = f64::from(co) / f64::from(a_ch.min(b_ch)).max(1.0);
            rows.push(FunctionCouplingRow {
                a: all_fns[i].to_string(),
                b: all_fns[j].to_string(),
                co_changes: co,
                a_changes: a_ch,
                b_changes: b_ch,
                confidence,
                p_value,
            });
        }
    }

    // Sort: p_value ASC, None first (degenerate marginal = perfectly coupled,
    // limit p → 0), then confidence DESC for ties.
    rows.sort_unstable_by(|x, y| match (x.p_value, y.p_value) {
        (Some(px), Some(py)) => px
            .partial_cmp(&py)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                y.confidence
                    .partial_cmp(&x.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }

    Ok(rows)
}
