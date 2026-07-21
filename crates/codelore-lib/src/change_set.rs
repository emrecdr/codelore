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
//!   `imports` table (`imports_source` stays `"imports"`). The DRY biomarker is
//!   *dual-sourced*: the baseline counts clones from the HEAD-faithful `clones`
//!   table (ingested from HEAD blobs) while the projection fingerprints the
//!   working tree, so a duplicate introduced only in the working tree raises
//!   the projection's DRY count without touching the baseline — a real
//!   negative delta and a `clone-introduction` finding, not a cancel to zero.
//!   On a clean tree the two clone sources agree, keeping the delta exactly
//!   `0.0`.
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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::analyses::code_health::{
    CloneSource, CodeHealthRow, HealthScanCtx, run_code_health_scoped,
};
use crate::analyses::coupling::{CouplingAbsence, compute_coupling_absences, run_coupling};
use crate::analyses::import_graph::{
    ImportGraph, build_import_graph, build_import_graph_from_edges, tarjan_scc,
};
use crate::analyses::query::query_map_collect;
use crate::complexity::{ComplexityEntity, Tier1Language, compute_for_file};
use crate::constants::{DEFAULT_FISHER_SIGNIFICANCE, DEFAULT_MIN_SHARED_REVS};
use crate::facts::FactsDb;
use crate::facts::ingest::consumer::{dedup_entities, f64_to_i32_clamped};
use crate::imports::{ImportLanguage, extract_imports, resolve_by_extension};
use crate::repo::{WorktreeChange, WorktreeChangeKind};
use crate::{CodeLoreError, Options, Result};

/// The temporary table the projection scores against: HEAD `complexity_metrics`
/// minus the change-set paths, plus the re-parsed working-tree rows.
const PROJECTED_COMPLEXITY_TABLE: &str = "complexity_metrics_projected";

/// The temporary table listing every change-set path (changed + deleted +
/// rename sources) whose HEAD complexity rows the projection replaces.
const CHANGED_PATHS_TABLE: &str = "changed_paths_v1";

/// The temporary table listing every path gone from the working tree
/// (deleted files + rename sources). The cycle splice drops HEAD import
/// edges INTO these paths — the file no longer exists to be imported.
const DELETED_PATHS_TABLE: &str = "deleted_paths_v1";

/// NUL-byte binary sniff window, mirroring the repo layer's blob heuristic
/// (`BINARY_SNIFF_BYTES` in `repo::gix_repo`). HEAD ingest never sees binary
/// files because the blob-enumeration layer filters them; the engine reads the
/// working tree directly, so it re-applies the same sniff here.
const BINARY_SNIFF_BYTES: usize = 8000;

const REASON_NOT_TIER1: &str = "not a Tier-1 source file";
const REASON_BINARY: &str = "binary content";
const REASON_SIZE_LIMIT: &str = "file exceeds the AST size limit";
/// A deleted file's per-file delta is intentionally `None` — deletions are
/// excluded from `delta_code_health_min_per_file` and `new_file_health_min`
/// (both skip rows with no delta / no projected score). There is no honest
/// numeric "projected score" for a file that no longer exists, so the row
/// stays an explicit absence rather than a synthetic one; `baseline_score`
/// is still reported so the deletion's context (what health the file HAD)
/// isn't lost. Note this is NOT the same as "no effect": the whole-repo
/// `baseline_median`/`projected_median` pair (which `delta_code_health_min`
/// reads) is computed over each run's own scored population, so a deleted
/// file's row leaves the projected population — but because the projection
/// also re-sources clone/duplication counts from the working tree (see the
/// module doc comment), OTHER files' scores can shift too, so no fixed
/// direction (the median moving up or down) is guaranteed from a deletion
/// alone.
const REASON_DELETED: &str = "deleted at gate time";
/// Visible to [`crate::quality_gates`] so the `new_file_health_min` gate can
/// identify added-file rows by their honest-absence reason rather than
/// re-deriving the same classification from `kind` (which also reads
/// `"added"` for rename destinations — the same population this reason
/// already covers).
pub(crate) const REASON_NEW_FILE: &str = "new file (no history baseline)";
const REASON_NO_HEAD_ROW: &str = "no code-health row at HEAD";
/// The projection produced no scoreable row for a file that had one at HEAD
/// (the working tree emptied its analyzable content). Unreachable for a file
/// that still parses to at least the file-unit entity, but kept honest.
const REASON_NO_PROJECTED_ROW: &str = "no code-health row after projection";

