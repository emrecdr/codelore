//! Plan 8 §7 — `codelore diff <base>..<head>` PR-mode delta analysis.
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
use codelore_lib::Options;
use codelore_lib::analyses::clones::{ClonesRow, run_clones};
use codelore_lib::analyses::coupling::{CouplingRow, run_coupling};
use codelore_lib::analyses::hotspots::{HotspotRow, run_hotspots};
use codelore_lib::facts::FactsDb;
use codelore_lib::repo::GixRepo;
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

#[derive(Debug, Clone, Serialize)]
pub struct CouplingAbsence {
    /// File present in the PR's changed set.
    pub touched_file: String,
    /// Expected co-change partner NOT in the PR — historically the two
    /// always change together at Fisher-significant rates.
    pub expected_partner: String,
    /// `degree_pct` (0-100) from the base analysis.
    pub historical_coupling: f64,
    pub fisher_p: f64,
    /// `shared_revs` from base — strength of the historical signal.
    pub historical_shared_revs: u32,
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
    // Routed through `codelore_lib::cache::default_cache_root()` so the
    // user-namespaced `/tmp` fallback is applied here too — earlier
    // versions hardcoded a bare `/tmp` which collided across users on
    // shared hosts when `dirs::cache_dir()` returned `None`.
    let cache_root = codelore_lib::cache::default_cache_root()
        .join("codelore")
        .join("diff-worktrees");
    std::fs::create_dir_all(&cache_root)?;
    let tmp = tempfile::Builder::new()
        .prefix(&format!("wt-{}-", &sha[..8.min(sha.len())]))
        .tempdir_in(&cache_root)?;
    let path = tmp.path().to_path_buf();
    // F6 fix: run `git worktree add` FIRST, then `tmp.keep()` only on
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
fn analyze_at_rev(repo: &Path, sha: &str, args: &DiffArgs) -> Result<RevAnalyses> {
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

    Ok(RevAnalyses {
        sha: sha.to_string(),
        hotspots,
        coupling,
        clones,
    })
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

fn compute_coupling_absences(
    base_coupling: &[CouplingRow],
    pr_files: &std::collections::HashSet<String>,
    min_shared: u32,
    fisher_p_gate: f64,
) -> Vec<CouplingAbsence> {
    // Strong historical signal only: shared >= --absence-min-shared
    // (default 5 per research brief mitigation 3) AND Fisher-significant
    // at --absence-fisher-p (default 0.05).
    base_coupling
        .iter()
        .filter(|c| c.shared >= min_shared && c.fisher_p < fisher_p_gate)
        .filter_map(|c| {
            let a_in = pr_files.contains(&c.entity_a);
            let b_in = pr_files.contains(&c.entity_b);
            if a_in && !b_in {
                Some(CouplingAbsence {
                    touched_file: c.entity_a.clone(),
                    expected_partner: c.entity_b.clone(),
                    historical_coupling: c.degree,
                    fisher_p: c.fisher_p,
                    historical_shared_revs: c.shared,
                })
            } else if b_in && !a_in {
                Some(CouplingAbsence {
                    touched_file: c.entity_b.clone(),
                    expected_partner: c.entity_a.clone(),
                    historical_coupling: c.degree,
                    fisher_p: c.fisher_p,
                    historical_shared_revs: c.shared,
                })
            } else {
                None
            }
        })
        .collect()
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

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Cache directory cleanup runs only on directories older than this many
/// hours. The grace window avoids racing with a concurrent in-progress run
/// that has not yet finished writing into its tempdir.
const STALE_WORKTREE_AGE_HOURS: u64 = 24;

/// Best-effort cleanup of orphan worktrees from prior aborted runs.
///
/// Two passes (order matters — F28 fix):
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
    // F4 fix: route through `codelore_lib::cache::default_cache_root()`
    // so the user-namespaced `/tmp` fallback applies here too. Earlier
    // versions hardcoded a bare `/tmp` which collided across users on
    // shared hosts and missed namespaced worktrees of the current user
    // (the namespacing fix in `add_worktree` was a no-op for cleanup as
    // long as the sweep stayed on bare `/tmp`).
    let cache_root = codelore_lib::cache::default_cache_root()
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

pub fn run_diff(args: &DiffArgs) -> Result<DiffOutput> {
    // Best-effort cleanup of orphans from prior aborted runs before we add
    // a new worktree. Idempotent; errors logged only.
    prune_stale_worktrees(&args.repo);

    let (base_sha, head_sha, merge_base_used) = parse_rev_range(&args.repo, &args.range)?;

    // Base analysis: load from --base-cache if present, otherwise compute + maybe cache.
    let base_analyses = if let Some(cache_path) = args.base_cache.as_ref() {
        if cache_path.exists() {
            tracing::info!("loading base analysis from {}", cache_path.display());
            load_base_cache(cache_path)?
        } else {
            let a = analyze_at_rev(&args.repo, &base_sha, args)?;
            write_base_cache(cache_path, &a)?;
            tracing::info!("wrote base analysis to {}", cache_path.display());
            a
        }
    } else {
        analyze_at_rev(&args.repo, &base_sha, args)?
    };

    let head_analyses = analyze_at_rev(&args.repo, &head_sha, args)?;

    let pr_files = list_pr_files(&args.repo, &base_sha, &head_sha)?;

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

    Ok(DiffOutput {
        base_sha,
        head_sha,
        merge_base_used,
        hotspots,
        coupling_absences,
        clones,
    })
}

/// Decide the process exit code based on `--fail-on`. Returns `true` if the
/// process should exit non-zero. Exhaustive match on the typed enum — no
/// silent fall-through on unknown values (clap validates at parse time).
pub fn should_fail(args: &DiffArgs, output: &DiffOutput) -> bool {
    use crate::args::DiffFailOn;
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

    /// F26: omitted base side of a two-dot range defaults to HEAD
    /// (matching `git log ..main` / `git diff ..main` semantics).
    #[test]
    fn parse_rev_range_two_dot_omitted_base_defaults_to_head() {
        let (dir, sha1, sha2) = tiny_two_commit_repo();
        let (base, head, mb) = parse_rev_range(dir.path(), "..HEAD~1").unwrap();
        assert_eq!(base, sha2, "empty base should default to HEAD");
        assert_eq!(head, sha1, "head should be HEAD~1");
        assert!(!mb, "two-dot form should not flag merge-base");
    }

    /// F26: omitted head side of a two-dot range defaults to HEAD.
    #[test]
    fn parse_rev_range_two_dot_omitted_head_defaults_to_head() {
        let (dir, sha1, sha2) = tiny_two_commit_repo();
        let (base, head, mb) = parse_rev_range(dir.path(), "HEAD~1..").unwrap();
        assert_eq!(base, sha1);
        assert_eq!(head, sha2, "empty head should default to HEAD");
        assert!(!mb);
    }

    /// F26: omitted head side of a three-dot range defaults to HEAD.
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

    /// F6 regression: when `git worktree add` fails (we point at a
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
        let cache_root = codelore_lib::cache::default_cache_root()
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
}
