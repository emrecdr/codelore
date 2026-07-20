//! Delta-scoped change-set engine: what a set of working-tree edits does to
//! the repository's health *before* they are committed.
//!
//! The engine never re-implements a health formula. It re-parses only the
//! changed files (via the pure buffer-level complexity extractor), substitutes
//! their rows into a temporary `complexity_metrics_projected` table, and runs
//! the *existing* [`run_code_health_scoped`] twice — once against today's HEAD
//! tables (the baseline) and once against the substituted table (the
//! projection). Everything the substitution does not touch stays frozen at HEAD
//! facts automatically:
//!
//! - **History terms** (churn `n_cn`, author fragmentation `n_au`, and
//!   shotgun-surgery via coupling centrality) read the untouched `changes` /
//!   `commits` tables, so they are identical in both runs.
//! - **Cross-file structure** — god-class fan-in/out reads the untouched
//!   `imports` table (`imports_source` stays `"imports"`); the DRY biomarker
//!   walks the working tree on *both* runs, so its delta cancels to zero.
//! - **Calibrated weights, clamps, and the no-DRY scale divisor** are inherited
//!   byte-for-byte because both runs go through the one real scoring engine.
//!
//! This is why a byte-identical file re-parses to byte-identical rows, which
//! rank identically, which yields a delta of exactly `0.0`.
//!
//! ## Median population
//!
//! Both scoped runs use `min_revs = 1` and no row cap (the `change_context`
//! precedent), so [`HealthProjection`]'s medians are taken over *every*
//! scoreable file, not `diff`'s min-revs-5 hotspot set. The delta-of-medians
//! semantics is preserved; only the population differs, and it is documented
//! here so the gate's `delta_code_health_min` reading is understood as
//! all-scoreable-files rather than the hotspot subset.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::analyses::code_health::{CodeHealthRow, HealthScanCtx, run_code_health_scoped};
use crate::complexity::{ComplexityEntity, Tier1Language, compute_for_file};
use crate::facts::FactsDb;
use crate::facts::ingest::consumer::{dedup_entities, f64_to_i32_clamped};
use crate::repo::{WorktreeChange, WorktreeChangeKind};
use crate::{CodeLoreError, Options, Result};

/// The temporary table the projection scores against: HEAD `complexity_metrics`
/// minus the change-set paths, plus the re-parsed working-tree rows.
const PROJECTED_COMPLEXITY_TABLE: &str = "complexity_metrics_projected";

/// The temporary table listing every change-set path (changed + deleted +
/// rename sources) whose HEAD complexity rows the projection replaces.
const CHANGED_PATHS_TABLE: &str = "changed_paths_v1";

/// NUL-byte binary sniff window, mirroring the repo layer's blob heuristic
/// (`BINARY_SNIFF_BYTES` in `repo::gix_repo`). HEAD ingest never sees binary
/// files because the blob-enumeration layer filters them; the engine reads the
/// working tree directly, so it re-applies the same sniff here.
const BINARY_SNIFF_BYTES: usize = 8000;

const REASON_NOT_TIER1: &str = "not a Tier-1 source file";
const REASON_BINARY: &str = "binary content";
const REASON_SIZE_LIMIT: &str = "file exceeds the AST size limit";
const REASON_DELETED: &str = "deleted at gate time";
const REASON_NEW_FILE: &str = "new file (no history baseline)";
const REASON_NO_HEAD_ROW: &str = "no code-health row at HEAD";
/// The projection produced no scoreable row for a file that had one at HEAD
/// (the working tree emptied its analyzable content). Unreachable for a file
/// that still parses to at least the file-unit entity, but kept honest.
const REASON_NO_PROJECTED_ROW: &str = "no code-health row after projection";

/// One changed file's HEAD-vs-projected code-health scores, or an honest reason
/// a score is absent.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDelta {
    /// Repo-relative, `/`-separated path (for a rename, the destination).
    pub path: String,
    /// `"added"` | `"modified"` | `"deleted"` | `"renamed"`.
    pub kind: String,
    /// HEAD score; `None` when [`reason`](Self::reason) is set.
    pub baseline_score: Option<f64>,
    /// Projected score; `None` when [`reason`](Self::reason) is set.
    pub projected_score: Option<f64>,
    /// `projected − baseline` when both are present.
    pub delta: Option<f64>,
    /// HEAD band (`"red"` | `"yellow"` | `"green"`), when scored.
    pub baseline_band: Option<String>,
    /// Projected band, when scored.
    pub projected_band: Option<String>,
    /// Why a score is absent (see the module's honest-absence set).
    pub reason: Option<String>,
}

