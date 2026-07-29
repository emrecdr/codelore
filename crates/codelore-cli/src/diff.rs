//! `codelore diff <base>..<head>` PR-mode delta analysis.
//!
//! Strategy A from the research brief: run full analyses at base + head
//! independently, then diff the result sets. Three-dot `<base>...<head>`
//! resolves via `git merge-base`. Base analysis is cacheable via
//! `--base-cache PATH` for cross-PR reuse.
//!
//! The non-destructive checkout strategy uses `git worktree add` to a
//! tempdir per rev. The worktree is removed after analysis. This means
//! `codelore diff` doesn't disturb the user's working tree.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use codelore_lib::cli_api::Options;
use codelore_lib::cli_api::analyses::clones::{ClonesRow, run_clones};
use codelore_lib::cli_api::analyses::code_health::run_code_health;
use codelore_lib::cli_api::analyses::coupling::{
    CouplingAbsence, CouplingRow, compute_coupling_absences, run_coupling,
};
use codelore_lib::cli_api::analyses::delta_health::{
    DeltaHealthSection, FunctionMetricRow, compute_delta_health, run_function_metrics,
};
use codelore_lib::cli_api::analyses::hotspots::{HotspotRow, run_hotspots};
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::GixRepo;
use serde::{Deserialize, Serialize};

use crate::args::DiffArgs;

/// What the diff runner produces. Each delta section is computed per the
/// research brief semantics (see field doc-comments).
#[derive(Debug, Default, Serialize)]
pub struct DiffOutput {
    pub base_sha: String,
    pub head_sha: String,
    pub merge_base_used: bool,
    pub hotspots: HotspotsDelta,
    pub coupling_absences: Vec<CouplingAbsence>,
    pub clones: ClonesDelta,
    /// Median `code_health` over the base-rev hotspots set. Computed
    /// only when `--thresholds-file` is set AND a `[diff]` gate is
    /// configured; otherwise `None` (field omitted from JSON output).
    /// Surfaced so downstream tools can re-evaluate the
    /// `[diff].delta_code_health_min` gate against their own
    /// thresholds without re-running the analysis.
    ///
    /// `Option<f64>` (not `f64`) so consumers can distinguish "gate
    /// not configured, no measurement taken" from "measured median
    /// = 0.0" — the latter would read as catastrophic health on the
    /// 0-100 scale and trigger false alarms in downstream dashboards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_median_code_health: Option<f64>,
    /// Median `code_health` over the head-rev hotspots set. Same
    /// triggering / absence semantics as `base_median_code_health`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_median_code_health: Option<f64>,
    /// `[diff]` quality-gate violations. Empty when the gate is
    /// vacuous (no `--thresholds-file` or no `[diff]` section), when the
    /// gate family was skipped (see `gate_skip_reason`), OR when a real,
    /// measured range passed cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_violations: Vec<GateViolationOut>,
    /// Set when the `[diff]` gate family was configured but skipped rather
    /// than evaluated: neither the base nor the head revision measured any
    /// hotspot rows (a blind ingest — e.g. a shallow checkout — not a
    /// genuinely unchanged repository, which still carries real rows on both
    /// sides). `None` when the gate is vacuous (not configured) or ran
    /// against real, measured data — including a real, measured range that
    /// passed with zero violations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_skip_reason: Option<String>,
    /// Change-level health verdict. `None` when the base analysis lacks
    /// function metrics (stale `--base-cache` written by an older binary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_health: Option<DeltaHealthSection>,
}

/// JSON-serialisable mirror of `quality_gates::GateViolation`.
/// We re-export the lib type with `Serialize` rather than depending on
/// the library's struct directly: keeps the library's gate type free
/// of `Serialize` derives that would propagate `serde` through every
/// gate-evaluating consumer.
#[derive(Debug, Clone, Serialize)]
pub struct GateViolationOut {
    pub gate: String,
    pub path: String,
    pub actual: String,
    pub threshold: String,
}

