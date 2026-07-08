//! `function-xray` analysis.
//!
//! Per-function change frequency for a single target file: for each function
//! (or method) that exists at HEAD in the target file, counts the number of
//! **distinct revisions** where at least one hunk overlapped the function's
//! line span.
//!
//! **Hunk-overlap attribution**: a function at `[start_line, end_line]` is
//! "changed" in revision R if any hunk for that path at R satisfies
//! `new_start <= end_line && new_start + new_lines > start_line`.
//! Pure deletions (`new_lines = 0`) are attributed to the function whose span
//! contains `new_start`; they are skipped if `new_start` is outside every
//! function span (e.g., a file-level deletion in the preamble).
//! A single revision that produces multiple overlapping hunks is counted
//! **once** per function — [`run_function_xray`] deduplicates on `(function,
//! rev)` after the overlap test.
//!
//! **HEAD-alive filter**: only functions whose span is detected in the HEAD
//! blob by `compute_for_file` are included. The HEAD blob is fetched via
//! [`crate::repo::Repo::read_blob_at`]`("HEAD", target)` and re-parsed with
//! the same tree-sitter extractor used during ingest. Functions that existed
//! at some point but are absent from the HEAD tree are excluded.
//!
//! **Rename limitation**: hunk attribution uses `WHERE h.path = ?` (the
//! current HEAD-relative path). Pre-rename history is not attributed — the
//! function receives a new identity after a rename and prior hunk records live
//! under the old path. `change_freq` therefore reflects activity since the
//! most recent rename, not the full file lifetime.
//!
//! **Span source**: reuses `compute_for_file` (the same tree-sitter extractor
//! used by the ingest pass) applied to the HEAD blob of the target file,
//! rather than materializing yet another temp table — the target is a single
//! file, so the overhead is negligible compared to a full ingest scan.
//!
//! Research basis: HistoryFinder-style function-level churn (Gall et al.,
//! ICSM 2003 "Predicting faults from cached history"; extended to hunk
//! attribution by Skoulis et al., MSR 2014).

use std::collections::{HashMap, HashSet};

use crate::complexity::{Tier1Language, compute_for_file};
use crate::facts::FactsDb;
use crate::facts::ingest::consumer::dedup_entities;
use crate::repo::Repo;
use crate::{CodeLoreError, Options, Result};

/// One row per HEAD-alive function/method in the target file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionXrayRow {
    /// Function or method name, as reported by tree-sitter.
    pub function: String,
    /// Number of revisions where at least one hunk overlapped this function's span.
    pub change_freq: u32,
    /// SLOC at HEAD (from `complexity_metrics`). Zero is impossible (HEAD-alive
    /// filter guarantees the function exists), but `0` is the safe sentinel.
    pub loc: u32,
    /// Cyclomatic complexity at HEAD. `None` if not recorded.
    pub cyclomatic: Option<i64>,
    /// Cognitive complexity at HEAD. `None` if not recorded.
    pub cognitive: Option<i64>,
    /// Author-date of the most recent revision that overlapped this function.
    /// Empty string if no hunk ever overlapped (`change_freq` = 0).
    pub last_changed: String,
}