/// The projected-health half of a change-set report.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthProjection {
    /// One row per change-set path, sorted `|delta|` descending (rows with no
    /// delta last), ties broken by path ascending.
    pub deltas: Vec<FileDelta>,
    /// Whole-repo median over the baseline run's scores; `None` when empty.
    pub baseline_median: Option<f64>,
    /// Whole-repo median over the projection run's scores (same population
    /// rule); `None` when empty.
    pub projected_median: Option<f64>,
}

/// Project the code-health effect of `changes` on the working tree vs HEAD.
///
/// Runs the existing scoring engine twice (HEAD baseline, then the
/// substituted-complexity projection) and joins per changed path. Reads the
/// fact store and writes only session-scoped temporary tables — the persistent
/// `complexity_metrics` (and every other fact table) is never touched.
///
/// Exposed `pub` (rather than the `pub(crate)` its role suggests) so the
/// integration test in `tests/change_set_test.rs` can drive it directly; the
/// production caller is the crate-internal report assembler.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on any SQL / temp-table error, and
/// propagates fact-store, repo, and parse errors from the feeds.
pub fn project_health<R: crate::Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    changes: &[WorktreeChange],
) -> Result<HealthProjection> {
    // Both scoped runs share this: `min_revs = 1` so every file with any
    // history is scoreable (maximising per-file delta coverage), and the row
    // cap cleared so a `--rows N` never truncates the scored set. Mirrors the
    // `change_context` / `fact_sheet` precedent.
    let opts_scan = {
        let mut o = opts.with_no_row_limit();
        o.min_revs = 1;
        o
    };

    // 1. Baseline: today's HEAD tables — byte-for-byte the standard scan.
    let baseline_rows = run_code_health_scoped(db, &opts_scan, &HealthScanCtx::head())?;

    // 2. Substitute the changed files' complexity, then re-run the SAME engine
    //    against the projected table. `include_clones: true` on both runs so
    //    the STRUCTURAL_SCALE_NO_DRY divisor matches (scale parity); the clones
    //    and coupling memos make the second walk / self-join free.
    let head_sha = repo.head_sha()?;
    let skip_reasons = build_projected_complexity_table(db, opts, changes, &head_sha)?;
    let projected_ctx = HealthScanCtx {
        complexity_source: PROJECTED_COMPLEXITY_TABLE.to_string(),
        imports_source: "imports".to_string(),
        history_cutoff: None,
        include_clones: true,
    };
    let projected_rows = run_code_health_scoped(db, &opts_scan, &projected_ctx)?;

    // 3. Join per changed path, then order deterministically.
    let mut deltas: Vec<FileDelta> = changes
        .iter()
        .map(|change| delta_for_change(change, &baseline_rows, &projected_rows, &skip_reasons))
        .collect();
    sort_deltas(&mut deltas);

    Ok(HealthProjection {
        deltas,
        baseline_median: median(baseline_rows.iter().map(|r| r.score)),
        projected_median: median(projected_rows.iter().map(|r| r.score)),
    })
}

