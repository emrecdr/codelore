//! `codelore calibrate` — build a corpus-calibration artifact from pinned repos.
//!
//! Each manifest repo is checked out at its pinned SHA in a throwaway location
//! (shallow fetch, full-clone fallback, or a local `git worktree`), ingested
//! HEAD-only, and pooled two ways: per-function raw complexity metrics per
//! language, and repo-level structural metrics from the resolved import graph.
//! The pooled observations are reduced to a calibration artifact written to
//! `--output`.

use anyhow::{Context, Result};
use codelore_lib::cli_api::Options;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::GixRepo;

use crate::args::CalibrateArgs;

/// Build a corpus-calibration artifact from a manifest of pinned repos.
///
/// For each `[[repos]]` entry the repo is checked out at its pinned SHA in a
/// throwaway location, ingested HEAD-only (the complexity facts and HEAD-time
/// imports the pooling below reads — no history walk), and pooled two ways:
/// per-function raw metrics (`cyclomatic`, `cognitive`, `sloc`, `nargs`,
/// `max_nesting`) per language (derived from each file's extension), and
/// repo-level structural metrics (`propagation_cost`, `cycle_file_share`)
/// derived from the resolved import graph. The per-language pools are reduced
/// to quantile breakpoints by the calibration builder; the repo-level pools
/// are attached as sorted raw-value vectors. A repo that fails to check out or
/// ingest is warned about and skipped; the run still succeeds and the
/// artifact's `repos_attempted` / `repos_included` counts record the tally.
/// With `--merge`, the build is folded into an existing artifact via the
/// library's weighted-blend merge (repo-level pools merge by exact
/// concatenation instead, since their raw values are available).
pub(crate) fn run_calibrate_cmd(args: &CalibrateArgs) -> Result<()> {
    use codelore_lib::calibration::{self, LangObservations};
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::quality_gates::ledger::now_utc_ts;

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);
    let generated_at = now_utc_ts();
    // Default vintage is `corpus-YYYY-MM`, sliced from the RFC 3339 timestamp
    // (`YYYY-MM-DDT…`) so we reuse the one timestamp helper rather than pull in
    // date formatting.
    let vintage = args
        .vintage
        .clone()
        .unwrap_or_else(|| format!("corpus-{}", &generated_at[..7]));

    let manifest = calibration::load_manifest(&args.repos).context("load corpus manifest")?;

    let mut obs = LangObservations::new();
    let mut pools = calibration::RepoMetrics::default();
    let mut attempted: u32 = 0;
    let mut included: u32 = 0;

    for repo in &manifest.repos {
        attempted += 1;
        // The per-repo progress line prints from inside `calibrate_one_repo`
        // once the checkout mode (shallow / full / worktree) is known.
        match calibrate_one_repo(&repo.source, &repo.sha, &cache_root, &mut obs, &mut pools) {
            Ok(()) => included += 1,
            Err(e) => eprintln!("calibrate: skip {} @ {}: {e:#}", repo.source, repo.sha),
        }
    }

    let mut artifact = calibration::build_from_observations(&vintage, &generated_at, &obs);
    artifact.repos_attempted = attempted;
    artifact.repos_included = included;
    calibration::attach_repo_metrics(&mut artifact, pools);

    if let Some(merge_path) = &args.merge {
        let base = calibration::load(merge_path)
            .with_context(|| format!("load --merge artifact {}", merge_path.display()))?;
        artifact = calibration::merge(base, artifact);
    }

    let json = serde_json::to_vec(&artifact).context("serialize calibration artifact")?;
    if let Some(parent) = args.output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output dir {}", parent.display()))?;
    }
    std::fs::write(&args.output, &json)
        .with_context(|| format!("write artifact {}", args.output.display()))?;

    eprintln!(
        "calibrate: {included}/{attempted} repo(s) → {} ({} language(s))",
        args.output.display(),
        artifact.languages.len(),
    );
    Ok(())
}

/// Check out one manifest repo at its pinned SHA, ingest it HEAD-only, and
/// pool its per-function raw metrics into `obs` plus its repo-level
/// architecture metrics into `pools`.
///
/// A `source` containing `://` or starting with `git@` is a clone URL; anything
/// else is a local filesystem path. Both converge on a throwaway checkout of
/// the pinned SHA — a depth-1 fetch (full clone fallback) tempdir for URLs, a
/// detached `git worktree` for local paths — so the user's own checkout is
/// never mutated and the tree matches the pin regardless of where HEAD points.
/// The ingest runs in head-only mode: only the pinned tree's complexity facts
/// and HEAD-time imports are extracted (`pool_complexity` /
/// `pool_repo_metrics` read nothing else), which is what makes history-less
/// shallow checkouts ingestible in the first place.
fn calibrate_one_repo(
    source: &str,
    sha: &str,
    cache_root: &std::path::Path,
    obs: &mut codelore_lib::calibration::LangObservations,
    pools: &mut codelore_lib::calibration::RepoMetrics,
) -> Result<()> {
    // The tempdir / worktree guard is held for the duration of the ingest.
    let (checkout, mode) = checkout_pinned(source, sha)?;
    eprintln!("calibrate: {source} @ {sha} ({})", mode.label());
    let repo = GixRepo::open(checkout.path()).context("open pinned checkout")?;
    let opts = Options {
        repo_path: checkout.path().to_path_buf(),
        head_only_ingest: true,
        ..Options::default()
    };
    let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, cache_root).context("ingest")?;
    pool_complexity(&db, obs).context("pool complexity metrics")?;
    pool_repo_metrics(&db, pools).context("pool repo-level architecture metrics")?;
    Ok(())
}