/// Run the `function-xray` analysis.
///
/// `target` is a repo-relative path (e.g. `src/foo.rs`). Returns rows sorted
/// by `change_freq` DESC, then `function` ASC for determinism. Only HEAD-alive
/// functions are included.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Analysis`] on `DuckDB` errors or if the
/// target file is not a supported Tier-1 language.
///
/// # Panics
///
/// Panics if an internal invariant is violated: every function name inserted
/// into `freq` (from `head_spans`) must be present when looked up during
/// overlap attribution. This cannot happen under normal operation.
#[tracing::instrument(name = "function-xray", skip_all, fields(target = target))]
pub fn run_function_xray<R: Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    target: &str,
) -> Result<Vec<FunctionXrayRow>> {
    // --- 1. Extract HEAD spans via tree-sitter ----------------------------
    let head_spans = extract_head_spans(repo, target)?;
    if head_spans.is_empty() {
        return Ok(Vec::new());
    }

    // --- 2. Fetch hunks for the target path from the DB -------------------
    // Each hunk row: (rev TEXT, date TEXT, new_start u32, new_lines u32)
    let hunk_rows = fetch_hunks_for_path(db, target)?;

    // --- 3. Count hunk-overlapping revisions per function -----------------
    // Build a map from function name → (change_freq, last_changed_date).
    // A function can appear multiple times if tree-sitter finds overloads or
    // re-uses the same name (unlikely in Rust, common in C++). dedup_entities
    // drops exact (name, start_line, end_line) duplicates in first-occurrence
    // order and suffixes each survivor with its span — matching the identity
    // key used by the HEAD complexity table.
    let mut freq: HashMap<String, (u32, String)> = head_spans
        .iter()
        .map(|(name, _, _)| (name.clone(), (0u32, String::new())))
        .collect();

    // Track (function_name, rev) pairs already counted so that a single
    // revision with multiple overlapping hunks increments change_freq once,
    // not once per hunk.
    let mut counted: HashSet<(String, String)> = HashSet::new();

    for (rev, date, new_start, new_lines) in &hunk_rows {
        let (new_start, new_lines) = (*new_start, *new_lines);
        for (name, start_line, end_line) in &head_spans {
            if hunk_overlaps(*start_line, *end_line, new_start, new_lines) {
                let key = (name.clone(), rev.clone());
                if counted.insert(key) {
                    let entry = freq.get_mut(name).expect("name always present");
                    entry.0 += 1;
                    // Keep the lexicographically latest date (ISO 8601 sorts correctly).
                    if date.as_str() > entry.1.as_str() {
                        entry.1.clone_from(date);
                    }
                }
            }
        }
    }

    // --- 4. Enrich with HEAD complexity metrics ---------------------------
    let metrics = fetch_head_metrics(db, target)?;

    // --- 5. Build output rows, sorted change_freq DESC, function ASC ------
    let mut rows: Vec<FunctionXrayRow> = head_spans
        .iter()
        .map(|(name, _, _)| {
            let (change_freq, last_changed) = freq.get(name).cloned().unwrap_or((0, String::new()));
            let (loc, cyclomatic, cognitive) =
                metrics.get(name).copied().unwrap_or((0, None, None));
            FunctionXrayRow {
                function: name.clone(),
                change_freq,
                loc,
                cyclomatic,
                cognitive,
                last_changed,
            }
        })
        .collect();

    rows.sort_unstable_by(|a, b| {
        b.change_freq
            .cmp(&a.change_freq)
            .then_with(|| a.function.cmp(&b.function))
    });

    if let Some(limit) = opts.rows_limit {
        rows.truncate(limit as usize);
    }

    Ok(rows)
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Returns `true` if the hunk `[new_start, new_start + new_lines)` overlaps
/// the function span `[start_line, end_line]`.
///
/// Line numbers are 1-based. The hunk range is half-open `[new_start, new_start + new_lines)`.
///
/// Pure-deletion edge case (`new_lines = 0`): the database records the anchor line
/// where the block was deleted. Attribute to the function if `new_start` falls
/// inside the span, i.e. `start_line <= new_start <= end_line`.
///
/// Both `start_line` and `new_start` are 1-based; `end_line` is the last line
/// of the function body (inclusive).
pub(crate) fn hunk_overlaps(
    start_line: u32,
    end_line: u32,
    new_start: u32,
    new_lines: u32,
) -> bool {
    if new_lines == 0 {
        // Pure deletion — attribute to function if anchor is inside the span.
        new_start >= start_line && new_start <= end_line
    } else {
        // Standard half-open range overlap:
        //   hunk covers [new_start, new_start + new_lines)
        //   function covers [start_line, end_line]
        new_start <= end_line && new_start + new_lines > start_line
    }
}

/// Extract HEAD spans for the target file using `compute_for_file`.
///
/// Returns `Vec<(name, start_line, end_line)>` for all "function" and "method"
/// entities in the file. Non-function entities (class, file) are excluded
/// because xray attribution is per-callable unit.
pub(super) fn extract_head_spans<R: Repo>(
    repo: &R,
    target: &str,
) -> Result<Vec<(String, u32, u32)>> {
    let Some(lang) = Tier1Language::from_path(target) else {
        return Ok(Vec::new());
    };

    let source = match repo.read_blob_at("HEAD", target) {
        Ok(Some(b)) => b,
        Ok(None) => {
            tracing::debug!("function-xray: {target} not tracked at HEAD");
            return Ok(Vec::new());
        }
        Err(e) => {
            return Err(CodeLoreError::Analysis(format!(
                "function-xray: blob read failed for {target}: {e}"
            )));
        }
    };

    if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
        return Err(CodeLoreError::Analysis(format!(
            "function-xray: {target} exceeds {}-byte AST cap",
            crate::constants::DEFAULT_MAX_AST_FILE_BYTES
        )));
    }

    let path = std::path::Path::new(target);
    let entities = compute_for_file(path, source, lang)?;
    // Dedup entities to match the HEAD complexity table (same name → keeps
    // the span with more lines, which dedup_entities guarantees by taking the
    // last entry with a given name after sorting by end_line DESC).
    let deduped = dedup_entities(entities);

    let spans: Vec<(String, u32, u32)> = deduped
        .into_iter()
        .filter(|e| e.kind == "function" || e.kind == "method")
        .map(|e| (e.name, e.start_line, e.end_line))
        .collect();

    Ok(spans)
}