/// Build the projected complexity table and return the per-path parse-gate
/// reasons for files that could not be re-parsed.
///
/// Creates `complexity_metrics_projected` as HEAD `complexity_metrics` minus
/// the change-set paths, then re-parses each non-deleted change from the
/// working tree and inserts its rows — replicating the HEAD ingest pipeline
/// exactly (2 MiB skip, 8 KiB NUL binary sniff, `Tier1Language` gate,
/// `compute_for_file`, `dedup_entities`, `f64_to_i32_clamped`). All writes go
/// to temporary tables via prepared `INSERT` (the read-only-safe `at_rev`
/// idiom — never `Appender`).
fn build_projected_complexity_table(
    db: &FactsDb,
    opts: &Options,
    changes: &[WorktreeChange],
    head_sha: &str,
) -> Result<HashMap<String, &'static str>> {
    // Every change-set path whose HEAD rows the projection drops: the change
    // path itself plus any rename source.
    let mut seen: HashSet<&str> = HashSet::new();
    let mut changed_paths: Vec<&str> = Vec::new();
    for change in changes {
        // The change path, plus the rename source when the entry is a rename
        // destination — both point at HEAD rows the projection must drop.
        let paths = std::iter::once(change.path.as_str()).chain(change.rename_from.as_deref());
        for path in paths {
            if seen.insert(path) {
                changed_paths.push(path);
            }
        }
    }

    db.execute_batch(&format!(
        "CREATE OR REPLACE TEMPORARY TABLE {CHANGED_PATHS_TABLE} (path TEXT NOT NULL)"
    ))?;
    {
        let mut stmt = db
            .conn()
            .prepare(&format!("INSERT INTO {CHANGED_PATHS_TABLE} VALUES (?)"))
            .map_err(|e| CodeLoreError::Analysis(format!("prepare {CHANGED_PATHS_TABLE}: {e}")))?;
        for path in &changed_paths {
            stmt.execute(duckdb::params![path]).map_err(|e| {
                CodeLoreError::Analysis(format!("insert {CHANGED_PATHS_TABLE}: {e}"))
            })?;
        }
    }

    // HEAD complexity for every file NOT in the change set. `CREATE … AS
    // SELECT *` clones the column shape (dropping constraints) so the prepared
    // INSERT below binds the same 19-column order the HEAD ingest / at-rev
    // paths use.
    db.execute_batch(&format!(
        "CREATE OR REPLACE TEMPORARY TABLE {PROJECTED_COMPLEXITY_TABLE} AS \
         SELECT * FROM complexity_metrics \
         WHERE path NOT IN (SELECT path FROM {CHANGED_PATHS_TABLE})"
    ))?;

    let mut skip_reasons: HashMap<String, &'static str> = HashMap::new();
    let mut insert = db
        .conn()
        .prepare(&format!(
            "INSERT INTO {PROJECTED_COMPLEXITY_TABLE} \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
        ))
        .map_err(|e| {
            CodeLoreError::Analysis(format!("prepare {PROJECTED_COMPLEXITY_TABLE}: {e}"))
        })?;

    for change in changes {
        if change.kind == WorktreeChangeKind::Deleted {
            continue; // Baseline-only; its HEAD rows are already dropped.
        }
        match parse_worktree_file(&opts.repo_path, &change.path)? {
            ParseOutcome::Skipped(reason) => {
                skip_reasons.insert(change.path.clone(), reason);
            }
            ParseOutcome::Entities(entities) => {
                // Bound in the exact column order of `consumer::append_metric_row`
                // / `at_rev::insert_complexity_rows`, clamping the INTEGER
                // columns identically so a byte-identical file yields
                // byte-identical rows. Bound inline (not via a helper) because
                // the `f64_to_i32_clamped` results are temporaries the prepared
                // statement borrows for the call.
                for ent in &entities {
                    insert
                        .execute(duckdb::params![
                            change.path,
                            ent.name,
                            head_sha,
                            f64_to_i32_clamped(ent.cyclomatic),
                            f64_to_i32_clamped(ent.cognitive),
                            ent.halstead_volume,
                            ent.halstead_difficulty,
                            ent.halstead_effort,
                            ent.mi,
                            i32::try_from(ent.nom).unwrap_or(i32::MAX),
                            i32::try_from(ent.nexits).unwrap_or(i32::MAX),
                            i32::try_from(ent.loc).unwrap_or(i32::MAX),
                            i32::try_from(ent.sloc).unwrap_or(i32::MAX),
                            i32::try_from(ent.max_nesting).unwrap_or(i32::MAX),
                            ent.mean_nesting,
                            ent.sd_nesting,
                            i32::try_from(ent.total_nesting).unwrap_or(i32::MAX),
                            i32::try_from(ent.nargs).unwrap_or(i32::MAX),
                            i32::try_from(ent.bool_ops).unwrap_or(i32::MAX),
                        ])
                        .map_err(|e| {
                            CodeLoreError::Analysis(format!(
                                "insert {PROJECTED_COMPLEXITY_TABLE}: {e}"
                            ))
                        })?;
                }
            }
        }
    }

    Ok(skip_reasons)
}

/// The result of re-parsing one changed working-tree file.
enum ParseOutcome {
    /// A parse-gate rejected the file; the caller records the reason and emits
    /// no projected rows.
    Skipped(&'static str),
    /// De-duplicated complexity entities ready to insert.
    Entities(Vec<ComplexityEntity>),
}

/// Re-parse one changed file from the working tree, replicating the HEAD
/// ingest pipeline exactly.
fn parse_worktree_file(repo_root: &Path, rel_path: &str) -> Result<ParseOutcome> {
    let Some(lang) = Tier1Language::from_path(rel_path) else {
        return Ok(ParseOutcome::Skipped(REASON_NOT_TIER1));
    };
    let source = std::fs::read(repo_root.join(rel_path))
        .map_err(|e| CodeLoreError::Analysis(format!("read worktree file {rel_path}: {e}")))?;
    if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
        return Ok(ParseOutcome::Skipped(REASON_SIZE_LIMIT));
    }
    let sniff_end = source.len().min(BINARY_SNIFF_BYTES);
    if source[..sniff_end].contains(&0u8) {
        return Ok(ParseOutcome::Skipped(REASON_BINARY));
    }
    let entities = compute_for_file(Path::new(rel_path), source, lang)?;
    Ok(ParseOutcome::Entities(dedup_entities(entities)))
}