/// One changed file's HEAD-vs-projected code-health scores, or an honest reason
/// a score is absent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// The full measured picture of what the current working-tree edits do to the
/// repository: enumerated changes, projected health deltas, cycle-membership
/// delta, absent historical co-change partners, and the advisory findings
/// derived from all of them.
///
/// Everything in here is MEASURED data — no verdicts. Consumers evaluate
/// thresholds against the report on every read, so a cached report can never
/// serve a stale verdict.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChangeSetReport {
    /// Full SHA-1 hex of the HEAD the projection is anchored to.
    pub head_sha: String,
    /// Whether the repository is partway through a merge / rebase /
    /// cherry-pick / revert. Recomputed on every build — including sidecar
    /// cache hits — because it is repo state, not measured content.
    pub merge_in_progress: bool,
    /// The enumerated working-tree changes, as the repo backend returned
    /// them (sorted by path).
    pub changes: Vec<WorktreeChange>,
    /// Projected code-health deltas plus whole-repo medians.
    pub health: HealthProjection,
    /// Files on some import cycle at HEAD, sorted.
    pub base_cyclic_paths: Vec<String>,
    /// Files cyclic in the projected graph but not at HEAD (cyclic-node
    /// membership difference, not a cycle-count comparison), sorted.
    pub newly_cyclic_paths: Vec<String>,
    /// Historically-coupled partners absent from the change set. The touched
    /// side is every non-deleted change-set path; a rename destination is a
    /// fresh path with no coupling rows in the fact store, so it inherits
    /// none of its source's partners.
    pub coupling_absences: Vec<CouplingAbsence>,
    /// Advisory findings assembled from the fields above, sorted by
    /// `(kind, path, detail)`.
    pub findings: Vec<Finding>,
}

/// One advisory observation about the change set. Findings carry no verdict;
/// gate verdicts come from evaluating thresholds against the report.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    /// First 12 hex chars of SHA-256 over `"{kind}|{path}|{detail}"` —
    /// stable across runs for identical content.
    pub id: String,
    /// `"health-drop"` | `"newly-cyclic"` | `"coupling-absence"` |
    /// `"clone-introduction"` | `"new-file"` | `"unparseable"`.
    pub kind: String,
    /// The repo-relative file the finding is about.
    pub path: String,
    /// One deterministic sentence of evidence.
    pub detail: String,
}