/// How a pinned tree was materialized. Feeds the per-repo progress line so
/// an operator can see which repos got the cheap path.
#[derive(Clone, Copy)]
enum CheckoutMode {
    /// Depth-1 fetch of exactly the pinned SHA — no history transferred.
    Shallow,
    /// Full clone fallback (the server refused the shallow SHA fetch).
    Full,
    /// Detached `git worktree` of a local repo.
    Worktree,
}

impl CheckoutMode {
    fn label(self) -> &'static str {
        match self {
            Self::Shallow => "shallow",
            Self::Full => "full",
            Self::Worktree => "worktree",
        }
    }
}

/// A checked-out pinned tree plus its cleanup guard. Dropping it removes the
/// clone tempdir or the git worktree.
enum PinnedCheckout {
    /// A fresh (shallow or full) clone in an owned tempdir; dropping the
    /// dir removes it.
    Clone(tempfile::TempDir),
    /// A detached worktree of a local repo; the guard removes it on drop.
    Worktree {
        origin: std::path::PathBuf,
        dir: tempfile::TempDir,
    },
}

impl PinnedCheckout {
    fn path(&self) -> &std::path::Path {
        match self {
            Self::Clone(dir) | Self::Worktree { dir, .. } => dir.path(),
        }
    }
}

impl Drop for PinnedCheckout {
    fn drop(&mut self) {
        // Only the worktree variant needs an explicit `git` cleanup so the
        // origin repo forgets the worktree registration; the clone variant is
        // just a tempdir. Detached stdio per the git-child-process invariant.
        if let Self::Worktree { origin, dir } = self
            && let Some(path) = dir.path().to_str()
        {
            let _ = std::process::Command::new("git")
                .args([
                    "-C",
                    origin.to_str().unwrap_or("."),
                    "worktree",
                    "remove",
                    "--force",
                    path,
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// Materialize `source` at `sha` into a throwaway location, reporting how
/// (shallow fetch / full clone / local worktree).
///
/// Every spawned `git` child runs with detached stdio (`stdin` nulled, stdout
/// nulled, stderr captured for diagnosis) — the same invariant the MCP server
/// documents for its git children.
fn checkout_pinned(source: &str, sha: &str) -> Result<(PinnedCheckout, CheckoutMode)> {
    let is_url = source.contains("://") || source.starts_with("git@");
    if is_url {
        // Shallow first: a depth-1 fetch of exactly the pinned SHA moves a
        // single tree instead of the whole history. Requires the server to
        // allow arbitrary-SHA fetches (`uploadpack.allowAnySHA1InWant` —
        // GitHub does); when any step fails, warn once and fall back to
        // the full clone + detached checkout.
        match shallow_pinned_clone(source, sha) {
            Ok(dir) => return Ok((PinnedCheckout::Clone(dir), CheckoutMode::Shallow)),
            Err(e) => tracing::warn!(
                "calibrate: shallow fetch failed for {source} ({e:#}); falling back to full clone"
            ),
        }
        let dir = tempfile::tempdir().context("create clone tempdir")?;
        run_git(
            &["clone", "--quiet", source, path_str(dir.path())?],
            "clone",
        )?;
        run_git(
            &[
                "-C",
                path_str(dir.path())?,
                "checkout",
                "--quiet",
                "--detach",
                sha,
            ],
            "checkout",
        )?;
        Ok((PinnedCheckout::Clone(dir), CheckoutMode::Full))
    } else {
        let origin = std::fs::canonicalize(source)
            .with_context(|| format!("resolve local repo path {source}"))?;
        // Confirm the pin is reachable before spending a worktree on it, so an
        // unknown SHA fails with a clear message rather than a git-internal one.
        run_git(
            &[
                "-C",
                path_str(&origin)?,
                "rev-parse",
                "--verify",
                "--quiet",
                sha,
            ],
            "rev-parse",
        )?;
        let dir = tempfile::tempdir().context("create worktree tempdir")?;
        run_git(
            &[
                "-C",
                path_str(&origin)?,
                "worktree",
                "add",
                "--detach",
                "--quiet",
                path_str(dir.path())?,
                sha,
            ],
            "worktree add",
        )?;
        Ok((
            PinnedCheckout::Worktree { origin, dir },
            CheckoutMode::Worktree,
        ))
    }
}

/// Depth-1 materialization of exactly `sha` from a remote `source`: empty
/// `git init`, `remote add origin`, `fetch --depth 1 origin <sha>`,
/// `checkout --detach FETCH_HEAD`. Any failing step surfaces as `Err` so
/// the caller can fall back to a full clone; the half-initialized tempdir
/// drops with the error. All git children go through `run_git` (detached
/// stdio).
fn shallow_pinned_clone(source: &str, sha: &str) -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir().context("create shallow clone tempdir")?;
    let dir_str = path_str(dir.path())?;
    run_git(&["init", "--quiet", dir_str], "init")?;
    run_git(
        &["-C", dir_str, "remote", "add", "origin", source],
        "remote add",
    )?;
    run_git(
        &[
            "-C", dir_str, "fetch", "--quiet", "--depth", "1", "origin", sha,
        ],
        "fetch --depth 1",
    )?;
    run_git(
        &[
            "-C",
            dir_str,
            "checkout",
            "--quiet",
            "--detach",
            "FETCH_HEAD",
        ],
        "checkout FETCH_HEAD",
    )?;
    Ok(dir)
}

/// Run a `git` subcommand with detached stdio, returning an error carrying
/// git's own stderr on failure.
fn run_git(git_args: &[&str], what: &str) -> Result<()> {
    let out = std::process::Command::new("git")
        .args(git_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .with_context(|| format!("spawn git {what}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "git {what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// `Path` → `&str`, erroring on non-UTF-8 rather than silently defaulting.
fn path_str(p: &std::path::Path) -> Result<&str> {
    p.to_str()
        .with_context(|| format!("non-UTF-8 path {}", p.display()))
}

/// Pool one repo's per-function raw metrics into `obs`, keyed by the language
/// derived from each file's extension. Rows for non-Tier-1 files are ignored.
/// Each metric is observed only when present (a NULL column is skipped) so a
/// language's pool reflects the functions that actually have that metric.
fn pool_complexity(
    db: &FactsDb,
    obs: &mut codelore_lib::calibration::LangObservations,
) -> Result<()> {
    use codelore_lib::complexity::Tier1Language;

    // (path, cyclomatic, cognitive, sloc, nargs, max_nesting). Each metric is
    // nullable in `complexity_metrics`, so read them as `Option<i64>` and pool
    // only the present values.
    type Row = (
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );

    let mut stmt = db
        .prepare(
            "SELECT path, cyclomatic, cognitive, sloc, nargs, max_nesting \
             FROM complexity_metrics",
        )
        .context("prepare complexity query")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })
        .context("run complexity query")?;

    for row in rows {
        let (path, cyclomatic, cognitive, sloc, nargs, max_nesting): Row =
            row.context("read complexity row")?;
        let Some(lang) = Tier1Language::from_path(&path) else {
            continue;
        };
        let lang = lang.as_str();
        for (metric, value) in [
            ("cyclomatic", cyclomatic),
            ("cognitive", cognitive),
            ("sloc", sloc),
            ("nargs", nargs),
            ("max_nesting", max_nesting),
        ] {
            if let Some(value) = value {
                obs.observe(lang, metric, value_to_f64(value));
            }
        }
    }
    Ok(())
}

/// Lossless `i64` → `f64` for the small, bounded per-function metric values
/// (complexity counts, SLOC, argument counts) — all far below `2^53`.
#[allow(clippy::cast_precision_loss)]
fn value_to_f64(n: i64) -> f64 {
    n as f64
}

/// Pool one repo's repo-level structural architecture metrics —
/// `propagation_cost` and `cycle_file_share` (the fraction of the resolved
/// import graph's files sitting in a non-trivial dependency cycle) — into
/// the corpus-level `pools`.
///
/// A repo whose resolved import graph is empty (no Tier-1 language present in
/// the checkout, or no resolvable HEAD-time imports) contributes NOTHING to
/// either metric — no observation is pushed at all. Pooling a synthetic zero
/// for such a repo would drag the corpus pool toward zero for repos that
/// simply carry no Tier-1 architecture signal, rather than a genuinely low
/// propagation cost.
fn pool_repo_metrics(
    db: &FactsDb,
    pools: &mut codelore_lib::calibration::RepoMetrics,
) -> Result<()> {
    use codelore_lib::cli_api::analyses::import_graph::{build_import_graph, graph_metrics};

    let graph = build_import_graph(db).context("build import graph")?;
    if graph.is_empty() {
        tracing::debug!(
            "calibrate: empty import graph (no Tier-1 imports); skipping repo-level metric pooling"
        );
        return Ok(());
    }
    let m = graph_metrics(&graph);
    // `m.n` is guaranteed non-zero here (the empty-graph case returned above),
    // so `.max(1)` only guards the cast helper's own contract, not a real
    // division-by-zero risk.
    let n = f64::from(u32::try_from(m.n.max(1)).unwrap_or(u32::MAX));
    let cycle_file_share = f64::from(m.cyclic_nodes) / n;

    pools
        .values
        .entry("propagation_cost".to_string())
        .or_default()
        .push(m.propagation_cost);
    pools
        .values
        .entry("cycle_file_share".to_string())
        .or_default()
        .push(cycle_file_share);
    Ok(())
}