/// Fetch all hunks for `target` from the `hunks` table.
///
/// Returns `Vec<(rev, date, new_start, new_lines)>` — one entry per hunk row,
/// joined with the `commits` table to get the author date. The `rev` is used
/// by the caller to deduplicate multi-hunk commits per function.
pub(super) fn fetch_hunks_for_path(
    db: &FactsDb,
    target: &str,
) -> Result<Vec<(String, String, u32, u32)>> {
    use crate::analyses::query::query_map_collect;

    query_map_collect(
        db,
        "SELECT h.rev, CAST(c.date AS TEXT), h.new_start, h.new_lines
         FROM hunks h
         JOIN commits c ON c.rev = h.rev
         WHERE h.path = ?
         ORDER BY c.date",
        duckdb::params![target],
        "function-xray:hunks",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, u32>(3)?,
            ))
        },
    )
}

/// Complexity triple: `(sloc, cyclomatic, cognitive)` for a single function.
type FnMetrics = (u32, Option<i64>, Option<i64>);

/// Fetch HEAD complexity metrics for functions in the target file.
///
/// Returns a map from `name → (sloc, cyclomatic, cognitive)`. Uses
/// `complexity_metrics` which is populated during ingest and holds HEAD data.
///
/// When `complexity_metrics` has multiple rows for the same function name
/// (possible with non-Rust languages that allow overloads), we keep the row
/// with the highest `sloc` as a proxy for the "most significant" variant.
/// This matches how the ingest pass resolves the same ambiguity.
fn fetch_head_metrics(db: &FactsDb, target: &str) -> Result<HashMap<String, FnMetrics>> {
    use crate::analyses::query::query_map_collect;

    let rows: Vec<(String, u32, Option<i64>, Option<i64>)> = query_map_collect(
        db,
        "SELECT name, COALESCE(sloc, 0), cyclomatic, cognitive
         FROM complexity_metrics
         WHERE path = ?
         GROUP BY name, sloc, cyclomatic, cognitive",
        duckdb::params![target],
        "function-xray:metrics",
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        },
    )?;

    let mut map: HashMap<String, FnMetrics> = HashMap::new();
    for (name, sloc, cy, cog) in rows {
        // Keep the entry with the higher sloc when there are duplicates
        // (can happen with overloaded names in non-Rust languages).
        map.entry(name)
            .and_modify(|e| {
                if sloc > e.0 {
                    *e = (sloc, cy, cog);
                }
            })
            .or_insert((sloc, cy, cog));
    }
    Ok(map)
}