impl From<codelore_lib::cli_api::quality_gates::GateViolation> for GateViolationOut {
    fn from(v: codelore_lib::cli_api::quality_gates::GateViolation) -> Self {
        Self {
            gate: v.gate,
            path: v.path,
            actual: v.actual,
            threshold: v.threshold,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct HotspotsDelta {
    /// Files that newly enter top-N at head (NOT in top-N at base).
    pub rank_entrants: Vec<HotspotRow>,
    /// Files in top-N at both ends; score grew by ≥ threshold.
    pub score_increased: Vec<ScoreDelta>,
    /// Files the PR touched that were already top-N hotspots at base.
    /// Information-only — context for the reviewer.
    pub pr_touched_existing: Vec<HotspotRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreDelta {
    pub path: String,
    pub base_score: f64,
    pub head_score: f64,
    pub delta: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct ClonesDelta {
    /// Clone families introduced by the PR (head families whose
    /// fingerprint did not appear in base).
    pub new_families: Vec<ClonesRow>,
    /// Clone families where the PR modified at least one member.
    pub pr_touched_existing: Vec<ClonesRow>,
}

/// Cached or freshly-computed analysis output at a single rev.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RevAnalyses {
    pub sha: String,
    pub hotspots: Vec<HotspotRow>,
    pub coupling: Vec<CouplingRow>,
    pub clones: Vec<ClonesRow>,
    /// Number of import-graph dependency cycles at this rev (non-trivial
    /// SCCs). Powers the `[diff] no_new_cycles` gate. `#[serde(default)]`
    /// so a base-cache written before this field deserialises to 0.
    #[serde(default)]
    pub dependency_cycles: u32,
    /// Per-function metric rows for delta-health. `#[serde(default)]` so a
    /// base-cache written before this field deserialises to empty; the
    /// consumer treats empty-with-nonempty-hotspots as "stale cache" and
    /// skips delta-health rather than misreading every head function as
    /// added.
    #[serde(default)]
    pub functions: Vec<FunctionMetricRow>,
    /// Paths whose file-level code-health band is red at this rev. Powers
    /// the delta-health context multiplier.
    #[serde(default)]
    pub red_files: Vec<String>,
    /// Digest of the analysis-affecting options (`min_revs`, `exclude`) that
    /// produced this base cache. Compared alongside `sha` on `--base-cache`
    /// reuse so a cache built under one option set is never served to a run
    /// with different options. `#[serde(default)]` so a pre-existing cache
    /// lacking the field deserialises to "" and is treated as a mismatch —
    /// recomputed once, never served.
    #[serde(default)]
    pub opts_digest: String,
}

// ---------------------------------------------------------------------------
// Rev range parsing
// ---------------------------------------------------------------------------

/// Parse `<base>..<head>` or `<base>...<head>` into base + head SHA strings.
/// For three-dot, the returned base is the merge-base of (`base_rev`, `head_rev`),
/// computed via `git merge-base`.
///
/// Implied-HEAD shortcuts (matching `git log`/`git diff` semantics): an
/// omitted side of a range expression defaults to `HEAD`. So `main..` is
/// `main..HEAD`, `..main` is `HEAD..main`, and `..` is `HEAD..HEAD`
/// (an empty range — `git rev-parse HEAD` still resolves, producing a
/// no-op diff). Same applies to the three-dot form.
pub fn parse_rev_range(repo: &Path, range: &str) -> Result<(String, String, bool)> {
    // Match three-dot first (longer match) before two-dot.
    if let Some((base_ref, head_ref)) = range.split_once("...") {
        let base_ref = if base_ref.is_empty() {
            "HEAD"
        } else {
            base_ref
        };
        let head_ref = if head_ref.is_empty() {
            "HEAD"
        } else {
            head_ref
        };
        let base_sha = git_rev_parse(repo, base_ref)?;
        let head_sha = git_rev_parse(repo, head_ref)?;
        let mb = git_merge_base(repo, &base_sha, &head_sha)?;
        return Ok((mb, head_sha, true));
    }
    if let Some((base_ref, head_ref)) = range.split_once("..") {
        let base_ref = if base_ref.is_empty() {
            "HEAD"
        } else {
            base_ref
        };
        let head_ref = if head_ref.is_empty() {
            "HEAD"
        } else {
            head_ref
        };
        let base_sha = git_rev_parse(repo, base_ref)?;
        let head_sha = git_rev_parse(repo, head_ref)?;
        return Ok((base_sha, head_sha, false));
    }
    Err(anyhow!("rev range must contain '..' or '...': {range:?}"))
}

fn git_rev_parse(repo: &Path, rev: &str) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", rev])
        .output()
        .with_context(|| format!("git rev-parse {rev}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git rev-parse failed for {rev:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

fn git_merge_base(repo: &Path, a: &str, b: &str) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", a, b])
        .output()
        .with_context(|| "git merge-base")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git merge-base {a}..{b} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

// ---------------------------------------------------------------------------
// Worktree-based non-destructive analysis at a rev
// ---------------------------------------------------------------------------

/// A `git worktree` checkout that auto-cleans on drop.
struct Worktree {
    repo_root: PathBuf,
    path: PathBuf,
}

impl Drop for Worktree {
    fn drop(&mut self) {
        // Best-effort cleanup; `git worktree remove --force` is idempotent.
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output();
        // Also remove the dir in case worktree-remove didn't.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn add_worktree(repo: &Path, sha: &str) -> Result<Worktree> {
    // Tempdir under codelore's cache root so cleanup is in our own scope.
    // Routed through `codelore_lib::cli_api::cache::default_cache_root()` so the
    // user-namespaced `/tmp` fallback is applied here too — earlier
    // versions hardcoded a bare `/tmp` which collided across users on
    // shared hosts when `dirs::cache_dir()` returned `None`.
    let cache_root = codelore_lib::cli_api::cache::default_cache_root()
        .join("codelore")
        .join("diff-worktrees");
    std::fs::create_dir_all(&cache_root)?;
    let tmp = tempfile::Builder::new()
        .prefix(&format!("wt-{}-", &sha[..8.min(sha.len())]))
        .tempdir_in(&cache_root)?;
    let path = tmp.path().to_path_buf();
    // Run `git worktree add` FIRST, then `tmp.keep()` only on
    // success. If `keep()` ran first (as the previous code did) and git
    // failed (invalid rev, local corruption, lock error), the TempDir
    // had already been demoted to a plain owned path and the directory
    // leaked under the cache root forever. With this ordering, a git
    // failure lets `tmp` Drop cleanly and the empty dir disappears.
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["worktree", "add", "--detach", "--quiet"])
        .arg(&path)
        .arg(sha)
        .output()
        .with_context(|| format!("git worktree add {sha}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // Git succeeded — promote the tempdir to owned. The Worktree's Drop
    // impl handles `git worktree remove` cleanup from here.
    let _ = tmp.keep();
    Ok(Worktree {
        repo_root: repo.to_path_buf(),
        path,
    })
}

/// Run hotspot + coupling + clones analyses against the given rev. Uses
/// a `git worktree` so the user's working tree is not disturbed.
///
/// Also returns the ingested [`FactsDb`] and [`Options`] for the caller's
/// subsequent use (e.g. evidence queries in the SARIF emitter). The worktree
/// is removed at the end of this function — the returned `FactsDb` is
/// in-memory and does not depend on the worktree path. Base-rev callers drop
/// the extra returns.
///
/// `want_red_files` gates the code-health pass that populates `red_files`.
/// Only the base rev's red band drives the delta-health context multiplier,
/// so the head rev passes `false` and skips a full (coupling + clone + SQL)
/// code-health analysis whose result would be discarded.
fn analyze_at_rev(
    repo: &Path,
    sha: &str,
    args: &DiffArgs,
    want_red_files: bool,
) -> Result<(RevAnalyses, FactsDb, Options)> {
    let wt = add_worktree(repo, sha)?;
    let opts = Options {
        repo_path: wt.path.clone(),
        min_revs: args.min_revs,
        exclude_patterns: args.exclude.clone(),
        ..Options::default()
    };
    let gix = GixRepo::open(&wt.path).context("open gix repo in worktree")?;
    let db = FactsDb::new_in_memory().context("open in-memory fact store")?;
    db.ingest(&gix, &opts).context("ingest in worktree")?;

    let hotspots = run_hotspots(&db, &opts).context("hotspots at rev")?;
    let coupling = run_coupling(&db, &opts).context("coupling at rev")?;
    let clones = run_clones(&opts).context("clones at rev")?;
    // Cheap (O(V+E)) relative to the analyses above; always computed so
    // the value is available for the `no_new_cycles` gate and cached.
    let graph = codelore_lib::cli_api::analyses::import_graph::build_import_graph(&db)
        .context("import graph at rev")?;
    let dependency_cycles =
        codelore_lib::cli_api::analyses::import_graph::graph_metrics(&graph).cycle_count;
    let functions = run_function_metrics(&db).context("function metrics at rev")?;
    let red_files: Vec<String> = if want_red_files {
        run_code_health(&db, &opts)
            .context("code health at rev")?
            .into_iter()
            .filter(|r| r.band == "red")
            .map(|r| r.path)
            .collect()
    } else {
        Vec::new()
    };

    // wt drops here, cleaning up the worktree on disk; db is in-memory and lives on.
    drop(wt);

    let analyses = RevAnalyses {
        sha: sha.to_string(),
        hotspots,
        coupling,
        clones,
        dependency_cycles,
        functions,
        red_files,
        opts_digest: base_cache_opts_digest(args.min_revs, &args.exclude),
    };
    Ok((analyses, db, opts))
}

fn load_base_cache(path: &Path) -> Result<RevAnalyses> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read --base-cache {}", path.display()))?;
    serde_json::from_str(&body).context("parse --base-cache JSON")
}

fn write_base_cache(path: &Path, analyses: &RevAnalyses) -> Result<()> {
    let body = serde_json::to_string_pretty(analyses)?;
    std::fs::write(path, body).with_context(|| format!("write --base-cache {}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Delta computation (per the research brief)
// ---------------------------------------------------------------------------

fn compute_hotspots_delta(
    base: &[HotspotRow],
    head: &[HotspotRow],
    pr_files: &std::collections::HashSet<String>,
    top_n: usize,
    score_threshold: f64,
) -> HotspotsDelta {
    use std::collections::{HashMap, HashSet};

    let base_top: HashSet<&str> = base.iter().take(top_n).map(|h| h.path.as_str()).collect();
    let head_top: Vec<&HotspotRow> = head.iter().take(top_n).collect();

    let mut rank_entrants: Vec<HotspotRow> = Vec::new();
    for h in &head_top {
        if !base_top.contains(h.path.as_str()) {
            rank_entrants.push((*h).clone());
        }
    }

    // Score-increased: files in both top-N where head.score - base.score ≥ threshold.
    let base_by_path: HashMap<&str, &HotspotRow> = base
        .iter()
        .take(top_n)
        .map(|h| (h.path.as_str(), h))
        .collect();
    let mut score_increased: Vec<ScoreDelta> = Vec::new();
    for h in &head_top {
        if let Some(b) = base_by_path.get(h.path.as_str()) {
            let delta = h.hotspot_score - b.hotspot_score;
            if delta >= score_threshold {
                score_increased.push(ScoreDelta {
                    path: h.path.clone(),
                    base_score: b.hotspot_score,
                    head_score: h.hotspot_score,
                    delta,
                });
            }
        }
    }

    // PR-touched existing: files the PR modified that were already top-N at base.
    let pr_touched_existing: Vec<HotspotRow> = base
        .iter()
        .take(top_n)
        .filter(|h| pr_files.contains(&h.path))
        .cloned()
        .collect();

    HotspotsDelta {
        rank_entrants,
        score_increased,
        pr_touched_existing,
    }
}

fn compute_clones_delta(
    base_clones: &[ClonesRow],
    head_clones: &[ClonesRow],
    pr_files: &std::collections::HashSet<String>,
) -> ClonesDelta {
    use std::collections::HashSet;
    let base_fps: HashSet<&str> = base_clones.iter().map(|c| c.fingerprint.as_str()).collect();
    let new_families: Vec<ClonesRow> = head_clones
        .iter()
        .filter(|c| !base_fps.contains(c.fingerprint.as_str()))
        .cloned()
        .collect();
    let pr_touched_existing: Vec<ClonesRow> = head_clones
        .iter()
        .filter(|c| base_fps.contains(c.fingerprint.as_str()) && pr_files.contains(&c.entity))
        .cloned()
        .collect();
    ClonesDelta {
        new_families,
        pr_touched_existing,
    }
}

fn list_pr_files(
    repo: &Path,
    base_sha: &str,
    head_sha: &str,
) -> Result<std::collections::HashSet<String>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        // `core.quotepath=false` so non-ASCII paths come back as raw UTF-8.
        // Git's default octal-escapes and quotes them (e.g. `"caf\303\251.rs"`),
        // which then never match the raw-UTF-8 paths the analyses ingest —
        // silently dropping those files from PR detection, delta-health, and
        // coupling-absence classification. Mirrors `GitCliRepo::run_git`.
        .args(["-c", "core.quotepath=false"])
        .args(["diff", "--name-only", &format!("{base_sha}..{head_sha}")])
        .output()
        .context("git diff --name-only")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// Fingerprint of the `DiffArgs` fields that shape a base-rev `RevAnalyses`,
/// plus the binary version, cache epoch, and fact-schema version. Only
/// `min_revs` and `exclude` flow into the `Options` that `analyze_at_rev`
/// builds; everything else comes from `Options::default()`. The base tree's
/// own content is pinned by the cached `sha`, so it needs no representation
/// here. Any new `args.*` field folded into `analyze_at_rev`'s `Options` must
/// be added here too.
///
/// The version/epoch/schema trio mirrors what the main fact-store cache key
/// hashes (`cache.rs::cache_key`) — a `--base-cache` file is itself a cache,
/// and without these components a base written by an older binary, an older
/// `CACHE_EPOCH`, or an older fact schema would be silently reused by a
/// newer one that no longer agrees on what the cached values mean.
fn base_cache_opts_digest(min_revs: u32, exclude: &[String]) -> String {
    let mut exclude = exclude.to_vec();
    exclude.sort();
    // Separate patterns with NUL, which cannot appear in a path glob or CLI
    // argument, so a pattern that itself contains the separator (e.g. a brace
    // glob `src/{gen,vendor}/**`) can never collide with a different set.
    format!(
        "min_revs={min_revs}|exclude=[{}]|version={}|cache_epoch={}|schema={}",
        exclude.join("\0"),
        env!("CARGO_PKG_VERSION"),
        codelore_lib::cli_api::cache::CACHE_EPOCH,
        codelore_lib::cli_api::facts::schema::CURRENT_SCHEMA_VERSION,
    )
}

/// A base cache is reusable only when it was built at the same base SHA AND
/// under the same analysis-affecting options. Keyed on both so a cache built
/// under one option set is never served to a run with different options.
fn base_cache_is_fresh(cached: &RevAnalyses, base_sha: &str, expected_digest: &str) -> bool {
    cached.sha == base_sha && cached.opts_digest == expected_digest
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Cache directory cleanup runs only on directories older than this many
/// hours. The grace window avoids racing with a concurrent in-progress run
/// that has not yet finished writing into its tempdir.
const STALE_WORKTREE_AGE_HOURS: u64 = 24;

/// Best-effort cleanup of orphan worktrees from prior aborted runs.
///
/// Two passes (order matters):
///   1. Walk `$XDG_CACHE_HOME/codelore/diff-worktrees/` and remove any
///      subdirectory whose mtime is older than [`STALE_WORKTREE_AGE_HOURS`].
///   2. `git -C <repo> worktree prune` — clears the git-side registry of
///      entries pointing to the directories we just deleted (removes the
///      "already exists" failure mode on the next `git worktree add`).
///
/// Earlier code ran `git worktree prune` BEFORE the directory sweep, which
/// caused a one-run lag: directories deleted in this run's sweep didn't
/// have their `.git/worktrees/<name>/` administrative metadata cleaned up
/// until the next invocation of `codelore diff`. Single-shot users would
/// leave orphan metadata indefinitely.
///
/// All errors are logged at warn level — pruning must never fail the caller.
/// SIGKILL / OOM / disk-full aborts bypass [`Worktree::drop`] so orphans
/// accumulate over time; this is the recovery path.
fn prune_stale_worktrees(repo_root: &Path) {
    // 1. Sweep $XDG_CACHE_HOME/codelore/diff-worktrees/ for old directories.
    //
    // Route through `codelore_lib::cli_api::cache::default_cache_root()`
    // so the user-namespaced `/tmp` fallback applies here too. Earlier
    // versions hardcoded a bare `/tmp` which collided across users on
    // shared hosts and missed namespaced worktrees of the current user
    // (the namespacing fix in `add_worktree` was a no-op for cleanup as
    // long as the sweep stayed on bare `/tmp`).
    let cache_root = codelore_lib::cli_api::cache::default_cache_root()
        .join("codelore")
        .join("diff-worktrees");
    if cache_root.exists()
        && let Ok(cutoff) = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(
                STALE_WORKTREE_AGE_HOURS * 3600,
            ))
            .ok_or("subtraction underflow")
        && let Ok(entries) = std::fs::read_dir(&cache_root)
    {
        for entry in entries.filter_map(std::result::Result::ok) {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_dir() {
                continue;
            }
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if modified < cutoff {
                let path = entry.path();
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!("failed to remove stale worktree {}: {e}", path.display());
                } else {
                    tracing::info!("pruned stale worktree directory: {}", path.display());
                }
            }
        }
    }

    // 2. git worktree prune in the user's repo — idempotent, removes orphan
    //    registry entries that point to deleted directories. Running AFTER
    //    the directory sweep means dirs we just deleted have their
    //    administrative metadata cleaned up in this same invocation.
    let prune_result = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "prune"])
        .output();
    if let Err(e) = prune_result {
        tracing::warn!(
            "git worktree prune failed during startup cleanup: {e}; \
             continuing — `git worktree add` may report 'already exists'"
        );
    }
}

#[allow(clippy::too_many_lines)] // long but linear: worktree setup → two independent ingests → delta computation → gate evaluation; splitting would break the sequential worktree-lifecycle flow
pub fn run_diff(args: &DiffArgs) -> Result<(DiffOutput, FactsDb, Options)> {
    // Best-effort cleanup of orphans from prior aborted runs before we add
    // a new worktree. Idempotent; errors logged only.
    prune_stale_worktrees(&args.repo);

    let (base_sha, head_sha, merge_base_used) = parse_rev_range(&args.repo, &args.range)?;

    // Reject base == head early. Without this guard the downstream
    // pipeline cheerfully runs two identical analyses, computes a
    // zero-everywhere delta, and emits an empty SARIF / JSON / markdown
    // diff with no signal that the input was vacuous. Hot failure mode
    // when a `gh pr checkout` refresh leaves the local branch at the
    // base SHA: the CI gate trivially passes on what should obviously
    // be a configuration error.
    if base_sha == head_sha {
        anyhow::bail!(
            "base and head resolve to the same commit {base_sha} \
             (range {:?}); nothing to diff",
            args.range
        );
    }

    // Base analysis: load from --base-cache if present, otherwise compute + maybe cache.
    //
    // Validate `cached.sha == base_sha` before reusing the cache.
    // When `main` (or any base ref) advances, or when multiple PR branches
    // in a shared CI environment reuse the same cache path, a stale cache
    // would silently poison the delta computation — yielding incorrect
    // hotspot entrants, false coupling absences, and wrong clones delta —
    // without any warning. On mismatch: warn, recompute, overwrite.
    let base_analyses = if let Some(cache_path) = args.base_cache.as_ref() {
        let expected_digest = base_cache_opts_digest(args.min_revs, &args.exclude);
        match cache_path.exists().then(|| load_base_cache(cache_path)) {
            Some(Ok(cached)) if base_cache_is_fresh(&cached, &base_sha, &expected_digest) => {
                tracing::info!("loading base analysis from {}", cache_path.display());
                cached
            }
            Some(Ok(cached)) if cached.sha == base_sha => {
                tracing::warn!(
                    "base-cache options mismatch at {} (SHA matches {base_sha}, but the \
                     cache was built under different analysis options such as --min-revs \
                     or --exclude); discarding cache and recomputing base analysis",
                    cache_path.display(),
                );
                // Base rev only needs the analyses; drop the db + opts.
                let (a, _db, _opts) = analyze_at_rev(&args.repo, &base_sha, args, true)?;
                write_base_cache(cache_path, &a)?;
                a
            }
            Some(Ok(cached)) => {
                tracing::warn!(
                    "base-cache SHA mismatch at {} (cached={}, expected={}); \
                     discarding cache and recomputing base analysis",
                    cache_path.display(),
                    cached.sha,
                    base_sha
                );
                // Base rev only needs the analyses; drop the db + opts.
                let (a, _db, _opts) = analyze_at_rev(&args.repo, &base_sha, args, true)?;
                write_base_cache(cache_path, &a)?;
                a
            }
            Some(Err(e)) => {
                tracing::warn!(
                    "failed to read base-cache {}: {e:#}; recomputing base analysis",
                    cache_path.display()
                );
                let (a, _db, _opts) = analyze_at_rev(&args.repo, &base_sha, args, true)?;
                write_base_cache(cache_path, &a)?;
                a
            }
            None => {
                let (a, _db, _opts) = analyze_at_rev(&args.repo, &base_sha, args, true)?;
                write_base_cache(cache_path, &a)?;
                tracing::info!("wrote base analysis to {}", cache_path.display());
                a
            }
        }
    } else {
        let (a, _db, _opts) = analyze_at_rev(&args.repo, &base_sha, args, true)?;
        a
    };

    // Head rev keeps the db + opts for downstream evidence queries; its red-band
    // code-health pass is skipped (only the base rev's red band feeds delta-health).
    let (head_analyses, head_db, head_opts) = analyze_at_rev(&args.repo, &head_sha, args, false)?;

    let pr_files = list_pr_files(&args.repo, &base_sha, &head_sha)?;

    // Delta health: always computed (not gated behind thresholds) — the
    // section is standalone review signal. Guard against a base-cache
    // written before function metrics existed: empty base functions with
    // a non-empty base analysis would misread every head function as
    // "added" and poison the verdict.
    let delta_health = if base_analyses.functions.is_empty()
        && !base_analyses.hotspots.is_empty()
        && !head_analyses.functions.is_empty()
    {
        tracing::warn!(
            "base analysis has no function metrics (stale --base-cache?); \
             skipping delta-health — delete the cache file to recompute"
        );
        None
    } else {
        let clone_members: std::collections::HashSet<(String, String)> = head_analyses
            .clones
            .iter()
            .map(|c| (c.entity.clone(), c.function.clone()))
            .collect();
        let red: std::collections::HashSet<String> =
            base_analyses.red_files.iter().cloned().collect();
        Some(compute_delta_health(
            &base_analyses.functions,
            &head_analyses.functions,
            &pr_files,
            &clone_members,
            &red,
        ))
    };

    let want_hotspots = args.analysis.wants_hotspots();
    let want_coupling = args.analysis.wants_coupling();
    let want_clones = args.analysis.wants_clones();

    let hotspots = if want_hotspots {
        compute_hotspots_delta(
            &base_analyses.hotspots,
            &head_analyses.hotspots,
            &pr_files,
            args.top_n as usize,
            args.score_threshold,
        )
    } else {
        HotspotsDelta::default()
    };
    let coupling_absences = if want_coupling {
        compute_coupling_absences(
            &base_analyses.coupling,
            &pr_files,
            args.absence_min_shared,
            args.absence_fisher_p,
        )
    } else {
        Vec::new()
    };
    let clones = if want_clones {
        compute_clones_delta(&base_analyses.clones, &head_analyses.clones, &pr_files)
    } else {
        ClonesDelta::default()
    };

    // [diff] gate evaluation. Vacuous (zeroes + empty violations
    // list) when --thresholds-file is unset OR no [diff] section is
    // present in the file. The thresholds file is auto-discovered
    // when the flag is omitted — same auto-discovery the `check`
    // subcommand uses — so `codelore diff` in a repo with a
    // committed `.codelore-thresholds.toml` automatically gates on
    // its `[diff]` section without needing the flag every time.
    let thresholds_opt = if let Some(path) = args.thresholds_file.as_ref() {
        Some(
            codelore_lib::cli_api::quality_gates::Thresholds::from_path(path)
                .context("load thresholds file")?,
        )
    } else {
        let discovered = codelore_lib::cli_api::quality_gates::Thresholds::discover(&args.repo)
            .context("discover thresholds file")?;
        if discovered.is_empty() {
            None
        } else {
            Some(discovered)
        }
    };
    let (base_median_code_health, head_median_code_health, gate_violations, gate_skip_reason) =
        if let Some(t) = thresholds_opt.as_ref()
            && (t.diff.delta_code_health_min.is_some()
                || t.diff.new_hotspot_max.is_some()
                || t.diff.no_new_cycles
                || t.diff.delta_health_min.is_some()
                || t.diff.deny_degrading_verdict)
        {
            let base_med = median_code_health(&base_analyses.hotspots);
            let head_med = median_code_health(&head_analyses.hotspots);
            let delta = head_med - base_med;
            let new_hotspot_count = u32::try_from(hotspots.rank_entrants.len()).unwrap_or(u32::MAX);
            let violations: Vec<GateViolationOut> =
                codelore_lib::cli_api::quality_gates::evaluate_diff_gate(
                    t,
                    new_hotspot_count,
                    delta,
                    base_analyses.dependency_cycles,
                    head_analyses.dependency_cycles,
                    delta_health.as_ref().and_then(|d| d.ratio),
                    delta_health.as_ref().map(|d| d.verdict.as_str()),
                )
                .into_iter()
                .map(Into::into)
                .collect();
            // Neither revision measured any hotspot rows: a blind ingest (e.g.
            // a shallow checkout) zeroes every scalar evaluate_diff_gate saw —
            // new_hotspot_count, delta, both cycle counts, delta_health_ratio —
            // identically to a genuinely unchanged repository, which stays a
            // real "passed" because it carries real (non-empty) rows on both
            // sides. Disclose the skip rather than let an empty violations
            // list read as a silent pass.
            let verdict = codelore_lib::cli_api::quality_gates::diff_gate_verdict(
                !base_analyses.hotspots.is_empty(),
                !head_analyses.hotspots.is_empty(),
                violations.len(),
            );
            let skip_reason = (verdict == "skipped").then(|| {
                "no hotspot rows measured at either revision (blind ingest — check for a \
                 shallow checkout / fetch-depth truncation, not a genuinely unchanged range)"
                    .to_string()
            });
            (Some(base_med), Some(head_med), violations, skip_reason)
        } else {
            (None, None, Vec::new(), None)
        };

    Ok((
        DiffOutput {
            base_sha,
            head_sha,
            merge_base_used,
            hotspots,
            coupling_absences,
            clones,
            base_median_code_health,
            head_median_code_health,
            gate_violations,
            gate_skip_reason,
            delta_health,
        },
        head_db,
        head_opts,
    ))
}

/// Median of `code_health` across a hotspots row vector. Returns 0.0
/// for empty inputs — consistent with the "vacuous" branch in the
/// caller, where no data means no signal means no violation.
fn median_code_health(rows: &[HotspotRow]) -> f64 {
    if rows.is_empty() {
        return 0.0;
    }
    let mut healths: Vec<f64> = rows.iter().map(|r| r.cognitive_health).collect();
    healths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = healths.len() / 2;
    if healths.len() % 2 == 1 {
        healths[mid]
    } else {
        f64::midpoint(healths[mid - 1], healths[mid])
    }
}

/// Decide the process exit code based on `--fail-on` AND the
/// `[diff]` quality gate. Returns `true` if the process should exit
/// non-zero.
///
/// `[diff]` violations are an unconditional fail signal — they
/// override `--fail-on=none`. Rationale: a thresholds file is opt-in
/// (user explicitly configured a gate), so a violation is exactly
/// the case where "do nothing" is the wrong default. The `--fail-on`
/// knob continues to gate the OTHER signals (rank entrants, score
/// increase, etc.) as a separate axis.
pub fn should_fail(args: &DiffArgs, output: &DiffOutput) -> bool {
    use crate::args::DiffFailOn;
    if !output.gate_violations.is_empty() {
        return true;
    }
    match args.fail_on {
        DiffFailOn::None => false,
        DiffFailOn::RankEntrant => !output.hotspots.rank_entrants.is_empty(),
        DiffFailOn::ScoreIncrease => !output.hotspots.score_increased.is_empty(),
        DiffFailOn::Any => {
            !output.hotspots.rank_entrants.is_empty()
                || !output.hotspots.score_increased.is_empty()
                || !output.coupling_absences.is_empty()
                || !output.clones.new_families.is_empty()
        }
    }
}

#[cfg(test)]
mod prune_tests {
    use super::*;

    /// Build a tiny git repo with 2 commits for `parse_rev_range` tests.
    /// Returns the repo dir and the first/second commit SHAs.
    fn tiny_two_commit_repo() -> (tempfile::TempDir, String, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(path)
                    .args(args)
                    .status()
                    .expect("spawn git")
                    .success(),
                "git {args:?} failed"
            );
        };
        git(&["init", "-b", "main", "--quiet"]);
        git(&["config", "user.email", "x@x"]);
        git(&["config", "user.name", "X"]);
        std::fs::write(path.join("a.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "c1", "--quiet"]);
        let sha1 = git_rev_parse(path, "HEAD").unwrap();
        std::fs::write(path.join("a.txt"), "2\n").unwrap();
        git(&["commit", "-am", "c2", "--quiet"]);
        let sha2 = git_rev_parse(path, "HEAD").unwrap();
        (dir, sha1, sha2)
    }

    /// Omitted base side of a two-dot range defaults to HEAD
    /// (matching `git log ..main` / `git diff ..main` semantics).
    #[test]
    fn parse_rev_range_two_dot_omitted_base_defaults_to_head() {
        let (dir, sha1, sha2) = tiny_two_commit_repo();
        let (base, head, mb) = parse_rev_range(dir.path(), "..HEAD~1").unwrap();
        assert_eq!(base, sha2, "empty base should default to HEAD");
        assert_eq!(head, sha1, "head should be HEAD~1");
        assert!(!mb, "two-dot form should not flag merge-base");
    }

    /// Omitted head side of a two-dot range defaults to HEAD.
    #[test]
    fn parse_rev_range_two_dot_omitted_head_defaults_to_head() {
        let (dir, sha1, sha2) = tiny_two_commit_repo();
        let (base, head, mb) = parse_rev_range(dir.path(), "HEAD~1..").unwrap();
        assert_eq!(base, sha1);
        assert_eq!(head, sha2, "empty head should default to HEAD");
        assert!(!mb);
    }

    /// Omitted head side of a three-dot range defaults to HEAD.
    #[test]
    fn parse_rev_range_three_dot_omitted_head_defaults_to_head() {
        let (dir, _sha1, sha2) = tiny_two_commit_repo();
        let (_base, head, mb) = parse_rev_range(dir.path(), "HEAD~1...").unwrap();
        assert_eq!(head, sha2, "empty head should default to HEAD");
        assert!(mb, "three-dot form should flag merge-base");
    }

    /// `prune_stale_worktrees` is best-effort: must not panic when the
    /// repo isn't a real git repo (the `git worktree prune` call will
    /// fail with a non-zero exit; we log and continue).
    #[test]
    fn prune_does_not_panic_on_non_git_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        prune_stale_worktrees(tmp.path());
    }

    /// `prune_stale_worktrees` is best-effort: must not panic when the
    /// cache root is empty (the very first run).
    #[test]
    fn prune_is_noop_on_missing_or_empty_cache_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        prune_stale_worktrees(tmp.path()); // doesn't panic
    }

    /// When `git worktree add` fails (we point at a
    /// non-git path so the command errors out), `add_worktree` must NOT
    /// leak the temp directory it allocated. Earlier code called
    /// `tmp.keep()` BEFORE running git, which converted the `TempDir` to a
    /// plain owned path; a subsequent git failure then returned `Err`
    /// without cleaning up, leaving an empty directory under the cache
    /// root forever. The fix runs git first and calls `keep()` only on
    /// success.
    #[test]
    fn add_worktree_does_not_leak_tempdir_on_git_failure() {
        // Point at a real cache root so we can inspect it after the
        // call. The "repo" is a tempdir that is NOT a git repository —
        // `git worktree add` will fail loudly.
        let not_a_repo = tempfile::tempdir().expect("tempdir");
        let cache_root = codelore_lib::cli_api::cache::default_cache_root()
            .join("codelore")
            .join("diff-worktrees");
        // Snapshot the cache directory state BEFORE the call so we can
        // diff against it afterwards. (The dir might not exist yet.)
        let before: std::collections::HashSet<std::path::PathBuf> = std::fs::read_dir(&cache_root)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();

        let result = add_worktree(not_a_repo.path(), "deadbeef");
        assert!(result.is_err(), "add_worktree on a non-git path must error");

        // After the call, no NEW wt-* entries should exist in the cache
        // root. Anything that was there before is fine (could be from
        // concurrent test runs); we only assert about the delta.
        let after: std::collections::HashSet<std::path::PathBuf> = std::fs::read_dir(&cache_root)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        let new_entries: Vec<_> = after.difference(&before).collect();
        assert!(
            new_entries.is_empty(),
            "F6 regression: add_worktree leaked a tempdir on git failure: {new_entries:?}",
        );
    }

    /// `list_pr_files` must return non-ASCII paths as raw UTF-8, matching the
    /// paths the analyses ingest. Git's default `core.quotepath=true`
    /// octal-escapes and quotes them; without the `core.quotepath=false`
    /// override the returned set holds the quoted form and the file silently
    /// drops from PR detection. Uses a non-decomposable Cyrillic name — Latin
    /// accented letters like `é`/`ï` decompose to NFD under macOS APFS and
    /// would flake.
    #[test]
    fn list_pr_files_returns_non_ascii_paths_unquoted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(path)
                    .args(args)
                    .status()
                    .expect("spawn git")
                    .success(),
                "git {args:?} failed"
            );
        };
        git(&["init", "-b", "main", "--quiet"]);
        git(&["config", "user.email", "x@x"]);
        git(&["config", "user.name", "X"]);
        // Force the bug regardless of the tester's global git config.
        git(&["config", "core.quotepath", "true"]);
        std::fs::write(path.join("seed.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "seed", "--quiet"]);
        let base = git_rev_parse(path, "HEAD").unwrap();
        std::fs::write(path.join("файл.rs"), "fn main() {}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "add cyrillic", "--quiet"]);
        let head = git_rev_parse(path, "HEAD").unwrap();

        let files = list_pr_files(path, &base, &head).unwrap();
        assert!(
            files.contains("файл.rs"),
            "expected raw-UTF-8 path in returned set, got {files:?}"
        );
    }

    /// A different `--min-revs` shapes a different base cache, so the digest
    /// must fold it in.
    #[test]
    fn base_cache_opts_digest_folds_in_min_revs() {
        assert_ne!(
            base_cache_opts_digest(2, &[]),
            base_cache_opts_digest(10, &[])
        );
    }

    /// A different `--exclude` set shapes a different base cache, so the
    /// digest must fold it in.
    #[test]
    fn base_cache_opts_digest_folds_in_exclude() {
        assert_ne!(
            base_cache_opts_digest(2, &["src/gen/**".to_string()]),
            base_cache_opts_digest(2, &[])
        );
    }

    /// `--exclude` order does not change the resulting analysis, so the
    /// digest must be order-stable.
    #[test]
    fn base_cache_opts_digest_is_exclude_order_stable() {
        assert_eq!(
            base_cache_opts_digest(2, &["a".to_string(), "b".to_string()]),
            base_cache_opts_digest(2, &["b".to_string(), "a".to_string()])
        );
    }

    /// The consts folded in below are fixed at compile time, so there is no
    /// before/after run to diff against within a single test binary (unlike
    /// `min_revs`/`exclude`, which vary per call). Instead assert the digest
    /// contains each component — proving they are actually folded in, which
    /// is what makes a base cache written by an older binary/epoch/schema
    /// distinguishable from one written by this one.
    #[test]
    fn base_cache_opts_digest_incorporates_version_epoch_and_schema() {
        let digest = base_cache_opts_digest(2, &[]);
        assert!(
            digest.contains(env!("CARGO_PKG_VERSION")),
            "digest must incorporate the binary version: {digest}"
        );
        assert!(
            digest.contains(codelore_lib::cli_api::cache::CACHE_EPOCH),
            "digest must incorporate the cache epoch: {digest}"
        );
        assert!(
            digest.contains(codelore_lib::cli_api::facts::schema::CURRENT_SCHEMA_VERSION),
            "digest must incorporate the fact-schema version: {digest}"
        );
    }

    /// A cache built under one option digest is not fresh for a run with a
    /// different digest (even at the same SHA), and not fresh for a different
    /// SHA (even at the same digest).
    #[test]
    fn base_cache_not_fresh_when_opts_digest_differs() {
        let cached = RevAnalyses {
            sha: "abc123".to_string(),
            opts_digest: base_cache_opts_digest(2, &[]),
            ..Default::default()
        };
        assert!(!base_cache_is_fresh(
            &cached,
            "abc123",
            &base_cache_opts_digest(10, &[])
        ));
        assert!(base_cache_is_fresh(
            &cached,
            "abc123",
            &base_cache_opts_digest(2, &[])
        ));
        assert!(!base_cache_is_fresh(
            &cached,
            "def456",
            &base_cache_opts_digest(2, &[])
        ));
    }

    /// A base cache written before `opts_digest` existed deserialises with an
    /// empty digest and is treated as a mismatch — recomputed once, never
    /// served.
    #[test]
    fn legacy_base_cache_missing_opts_digest_deserialises_empty_and_is_not_served() {
        let parsed: RevAnalyses =
            serde_json::from_str(r#"{"sha":"abc","hotspots":[],"coupling":[],"clones":[]}"#)
                .expect("legacy base-cache JSON should deserialise");
        assert_eq!(parsed.opts_digest, "");
        assert!(!base_cache_is_fresh(
            &parsed,
            "abc",
            &base_cache_opts_digest(2, &[])
        ));
    }
}