/// Build the full change-set report for the current working tree vs HEAD.
///
/// Enumerates the tracked changes itself via [`crate::Repo::worktree_changes`],
/// then runs the health projection, the cycle splice, and the coupling-absence
/// scan, and assembles the advisory findings. The whole report is memoised in
/// a content-keyed JSON sidecar under `cache_root`: a hit is returned as-is
/// except for `merge_in_progress`, which is recomputed because it is repo
/// state rather than measured content. Thresholds and calibration are
/// deliberately NOT part of the cache key — the report stores measured data
/// only, and consumers re-evaluate verdicts on every read.
///
/// # Errors
///
/// Propagates repo errors (including the unmerged-paths refusal from
/// `worktree_changes`), fact-store / SQL errors as
/// [`CodeLoreError::Analysis`], and working-tree read failures.
pub fn build_change_set_report<R: crate::Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    cache_root: &Path,
) -> Result<ChangeSetReport> {
    let head_sha = repo.head_sha()?;
    let changes = repo.worktree_changes()?;

    let key = cache::report_key(&head_sha, &changes, opts)?;
    if let Some(mut cached) = cache::read(cache_root, &opts.repo_path, &key) {
        cached.merge_in_progress = repo.merge_or_rebase_in_progress();
        return Ok(cached);
    }

    let health = project_health(db, repo, opts, &changes)?;
    let (base_cyclic_paths, newly_cyclic_paths) = project_cycles(db, repo, opts, &changes)?;

    // Touched side of the absence scan: every non-deleted change-set path.
    // Deleted files are excluded (there is nothing to co-change with), and a
    // rename destination is a fresh path that inherits no coupling history.
    let touched: HashSet<String> = changes
        .iter()
        .filter(|c| c.kind != WorktreeChangeKind::Deleted)
        .map(|c| c.path.clone())
        .collect();
    let coupling = run_coupling(db, opts)?;
    let coupling_absences = compute_coupling_absences(
        &coupling,
        &touched,
        DEFAULT_MIN_SHARED_REVS,
        DEFAULT_FISHER_SIGNIFICANCE,
    );

    let clone_intros = clone_introductions(db, opts, &changes)?;
    let findings = assemble_findings(
        &health,
        &newly_cyclic_paths,
        &coupling_absences,
        &clone_intros,
        &changes,
    );

    let report = ChangeSetReport {
        head_sha,
        merge_in_progress: repo.merge_or_rebase_in_progress(),
        changes,
        health,
        base_cyclic_paths,
        newly_cyclic_paths,
        coupling_absences,
        findings,
    };
    cache::write(cache_root, &opts.repo_path, &key, &report);
    Ok(report)
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

    // 1. Baseline: today's HEAD tables. `CloneSource::Head` reads the DRY
    //    biomarker from the ingested `clones` table (HEAD blobs) rather than
    //    the working tree, so `baseline_score` is HEAD-faithful and a
    //    working-tree-introduced duplicate is absent here — it stops cancelling
    //    against the projection below.
    let baseline_ctx = HealthScanCtx {
        clone_source: CloneSource::Head,
        ..HealthScanCtx::head()
    };
    let baseline_rows = run_code_health_scoped(db, &opts_scan, &baseline_ctx)?;

    // 2. Substitute the changed files' complexity, then re-run the SAME engine
    //    against the projected table. `include_clones: true` on both runs so
    //    the STRUCTURAL_SCALE_NO_DRY divisor matches (scale parity). The
    //    projection counts clones from the working tree (`CloneSource::WorkingTree`,
    //    served from the clones memo), so a newly duplicated function raises its
    //    DRY count above the HEAD baseline instead of appearing in both runs.
    let head_sha = repo.head_sha()?;
    let skip_reasons = build_projected_complexity_table(db, opts, changes, &head_sha)?;
    let projected_ctx = HealthScanCtx {
        complexity_source: PROJECTED_COMPLEXITY_TABLE.to_string(),
        imports_source: "imports".to_string(),
        history_cutoff: None,
        include_clones: true,
        clone_source: CloneSource::WorkingTree,
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
    populate_path_table(db, CHANGED_PATHS_TABLE, changed_set_paths(changes))?;

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

/// Every change-set path whose HEAD facts the projection replaces or drops:
/// the change path itself plus any rename source (both point at HEAD rows
/// the substituted table must not carry), deduped in first-seen order.
fn changed_set_paths(changes: &[WorktreeChange]) -> Vec<&str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut paths: Vec<&str> = Vec::new();
    for change in changes {
        let candidates = std::iter::once(change.path.as_str()).chain(change.rename_from.as_deref());
        for path in candidates {
            if seen.insert(path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// Paths that no longer exist in the working tree: every `Deleted` entry plus
/// any rename source (the backend contract emits the source as its own
/// `Deleted` entry, so the explicit `rename_from` sweep is belt-and-braces),
/// deduped in first-seen order.
fn deleted_set_paths(changes: &[WorktreeChange]) -> Vec<&str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut paths: Vec<&str> = Vec::new();
    for change in changes {
        let deleted = (change.kind == WorktreeChangeKind::Deleted).then_some(change.path.as_str());
        for path in deleted.into_iter().chain(change.rename_from.as_deref()) {
            if seen.insert(path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// (Re)create the single-column temporary table `name (path TEXT NOT NULL)`
/// holding `paths`, inserted via prepared `INSERT` (the read-only-safe
/// `at_rev` idiom — never `Appender`). `name` is always one of this module's
/// compile-time table-name constants, never user input, so the SQL
/// interpolation is safe.
fn populate_path_table<'a>(
    db: &FactsDb,
    name: &str,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    db.execute_batch(&format!(
        "CREATE OR REPLACE TEMPORARY TABLE {name} (path TEXT NOT NULL)"
    ))?;
    let mut stmt = db
        .conn()
        .prepare(&format!("INSERT INTO {name} VALUES (?)"))
        .map_err(|e| CodeLoreError::Analysis(format!("prepare {name}: {e}")))?;
    for path in paths {
        stmt.execute(duckdb::params![path])
            .map_err(|e| CodeLoreError::Analysis(format!("insert {name}: {e}")))?;
    }
    Ok(())
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

/// Compare cyclic-node MEMBERSHIP between the HEAD import graph and the
/// working-tree projection: `(base cyclic paths, newly cyclic paths)`, both
/// sorted. Membership (not a cycle-count comparison) names the files and is
/// immune to the count blind spot where two HEAD cycles merging into one
/// bigger tangle DROPS the count.
///
/// The projected edge set is a three-part rebuild — a naive out-edge
/// replacement would keep stale edges INTO deleted files and miss imports in
/// unchanged files that only became resolvable now:
///
/// 1. HEAD edges that survive: resolved `imports` rows whose source is not a
///    change-set path (changed sources are re-extracted below; deleted
///    sources are gone) AND whose target still exists in the working tree.
/// 2. Re-extracted edges: each changed non-deleted file with an import
///    grammar is parsed from its working-tree bytes and resolved against the
///    updated live set.
/// 3. Re-resolution sweep: unresolved `imports` rows from UNCHANGED files are
///    retried against the updated live set — an added file can make a
///    previously-unresolvable import resolvable.
fn project_cycles<R: crate::Repo>(
    db: &FactsDb,
    repo: &R,
    opts: &Options,
    changes: &[WorktreeChange],
) -> Result<(Vec<String>, Vec<String>)> {
    let base_graph = build_import_graph(db)?;
    let base_cyclic = cyclic_paths(&base_graph);

    // Updated live set = tracked-at-HEAD − deleted − rename sources
    // + added / rename-destination paths (modified paths are already
    // tracked at HEAD).
    let gone: HashSet<&str> = deleted_set_paths(changes).into_iter().collect();
    let mut live: HashSet<String> = repo
        .tracked_paths_at_head()?
        .into_iter()
        .filter(|p| !gone.contains(p.as_str()))
        .collect();
    for change in changes {
        if change.kind == WorktreeChangeKind::Added {
            live.insert(change.path.clone());
        }
    }

    populate_path_table(db, CHANGED_PATHS_TABLE, changed_set_paths(changes))?;
    populate_path_table(db, DELETED_PATHS_TABLE, deleted_set_paths(changes))?;

    // Part 1: surviving HEAD edges.
    let mut edges: Vec<(String, String)> = query_map_collect(
        db,
        &format!(
            "SELECT src_path, target_path FROM imports \
             WHERE target_path IS NOT NULL \
               AND src_path NOT IN (SELECT path FROM {CHANGED_PATHS_TABLE}) \
               AND target_path NOT IN (SELECT path FROM {DELETED_PATHS_TABLE})"
        ),
        [],
        "change-set surviving import edges",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;

    // Part 2: re-extracted edges from the changed files' working-tree bytes.
    // Ingest parity with `populate_imports_at_head`: the AST size cap gates
    // extraction, and a per-file extraction error is logged and skipped
    // rather than failing the run.
    for change in changes {
        if change.kind == WorktreeChangeKind::Deleted {
            continue;
        }
        let Some(lang) = ImportLanguage::from_path(Path::new(&change.path)) else {
            continue;
        };
        let source = std::fs::read(opts.repo_path.join(&change.path)).map_err(|e| {
            CodeLoreError::Analysis(format!("read worktree file {}: {e}", change.path))
        })?;
        if source.len() > crate::constants::DEFAULT_MAX_AST_FILE_BYTES {
            continue;
        }
        let imports = match extract_imports(&source, lang) {
            Ok(imports) => imports,
            Err(e) => {
                tracing::warn!("change-set: import extract failed for {}: {e}", change.path);
                continue;
            }
        };
        for import in imports {
            if let Some(target_path) = resolve_by_extension(&change.path, &import.target, &live) {
                edges.push((change.path.clone(), target_path));
            }
        }
    }

    // Part 3: re-resolution sweep over unchanged files' unresolved imports.
    let unresolved: Vec<(String, String)> = query_map_collect(
        db,
        &format!(
            "SELECT src_path, target FROM imports \
             WHERE NOT resolved \
               AND src_path NOT IN (SELECT path FROM {CHANGED_PATHS_TABLE})"
        ),
        [],
        "change-set unresolved import sweep",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )?;
    for (src_path, target) in unresolved {
        if let Some(target_path) = resolve_by_extension(&src_path, &target, &live) {
            edges.push((src_path, target_path));
        }
    }

    let projected_cyclic = cyclic_paths(&build_import_graph_from_edges(&edges));
    let newly: Vec<String> = projected_cyclic.difference(&base_cyclic).cloned().collect();
    Ok((base_cyclic.into_iter().collect(), newly))
}

/// The paths sitting on some import cycle of `graph` (members of any SCC of
/// size ≥ 2), as a sorted set.
fn cyclic_paths(graph: &ImportGraph) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for component in tarjan_scc(&graph.adj) {
        if component.len() >= 2 {
            for id in component {
                out.insert(graph.id_to_path[id].clone());
            }
        }
    }
    out
}

/// One changed file whose working-tree clone-family membership exceeds its
/// HEAD membership — a duplicate the edit introduced (or enlarged).
struct CloneIntroduction {
    /// Repo-relative, `/`-separated path of the changed file.
    path: String,
    /// Clone-family members the file carries at HEAD (from the `clones` table).
    head_members: u32,
    /// Clone-family members the file carries in the working tree.
    worktree_members: u32,
}

/// Detect duplicated functions introduced by the working-tree edits.
///
/// Compares each changed non-deleted file's HEAD clone-family membership (the
/// ingested `clones` table, via [`crate::analyses::clones::head_clone_counts`])
/// against its working-tree membership (the same
/// [`crate::analyses::clones::run_clones_memoised`] walk the projection already
/// warmed) and reports the files whose working-tree count is higher. Only
/// changed files are considered — an unchanged file that happens to become a
/// clone of an edited one is not the user's introduction to act on, mirroring
/// how coupling absences report only the touched side.
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] on a fact-store / SQL / clone-walk error.
fn clone_introductions(
    db: &FactsDb,
    opts: &Options,
    changes: &[WorktreeChange],
) -> Result<Vec<CloneIntroduction>> {
    let head = crate::analyses::clones::head_clone_counts(db)?;
    let worktree_rows = crate::analyses::clones::run_clones_memoised(db, opts)?;
    let mut worktree: HashMap<&str, u32> = HashMap::new();
    for c in worktree_rows.iter() {
        *worktree.entry(c.entity.as_str()).or_insert(0) += 1;
    }

    let mut intros = Vec::new();
    for change in changes {
        if change.kind == WorktreeChangeKind::Deleted {
            continue;
        }
        let head_members = head.get(&change.path).copied().unwrap_or(0);
        let worktree_members = worktree.get(change.path.as_str()).copied().unwrap_or(0);
        if worktree_members > head_members {
            intros.push(CloneIntroduction {
                path: change.path.clone(),
                head_members,
                worktree_members,
            });
        }
    }
    Ok(intros)
}

/// Assemble the advisory findings from the measured report parts, sorted by
/// `(kind, path, detail)` — deterministic even when one path carries several
/// findings of the same kind (a file with two absent coupling partners).
fn assemble_findings(
    health: &HealthProjection,
    newly_cyclic: &[String],
    absences: &[CouplingAbsence],
    clone_intros: &[CloneIntroduction],
    changes: &[WorktreeChange],
) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();

    for delta_row in &health.deltas {
        let (Some(delta), Some(baseline), Some(projected)) = (
            delta_row.delta,
            delta_row.baseline_score,
            delta_row.projected_score,
        ) else {
            continue;
        };
        if delta < 0.0 {
            findings.push(finding(
                "health-drop",
                &delta_row.path,
                &format!(
                    "projected code health drops from {baseline:.1} to {projected:.1} ({delta:+.1})."
                ),
            ));
        }
    }
    for path in newly_cyclic {
        findings.push(finding(
            "newly-cyclic",
            path,
            "enters an import cycle that does not exist at HEAD.",
        ));
    }
    for absence in absences {
        findings.push(finding(
            "coupling-absence",
            &absence.touched_file,
            &format!(
                "historically co-changes with {} ({:.0}% of commits, {} shared revisions), \
                 which is not in this change set.",
                absence.expected_partner,
                absence.historical_coupling,
                absence.historical_shared_revs,
            ),
        ));
    }
    for intro in clone_intros {
        let gained = intro.worktree_members.saturating_sub(intro.head_members);
        findings.push(finding(
            "clone-introduction",
            &intro.path,
            &format!(
                "introduces {gained} duplicated function(s) absent at HEAD \
                 (clone-family members rise from {} to {}).",
                intro.head_members, intro.worktree_members,
            ),
        ));
    }
    for change in changes {
        if change.kind == WorktreeChangeKind::Added {
            let detail = match change.rename_from.as_deref() {
                Some(source) => {
                    format!("renamed from {source}; history does not carry over to the new path.")
                }
                None => "new file with no history baseline.".to_string(),
            };
            findings.push(finding("new-file", &change.path, &detail));
        }
    }
    for delta_row in &health.deltas {
        let Some(reason) = delta_row.reason.as_deref() else {
            continue;
        };
        if reason == REASON_BINARY || reason == REASON_SIZE_LIMIT {
            findings.push(finding(
                "unparseable",
                &delta_row.path,
                &format!("could not be re-parsed for the projection: {reason}."),
            ));
        }
    }

    findings.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.detail.cmp(&b.detail))
    });
    findings
}

/// Construct one [`Finding`], deriving its content-stable id: the first 12
/// hex chars of SHA-256 over `"{kind}|{path}|{detail}"`.
fn finding(kind: &str, path: &str, detail: &str) -> Finding {
    let digest = Sha256::digest(format!("{kind}|{path}|{detail}").as_bytes());
    let mut id = hex::encode(digest);
    id.truncate(12);
    Finding {
        id,
        kind: kind.to_string(),
        path: path.to_string(),
        detail: detail.to_string(),
    }
}

pub mod cache {
    //! Content-keyed JSON sidecar for [`ChangeSetReport`], mirroring the
    //! enrichment narrative cache's shape: plain-hex key, per-repo directory
    //! under the shared cache root, best-effort read/write, corrupt = miss.
    //!
    //! The key covers HEAD plus the exact worktree content of every
    //! change-set path, so any edit — or a commit — moves it. Thresholds and
    //! calibration are deliberately EXCLUDED: the sidecar stores only
    //! MEASURED data, and consumers recompute verdicts on every read, so an
    //! improved gate configuration reaches warm caches without invalidation.

    use std::path::{Path, PathBuf};

    use sha2::{Digest, Sha256};

    use super::ChangeSetReport;
    use crate::cache::repo_cache_dir;
    use crate::repo::{WorktreeChange, WorktreeChangeKind};
    use crate::{CodeLoreError, Options, Result};

    /// Report-shape tag folded into the key so a future incompatible report
    /// change invalidates old sidecars by construction.
    const KEY_SCHEMA: &str = "change-set-v1";

    /// Filename length: the first 16 hex chars of the 64-char key.
    const FILE_STEM_LEN: usize = 16;

    /// The content key for a change set: lowercase-hex SHA-256 of
    /// `head_sha | sorted "path\0content-sha256" lines | crate version |`
    /// `calib=<digest> | rows_limit=<n> | opts=<canonical-json digest> | `
    /// [`KEY_SCHEMA`]. A deleted path contributes the literal `"deleted"` in
    /// place of its content hash.
    ///
    /// The defect-calibration artifact's CONTENT digest is folded in because
    /// `--defect-calibration` substitutes smell weights inside the scoring
    /// engine — it changes the MEASURED health scores in the report, not just
    /// the verdict. Two runs on the same worktree and HEAD, one calibrated and
    /// one not, must not share a cache entry. `calib=` is empty when no
    /// artifact is configured. Thresholds stay excluded: they only affect
    /// verdicts, which consumers always recompute from the cached report.
    ///
    /// Every other report-affecting `Options` knob is folded in via
    /// [`Options::canonical_json`] — the same digest the ingest cache uses —
    /// so `min_revs`, `exclude_patterns`/`include_ignored`, the clone
    /// thresholds, and any future field are covered with zero per-field
    /// maintenance: `build_change_set_report`'s `run_coupling(db, opts)` and
    /// `clone_introductions(db, opts, …)` calls pass the caller's `opts`
    /// straight through (unlike the health projection's own `opts_scan`,
    /// which pins `min_revs = 1`), so any of those knobs can change which
    /// coupling-absence or clone-introduction findings this report contains.
    /// `rows_limit` is folded in explicitly ALONGSIDE the canonical digest
    /// rather than relying on it: `canonical_json` deliberately drops
    /// `rows_limit` as cosmetic for the ingest cache, but
    /// `run_coupling(db, opts)` truncates to it before the coupling-absence
    /// filter runs, so it is not cosmetic here.
    ///
    /// # Errors
    ///
    /// Returns [`CodeLoreError::Analysis`] when a non-deleted change-set
    /// file cannot be read from the working tree — the engine could not
    /// project it either, so failing early is honest.
    pub fn report_key(
        head_sha: &str,
        changes: &[WorktreeChange],
        opts: &Options,
    ) -> Result<String> {
        let mut lines: Vec<String> = Vec::with_capacity(changes.len());
        for change in changes {
            let content = if change.kind == WorktreeChangeKind::Deleted {
                "deleted".to_string()
            } else {
                let bytes = std::fs::read(opts.repo_path.join(&change.path)).map_err(|e| {
                    CodeLoreError::Analysis(format!("read worktree file {}: {e}", change.path))
                })?;
                hex::encode(Sha256::digest(&bytes))
            };
            lines.push(format!("{}\0{content}", change.path));
        }
        lines.sort();
        // Content digest, not path — byte-identical artifacts from different
        // locations share a key, and editing the artifact in place is visible.
        // Unset or unreadable both yield an empty segment; an unreadable
        // artifact hard-errors in the scoring engine before any report is
        // cached, so the collision with "unset" is unreachable.
        let calib = opts
            .defect_calibration
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .map(|bytes| hex::encode(Sha256::digest(&bytes)))
            .unwrap_or_default();
        let opts_digest = opts.canonical_json().to_string();
        let material = format!(
            "{head_sha}|{}|{}|calib={calib}|rows_limit={:?}|opts={opts_digest}|{KEY_SCHEMA}",
            lines.join("\n"),
            env!("CARGO_PKG_VERSION"),
            opts.rows_limit,
        );
        Ok(hex::encode(Sha256::digest(material.as_bytes())))
    }

    /// The on-disk sidecar path for `key`:
    /// `repo_cache_dir(cache_root, repo_path)/change-set/<first 16 hex>.json`.
    #[must_use]
    pub fn cache_path(cache_root: &Path, repo_path: &Path, key: &str) -> PathBuf {
        let stem = &key[..FILE_STEM_LEN.min(key.len())];
        repo_cache_dir(cache_root, repo_path)
            .join("change-set")
            .join(format!("{stem}.json"))
    }

    /// Read the cached report at `key`, or `None` when absent or corrupt. A
    /// missing file is the ordinary miss and stays silent; a file that no
    /// longer deserializes is logged at `warn` and treated as a miss.
    #[must_use]
    pub fn read(cache_root: &Path, repo_path: &Path, key: &str) -> Option<ChangeSetReport> {
        let path = cache_path(cache_root, repo_path, key);
        let text = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str(&text) {
            Ok(report) => Some(report),
            Err(e) => {
                tracing::warn!(
                    "change-set cache: ignoring corrupt entry {}: {e}",
                    path.display()
                );
                None
            }
        }
    }

    /// Write `report` to the sidecar for `key`, creating the directory if
    /// needed. Best-effort: a cache is an optimization, so any failure is
    /// logged at `warn` and swallowed — a gate run must never fail because
    /// its report could not be cached.
    pub fn write(cache_root: &Path, repo_path: &Path, key: &str, report: &ChangeSetReport) {
        let path = cache_path(cache_root, repo_path, key);
        let Some(parent) = path.parent() else {
            tracing::warn!(
                "change-set cache: entry path {} has no parent directory",
                path.display()
            );
            return;
        };
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                "change-set cache: could not create {}: {e}",
                parent.display()
            );
            return;
        }
        let json = match serde_json::to_string_pretty(report) {
            Ok(json) => json,
            Err(e) => {
                tracing::warn!("change-set cache: could not serialize entry for {key}: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!("change-set cache: could not write {}: {e}", path.display());
        }
    }
}