/// Join one change against the baseline and projected row sets into a
/// [`FileDelta`], choosing the most specific honest-absence reason.
fn delta_for_change(
    change: &WorktreeChange,
    baseline_rows: &[CodeHealthRow],
    projected_rows: &[CodeHealthRow],
    skip_reasons: &HashMap<String, &'static str>,
) -> FileDelta {
    let kind = kind_str(change);
    let baseline = baseline_rows.iter().find(|r| r.path == change.path);

    // A deleted file has a baseline side only.
    if change.kind == WorktreeChangeKind::Deleted {
        return FileDelta {
            path: change.path.clone(),
            kind,
            baseline_score: baseline.map(|r| r.score),
            projected_score: None,
            delta: None,
            baseline_band: baseline.map(|r| r.band.clone()),
            projected_band: None,
            reason: Some(REASON_DELETED.to_string()),
        };
    }

    // A parse-gate skip is the most specific reason the projection is absent.
    if let Some(reason) = skip_reasons.get(change.path.as_str()) {
        return FileDelta {
            path: change.path.clone(),
            kind,
            baseline_score: baseline.map(|r| r.score),
            projected_score: None,
            delta: None,
            baseline_band: baseline.map(|r| r.band.clone()),
            projected_band: None,
            reason: Some((*reason).to_string()),
        };
    }

    let projected = projected_rows.iter().find(|r| r.path == change.path);
    match (baseline, projected) {
        (Some(b), Some(p)) => FileDelta {
            path: change.path.clone(),
            kind,
            baseline_score: Some(b.score),
            projected_score: Some(p.score),
            delta: Some(p.score - b.score),
            baseline_band: Some(b.band.clone()),
            projected_band: Some(p.band.clone()),
            reason: None,
        },
        (None, projected) => {
            // No HEAD row. An added file (or rename destination) has no history
            // so it can never be scored; any other file simply had no
            // code-health row at HEAD.
            let reason = if change.kind == WorktreeChangeKind::Added {
                REASON_NEW_FILE
            } else {
                REASON_NO_HEAD_ROW
            };
            FileDelta {
                path: change.path.clone(),
                kind,
                baseline_score: None,
                projected_score: projected.map(|p| p.score),
                delta: None,
                baseline_band: None,
                projected_band: projected.map(|p| p.band.clone()),
                reason: Some(reason.to_string()),
            }
        }
        (Some(b), None) => FileDelta {
            path: change.path.clone(),
            kind,
            baseline_score: Some(b.score),
            projected_score: None,
            delta: None,
            baseline_band: Some(b.band.clone()),
            projected_band: None,
            reason: Some(REASON_NO_PROJECTED_ROW.to_string()),
        },
    }
}

/// `"renamed"` when the backend reported a rename source, else the net kind.
fn kind_str(change: &WorktreeChange) -> String {
    if change.rename_from.is_some() {
        return "renamed".to_string();
    }
    match change.kind {
        WorktreeChangeKind::Added => "added",
        WorktreeChangeKind::Modified => "modified",
        WorktreeChangeKind::Deleted => "deleted",
    }
    .to_string()
}

/// Order deltas by `|delta|` descending (rows with no delta last), ties broken
/// by path ascending. Uses [`f64::total_cmp`] so ordering is total and
/// deterministic; no `HashMap` iteration reaches the output.
fn sort_deltas(deltas: &mut [FileDelta]) {
    deltas.sort_by(|a, b| match (a.delta, b.delta) {
        (Some(x), Some(y)) => y
            .abs()
            .total_cmp(&x.abs())
            .then_with(|| a.path.cmp(&b.path)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.path.cmp(&b.path),
    });
}

/// Median of a score stream, or `None` when empty. Even-length medians average
/// the two central values (`f64::midpoint`); sorting is total via
/// [`f64::total_cmp`].
fn median(scores: impl Iterator<Item = f64>) -> Option<f64> {
    let mut v: Vec<f64> = scores.collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[mid]
    } else {
        f64::midpoint(v[mid - 1], v[mid])
    })
}