/// Build a map from rev → set of function names touched in that revision
/// for `target`. Uses the same HEAD-span extraction and hunk-overlap logic
/// as `run_function_xray`. Returns only revisions that touched ≥1
/// HEAD-alive function.
///
/// Called by `function_coupling` to share the blob-parse + hunk-overlap
/// logic without re-implementing it.
///
/// The overlap loop here is intentionally separate from `run_function_xray`'s
/// loop even though both walk the same hunk rows: `run_function_xray` must
/// track the per-function last-changed date alongside the frequency count,
/// which requires a different accumulator shape. Extracting a common inner
/// loop would either require threading the date through this function's API
/// or a two-pass approach — neither improves clarity. The expensive steps
/// (`extract_head_spans` blob fetch + tree-sitter parse, `fetch_hunks_for_path`
/// DB query) are shared; the cheap overlap scan runs twice.
pub(super) fn rev_to_function_sets<R: Repo>(
    db: &FactsDb,
    repo: &R,
    target: &str,
) -> Result<HashMap<String, HashSet<String>>> {
    let head_spans = extract_head_spans(repo, target)?;
    if head_spans.is_empty() {
        return Ok(HashMap::new());
    }
    let hunk_rows = fetch_hunks_for_path(db, target)?;
    let mut rev_sets: HashMap<String, HashSet<String>> = HashMap::new();
    for (rev, _date, new_start, new_lines) in &hunk_rows {
        let (new_start, new_lines) = (*new_start, *new_lines);
        for (name, start_line, end_line) in &head_spans {
            if hunk_overlaps(*start_line, *end_line, new_start, new_lines) {
                rev_sets
                    .entry(rev.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
    }
    Ok(rev_sets)
}

// ── unit tests for the hunk-overlap predicate ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::hunk_overlaps;

    struct Case {
        label: &'static str,
        start: u32,
        end: u32,
        new_start: u32,
        new_lines: u32,
        expected: bool,
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn overlap_predicate() {
        let cases = vec![
            Case {
                label: "hunk fully inside function",
                start: 10,
                end: 20,
                new_start: 12,
                new_lines: 3,
                expected: true,
            },
            Case {
                label: "hunk starts before and overlaps",
                start: 10,
                end: 20,
                new_start: 8,
                new_lines: 5,
                expected: true,
            },
            Case {
                label: "hunk starts after and overlaps",
                start: 10,
                end: 20,
                new_start: 18,
                new_lines: 5,
                expected: true,
            },
            Case {
                label: "hunk completely before function",
                start: 10,
                end: 20,
                new_start: 3,
                new_lines: 5,
                expected: false,
            },
            Case {
                label: "hunk completely after function",
                start: 10,
                end: 20,
                new_start: 22,
                new_lines: 5,
                expected: false,
            },
            Case {
                label: "hunk ends exactly at function start",
                start: 10,
                end: 20,
                new_start: 8,
                new_lines: 2, // covers [8, 10) — does NOT touch line 10
                expected: false,
            },
            Case {
                label: "hunk starts exactly at function end (inclusive overlap)",
                start: 10,
                end: 20,
                new_start: 20,
                new_lines: 1,
                expected: true,
            },
            // Pure-deletion cases (new_lines = 0).
            Case {
                label: "pure deletion inside function",
                start: 10,
                end: 20,
                new_start: 15,
                new_lines: 0,
                expected: true,
            },
            Case {
                label: "pure deletion at function start",
                start: 10,
                end: 20,
                new_start: 10,
                new_lines: 0,
                expected: true,
            },
            Case {
                label: "pure deletion at function end",
                start: 10,
                end: 20,
                new_start: 20,
                new_lines: 0,
                expected: true,
            },
            Case {
                label: "pure deletion before function",
                start: 10,
                end: 20,
                new_start: 9,
                new_lines: 0,
                expected: false,
            },
            Case {
                label: "pure deletion after function",
                start: 10,
                end: 20,
                new_start: 21,
                new_lines: 0,
                expected: false,
            },
        ];

        for c in &cases {
            assert_eq!(
                hunk_overlaps(c.start, c.end, c.new_start, c.new_lines),
                c.expected,
                "FAILED: {}",
                c.label
            );
        }
    }
}
