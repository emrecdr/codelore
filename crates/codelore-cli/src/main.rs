//! codelore — Behavioral Code Analyzer CLI.

mod args;
mod diff;
mod diff_output;
mod mcp;

use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::{GixRepo, Repo as _};
use codelore_lib::cli_api::{AnalysisName, CodeLoreError, Options};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::args::{
    AnalyzeArgs, CalibrateArgs, CalibrateDefectsArgs, CheckFormat, Cli, Command, DiffArgs,
    GateFormat, IngestSarifArgs, McpArgs,
};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        // Map CodeLoreError to its spec §6.6 exit code if present in the chain.
        // Falls back to 1 for non-CodeLoreError errors (e.g. clap parse errors).
        let code = e
            .chain()
            .find_map(|cause| cause.downcast_ref::<codelore_lib::cli_api::CodeLoreError>())
            .map_or(1, codelore_lib::cli_api::CodeLoreError::exit_code);
        std::process::exit(code);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Analyze(args) => analyze(&args, cli.no_banner),
        Command::Diff(args) => run_diff_cmd(&args),
        Command::Completions(args) => {
            run_completions_cmd(&args);
            Ok(())
        }
        Command::Explain(args) => run_explain_cmd(&args),
        Command::Schema(args) => run_schema_cmd(&args),
        Command::Profile => run_profile_cmd(),
        Command::Docs => run_docs_cmd(),
        Command::Check(args) => run_check_cmd(&args),
        Command::Gate(args) => run_gate_cmd(&args),
        Command::Mcp(args) => run_mcp_cmd(&args),
        Command::IngestSarif(args) => run_ingest_sarif_cmd(&args),
        Command::Calibrate(args) => run_calibrate_cmd(&args),
        Command::CalibrateDefects(args) => run_calibrate_defects_cmd(&args),
    }
}

fn run_mcp_cmd(args: &McpArgs) -> Result<()> {
    mcp::run_mcp_server(
        args.repo.clone(),
        args.defect_calibration.clone(),
        args.allow_foreign_calibration,
    )
}

/// Ingest one or more SARIF files into the per-repo external-findings sidecar.
///
/// For each file, parses all SARIF runs, groups findings by engine, and
/// calls `ExternalStore::replace_engine` per engine so re-ingest is
/// idempotent. Prints a summary line to stdout on success.
fn run_ingest_sarif_cmd(args: &IngestSarifArgs) -> Result<()> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::external::{
        ExternalStore, group_findings_by_engine, parse_sarif_with_engines,
    };

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    let store = ExternalStore::open_or_create(&cache_root, &args.repo)
        .context("open external findings store")?;

    // Parse all input files into a flat vec, then group by engine so that
    // two SARIF files from the same engine are combined rather than
    // overwriting each other. Track every engine name present across all
    // inputs — including runs that produced zero results — so a clean re-scan
    // clears the engine's stale rows instead of leaving them behind.
    let mut all_findings = Vec::new();
    let mut all_engines: Vec<String> = Vec::new();
    for path in &args.file {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read SARIF file {}", path.display()))?;
        let (findings, engines) = parse_sarif_with_engines(&raw)
            .with_context(|| format!("parse SARIF file {}", path.display()))?;
        all_findings.extend(findings);
        for engine in engines {
            if !all_engines.contains(&engine) {
                all_engines.push(engine);
            }
        }
    }
    let mut by_engine = group_findings_by_engine(all_findings);
    // Seed empty batches for engines that ran but flagged nothing this pass.
    // `replace_engine` with an empty slice deletes that engine's prior rows,
    // keeping the stored count aligned with the current scanner run.
    for engine in all_engines {
        by_engine.entry(engine).or_default();
    }

    let engine_count = by_engine.len();
    let mut total_ingested: usize = 0;
    for (engine, findings) in &by_engine {
        let n = store
            .replace_engine(engine, findings)
            .with_context(|| format!("ingest findings for engine {engine}"))?;
        total_ingested += n;
    }

    println!(
        "ingested {} finding(s) from {} engine(s) → {}",
        total_ingested,
        engine_count,
        store.path().display(),
    );

    Ok(())
}

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
fn run_calibrate_cmd(args: &CalibrateArgs) -> Result<()> {
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

/// Mine a repository's own fix-commit history (AG-SZZ), validate whether
/// `code-health` predicted where the mined defects landed, and — when the
/// evidence clears an honesty floor — tune the eight smell weights to this
/// repository. Writes a `defects.calib.json` artifact.
///
/// Flow: open the repo → build a dedicated MINING `FactsDb` (in-memory, full
/// history, `include_merges = true` so `commit_parents` covers every commit)
/// → classify fix commits with `DefectOracle` → `link_defects` with the
/// CLI-side `GitBlameOrigin` → `band_history` → the HEAD code-health scan
/// with `capture_intensities` read immediately after (the two share the
/// `code_health_biomarkers_v1` temp table — nothing may run between them) →
/// `validate` → `tune_weights` → assemble and save the artifact.
///
/// Progress is reported to stderr per phase, mirroring `calibrate`'s idiom.
fn run_calibrate_defects_cmd(args: &CalibrateDefectsArgs) -> Result<()> {
    use codelore_lib::cli_api::analyses::code_health::run_code_health;
    use codelore_lib::cli_api::quality_gates::ledger::now_utc_ts;
    use codelore_lib::defect_calibration::validate::{
        band_history, capture_intensities, default_weights, tune_weights, validate,
    };
    use codelore_lib::defect_calibration::{self, DefectArtifact, OracleConfig};

    let repo =
        GixRepo::open(&args.repo).with_context(|| format!("open repo {}", args.repo.display()))?;
    ensure_mining_tree_clean(&repo, args.allow_dirty)?;

    eprintln!(
        "calibrate-defects: mining full history of {}",
        args.repo.display()
    );
    let opts = Options {
        repo_path: args.repo.clone(),
        include_merges: true,
        ..Options::default()
    };
    let db = FactsDb::new_in_memory().context("open in-memory mining fact store")?;
    db.ingest(&repo, &opts).context("ingest full history")?;

    let oracle_cfg = OracleConfig::default();
    let (links, mining_stats, commit_dates) = mine_fix_links(&db, &repo, args, &oracle_cfg)?;

    eprintln!("calibrate-defects: scanning historical code-health bands");
    let bands = band_history(&db, &repo, &opts).context("historical band scan")?;

    eprintln!("calibrate-defects: scanning HEAD code-health");
    let head_health = run_code_health(&db, &opts).context("HEAD code-health scan")?;
    // MUST run immediately after the HEAD scan above, before any other
    // analysis call touches `code_health_biomarkers_v1` — see the module
    // doc comment on `capture_intensities`.
    let intensities = capture_intensities(&db).context("capture biomarker intensities")?;

    eprintln!("calibrate-defects: validating structural risk against mined defects");
    let validation = validate(&links, &commit_dates, &bands, &head_health);

    eprintln!("calibrate-defects: tuning smell weights");
    let (train, validation_split) =
        build_train_validation_split(&links, &commit_dates, &intensities);
    let defaults = default_weights();
    let (weights, tuning) = tune_weights(&intensities, &train, &validation_split, &defaults);

    let generated_at = now_utc_ts();
    let vintage = args
        .vintage
        .clone()
        .unwrap_or_else(|| format!("defects-{}", &generated_at[..10]));
    let artifact = DefectArtifact {
        format_version: defect_calibration::DEFECT_FORMAT_VERSION,
        repo_identity: defect_calibration::repo_identity(&args.repo),
        head_at_mining: repo.head_sha().context("resolve HEAD sha")?,
        vintage,
        generated_at,
        oracle: oracle_cfg,
        mining: mining_stats,
        validation,
        weights,
        tuning,
    };
    defect_calibration::save(&artifact, &args.output)
        .with_context(|| format!("write artifact {}", args.output.display()))?;

    eprintln!(
        "calibrate-defects: wrote {} (vintage {})",
        args.output.display(),
        artifact.vintage,
    );
    Ok(())
}

/// Refuse to mine from a dirty working tree unless `allow_dirty` opts in.
/// Mining reads only committed state (git history + object-database blobs at
/// HEAD), so uncommitted edits are invisible to it — the artifact describes
/// the commit stamped as `head_at_mining`, not the tree the user is looking
/// at. Surfacing that mismatch loudly (instead of silently mining something
/// other than what is on disk) is the point of this guard.
fn ensure_mining_tree_clean(repo: &GixRepo, allow_dirty: bool) -> Result<()> {
    if !repo.is_worktree_dirty() {
        return Ok(());
    }
    if allow_dirty {
        eprintln!(
            "calibrate-defects: warning: working tree has uncommitted changes; \
             mining reads only committed state, so the artifact describes HEAD, \
             not your uncommitted edits (--allow-dirty set, continuing)"
        );
        return Ok(());
    }
    Err(CodeLoreError::InvalidOptions(
        "working tree has uncommitted changes; mining reads only committed state, \
         so the artifact would describe HEAD rather than your current edits — \
         commit them or pass --allow-dirty to proceed"
            .to_string(),
    )
    .into())
}

/// `(links, mining_stats, commit_dates)` — see [`mine_fix_links`].
type MinedLinks = (
    Vec<codelore_lib::defect_calibration::szz::SzzLink>,
    codelore_lib::defect_calibration::MiningStats,
    std::collections::HashMap<String, String>,
);

/// The oracle + AG-SZZ mining phase: classify fix commits, then trace their
/// deleted pre-image lines back to the commits that introduced them via
/// [`GitBlameOrigin`]. Returns the surviving links, the mining tallies, and
/// the full rev→date map (`link_defects`' clock-skew guard input, reused by
/// `validate`'s band lookups and the temporal train/validation split).
fn mine_fix_links(
    db: &FactsDb,
    repo: &GixRepo,
    args: &CalibrateDefectsArgs,
    oracle_cfg: &codelore_lib::defect_calibration::OracleConfig,
) -> Result<MinedLinks> {
    use codelore_lib::defect_calibration::DefectOracle;
    use codelore_lib::defect_calibration::szz::link_defects;

    eprintln!("calibrate-defects: classifying fix commits");
    let oracle = DefectOracle::new(oracle_cfg).context("build defect oracle")?;
    let window_cutoff = match args.window_days {
        Some(0) => {
            return Err(
                CodeLoreError::InvalidOptions("--window-days must be > 0".to_string()).into(),
            );
        }
        Some(days) => window_cutoff_date(db, days).context("compute window cutoff")?,
        None => None,
    };
    let (fixes, commit_dates, root_fixes_skipped, window_excluded) =
        collect_fixes(db, &oracle, window_cutoff.as_deref()).context("collect fix commits")?;
    eprintln!(
        "calibrate-defects: {} fix commit(s) found ({root_fixes_skipped} root-commit fix(es) \
         skipped — no parent to blame; {window_excluded} outside --window-days excluded)",
        fixes.len(),
    );

    eprintln!("calibrate-defects: linking defects (AG-SZZ)");
    let origin = GitBlameOrigin {
        repo_path: args.repo.clone(),
    };
    let (links, mining_stats) = link_defects(db, repo, &origin, &fixes, &commit_dates)
        .context("link defects to their introducing commits")?;
    eprintln!(
        "calibrate-defects: {} link(s) found ({} file(s) blamed, {} cosmetic line(s) \
         dropped, {} blame failure(s))",
        mining_stats.links_found,
        mining_stats.files_blamed,
        mining_stats.lines_dropped_cosmetic,
        mining_stats.blame_failures,
    );
    Ok((links, mining_stats, commit_dates))
}

/// The `commits.date - INTERVAL '<days> days'` cutoff for `--window-days`,
/// as `CAST(... AS TEXT)` — zero-padded UTC, lexicographically comparable
/// against the same-format dates [`collect_fixes`] reads. `None` when the
/// mining store has no commits at all (empty repo — `MAX(date)` is `NULL`).
fn window_cutoff_date(db: &FactsDb, days: u32) -> Result<Option<String>> {
    let sql =
        format!("SELECT CAST((SELECT MAX(date) FROM commits) - INTERVAL '{days} days' AS TEXT)");
    db.query_row(&sql, [], |r| r.get::<_, Option<String>>(0))
        .context("query window cutoff")
}

/// `(fixes, commit_dates, root_fixes_skipped, window_excluded_fixes)` —
/// see [`collect_fixes`].
type FixCollection = (
    Vec<(String, String, String)>,
    std::collections::HashMap<String, String>,
    u32,
    u32,
);

/// Collect every commit's date (for the clock-skew guard + band lookups)
/// plus the `(rev, first_parent, date)` triples for fix commits the oracle
/// classifies, restricted to `window_cutoff` when set (only which FIXES are
/// mined is narrowed — a candidate defect-introducing commit may predate the
/// window freely). A classified fix with no first parent (a root commit) has
/// nothing to blame against and is skipped, tallied in the returned count.
fn collect_fixes(
    db: &FactsDb,
    oracle: &codelore_lib::defect_calibration::DefectOracle,
    window_cutoff: Option<&str>,
) -> Result<FixCollection> {
    use std::collections::HashMap;

    let commit_rows: Vec<(String, String, String, bool)> = db
        .prepare("SELECT rev, CAST(date AS TEXT), message, is_merge FROM commits")
        .context("prepare commits query")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })
        .context("run commits query")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collect commits rows")?;

    let first_parents: HashMap<String, String> = db
        .prepare("SELECT rev, parent_rev FROM commit_parents WHERE position = 0")
        .context("prepare first-parent query")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .context("run first-parent query")?
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .context("collect first-parent rows")?;

    let mut commit_dates = HashMap::with_capacity(commit_rows.len());
    let mut fixes = Vec::new();
    let mut root_fixes_skipped = 0u32;
    let mut window_excluded = 0u32;
    for (rev, date, message, is_merge) in commit_rows {
        if !oracle.is_fix(&message, is_merge) {
            commit_dates.insert(rev, date);
            continue;
        }
        if let Some(cutoff) = window_cutoff
            && date.as_str() < cutoff
        {
            window_excluded += 1;
            commit_dates.insert(rev, date);
            continue;
        }
        match first_parents.get(&rev) {
            Some(parent) => fixes.push((rev.clone(), parent.clone(), date.clone())),
            None => root_fixes_skipped += 1,
        }
        commit_dates.insert(rev, date);
    }
    Ok((fixes, commit_dates, root_fixes_skipped, window_excluded))
}

/// `(train, validation)` — see [`build_train_validation_split`].
type TrainValidationSplit = (Vec<(String, bool)>, Vec<(String, bool)>);

/// Build the `(path, label)` train/validation split `tune_weights` expects.
///
/// Positive rows are one per [`SzzLink`](codelore_lib::defect_calibration::szz::SzzLink)
/// — deliberately NOT deduplicated by path, matching `tune_weights`'s own
/// honesty-floor semantics (it counts `(defect, file)` incidences, not
/// distinct files: a file hit by three separate defects contributes three
/// rows). They are ordered by the FIX commit's date — the spec's "temporal
/// split ... by fix date" — so the older 60% trains and the newer 40%
/// validates: a leakage guard asking "would tuning generalize to the NEXT
/// defect", not "does it fit today's".
///
/// This is deliberately independent of `ValidationMetrics::linked_defects`
/// (the distinct-defect-rev count `validate()` reports) — that field and
/// `tune_weights`'s internal floor count different things over different
/// inputs; feeding one where the other belongs would silently corrupt the
/// honesty floor.
///
/// Negative rows are every file the HEAD scan captured biomarker intensities
/// for that no link ever touched — deduplicated (a file's "never
/// implicated" status is one static fact, not a repeated incidence).
/// Negatives carry no fix date, so they are split in the same 60/40 ratio by
/// a deterministic path-sorted order instead, disjoint between the two
/// splits so no single file's intensity vector is memorized across both.
///
/// Returns `(train, validation)`.
fn build_train_validation_split(
    links: &[codelore_lib::defect_calibration::szz::SzzLink],
    commit_dates: &std::collections::HashMap<String, String>,
    intensities: &std::collections::HashMap<String, [f64; 8]>,
) -> TrainValidationSplit {
    let mut positives: Vec<(&str, &str)> = links
        .iter()
        .filter_map(|link| {
            commit_dates
                .get(&link.fix_rev)
                .map(|date| (date.as_str(), link.path.as_str()))
        })
        .collect();
    positives.sort_unstable();

    let implicated: std::collections::HashSet<&str> =
        links.iter().map(|l| l.path.as_str()).collect();
    let mut negatives: Vec<&str> = intensities
        .keys()
        .map(String::as_str)
        .filter(|path| !implicated.contains(path))
        .collect();
    negatives.sort_unstable();

    let train_share = |n: usize| n * 60 / 100;
    let (pos_train, pos_val) = positives.split_at(train_share(positives.len()));
    let (neg_train, neg_val) = negatives.split_at(train_share(negatives.len()));

    let mut train: Vec<(String, bool)> = pos_train
        .iter()
        .map(|&(_, path)| (path.to_string(), true))
        .collect();
    train.extend(neg_train.iter().map(|&path| (path.to_string(), false)));

    let mut validation: Vec<(String, bool)> = pos_val
        .iter()
        .map(|&(_, path)| (path.to_string(), true))
        .collect();
    validation.extend(neg_val.iter().map(|&path| (path.to_string(), false)));

    (train, validation)
}

/// Production [`LineOriginSource`](codelore_lib::defect_calibration::szz::LineOriginSource):
/// shells `git blame -w --porcelain` once per `(rev, path)` pair requested by
/// the AG-SZZ engine, batching every requested line into that single
/// invocation via one `-L` range per contiguous run (multiple `-L` flags in
/// one `git blame` call, never one call per range). Detached stdio (`stdin`
/// nulled; `stdout`/`stderr` captured via `Command::output`'s own default
/// piping) — the same child-process invariant `calibrate`'s git children use.
struct GitBlameOrigin {
    repo_path: std::path::PathBuf,
}

impl codelore_lib::defect_calibration::szz::LineOriginSource for GitBlameOrigin {
    fn origins(
        &self,
        rev: &str,
        path: &str,
        lines: &[u32],
    ) -> codelore_lib::cli_api::Result<Vec<(u32, String)>> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let repo_str = self
            .repo_path
            .to_str()
            .ok_or_else(|| CodeLoreError::Analysis("non-UTF-8 repo path".to_string()))?;

        let mut cmd_args: Vec<String> = vec![
            "-C".to_string(),
            repo_str.to_string(),
            "blame".to_string(),
            "-w".to_string(),
            "--porcelain".to_string(),
        ];
        for (start, end) in merge_line_ranges(lines) {
            cmd_args.push("-L".to_string());
            cmd_args.push(format!("{start},{end}"));
        }
        cmd_args.push(rev.to_string());
        cmd_args.push("--".to_string());
        cmd_args.push(path.to_string());

        let out = std::process::Command::new("git")
            .args(&cmd_args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| {
                CodeLoreError::Analysis(format!("spawn git blame for {path}@{rev}: {e}"))
            })?;
        if !out.status.success() {
            return Err(CodeLoreError::Analysis(format!(
                "git blame failed for {path}@{rev}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        codelore_lib::defect_calibration::szz::parse_blame_porcelain(&stdout)
    }
}

/// Merge a set of (possibly duplicate, unsorted) 1-based line numbers into
/// the minimal set of contiguous inclusive ranges, sorted ascending —
/// [`GitBlameOrigin`]'s batching so every requested line is covered by ONE
/// `git blame` invocation regardless of how many separate hunks they came
/// from.
fn merge_line_ranges(lines: &[u32]) -> Vec<(u32, u32)> {
    let mut sorted: Vec<u32> = lines.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for line in sorted {
        match ranges.last_mut() {
            Some((_, end)) if line == *end + 1 => *end = line,
            _ => ranges.push((line, line)),
        }
    }
    ranges
}

/// Quality-gate check. Loads thresholds, runs the hotspots analysis
/// against the repo, evaluates each row against the gates, and
/// exits 0 (pass) or 1 (fail). Writes `result=pass|fail` to
/// `$GITHUB_OUTPUT` for direct GitHub Actions step-output
/// consumption.
#[allow(clippy::too_many_lines)]
fn run_check_cmd(args: &args::CheckArgs) -> Result<()> {
    use codelore_lib::cli_api::Options;
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::facts::FactsDb;
    use codelore_lib::cli_api::quality_gates::Thresholds;
    use codelore_lib::cli_api::quality_gates::ledger::{
        GateRunRecord, append_gate_runs, format_history, now_utc_ts, read_gate_runs,
    };
    use codelore_lib::cli_api::quality_gates::ratchet::{
        RatchetMetrics, RatchetOutcome, evaluate_ratchet, format_ratchet_outcome, read_snapshot,
        snapshot_from_metrics, write_snapshot,
    };
    use codelore_lib::cli_api::repo::GixRepo;

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    // --history: print ledger without running any gate evaluations.
    if args.history {
        let records = read_gate_runs(&cache_root, &args.repo).context("read gate-run ledger")?;
        print!("{}", format_history(&records, 20));
        return Ok(());
    }

    let thresholds = if let Some(path) = &args.thresholds_file {
        Thresholds::from_path(path).context("load thresholds file")?
    } else {
        Thresholds::discover(&args.repo).context("discover thresholds file")?
    };

    if thresholds.is_empty() && !args.ratchet {
        if !args.quiet {
            eprintln!(
                "codelore check: no thresholds configured (no `.codelore-thresholds.toml` at repo root); vacuously passing."
            );
        }
        write_github_output("result", "pass");
        // A vacuous pass under `--format sarif` must still emit a valid
        // zero-result SARIF document to stdout — the documented upload-sarif
        // pipeline (docs/advanced-usage.md §11.8) breaks if a run prints
        // nothing. Reuse the check emitter with an empty violation set.
        if matches!(args.format, CheckFormat::Sarif) {
            let repo = GixRepo::open(&args.repo).context("open repo")?;
            let head_sha = repo.head_sha().context("get HEAD sha")?;
            emit_check_sarif(
                &args.repo,
                &head_sha,
                &[],
                &std::collections::HashMap::new(),
            )?;
        }
        return Ok(());
    }

    // Mirrors `quality_gates::resolve_defect_calibration`, but reuses the
    // `thresholds` value already loaded above instead of re-discovering
    // (and re-parsing) the thresholds file.
    let resolved_defect_calibration = args.defect_calibration.clone().or_else(|| {
        thresholds.calibration.defect_artifact.clone().map(|p| {
            if p.is_absolute() {
                p
            } else {
                args.repo.join(p)
            }
        })
    });

    let opts = Options {
        repo_path: args.repo.clone(),
        calibration: args.calibration.clone(),
        defect_calibration: resolved_defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        temp_dir: args.temp_dir.clone(),
        ..Options::default()
    };
    opts.validate().context("validate options")?;
    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let head_sha = repo.head_sha().context("get HEAD sha")?;
    let db =
        FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root).context("ingest")?;
    let ts = now_utc_ts();

    // Open the external-findings sidecar without creating it, only when it
    // holds findings. `None` when absent OR present-but-empty — both mean "no
    // external findings to evaluate", and the gate is skipped gracefully inside
    // evaluate_all_gates.
    let external_store = if thresholds.gates.max_findings_in_hot_files.is_some() {
        codelore_lib::cli_api::external::ExternalStore::open_nonempty(&cache_root, &args.repo)
            .context("open external store")?
    } else {
        None
    };

    let (violations, mut ledger_records, hotspot_count, code_health) = evaluate_all_gates(
        &thresholds,
        &db,
        &opts,
        &head_sha,
        &ts,
        external_store.as_ref(),
    )
    .context("evaluate gates")?;

    // ── Per-gate notices (stderr) ────────────────────────────────────────────
    // Rendered from the ledger records the evaluator already produced, so the
    // compute layer prints nothing itself. Emitted before the ratchet/report
    // branches so both paths surface them; stderr keeps stdout a clean SARIF
    // document in --format sarif; suppressed under --quiet.
    if !args.quiet {
        emit_gate_notices(&ledger_records);
    }
    // One-per-run hint when the corpus lens is inactive — the check path always
    // computes code-health rows, which carry no corpus_percentile without an
    // artifact. Suppressed under --quiet (handled inside).
    notice_corpus_lens_absent(&opts, args.quiet);

    // ── Ratchet ───────────────────────────────────────────────────────────────
    if args.ratchet {
        // Build ratchet metrics from already-computed gate outputs.
        let worst_health = code_health
            .iter()
            .map(|r| r.score)
            .fold(f64::INFINITY, f64::min);
        // red_effort_pct: read from the effort-exposure ledger record if present.
        let red_effort_pct = ledger_records
            .iter()
            .find(|r| r.gate == "max_red_effort_pct")
            .map(|r| r.value);
        // dependency_cycles: read from the arch ledger record if present.
        let dep_cycles = ledger_records
            .iter()
            .find(|r| r.gate == "max_dependency_cycles")
            .map(|r| r.value);
        let metrics = RatchetMetrics {
            code_health_min_observed: if worst_health.is_infinite() {
                None
            } else {
                Some(worst_health)
            },
            red_effort_pct_observed: red_effort_pct,
            dependency_cycles_observed: dep_cycles,
        };

        match read_snapshot(&args.repo).context("read ratchet snapshot")? {
            None => {
                // First run: initialize.
                let snap = snapshot_from_metrics(&metrics);
                write_snapshot(&args.repo, &snap).context("write ratchet snapshot")?;
                let tracked: Vec<&str> = [
                    metrics
                        .code_health_min_observed
                        .map(|_| "code_health_min_observed"),
                    metrics
                        .red_effort_pct_observed
                        .map(|_| "red_effort_pct_observed"),
                    metrics
                        .dependency_cycles_observed
                        .map(|_| "dependency_cycles_observed"),
                ]
                .into_iter()
                .flatten()
                .collect();
                emit_ratchet_message(
                    args,
                    &format!(
                        "✅ ratchet initialized — tracking {} metric(s): {}. \
                         Configure max_red_effort_pct / max_dependency_cycles gates to ratchet \
                         effort and cycles. Commit `.codelore-ratchet.toml` to enable regression detection.\n",
                        tracked.len(),
                        if tracked.is_empty() {
                            "(none)".to_owned()
                        } else {
                            tracked.join(", ")
                        },
                    ),
                );
                ledger_records.push(GateRunRecord {
                    ts: ts.clone(),
                    head_sha: head_sha.clone(),
                    gate: "ratchet".into(),
                    threshold: 0.0,
                    value: 0.0,
                    verdict: "initialized".into(),
                    mode: "ratchet".into(),
                });
                append_gate_runs(&cache_root, &args.repo, &ledger_records);
                // `--format sarif` must still yield a valid document on the
                // ratchet exit paths, exactly like the non-ratchet path — the
                // gates already ran, so emit their violations before returning.
                emit_check_sarif_when_requested(args, &db, &opts, &head_sha, &violations)?;
                return Ok(());
            }
            Some(snap) => {
                let outcome = evaluate_ratchet(&snap, &metrics);
                emit_ratchet_message(args, &format_ratchet_outcome(&outcome));
                let (verdict, ratchet_failed) = match &outcome {
                    RatchetOutcome::Improved { .. } => ("improved", false),
                    RatchetOutcome::Regressed { .. } => ("regressed", true),
                };
                ledger_records.push(GateRunRecord {
                    ts: ts.clone(),
                    head_sha: head_sha.clone(),
                    gate: "ratchet".into(),
                    threshold: 0.0,
                    value: 0.0,
                    verdict: verdict.into(),
                    mode: "ratchet".into(),
                });
                append_gate_runs(&cache_root, &args.repo, &ledger_records);
                // Emit the standard check SARIF on both ratchet outcomes before
                // returning/bailing, so `--format sarif` always yields a valid
                // document. Exit code is unchanged: a regression still bails.
                emit_check_sarif_when_requested(args, &db, &opts, &head_sha, &violations)?;
                if ratchet_failed {
                    anyhow::bail!("ratchet: regression detected — see above");
                }
                // Tighten: rewrite snapshot with improved values.
                let tightened = snapshot_from_metrics(&metrics);
                write_snapshot(&args.repo, &tightened).context("tighten ratchet snapshot")?;
                return Ok(());
            }
        }
    }

    // ── Ledger write (IO errors warn, never alter exit code) ─────────────────
    append_gate_runs(&cache_root, &args.repo, &ledger_records);

    // ── SARIF emission (when --format sarif) ─────────────────────────────────
    emit_check_sarif_when_requested(args, &db, &opts, &head_sha, &violations)?;

    // ── Report ────────────────────────────────────────────────────────────────
    let degraded_count = ledger_records
        .iter()
        .filter(|r| r.verdict == "degraded")
        .count();

    if violations.is_empty() {
        if degraded_count > 0 {
            let warning = format!(
                "⚠ codelore check: WARNING — {degraded_count} gate(s) degraded (non-degraded gates pass)"
            );
            // SARIF mode keeps stdout a clean SARIF document, so the warning
            // goes to stderr; text mode prints it to stdout with the report.
            if matches!(args.format, CheckFormat::Sarif) {
                eprintln!("{warning}");
            } else {
                println!("{warning}");
            }
        } else if matches!(args.format, CheckFormat::Text) {
            println!("✅ codelore check: PASS ({hotspot_count} files evaluated)");
        }
        write_github_output("result", "pass");
        write_github_output("violations", "0");
        Ok(())
    } else {
        eprintln!(
            "❌ codelore check: FAIL — {} violation(s)",
            violations.len()
        );
        if !args.quiet && matches!(args.format, CheckFormat::Text) {
            for v in &violations {
                eprintln!(
                    "  - {gate}: {path} — actual {actual} vs threshold {threshold}",
                    gate = v.gate,
                    path = v.path,
                    actual = v.actual,
                    threshold = v.threshold,
                );
            }
        }
        write_github_output("result", "fail");
        write_github_output("violations", &violations.len().to_string());
        // Inside GitHub Actions, emit each violation as an inline `::error`
        // annotation so the failing gate shows up against the file in the
        // PR's Files-changed view — not just as a red check.
        if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && matches!(args.format, CheckFormat::Text)
        {
            let mut stdout = std::io::stdout();
            codelore_lib::cli_api::output::gha::write_gate_violations_gha(&violations, &mut stdout)
                .context("emit gate annotations")?;
        }
        // Plain anyhow::bail carries no CodeLoreError, so main()'s chain-walk
        // falls through to the default exit code 1. Gate failure exits 1 by
        // design; typed CodeLoreError variants are reserved for repo/output
        // failures (exit codes 3, 4, 5).
        anyhow::bail!("{} gate violation(s) — see above", violations.len());
    }
}

/// Emit the per-gate skip / degraded notices to stderr, derived from the
/// ledger records the evaluator produced. Keeping this out of the compute layer
/// means `evaluate_all_gates` stays print-free and the notice wording lives
/// beside the rest of `run_check_cmd`'s reporting.
fn emit_gate_notices(
    ledger_records: &[codelore_lib::cli_api::quality_gates::ledger::GateRunRecord],
) {
    for r in ledger_records {
        match (r.gate.as_str(), r.verdict.as_str()) {
            ("max_findings_in_hot_files", "skipped") => eprintln!(
                "  ⚠ max_findings_in_hot_files: skipped — run `codelore ingest-sarif` first"
            ),
            ("corpus_percentile_max", "skipped") => eprintln!(
                "  ⚠ corpus_percentile_max: skipped — no corpus percentile data (no calibration artifact active, or no analyzed file resolved a percentile)"
            ),
            ("code_health_min", "degraded") => eprintln!(
                "  ⚠ code_health_min: degraded — health scan returned no rows on a non-empty repo"
            ),
            _ => {}
        }
    }
}

/// Print the one-per-run notice that the corpus-percentile lens is inactive:
/// no `--calibration` artifact was passed and no world corpus is embedded, so
/// code-health rows carry no `corpus_percentile`. Called exactly once on each
/// code-health-producing path, so the "deduped per run" contract is structural.
/// Suppressed under `quiet` and when stderr is not a TTY (so redirected /
/// CI output stays clean), mirroring the pre-flight banner's print policy.
fn notice_corpus_lens_absent(opts: &Options, quiet: bool) {
    use std::io::IsTerminal as _;
    if quiet || !std::io::stderr().is_terminal() {
        return;
    }
    let embedded_absent = codelore_lib::cli_api::calibration::embedded_world().is_none();
    if opts.calibration.is_none() && embedded_absent {
        eprintln!(
            "note: corpus-percentile lens inactive — no calibration artifact (pass --calibration <path> or build one with `codelore calibrate`)."
        );
    }
}

/// Gate result bundle: violations + ledger records from one gate group.
type GateGroupResult = (
    Vec<codelore_lib::cli_api::quality_gates::GateViolation>,
    Vec<codelore_lib::cli_api::quality_gates::ledger::GateRunRecord>,
);

/// Build one ledger record for a simple scalar gate.
fn make_rec(
    gate: &str,
    threshold: f64,
    value: f64,
    failed: bool,
    ts: &str,
    head_sha: &str,
) -> codelore_lib::cli_api::quality_gates::ledger::GateRunRecord {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;
    GateRunRecord {
        ts: ts.to_owned(),
        head_sha: head_sha.to_owned(),
        gate: gate.to_owned(),
        threshold,
        value,
        verdict: if failed { "failed" } else { "passed" }.to_owned(),
        mode: "check".to_owned(),
    }
}

/// Evaluate hotspot-based gates (`cognitive_max`, `hotspot_score_max`).
/// Returns the gate result bundle and the hotspot rows (reused by
/// `run_finding_hotspot_overlap_with` to avoid a second hotspot query).
fn eval_hotspot_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    ts: &str,
    head_sha: &str,
) -> Result<(
    GateGroupResult,
    Vec<codelore_lib::cli_api::analyses::hotspots::HotspotRow>,
)> {
    use codelore_lib::cli_api::analyses::hotspots::run_hotspots;
    use codelore_lib::cli_api::quality_gates::evaluate_full_tree;
    let hotspots = run_hotspots(db, opts).context("run hotspots")?;
    let hs_violations = evaluate_full_tree(thresholds, &hotspots);
    let g = &thresholds.gates;
    let mut recs = Vec::new();
    if let Some(max) = g.cognitive_max {
        let failed = hs_violations.iter().any(|v| v.gate == "cognitive_max");
        let value = hotspots
            .iter()
            .map(|r| r.cognitive)
            .fold(f64::NAN, f64::max);
        recs.push(make_rec(
            "cognitive_max",
            max,
            if value.is_nan() { 0.0 } else { value },
            failed,
            ts,
            head_sha,
        ));
    }
    if let Some(max) = g.hotspot_score_max {
        let failed = hs_violations.iter().any(|v| v.gate == "hotspot_score_max");
        let value = hotspots
            .iter()
            .map(|r| r.hotspot_score)
            .fold(f64::NAN, f64::max);
        recs.push(make_rec(
            "hotspot_score_max",
            max,
            if value.is_nan() { 0.0 } else { value },
            failed,
            ts,
            head_sha,
        ));
    }
    Ok(((hs_violations, recs), hotspots))
}

/// Evaluate `code_health_min` gate with degraded-detection.
/// Returns the gate result bundle + the raw `CodeHealthRow` vec (reused by ratchet).
fn eval_code_health_gate(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    ts: &str,
    head_sha: &str,
) -> Result<(
    GateGroupResult,
    Vec<codelore_lib::cli_api::analyses::code_health::CodeHealthRow>,
)> {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;
    use codelore_lib::cli_api::quality_gates::{GateViolation, evaluate_code_health_gate};
    let code_health = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
        .context("run code-health")?;
    let g = &thresholds.gates;
    let Some(min) = g.code_health_min else {
        return Ok(((Vec::new(), Vec::new()), code_health));
    };
    let ch_violations = evaluate_code_health_gate(thresholds, &code_health);
    // Degraded: empty result when the repo has scorable files.
    let degraded = code_health.is_empty() && {
        db.query_row("SELECT COUNT(*) FROM complexity_metrics", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0)
            > 0
    };
    let worst = code_health
        .iter()
        .map(|r| r.score)
        .fold(f64::INFINITY, f64::min);
    let verdict = if degraded {
        "degraded"
    } else if ch_violations.is_empty() {
        "passed"
    } else {
        "failed"
    };
    let rec = GateRunRecord {
        ts: ts.to_owned(),
        head_sha: head_sha.to_owned(),
        gate: "code_health_min".into(),
        threshold: min,
        value: if worst.is_infinite() { 0.0 } else { worst },
        verdict: verdict.to_owned(),
        mode: "check".into(),
    };
    let mut violations = Vec::new();
    if degraded && g.fail_on_degraded {
        violations.push(GateViolation {
            gate: "code_health_min".into(),
            path: "(degraded)".into(),
            actual: "no-data".into(),
            threshold: format!("{min:.1}"),
        });
    } else {
        violations.extend(ch_violations);
    }
    Ok(((violations, vec![rec]), code_health))
}

/// Evaluate architecture gates (`max_dependency_cycles`, `max_propagation_cost`).
fn eval_arch_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    ts: &str,
    head_sha: &str,
) -> Result<GateGroupResult> {
    let (arch_v, measured) =
        codelore_lib::cli_api::quality_gates::evaluate_architecture_gate_measured(thresholds, db)
            .context("evaluate architecture gate")?;
    let g = &thresholds.gates;
    let mut recs = Vec::new();
    if let (Some(max), Some(m)) = (g.max_dependency_cycles, measured) {
        let failed = arch_v.iter().any(|v| v.gate == "max_dependency_cycles");
        recs.push(make_rec(
            "max_dependency_cycles",
            f64::from(max),
            f64::from(m.cycle_count),
            failed,
            ts,
            head_sha,
        ));
    }
    if let (Some(max), Some(m)) = (g.max_propagation_cost, measured) {
        let failed = arch_v.iter().any(|v| v.gate == "max_propagation_cost");
        recs.push(make_rec(
            "max_propagation_cost",
            max,
            m.propagation_cost,
            failed,
            ts,
            head_sha,
        ));
    }
    Ok((arch_v, recs))
}

/// Evaluate all configured gates and build ledger records for this run.
///
/// Returns `(violations, ledger_records, hotspot_count, code_health_rows)`.
/// `code_health_rows` is returned so callers (e.g. `--ratchet`) can extract
/// ratchet metrics without re-running the analysis.
///
/// `external_store` is the pre-opened sidecar for the
/// `max_findings_in_hot_files` gate. Pass `Some(store)` when the sidecar exists
/// and holds findings; `None` when absent or empty (gate skipped, no sidecar
/// created).
///
/// This is a pure compute layer: it records each gate's verdict in the returned
/// ledger records (including `"skipped"` and `"degraded"`) and prints nothing.
/// `run_check_cmd` renders the skip/degraded notices from those records.
#[allow(clippy::type_complexity, clippy::too_many_lines)]
fn evaluate_all_gates(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    head_sha: &str,
    ts: &str,
    external_store: Option<&codelore_lib::cli_api::external::ExternalStore>,
) -> Result<(
    Vec<codelore_lib::cli_api::quality_gates::GateViolation>,
    Vec<codelore_lib::cli_api::quality_gates::ledger::GateRunRecord>,
    usize,
    Vec<codelore_lib::cli_api::analyses::code_health::CodeHealthRow>,
)> {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;

    let mut violations = Vec::new();
    let mut recs = Vec::new();
    let g = &thresholds.gates;

    let ((hs_v, hs_r), hotspot_rows) = eval_hotspot_gates(thresholds, db, opts, ts, head_sha)?;
    let hotspot_count = hotspot_rows.len();
    violations.extend(hs_v);
    recs.extend(hs_r);

    let ((ch_v, ch_r), code_health) = eval_code_health_gate(thresholds, db, opts, ts, head_sha)?;
    violations.extend(ch_v);
    recs.extend(ch_r);

    if g.disallow_clone_type_1 {
        let clone_v = codelore_lib::cli_api::quality_gates::evaluate_clone_gate(thresholds, db)
            .context("evaluate clone gate")?;
        let count = clone_v
            .first()
            .and_then(|v| v.actual.parse::<f64>().ok())
            .unwrap_or(0.0);
        recs.push(make_rec(
            "disallow_clone_type_1",
            0.0,
            count,
            !clone_v.is_empty(),
            ts,
            head_sha,
        ));
        violations.extend(clone_v);
    }

    let (arch_v, arch_r) = eval_arch_gates(thresholds, db, ts, head_sha)?;
    violations.extend(arch_v);
    recs.extend(arch_r);

    if let Some(max) = g.max_red_effort_pct {
        // Reuse the code-health rows already computed for `code_health_min` —
        // effort-exposure's band table derives from the same HEAD scan, and
        // the measured red-band churn share must be recorded on passing runs
        // too (the ratchet and `--history` read it from the ledger).
        let rows =
            codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure_with_health(
                db,
                &opts.with_no_row_limit(),
                &code_health,
            )
            .context("run effort-exposure for gate")?;
        let value = rows
            .iter()
            .find(|r| r.band == "red")
            .map_or(0.0, |r| r.churn_share_pct);
        let effort_v =
            codelore_lib::cli_api::quality_gates::evaluate_effort_exposure_rows(max, &rows);
        recs.push(make_rec(
            "max_red_effort_pct",
            max,
            value,
            !effort_v.is_empty(),
            ts,
            head_sha,
        ));
        violations.extend(effort_v);
    }

    if let Some(min) = g.code_familiarity_min {
        let rows =
            codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                .context("run code-familiarity for gate")?;
        // Measured familiarity is recorded pass or fail; an empty row set
        // (no recognized source files) records 0.0 with a vacuous pass.
        // Unlike `code_health_min` this gate has no degraded sentinel: an
        // empty result IS the documented no-source-files contract, not a
        // scan failure.
        let value = rows.first().map_or(0.0, |r| r.familiarity_pct);
        let fam_v = codelore_lib::cli_api::quality_gates::evaluate_familiarity_rows(min, &rows);
        recs.push(make_rec(
            "code_familiarity_min",
            min,
            value,
            !fam_v.is_empty(),
            ts,
            head_sha,
        ));
        violations.extend(fam_v);
    }

    // ── max_findings_in_hot_files gate ───────────────────────────────────────
    if let Some(threshold) = g.max_findings_in_hot_files {
        // The gate is skipped (not failed) when the sidecar is absent OR present
        // but empty — both mean "no external findings to evaluate yet".
        // `external_store` already collapses those two states to `None` (via
        // `open_nonempty`), mirroring the MCP overlap tool. The `"skipped"`
        // verdict recorded here is what run_check_cmd renders the notice from.
        match external_store {
            None => {
                recs.push(GateRunRecord {
                    ts: ts.to_owned(),
                    head_sha: head_sha.to_owned(),
                    gate: "max_findings_in_hot_files".into(),
                    threshold: f64::from(threshold),
                    value: 0.0,
                    verdict: "skipped".into(),
                    mode: "check".into(),
                });
            }
            Some(store) => {
                // Reuse already-computed hotspot and code-health rows so we
                // don't re-run those analyses a second time (mirrors how
                // max_red_effort_pct reuses code_health above).
                let overlap_rows = codelore_lib::cli_api::analyses::finding_hotspot_overlap::run_finding_hotspot_overlap_with(
                    store,
                    &hotspot_rows,
                    &code_health,
                )
                .context("run finding-hotspot-overlap for gate")?;
                let act_now_count = overlap_rows
                    .iter()
                    .filter(|r| r.priority == "act-now")
                    .count();
                let overlap_v = codelore_lib::cli_api::quality_gates::evaluate_finding_overlap_rows(
                    threshold,
                    &overlap_rows,
                );
                #[allow(clippy::cast_precision_loss)]
                // act_now_count is a repo-scale count; precision loss negligible
                let act_now_f64 = act_now_count as f64;
                recs.push(GateRunRecord {
                    ts: ts.to_owned(),
                    head_sha: head_sha.to_owned(),
                    gate: "max_findings_in_hot_files".into(),
                    threshold: f64::from(threshold),
                    value: act_now_f64,
                    verdict: if overlap_v.is_empty() {
                        "passed"
                    } else {
                        "failed"
                    }
                    .into(),
                    mode: "check".into(),
                });
                violations.extend(overlap_v);
            }
        }
    }

    // ── corpus_percentile_max gate ───────────────────────────────────────────
    if let Some(max) = g.corpus_percentile_max {
        // Reuse the already-computed code-health rows. The lens is active only
        // when a calibration artifact is (`--calibration` or an embedded world
        // corpus); without one every row carries `corpus_percentile = None`.
        // That is a SKIP (not a pass, not a fail) — there is no reference corpus
        // to compare against — mirroring the max_findings sidecar-absent skip.
        let has_calibration = code_health.iter().any(|r| r.corpus_percentile.is_some());
        if has_calibration {
            let corpus_v = codelore_lib::cli_api::quality_gates::evaluate_corpus_percentile_rows(
                max,
                &code_health,
            );
            let value = code_health
                .iter()
                .filter_map(|r| r.corpus_percentile)
                .fold(0.0, f64::max);
            recs.push(make_rec(
                "corpus_percentile_max",
                max,
                value,
                !corpus_v.is_empty(),
                ts,
                head_sha,
            ));
            violations.extend(corpus_v);
        } else {
            recs.push(GateRunRecord {
                ts: ts.to_owned(),
                head_sha: head_sha.to_owned(),
                gate: "corpus_percentile_max".into(),
                threshold: max,
                value: 0.0,
                verdict: "skipped".into(),
                mode: "check".into(),
            });
        }
    }

    Ok((violations, recs, hotspot_count, code_health))
}

/// Emit a check SARIF document for `violations` (with their `evidence` chains)
/// to stdout. Canonicalizes `repo` for the artifact-URI prefix, falling back to
/// the path as-given when canonicalization fails (e.g. the path does not exist
/// on disk). Shared by the vacuous-pass path (empty violations + evidence) and
/// the violation path so the canonicalize + stdout + writer wiring lives once.
/// Write a ratchet status message to the right stream for the output format.
///
/// Under `--format sarif` the message goes to stderr so stdout stays a clean
/// SARIF document (mirroring the report path, which routes verdict lines to
/// stderr in SARIF mode); in text mode it goes to stdout with the rest of the
/// report. `msg` is written verbatim, so the caller supplies any trailing
/// newline.
fn emit_ratchet_message(args: &args::CheckArgs, msg: &str) {
    if matches!(args.format, CheckFormat::Sarif) {
        eprint!("{msg}");
    } else {
        print!("{msg}");
    }
}

/// Emit the check SARIF document to stdout when `--format sarif` is set;
/// no-op otherwise.
///
/// Collects a commit evidence chain for each violated per-file path and hands
/// the violations + evidence to [`emit_check_sarif`]. Shared by the normal
/// check path and every `--ratchet` exit path so the flag combination always
/// yields a valid document rather than silently emitting nothing.
fn emit_check_sarif_when_requested(
    args: &args::CheckArgs,
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    head_sha: &str,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
) -> Result<()> {
    use codelore_lib::cli_api::quality_gates::evidence::{EvidenceCommit, evidence_for_path};
    use std::collections::HashMap;

    if !matches!(args.format, CheckFormat::Sarif) {
        return Ok(());
    }

    // Collect evidence only for violated per-file paths (not repo-wide). A
    // failed lookup degrades that result to chainless (empty evidence) rather
    // than failing the SARIF emission; the failure is systemic and repeats per
    // path, so warn at most once per run via the ⚠-prefixed stderr convention.
    let mut evidence_map: HashMap<String, Vec<EvidenceCommit>> = HashMap::new();
    let mut evidence_warned = false;
    for v in violations {
        if v.path != "(repo-wide)" && v.path != "(degraded)" {
            evidence_map.entry(v.path.clone()).or_insert_with(|| {
                evidence_for_path(db, opts, &v.path, 5).unwrap_or_else(|e| {
                    if !evidence_warned {
                        evidence_warned = true;
                        eprintln!(
                            "  ⚠ check: evidence lookup failed ({e}); SARIF results will be emitted without commit chains"
                        );
                    }
                    Vec::new()
                })
            });
        }
    }

    emit_check_sarif(&args.repo, head_sha, violations, &evidence_map)
}

fn emit_check_sarif(
    repo: &std::path::Path,
    head_sha: &str,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
    evidence: &std::collections::HashMap<
        String,
        Vec<codelore_lib::cli_api::quality_gates::evidence::EvidenceCommit>,
    >,
) -> Result<()> {
    let repo_root = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let mut stdout = std::io::stdout();
    codelore_lib::cli_api::output::sarif::write_check_sarif(
        violations,
        evidence,
        &repo_root,
        head_sha,
        &mut stdout,
    )
    .context("emit check SARIF")
}

/// Working-tree quality gate. Projects what the uncommitted edits do to code
/// health and the import graph vs HEAD (the change-set engine), evaluates the
/// working-tree `[diff]` gates against the projection, and exits 0 (pass) or
/// 1 (fail) — the same exit contract as `codelore check`. Writes
/// `result=pass|fail` to `$GITHUB_OUTPUT` for direct GitHub Actions
/// step-output consumption.
fn run_gate_cmd(args: &args::GateArgs) -> Result<()> {
    use codelore_lib::change_set::build_change_set_report;
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::quality_gates::ledger::{append_gate_runs, now_utc_ts};
    use codelore_lib::cli_api::quality_gates::{Thresholds, evaluate_gate_thresholds};

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    let thresholds = if let Some(path) = &args.thresholds_file {
        Thresholds::from_path(path).context("load thresholds file")?
    } else {
        Thresholds::discover(&args.repo).context("discover thresholds file")?
    };

    if thresholds.is_empty() {
        if !args.quiet {
            eprintln!(
                "codelore gate: no thresholds configured (no `.codelore-thresholds.toml` at repo root); vacuously passing."
            );
        }
        // A JSON consumer still gets one contract document on stdout — the same
        // empty shape a clean tree emits — so an agent hook that always runs
        // `gate --format json` never has to special-case a repo with no
        // thresholds configured.
        if matches!(args.format, GateFormat::Json) {
            println!(
                "{}",
                serde_json::json!({ "changes": [], "findings": [], "violations": [] })
            );
        }
        write_github_output("result", "pass");
        return Ok(());
    }

    // Mirrors `quality_gates::resolve_defect_calibration`, but reuses the
    // `thresholds` value already loaded above instead of re-discovering
    // (and re-parsing) the thresholds file.
    let resolved_defect_calibration = args.defect_calibration.clone().or_else(|| {
        thresholds.calibration.defect_artifact.clone().map(|p| {
            if p.is_absolute() {
                p
            } else {
                args.repo.join(p)
            }
        })
    });

    let opts = Options {
        repo_path: args.repo.clone(),
        defect_calibration: resolved_defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        temp_dir: args.temp_dir.clone(),
        ..Options::default()
    };
    opts.validate().context("validate options")?;
    let repo = GixRepo::open(&args.repo).context("open repo")?;
    let db =
        FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root).context("ingest")?;

    let changes = repo
        .worktree_changes()
        .context("enumerate working-tree changes")?;
    if changes.is_empty() {
        report_gate_clean_tree(args);
        return Ok(());
    }

    let report = build_change_set_report(&db, &repo, &opts, &cache_root)
        .context("build change-set report")?;
    let violations = evaluate_gate_thresholds(&thresholds, &report);

    let ts = now_utc_ts();
    append_gate_runs(
        &cache_root,
        &args.repo,
        &gate_ledger_records(&thresholds, &report, &violations, &ts),
    );

    emit_gate_run_notices(args, &thresholds, &report);
    if matches!(args.format, GateFormat::Json) {
        render_gate_json(&report, &violations)?;
    }
    render_gate_verdict(args, &report, &violations)
}

/// Report the empty-change-set pass: a clean tree means there is nothing to
/// gate, which is an explicit PASS (exit 0), not a skipped evaluation. The
/// JSON format still gets one document on stdout — with the contract keys
/// present and empty — so downstream parsers never special-case a clean tree.
fn report_gate_clean_tree(args: &args::GateArgs) {
    let verdict = "✅ codelore gate: PASS (no working-tree changes to gate)";
    if matches!(args.format, GateFormat::Json) {
        println!(
            "{}",
            serde_json::json!({ "changes": [], "findings": [], "violations": [] })
        );
        eprintln!("{verdict}");
    } else {
        println!("{verdict}");
    }
    write_github_output("result", "pass");
    write_github_output("violations", "0");
}

/// Emit the gate run's stderr notices (suppressed under `--quiet`): the
/// merge-in-progress note, and the skip notice when `delta_code_health_min`
/// is configured but a whole-repo code-health median is unavailable on
/// either side (no scoreable files) — skipped, not failed.
fn emit_gate_run_notices(
    args: &args::GateArgs,
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    report: &codelore_lib::change_set::ChangeSetReport,
) {
    if args.quiet {
        return;
    }
    if report.merge_in_progress {
        eprintln!("note: merge/rebase in progress — projection reflects committed HEAD history");
    }
    if thresholds.diff.delta_code_health_min.is_some()
        && (report.health.baseline_median.is_none() || report.health.projected_median.is_none())
    {
        eprintln!(
            "  ⚠ delta_code_health_min: skipped — no whole-repo code-health median to compare"
        );
    }
}

/// Render the gate's JSON document: the full change-set report with the
/// evaluated `violations` array folded in as a sibling key, one document on
/// stdout. Verdict lines stay on stderr so stdout is clean JSON.
fn render_gate_json(
    report: &codelore_lib::change_set::ChangeSetReport,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
) -> Result<()> {
    let mut doc = serde_json::to_value(report).context("serialize change-set report")?;
    doc["violations"] = serde_json::to_value(violations).context("serialize gate violations")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&doc).context("render gate JSON")?
    );
    Ok(())
}

/// Number of delta rows the text render shows; the rest fold into a
/// `(+n more files)` tail. The JSON document always carries every row.
const GATE_DELTA_TABLE_ROWS: usize = 10;

/// Number of advisory-finding rows the text render shows; the rest fold into
/// a `(+n more findings)` tail. Mirrors [`GATE_DELTA_TABLE_ROWS`]'s
/// render-only cap — `report.findings` (the JSON document, and the in-memory
/// `ChangeSetReport`) always carries every finding by design (spec §6); only
/// the rendered text is bounded, so a large coupling cluster or a big batch
/// of added files can never blow the token budget.
const GATE_FINDINGS_ROWS: usize = 10;

/// Print the advisory (non-verdict) text sections to stdout: one line per
/// finding (capped at [`GATE_FINDINGS_ROWS`] with a `(+n more findings)`
/// tail), then the per-file delta table in the engine's order (|delta|
/// descending, unscored rows last). Suppressed under `--quiet`.
fn render_gate_advisories(
    args: &args::GateArgs,
    report: &codelore_lib::change_set::ChangeSetReport,
) {
    if args.quiet {
        return;
    }
    for f in report.findings.iter().take(GATE_FINDINGS_ROWS) {
        println!("[{}] {}: {}", f.kind, f.path, f.detail);
    }
    let hidden_findings = report.findings.len().saturating_sub(GATE_FINDINGS_ROWS);
    if hidden_findings > 0 {
        println!("(+{hidden_findings} more findings)");
    }
    for d in report.health.deltas.iter().take(GATE_DELTA_TABLE_ROWS) {
        match (d.baseline_score, d.projected_score, d.delta) {
            (Some(b), Some(p), Some(delta)) => {
                println!("{}  {b:.1} → {p:.1}  ({delta:+.1})", d.path);
            }
            _ => println!(
                "{}  — {}",
                d.path,
                d.reason.as_deref().unwrap_or("not scored")
            ),
        }
    }
    let hidden = report
        .health
        .deltas
        .len()
        .saturating_sub(GATE_DELTA_TABLE_ROWS);
    if hidden > 0 {
        println!("(+{hidden} more files)");
    }
}

/// Print the verdict, write the GitHub Actions step outputs, and apply
/// check's exit contract: any violation bails (exit 1).
fn render_gate_verdict(
    args: &args::GateArgs,
    report: &codelore_lib::change_set::ChangeSetReport,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
) -> Result<()> {
    if violations.is_empty() {
        if matches!(args.format, GateFormat::Text) {
            println!(
                "✅ codelore gate: PASS ({} changed file(s) evaluated)",
                report.changes.len()
            );
            render_gate_advisories(args, report);
        } else {
            // JSON keeps stdout pure for the report document (already printed),
            // so the verdict line goes to stderr — mirroring the clean-tree and
            // FAIL paths, and honoring the contract that a verdict line is
            // emitted regardless of format.
            eprintln!(
                "✅ codelore gate: PASS ({} changed file(s) evaluated)",
                report.changes.len()
            );
        }
        write_github_output("result", "pass");
        write_github_output("violations", "0");
        return Ok(());
    }
    eprintln!("❌ codelore gate: FAIL — {} violation(s)", violations.len());
    if matches!(args.format, GateFormat::Text) {
        if !args.quiet {
            for v in violations {
                eprintln!(
                    "  - {gate}: {path} — actual {actual} vs threshold {threshold}",
                    gate = v.gate,
                    path = v.path,
                    actual = v.actual,
                    threshold = v.threshold,
                );
            }
        }
        render_gate_advisories(args, report);
    }
    write_github_output("result", "fail");
    write_github_output("violations", &violations.len().to_string());
    // Plain anyhow::bail carries no CodeLoreError, so main()'s chain-walk
    // falls through to the default exit code 1 — check parity by design;
    // typed CodeLoreError variants keep their repo/output exit codes.
    anyhow::bail!("{} gate violation(s) — see above", violations.len());
}

/// Build the ledger records for one gate run: one record per configured
/// working-tree gate, `mode: "gate"`. Counts (offending files, newly cyclic
/// paths) are recorded as the measured value, mirroring the ledger's
/// violation-count convention for gates without a single scalar.
fn gate_ledger_records(
    thresholds: &codelore_lib::cli_api::quality_gates::Thresholds,
    report: &codelore_lib::change_set::ChangeSetReport,
    violations: &[codelore_lib::cli_api::quality_gates::GateViolation],
    ts: &str,
) -> Vec<codelore_lib::cli_api::quality_gates::ledger::GateRunRecord> {
    use codelore_lib::cli_api::quality_gates::ledger::GateRunRecord;
    let rec = |gate: &str, threshold: f64, value: f64, verdict: &str| GateRunRecord {
        ts: ts.to_owned(),
        head_sha: report.head_sha.clone(),
        gate: gate.to_owned(),
        threshold,
        value,
        verdict: verdict.to_owned(),
        mode: "gate".to_owned(),
    };
    let count = |gate: &str| violations.iter().filter(|v| v.gate == gate).count();
    let count_f64 = |gate: &str| f64::from(u32::try_from(count(gate)).unwrap_or(u32::MAX));
    let verdict = |gate: &str| if count(gate) == 0 { "passed" } else { "failed" };

    let mut records = Vec::new();
    let d = &thresholds.diff;
    if let Some(min) = d.delta_code_health_min {
        let record = match (
            report.health.baseline_median,
            report.health.projected_median,
        ) {
            (Some(base), Some(projected)) => rec(
                "delta_code_health_min",
                min,
                projected - base,
                verdict("delta_code_health_min"),
            ),
            _ => rec("delta_code_health_min", min, 0.0, "skipped"),
        };
        records.push(record);
    }
    if let Some(min) = d.delta_code_health_min_per_file {
        records.push(rec(
            "delta_code_health_min_per_file",
            min,
            count_f64("delta_code_health_min_per_file"),
            verdict("delta_code_health_min_per_file"),
        ));
    }
    if let Some(min) = d.new_file_health_min {
        records.push(rec(
            "new_file_health_min",
            min,
            count_f64("new_file_health_min"),
            verdict("new_file_health_min"),
        ));
    }
    if d.no_new_cycles {
        records.push(rec(
            "no_new_cycles",
            0.0,
            count_f64("no_new_cycles"),
            verdict("no_new_cycles"),
        ));
    }
    records
}

/// Write a single `key=value` line to `$GITHUB_OUTPUT` when the env
/// var is set. No-op outside GitHub Actions.
fn write_github_output(key: &str, value: &str) {
    if let Ok(path) = std::env::var("GITHUB_OUTPUT")
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{key}={value}");
    }
}

/// Operational telemetry. Prints what `CodeLore` ships under the
/// hood — schema version, pinned dependency versions, supported
/// analysis count, supported output format count. Useful for triage
/// when behaviour surprises a user.
#[allow(clippy::unnecessary_wraps)] // dispatcher uniformity — every arm returns Result<()>
fn run_profile_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    println!("# CodeLore profile\n");
    println!("**Version**: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "**Schema**: schema_v{} (`facts/schema_v1.sql`)",
        codelore_lib::cli_api::facts::schema::CURRENT_SCHEMA_VERSION
    );
    println!("**Analyses**: {} registered", AnalysisName::all().len());
    println!("**Output formats**: csv | json | sarif | markdown | parquet | sqlite | html | spa");
    println!(
        "**Pinned third-party**:\n  - gix {gix}\n  - DuckDB {duckdb}\n  - tree-sitter 0.25.x (Rust/Python/Java/JS/TS/TSX/C++)",
        gix = codelore_lib::cli_api::provenance::GIX_VERSION,
        duckdb = codelore_lib::cli_api::provenance::DUCKDB_VERSION,
    );
    println!("\n**Cache root**:");
    if let Some(dir) = dirs::cache_dir() {
        println!("  {}/codelore/", dir.display());
    } else {
        println!("  <unavailable on this platform>");
    }
    println!(
        "\n**SPA feature**: {}",
        if cfg!(feature = "spa") {
            "ENABLED"
        } else {
            "disabled (build with --features spa to opt in)"
        }
    );
    println!("\n_For per-analysis SQL + citations, run `codelore explain <topic>`._");
    Ok(())
}

/// Markdown dump of every supported analysis. Seeds the planned
/// full static-HTML doc site.
#[allow(clippy::unnecessary_wraps)] // dispatcher uniformity — every arm returns Result<()>
fn run_docs_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    println!("# CodeLore — Analysis catalogue\n");
    println!(
        "Auto-generated from `AnalysisName::all()`. Run `codelore explain <topic>` for per-analysis citations and formulas. The full citation chain lives in `docs/research-foundations.md`.\n"
    );
    println!("## Supported analyses\n");
    for analysis in AnalysisName::all() {
        println!("- `{}`", analysis.as_str());
    }
    println!("\n## Output formats\n");
    for fmt in &[
        ("csv", "code-maat-compatible flat tables"),
        ("json", "stable JSON shape per row type"),
        (
            "ndjson",
            "newline-delimited JSON — one row per line for stream consumers (LSP, `jq -c`, CI pipelines)",
        ),
        ("sarif", "SARIF 2.1.0 — surfaces in GitHub Code Scanning"),
        ("markdown", "GFM tables for `$GITHUB_STEP_SUMMARY`"),
        (
            "gha",
            "GitHub Actions workflow commands — `::error::` / `::warning::` / `::notice::` on stdout, surfaced as inline PR annotations",
        ),
        ("parquet", "columnar bulk export for analytical pipelines"),
        ("sqlite", "full DuckDB fact-store dump"),
        ("html", "self-contained per-analysis HTML report"),
        (
            "spa",
            "single-file interactive dashboard (opt-in via `spa` feature)",
        ),
    ] {
        println!("- `{}` — {}", fmt.0, fmt.1);
    }
    println!("\n## Conventions\n");
    println!("- Files alive at HEAD only (deleted files excluded from path-aggregating analyses)");
    println!("- Mailmap + `.codelore-teams` + `.codelorebots` consulted at ingest time");
    println!("- `.gitignore` / `.codeloreignore` honoured");
    println!("- `--time-bucket` supported on: hotspots, coupling, soc, code-health");
    println!(
        "\n## Reproducibility\n\nEvery file output is paired with a `.provenance.json` sidecar capturing the run's full `Options` shape. SQLite outputs embed the equivalent inside the `provenance` table."
    );
    println!(
        "\n_See also: `codelore profile` for operational telemetry, `codelore schema <type>` for row schemas, `docs/research-foundations.md` for citations._"
    );
    Ok(())
}

/// Emit shell-completion script for the given shell to stdout. The
/// `clap_complete` derive macro consumes our existing clap spec —
/// no hand-maintained completion files.
fn run_completions_cmd(args: &args::CompletionsArgs) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(args.shell, &mut cmd, bin_name, &mut std::io::stdout());
}

/// Print formula + citation + SQL for the named metric or analysis.
/// With no topic, lists every supported topic. Makes the auditable-
/// formulas brand promise tactile on the CLI side — `codelore
/// explain hotspot-score` is the answer to "why does this file have
/// score 8.4?".
#[allow(clippy::too_many_lines)] // catalogue table — splitting would hurt readability
fn run_explain_cmd(args: &args::ExplainArgs) -> Result<()> {
    let topics: &[(&str, &str, &str, &str)] = &[
        (
            "hotspot-score",
            "Tornhill 2018 — Software Design X-Rays",
            "percentile_rank(revisions) × percentile_rank(cognitive) × (100 − code_health) / 4. Range [0, 10].",
            "See analyses/hotspots.rs::SQL (file_revs + file_complexity + joined CTEs).",
        ),
        (
            "code-health",
            "code-health composite: biomarker structural risk (Complex/Large Method, God Class, DRY, Shotgun Surgery, Deep Nesting, Many Args, Complex Conditional) fused with behavioral signal (Nagappan & Ball 2005 churn + Mockus & Herbsleb 2002 ownership); coupling centrality enters once via the Shotgun Surgery biomarker (Tornhill 2018); self-relative percentile banding (Alves/Ypma/Visser 2010) plus an additive corpus-relative percentile when a calibration artifact is active",
            "100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv), where structural_risk is a weighted sum of biomarker intensities (complex-method 0.22, god-class 0.18, large-method 0.12, dry 0.12, shotgun-surgery 0.12, deep-nesting 0.10, many-args 0.07, complex-conditional 0.07); band from structural_risk thresholds (≥0.55 red, ≥0.28 yellow, else green); percentile = per-language PERCENT_RANK of structural_risk.",
            "See analyses/code_health.rs.",
        ),
        (
            "refactoring-targets",
            "effort-aware refactoring priority: (code-health structural_risk × hotspot_score) ÷ inspection effort, with a ManualUp baseline (Popt / PofB20 framing)",
            "priority = (structural_risk × hotspot_score) / max(loc, 25). Ranked DESC. `manual_up_rank` ranks the same files by ascending LOC (the \"inspect the small dense files first\" baseline the composite is meant to beat); `dominant_type` is the file's highest-intensity biomarker.",
            "See analyses/refactoring_targets.rs.",
        ),
        (
            "mi",
            "Coleman 1994 + SEI 1997",
            "171 − 5.2·log₂(V) − 0.23·CC − 16.2·log₂(SLOC) + 50·sin(√(2.4·comments%)). file-level `kind='unit'` entry.",
            "Surfaced by rust-code-analysis via codelore-rca.",
        ),
        (
            "coupling-density",
            "Newman 2010 §6.10 — graph density",
            "edges / (V·(V−1)/2) where V is the candidate node set (files with revs ≥ min_revs) and edges are Fisher-significant coupling pairs.",
            "See analyses/coupling.rs::density.",
        ),
        (
            "hotspots",
            "Tornhill 2015 + Bird et al. 2011",
            "Per-file behavioural risk surface: revisions × max(cognitive) × code-health composite. The flagship CodeLore analysis.",
            "See analyses/hotspots.rs.",
        ),
        (
            "hotspot-velocity",
            "Change-acceleration early warning (recent vs baseline churn)",
            "Per file: acceleration = recent_per_week − baseline_per_week over a 30-day recent window vs the 90 days before it, anchored at MAX(commits.date). Positive = heating up (becoming a hotspot before its all-time count shows it); negative = cooling down.",
            "See analyses/hotspot_velocity.rs.",
        ),
        (
            "god-classes",
            "Brown et al. 1998 *AntiPatterns* §3.1 + Riel 1996 *Object-Oriented Design Heuristics*",
            "(cognitive / 100.0) × (fan_in + fan_out). Ranks files where all three pull up.",
            "See analyses/god_classes.rs.",
        ),
        (
            "architecture-roles",
            "Baldwin, MacCormack & Rusnak 2014 — Hidden Structure (Research Policy 43:8)",
            "Per-file Core/Shared/Control/Periphery from transitive visibility fan-in (vfi) / fan-out (vfo) on the import graph: Core = the largest cyclic group; Shared = vfi≥core, vfo<core; Control = vfi<core, vfo≥core; Periphery = both below. reach_pct = vfo/n×100; mean(vfo/n) = MacCormack propagation cost.",
            "See analyses/architecture_roles.rs + analyses/import_graph.rs::reachability.",
        ),
        (
            "instability",
            "Martin 1994 — OO Design Quality Metrics (Clean Architecture 2017)",
            "Per file: afferent coupling ca (files importing it / in-degree), efferent coupling ce (files it imports / out-degree), instability I = ce/(ca+ce) in [0,1]. 0 = stable (depended-on, depends on nothing), 1 = unstable. Resolved import graph; Abstractness/Distance need symbol data and are out of scope.",
            "See analyses/instability.rs.",
        ),
        (
            "cycle-health",
            "behavioral heat + extraction candidate per import cycle",
            "Per non-trivial SCC of the resolved import graph: heat_pct = the cycle \
             members' share of repo LOC churn over the trailing --window-days window \
             (same anchoring as effort-exposure); verdict = live when at least one \
             member appears in a window commit (a zero-LOC touch still counts), fossil \
             otherwise; extract_candidate = the member whose trial removal minimises \
             the largest surviving SCC of the induced subgraph (Tarjan per member, ties \
             by fewest surviving cyclic nodes then lexicographic path); \
             predicted_pc_drop = whole-graph MacCormack propagation-cost delta if the \
             candidate were extracted. Trial removal and the prediction run only for \
             cycles of ≤ 64 members; above that bound the prediction is absent and the \
             candidate falls back to the highest in-cycle degree.",
            "See analyses/cycle_health.rs.",
        ),
        (
            "defect-validation",
            "Śliwerski, Zimmermann & Zeller 2005 (SZZ) + Kim et al. 2006 (AG-SZZ)",
            "Reads an own-repo defect-calibration artifact (built by `codelore \
             calibrate-defects`) and reports its evidence as flat (metric, value) \
             rows: the band table (share of defect-introducing changes that landed \
             in files red / yellow / green at the time), AUC and precision@k of \
             HEAD structural_risk against the defect-implicated file labels, mining \
             tallies, and the weight-tuning decision with both validation AUCs. \
             Association, not causation — a defect touching a red file is evidence \
             the score ranks it high, not proof the score caused the defect. Reads \
             the artifact only; without one it emits zero rows and a stderr hint.",
            "See analyses/defect_validation.rs + defect_calibration/.",
        ),
        (
            "architecture-metrics",
            "Lakos 1996 (CCD/ACD/NCCD) + MacCormack/Rusnak/Baldwin 2006/2014",
            "Repo-level (metric, value) rows: propagation_cost = density of the transitive-closure matrix; acd = mean transitive dependency set size; nccd = CCD / balanced-binary-tree CCD (<1 flat, >1 layered, >2 likely cyclic); dependency_cycles, largest_cycle; architecture_type = hierarchical / core-periphery / multi-core.",
            "See analyses/architecture_metrics.rs.",
        ),
        (
            "architecture-trend",
            "Architectural decay over the commit sequence",
            "Recomputes propagation cost, dependency-cycle count and largest tangle at up to 12 historical revs (evenly spaced across history), rebuilding the import graph in memory at each by reading + resolving source blobs at that rev. Shows whether structure is decaying and roughly when it started. Heavier than the SQL-only analyses (it re-parses source per sample); computed on demand, never cached.",
            "See analyses/architecture_trend.rs.",
        ),
        (
            "health-trend",
            "Repo health (architectural + code + combined) over the commit sequence",
            "Computes three 0-100 scores at up to 12 historical revs (evenly spaced): \
             architectural health (structural — propagation cost + dependency tangle), \
             code health (the rev-parameterized code-health engine with duplication \
             excluded, averaged over files), and their equal blend. Bands: green >= 70, \
             yellow 40-69, red < 40. Rebuilds the import graph + re-scans complexity per \
             sample, so it is heavier than SQL-only analyses; computed on demand, never \
             cached.",
            "See analyses/health_trend.rs.",
        ),
        (
            "effort-exposure",
            "Engineering effort distribution across code-health bands",
            "For each code-health band (red / yellow / green) reports the percentage of \
             files, SLOC, trailing-window commits, and LOC churn in that band. Answers \
             the hero KPI question: are we spending most effort fighting fires in red code \
             or extending healthy green code? Commit-share Wilson 95% CI is included per \
             band. Window anchors to the repo's last commit date (not wall-clock) via \
             --window-days (default 90).",
            "See analyses/effort_exposure.rs.",
        ),
        (
            "code-familiarity",
            "Decayed-knowledge familiarity score for the active team",
            "Computes what fraction of SLOC is actively known by current contributors \
             (authors with ≥1 commit in the trailing window). Uses exponentially-decayed \
             knowledge shares (Jabrayilzade et al., ICSE-SEIP 2022). Also reports islands \
             percentage: SLOC in files where one person holds ≥80% of knowledge with no \
             meaningful backup. Low familiarity or high islands percentage signals knowledge \
             risk. Verdict threshold configurable via [gates] code_familiarity_min in \
             .codelore-thresholds.toml (default 70.0).",
            "See analyses/code_familiarity.rs.",
        ),
        (
            "team-composition",
            "Contribution-span tenure buckets with behavioral veteran gate and onboarding velocity",
            "Buckets each author by contribution span (last − first commit): onboarded \
             (<90 d), experienced (90–364 d), veteran (≥365 d). Veterans who have not \
             touched a breadth of files comparable to the current 80%-core set are capped \
             at 'experienced' (veteran_breadth_ok = false). Also reports onboarding_weeks: \
             how many weeks from an author's first commit to their first week in the weekly \
             80%-core set. Authors whose first commit falls within the project's first 12 \
             weeks (founders) receive NULL for onboarding_weeks.",
            "See analyses/team_composition.rs.",
        ),
        (
            "coordination-needs",
            "Per-file coordination overhead: fragmentation, interleave, co-change entropy",
            "For each file reports: knowledge fragmentation (1 − HHI, 0 = single owner, \
             near 1 = evenly spread knowledge); author-switch interleave between adjacent \
             commits (0 = always same author, 1 = always alternating); and co-change graph \
             entropy contribution (EASE 2025, arXiv 2504.18511; window-scoped, commits \
             touching >30 files excluded). Tier: single (1 author) | low (frag<0.25) | \
             medium | high (frag≥0.50 AND interleave≥0.50). Joined with code-health band \
             so high-fragmentation files in the red band surface first.",
            "See analyses/coordination_needs.rs.",
        ),
        (
            "marginal-owner-risk",
            "Ownership concentration × code-health fusion: files where active authors have shallow familiarity",
            "For each file in the yellow or red health band, reports the maximum knowledge \
             share held by any author who committed within window_days. Risk tiers: high \
             (red band AND top active share <0.10); elevated ((red AND <0.30) OR (yellow \
             AND <0.10)). Rows that do not meet either threshold are excluded. The \
             ownership × code-quality interaction is correlational, not causal \
             (Palomba et al., EASE 2023, arXiv 2304.11636).",
            "See analyses/marginal_owner_risk.rs.",
        ),
        (
            "release-cadence",
            "Inter-release gap statistics from git tags (median, IQR, trend)",
            "Filters tags by --release-tag-glob (default 'v*'), then computes the \
             number of days between consecutive release tags. Emits one row per \
             matched tag (date, days_since_prev) plus a synthetic '__summary__' \
             row carrying the median gap, IQR, and a trend label: 'accelerating' \
             (negative OLS slope < −0.1 d/release), 'slowing' (slope > +0.1), or \
             'stable' (within ±0.1). Tags are proxies for releases, not \
             deployments; cadence reflects tagging discipline as much as actual \
             release velocity. First tag always has no predecessor gap.",
            "See analyses/release_cadence.rs.",
        ),
        (
            "delivery-metrics",
            "Repo-level delivery flow distributions: batch size, branch duration, rework, and lead-proxy (p50/p75/p90)",
            "Five percentile distributions over merge units and commits: batch_size_files \
             (distinct paths per merge), batch_size_loc (LOC churn per merge), \
             branch_duration_hours (merge date − earliest branch-side author date), \
             rework_pct (hunk-overlap within --rework-window-days, approximate), and \
             lead_proxy_hours (author→committer date gap, positive only, non-merge commits). \
             Requires commit_parents table (schema v4) and merges ingested with \
             include_merges=true. Branch metrics are unreliable on squash/rebase workflows \
             (emits a warning when merge count < 3 and commit count > 50).",
            "See analyses/delivery_metrics.rs.",
        ),
        (
            "function-xray",
            "Per-function change frequency for a single target file (Gall et al. ICSM 2003 HistoryFinder)",
            "Requires --target <repo-relative-path>. For each function/method alive at HEAD \
             in the target file, counts revisions where at least one hunk overlapped the \
             function's line span. Hunk-overlap attribution is more accurate than file-level \
             blame: it uses the span at change time. Pure deletions (new_lines=0) are attributed \
             to the function whose span contained the deleted anchor line. Sorted by change_freq \
             DESC.",
            "See analyses/function_xray.rs.",
        ),
        (
            "function-coupling",
            "Per-function-pair co-change frequency with Fisher significance for a single target file",
            "Requires --target <repo-relative-path>. For each pair of HEAD-alive functions in the \
             target file that co-changed (both touched in the same revision) in ≥2 revisions, \
             emits the pair with co-change count, per-function change counts, confidence \
             (co/min(a,b)), and two-tailed Fisher exact p-value. \
             Sorted by p-value ASC. Research: Adams et al. ICSM 2006.",
            "See analyses/function_coupling.rs.",
        ),
        (
            "cycle-origins",
            "Commit-level archaeology for dependency cycles",
            "For each dependency cycle at HEAD, binary-searches history (reading + resolving source at past revisions) to find the earliest commit where that cycle existed — the commit that closed the loop. Reports the forming commit's SHA + date per cycle. Assumes a cycle, once formed, stays formed; traces the largest cycles first to bound cost.",
            "See analyses/cycle_origins.rs.",
        ),
        (
            "dependency-cycles",
            "Tarjan 1972 SCC + Fontana et al. 2017 (Arcan) Cyclic Dependency smell",
            "Non-trivial strongly-connected components (size ≥ 2) of the structural import graph — files that import each other transitively. cycle_id groups a tangle; size is its member count. Accuracy follows the import resolver's language coverage.",
            "See analyses/dependency_cycles.rs + analyses/import_graph.rs.",
        ),
        (
            "modularity-violations",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8) + Baldwin/MacCormack 2014 hidden structure",
            "Fisher-significant co-change pairs (from coupling) with NO directed dependency path between them (transitive reachability, either direction) — implicit cross-module dependencies. Ranked by coupling degree. Accuracy follows the import resolver's language coverage.",
            "See analyses/modularity_violations.rs.",
        ),
        (
            "unstable-interface",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8)",
            "revisions × coupled_dependents, gated on fan_in ≥ 3 and revisions ≥ min_revs. A widely-imported file that changes often and co-changes with its dependents, so its instability propagates.",
            "See analyses/unstable_interface.rs.",
        ),
        (
            "crossing",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8)",
            "A structural 'X' — fan_in ≥ 3 AND fan_out ≥ 3 — that co-changes with ≥1 importer AND ≥1 import, coupling upstream and downstream through itself. crossing_score = coupled_upstream + coupled_downstream.",
            "See analyses/crossing.rs.",
        ),
        (
            "bus-factor",
            "Filatov 2010 (commits mode) / Cury & Avelino SBES'24 (doe mode)",
            "Min number of authors whose departure would leave a module unmaintained. \
             Default mode (--knowledge-model commits): smallest set covering ≥80% of \
             module commits (Filatov 2010). DOE mode (--knowledge-model doe): greedy \
             truck-factor procedure — repeatedly remove the author expert on the most \
             remaining files until >50% of files have no expert (Cury & Avelino, \
             SBES'24 arXiv 2408.08733). DOE mode emits the same per-module row shape \
             with model='doe'.",
            "See analyses/bus_factor.rs.",
        ),
        (
            "stale-code",
            "code-age follow-up + Sonar 'trivial' threshold",
            "Files alive at HEAD AND untouched ≥12 months AND max(cognitive) ≤ 5. Intersection minimises false positives.",
            "See analyses/stale_code.rs.",
        ),
        (
            "pair-programming",
            "Co-Authored-By trailer convention (GitHub 2017)",
            "Counts commits where ≥1 `Co-Authored-By:` trailer present, by unique author pair.",
            "See analyses/pair_programming.rs.",
        ),
        (
            "lead-time",
            "DORA 2018 Accelerate",
            "Seconds between commit author-date and committer-date (proxy for in-flight review time). Schema_v3 carries only committer-date; schema_v4 will add author-date for real values.",
            "See analyses/lead_time.rs.",
        ),
        (
            "knowledge-islands",
            "T8 design + Bird et al. 2011 risk-author",
            "Per-file bus-factor risk: primary author hasn't committed in `--departed-threshold-days` days AND no substantial other owners.",
            "See analyses/knowledge_islands.rs.",
        ),
        (
            "communities",
            "Leiden algorithm (Traag, Waltman, van Eck 2019)",
            "Modularity-optimising community detection on the Fisher-significant coupling graph. Surfaces Conway's-law clusters.",
            "See analyses/communities.rs.",
        ),
        (
            "centrality",
            "Newman 2010 §7",
            "Per-file degree, weighted-degree, and PageRank on the Fisher-significant coupling graph.",
            "See analyses/centrality.rs.",
        ),
        (
            "architecture-violations",
            "Layered architecture rules (Buschmann et al. 1996)",
            "Imports that cross a forbidden layer boundary per `.codelore-arch-rules.toml`. Empty rule set → empty output.",
            "See arch_rules/mod.rs + analyses/arch_violations.rs.",
        ),
        (
            "kamei-risk",
            "Kamei et al. 2013 (Just-In-Time Software Defect Prediction)",
            "Per-commit 14-feature vector (la, ld, nf, nd, ns, entropy, fix, ndev, age, nuc, exp, rexp, sexp, lt). Composite risk dimension explanation in the SPA's Delivery Risk Sparkline.",
            "See output/spa.rs::run_kamei_risk + facts/schema_v1.sql commits table.",
        ),
        (
            "revisions",
            "Nagappan & Ball 2005 — relative churn predicts defect density",
            "COUNT(rev) per file — distinct commits touching the path. Gated by --min-revs, ordered by n-revs descending.",
            "See analyses/revisions.rs.",
        ),
        (
            "authors",
            "Bird, Nagappan, Murphy, Devanbu & Zeller 2011 — \"Don't Touch My Code!\"",
            "Distinct canonical authors per file (n-authors = COUNT(DISTINCT author)), split into human vs bot via .mailmap + bot/AI attribution; n-revs = Σ per-author commit counts.",
            "See analyses/authors.rs.",
        ),
        (
            "ownership",
            "Mockus & Herbsleb 2002 + Hirschman 1980 (Herfindahl–Hirschman index)",
            "Fractal Value = 1 − Σᵢ (aᵢ / nc)², where aᵢ is author i's commit count on the file and nc the file's total commits. 0 = single owner, → 1 = fragmented. main-author = author with the most revisions.",
            "See analyses/ownership.rs.",
        ),
        (
            "code-age",
            "Tornhill 2015 — Your Code as a Crime Scene (software half-life)",
            "Whole calendar months between the file's latest commit (at-or-before the --age-time-now anchor, default now) and the anchor: 12·(yr−yr) + (mo−mo) − 1 if the anchor day-of-month is earlier than the last-commit day. Only files live at the anchor.",
            "See analyses/code_age.rs.",
        ),
        (
            "soc",
            "Tornhill 2018 — Software Design X-Rays (Sum of Coupling)",
            "Σ (commit_size − 1) over every commit the file appears in; a solo commit contributes 0. Per-file centrality across the change-coupling graph. Gated by --min-soc.",
            "See analyses/soc.rs.",
        ),
        (
            "abs-churn",
            "Nagappan & Ball 2005 — relative code churn predicts defect density",
            "Per calendar day across the repo: SUM(lines added), SUM(lines deleted), COUNT(commits). The absolute-churn time series.",
            "See analyses/churn.rs.",
        ),
    ];
    match &args.topic {
        None => {
            println!("Supported topics:");
            for (name, _, _, _) in topics {
                println!("  {name}");
            }
            println!("\nUsage: codelore explain <topic>");
            Ok(())
        }
        Some(topic) => {
            let found = topics.iter().find(|(n, ..)| n.eq_ignore_ascii_case(topic));
            match found {
                Some((name, citation, formula, source)) => {
                    println!("# {name}\n");
                    println!("**Citation**\n  {citation}\n");
                    println!("**Formula**\n  {formula}\n");
                    println!("**Source**\n  {source}\n");
                    println!(
                        "**Foundations**\n  See docs/research-foundations.md for the full citation chain."
                    );
                    Ok(())
                }
                None => match resolve_explain_file(&args.repo, topic) {
                    Some(repo_relative) => run_explain_file(args, &repo_relative),
                    None => Err(CodeLoreError::Analysis(format!(
                        "unknown topic `{topic}` — run `codelore explain` (no arg) to list \
                         supported topics, or pass an existing file path (with --repo) to print \
                         that file's evidence dossier"
                    ))
                    .into()),
                },
            }
        }
    }
}

/// Resolve an `explain` argument that missed the topic table to a repo-relative
/// source-file path, or `None` when it names no existing file.
///
/// The argument is joined onto `--repo`; `Path::join` lets an absolute argument
/// replace the repo, so a repo-relative `src/x.rs`, a `--repo`-prefixed path,
/// and an absolute path to the same file all resolve to the same target. The
/// fact store keys on repo-relative, forward-slash paths, so the resolved path
/// is made relative to `--repo` and its separators are normalized to `/`.
fn resolve_explain_file(repo: &std::path::Path, arg: &str) -> Option<String> {
    let candidate = repo.join(arg);
    if !candidate.is_file() {
        return None;
    }
    let relative = match candidate.strip_prefix(repo) {
        Ok(stripped) => stripped.to_path_buf(),
        Err(_) => std::path::PathBuf::from(arg),
    };
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Print the deterministic evidence dossier for a repo-relative source file,
/// and — with `--llm` — an advisory grounded narrative plus its citation-check
/// stamp.
///
/// This surface is strictly read-only: it opens (or ingests) the fact store and
/// assembles a fact sheet from the same analyses the CLI already exposes, never
/// touching an analysis row, a gate verdict, or a provenance manifest. Analysis
/// `min_revs` is forced to 1 so any single named file can be explained — the
/// default corpus gate would otherwise hide most files from their own dossier.
/// That 1-revision floor also applies to the dossier's hotspot, coupling, and
/// ownership sections, so their numbers can differ from a default `analyze` run
/// that gates low-revision files out.
///
/// Without `--llm`, when this file's own previously generated narrative exists
/// for a now-changed fact sheet, a one-line staleness note is printed. With
/// `--llm`, a missing LLM configuration is a hard error carrying a setup hint.
fn run_explain_file(args: &args::ExplainArgs, repo_relative: &str) -> Result<()> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::enrichment::client::{LlmEnv, resolve_client};
    use codelore_lib::cli_api::enrichment::fact_sheet::FileFactSheet;
    use codelore_lib::cli_api::enrichment::prompt::Lens;
    use codelore_lib::cli_api::enrichment::{cache, engine};

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);
    let defect_calibration = codelore_lib::cli_api::quality_gates::resolve_defect_calibration(
        args.defect_calibration.clone(),
        &args.repo,
    )
    .context("resolve defect calibration")?;
    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: 1,
        defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        ..Options::default()
    };
    let repo = GixRepo::open(&args.repo)
        .with_context(|| format!("open git repo at {}", args.repo.display()))?;
    let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
        .context("open or ingest the fact store")?;
    let sheet = FileFactSheet::build(&db, &repo, &opts, repo_relative)
        .with_context(|| format!("build the evidence dossier for {repo_relative}"))?;

    print!("{}", sheet.to_human_text());

    if args.llm {
        let client = resolve_client(&LlmEnv::from_process_env()).context(
            "configure an LLM endpoint — set CODELORE_LLM_MODEL for a local OpenAI-compatible \
             runner (e.g. a model from `ollama list`), or ANTHROPIC_API_KEY for Anthropic; see \
             the CODELORE_LLM_* variables in the docs",
        )?;
        let canonical = sheet.to_canonical_text();
        let values = sheet.numeric_values();
        let result = engine::narrate(
            client.as_ref(),
            Lens::FileDiagnosis,
            repo_relative,
            engine::SheetFacts {
                text: &canonical,
                values: &values,
            },
            &cache_root,
            &args.repo,
            args.llm_refresh,
        )
        .context("generate the advisory narrative")?;
        println!("\n{}", result.narrative);
        println!("{}", engine::stamp(&result));
    } else if let Some(latest) = cache::latest_for_subject(&cache_root, &args.repo, repo_relative)
        && latest.fact_digest != sheet.digest()
    {
        println!("note: cached narrative is stale — evidence changed; re-run with --llm");
    }

    Ok(())
}

/// JSON Schema export. The CLI surfaces the row-type catalogue and
/// emits a minimal envelope per type today; full `JsonSchema` derive
/// on every row type is a planned enhancement (~80 LOC of
/// `#[derive(JsonSchema)]` adds across the analyses module) and
/// will populate the `items` shape once `schemars` derive lands.
fn run_schema_cmd(args: &args::SchemaArgs) -> Result<()> {
    let row_types: Vec<&str> = AnalysisName::all().iter().map(|a| a.as_str()).collect();
    match &args.row_type {
        None => {
            println!("Supported row types ({}):", row_types.len());
            for name in &row_types {
                println!("  {name}");
            }
            println!(
                "\nUsage: codelore schema <row-type>\n\nNote: today's emitter ships the row-type catalogue and a minimal envelope. The full JSON Schema documents populate the `items` shape once `schemars` derive is applied to every analyses/* row type."
            );
            Ok(())
        }
        Some(name) => {
            if row_types.contains(&name.as_str()) {
                println!(
                    "{{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"$id\": \"https://codelore.dev/schemas/{name}.json\",\n  \"title\": \"{name}\",\n  \"type\": \"array\",\n  \"items\": {{\n    \"$comment\": \"Full row-shape schema populates once schemars derive is applied.\"\n  }}\n}}"
                );
                Ok(())
            } else {
                Err(CodeLoreError::Analysis(format!(
                    "unknown row type `{name}` — run `codelore schema` (no arg) to list supported row types"
                ))
                .into())
            }
        }
    }
}

fn run_diff_cmd(args: &DiffArgs) -> Result<()> {
    let (output, head_db, head_opts) = diff::run_diff(args).context("codelore diff")?;

    // Advisory LLM narrative. Best-effort and format-scoped: it is produced only
    // for `text`/`markdown` with `--llm`, any failure degrades to a stderr
    // warning, and it never touches the deterministic output below or the
    // `should_fail` exit code.
    let format = args.format.as_str();
    let narrative: Option<(String, String)> = if args.llm {
        match format {
            "text" | "markdown" => diff_llm_narrative(args, &output),
            _ => {
                eprintln!("note: --llm applies to text/markdown output only; ignored for {format}");
                None
            }
        }
    } else {
        None
    };

    let mut out: Box<dyn Write> = match args.output.as_ref() {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    diff_output::emit(
        &mut out,
        &output,
        format,
        &args.repo,
        Some((&head_db, &head_opts)),
        narrative,
    )?;
    drop(out);

    if diff::should_fail(args, &output) {
        // Per spec §6.6: analysis-failure exit code is 4.
        std::process::exit(4);
    }
    Ok(())
}

/// Produce the advisory LLM narrative for a diff run, degrading gracefully.
///
/// The `DiffOutput` is flattened into a deterministic [`DiffFactSheet`], a chat
/// client is resolved from the `CODELORE_LLM_*` environment, and the narrative
/// is generated under [`Lens::DiffNarrative`] with a citation-check stamp. Any
/// failure — no endpoint configured, network error, or narration error — is
/// reported on stderr and yields `None`, so the caller's deterministic output
/// and exit code stay untouched. The narrative cache lives under the default
/// cache root (diff has no `--cache-dir`).
fn diff_llm_narrative(args: &DiffArgs, output: &diff::DiffOutput) -> Option<(String, String)> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::enrichment::client::{LlmEnv, resolve_client};
    use codelore_lib::cli_api::enrichment::engine;
    use codelore_lib::cli_api::enrichment::fact_sheet::DiffFactSheet;
    use codelore_lib::cli_api::enrichment::prompt::Lens;

    let sheet = DiffFactSheet::from_sections(diff_fact_sections(output));
    let canonical = sheet.to_canonical_text();
    let values = sheet.numeric_values();
    let cache_root = default_cache_root();

    let result = resolve_client(&LlmEnv::from_process_env()).and_then(|client| {
        engine::narrate(
            client.as_ref(),
            Lens::DiffNarrative,
            "diff",
            engine::SheetFacts {
                text: &canonical,
                values: &values,
            },
            &cache_root,
            &args.repo,
            args.llm_refresh,
        )
    });
    match result {
        Ok(result) => Some((result.narrative.clone(), engine::stamp(&result))),
        Err(e) => {
            eprintln!("warning: llm narrative unavailable: {e}");
            None
        }
    }
}

/// Flatten a `DiffOutput` into ordered fact-sheet sections for the advisory diff
/// narrative. Only sections with data are emitted, and every numeric value is
/// rendered through the shared [`fmt_num`] formatter so the narrative's citation
/// check can match each quoted number back to a fact.
fn diff_fact_sections(output: &diff::DiffOutput) -> Vec<(String, Vec<(String, String)>)> {
    use codelore_lib::cli_api::enrichment::fact_sheet::fmt_num;

    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();

    // verdict — change-level health ratio, verdict, and change counts.
    if let Some(dh) = &output.delta_health {
        let mut facts = vec![("verdict".to_string(), dh.verdict.clone())];
        if let Some(ratio) = dh.ratio {
            facts.push(("ratio".to_string(), fmt_num(ratio)));
        }
        facts.push(("added".to_string(), dh.counts.added.to_string()));
        facts.push(("modified".to_string(), dh.counts.modified.to_string()));
        facts.push(("removed".to_string(), dh.counts.removed.to_string()));
        facts.push(("skipped".to_string(), dh.counts.skipped.to_string()));
        sections.push(("verdict".to_string(), facts));
    }

    // gates — [diff] quality-gate violations.
    if !output.gate_violations.is_empty() {
        let mut facts = Vec::new();
        for (i, v) in output.gate_violations.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.gate"), v.gate.clone()));
            facts.push((format!("{n}.path"), v.path.clone()));
            facts.push((format!("{n}.actual"), v.actual.clone()));
            facts.push((format!("{n}.threshold"), v.threshold.clone()));
        }
        sections.push(("gates".to_string(), facts));
    }

    // entrants — files newly entering the top-N hotspot list.
    if !output.hotspots.rank_entrants.is_empty() {
        let mut facts = Vec::new();
        for (i, h) in output.hotspots.rank_entrants.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.path"), h.path.clone()));
            facts.push((format!("{n}.hotspot_score"), fmt_num(h.hotspot_score)));
            facts.push((format!("{n}.revisions"), h.revisions.to_string()));
            facts.push((format!("{n}.cognitive"), fmt_num(h.cognitive)));
            facts.push((format!("{n}.code_health"), fmt_num(h.code_health)));
        }
        sections.push(("entrants".to_string(), facts));
    }

    // score-increased — existing hotspots whose score grew past the threshold.
    if !output.hotspots.score_increased.is_empty() {
        let mut facts = Vec::new();
        for (i, s) in output.hotspots.score_increased.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.path"), s.path.clone()));
            facts.push((format!("{n}.base_score"), fmt_num(s.base_score)));
            facts.push((format!("{n}.head_score"), fmt_num(s.head_score)));
            facts.push((format!("{n}.delta"), fmt_num(s.delta)));
        }
        sections.push(("score-increased".to_string(), facts));
    }

    // absences — historically-coupled files omitted from the PR.
    if !output.coupling_absences.is_empty() {
        let mut facts = Vec::new();
        for (i, a) in output.coupling_absences.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.touched_file"), a.touched_file.clone()));
            facts.push((format!("{n}.expected_partner"), a.expected_partner.clone()));
            facts.push((
                format!("{n}.historical_coupling"),
                fmt_num(a.historical_coupling),
            ));
            facts.push((format!("{n}.fisher_p"), fmt_num(a.fisher_p)));
            facts.push((
                format!("{n}.historical_shared_revs"),
                a.historical_shared_revs.to_string(),
            ));
        }
        sections.push(("absences".to_string(), facts));
    }

    // clones — new clone-family members introduced by the PR.
    if !output.clones.new_families.is_empty() {
        let mut facts = Vec::new();
        for (i, c) in output.clones.new_families.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.clone_group_id"), c.clone_group_id.to_string()));
            facts.push((format!("{n}.entity"), c.entity.clone()));
            facts.push((format!("{n}.function"), c.function.clone()));
            facts.push((format!("{n}.start_line"), c.start_line.to_string()));
            facts.push((format!("{n}.end_line"), c.end_line.to_string()));
            facts.push((format!("{n}.node_count"), c.node_count.to_string()));
        }
        sections.push(("clones".to_string(), facts));
    }

    sections
}

fn init_logging(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("info,codelore=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    // Emit a span-close event with elapsed time whenever a span exits.
    // Enables `RUST_LOG=codelore::bench=info codelore analyze …` to print
    // per-stage timing — no `--bench` flag needed. The CLOSE event is
    // suppressed by default at WARN level, so this has zero overhead for
    // normal runs.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_span_events(FmtSpan::CLOSE)
        .init();
}

#[allow(clippy::too_many_lines)] // long but linear: pre-flight → format-validate → ingest → 43-analysis dispatch → emit; splitting would obscure the top-level orchestration flow
fn analyze(args: &AnalyzeArgs, no_banner: bool) -> Result<()> {
    use codelore_lib::cli_api::output::banner;
    // Bracket the whole run with a wall-clock timer so the footer can report
    // "completed in 4.3s". Started before any work so pre-flight, ingest,
    // analysis, and emit all count toward the displayed duration — matches
    // what `cargo build`'s `Finished in Xs` includes.
    let started_at = std::time::Instant::now();

    let analysis = AnalysisName::from_str(&args.analysis)
        .with_context(|| format!("parsing --analysis {:?}", args.analysis))?;

    let format = args.format.as_str();
    match format {
        "csv" | "json" | "ndjson" | "sarif" | "markdown" | "parquet" | "sqlite" | "html"
        | "spa" | "step-summary" | "gha" => {}
        other => {
            return Err(CodeLoreError::Analysis(format!(
                "unknown --format {other:?}. Supported: csv, json, ndjson, sarif, markdown, parquet, sqlite, html, spa, step-summary, gha"
            ))
            .into());
        }
    }

    // Format constraints. parquet + sqlite are binary fact-store dumps with no
    // sensible default filename, so they still require --output. `spa` defaults
    // to ./.codelore/spa.html under the current directory when --output is
    // omitted (handled in the spa block below).
    if matches!(format, "parquet" | "sqlite") && args.output.is_none() {
        return Err(CodeLoreError::Output(format!(
            "--format {format} requires --output PATH (binary format, cannot stream to stdout)"
        ))
        .into());
    }
    // step-summary can stream to stdout (it's small GFM text), but typically
    // gets redirected to $GITHUB_STEP_SUMMARY by the caller's CI workflow.
    // SARIF: hotspots, clones, clone-coupling.
    if format == "sarif"
        && !matches!(
            analysis,
            AnalysisName::Hotspots | AnalysisName::Clones | AnalysisName::CloneCoupling
        )
    {
        return Err(CodeLoreError::Analysis(
            "--format sarif currently supports --analysis hotspots, clones, and clone-coupling (other analyses land in Plan 9)"
                .to_string(),
        )
        .into());
    }

    let complexity_sample = match args.complexity_sample.as_str() {
        "head" => codelore_lib::cli_api::options::ComplexitySample::Head,
        "adaptive" | "full" => {
            return Err(CodeLoreError::InvalidOptions(
                "--complexity-sample currently supports only 'head'; \
                 'adaptive' and 'full' are not yet available"
                    .to_string(),
            )
            .into());
        }
        other => {
            return Err(CodeLoreError::InvalidOptions(format!(
                "unknown --complexity-sample value {other:?}; expected 'head'"
            ))
            .into());
        }
    };

    let defect_calibration = codelore_lib::cli_api::quality_gates::resolve_defect_calibration(
        args.defect_calibration.clone(),
        &args.repo,
    )
    .context("resolve defect calibration")?;

    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: args.min_revs,
        rows_limit: args.rows,
        complexity_sample,
        group_file: args.group_file.clone(),
        team_map_file: args.team_map_file.clone(),
        explain: args.explain,
        // Canonical lineage defaults to ON. `--no-canonical-lineage`
        // disables it explicitly; `--code-maat-compat` also disables
        // it to preserve bit-for-bit code-maat output (code-maat uses
        // `--no-renames` in its git log so it never canonicalizes).
        use_canonical_lineage: !args.no_canonical_lineage && !args.code_maat_compat,
        exclude_patterns: args.exclude.clone(),
        include_ignored: args.include_ignored,
        min_shared_revs: args.min_shared_revs,
        min_coupling_pct: args.min_coupling_pct,
        max_coupling_pct: args.max_coupling_pct,
        max_changeset_size: args.max_changeset_size,
        age_time_now: args.age_time_now,
        // R2/R3: date-range filters on the commit walk.
        after: args.after,
        before: args.before,
        // R4: opt-in merge-commit inclusion (default off matches code-maat
        // semantics and the GitCliRepo backend).
        include_merges: args.include_merges,
        message_regex: args.message_regex.clone(),
        min_soc: args.min_soc,
        // --code-maat-compat. Implies --strict-grouping (code-maat
        // is always-strict). Other compat behaviors are gated at the
        // analysis / emitter layer.
        code_maat_compat: args.code_maat_compat,
        strict_grouping: args.strict_grouping || args.code_maat_compat,
        // --time-bucket. Maps from the CLI's enum to the lib's enum.
        time_bucket: args.time_bucket.map(Into::into),
        // T8: knowledge-islands analysis "departed author" threshold.
        departed_threshold_days: args.departed_threshold_days,
        window_days: args.window_days,
        knowledge_model: args.knowledge_model.clone(),
        rework_window_days: args.rework_window_days,
        release_tag_glob: args.release_tag_glob.clone(),
        target: args.target.clone(),
        calibration: args.calibration.clone(),
        defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        temp_dir: args.temp_dir.clone(),
        ..Options::default()
    };

    // Catch pathological flag combinations (e.g. --min-coupling 60
    // --max-coupling 30) at the boundary rather than silently producing
    // empty output downstream.
    opts.validate().context("validate options")?;

    // `--time-bucket` is only semantically valid for
    // four analyses (coupling, soc, hotspots, code-health). The other
    // 18 either crash with a Catalog Error (no `changes_bucketed`
    // table) or silently return empty rows (rev-on-rev JOIN
    // against the date-string-keyed bucketed table fails). Reject
    // at the CLI boundary with a descriptive error rather than letting
    // either failure mode surprise the user downstream.
    if opts.time_bucket.is_some() && !analysis.supports_time_bucket() {
        return Err(CodeLoreError::Analysis(format!(
            "--time-bucket is not supported for analysis {:?}. \
             Bucketing only applies to co-change analyses; supported: \
             coupling, soc, hotspots, code-health. Remove --time-bucket \
             or switch to one of those analyses.",
            analysis.as_str()
        ))
        .into());
    }

    let analysis_name = args.analysis.as_str();

    // clones is a HEAD-only filesystem + tree-sitter walk — no git
    // history is needed. Short-circuit BEFORE opening the repo so shallow
    // clones, working trees with uncommitted changes, and "not a git repo"
    // directories all work for any of the 4 clone-supporting output formats
    // (csv | json | markdown | sarif). Earlier this short-circuit only
    // covered `csv`; the other formats fell through to the full git-ingest
    // path, which was both ~10–100× slower and broke entirely on non-git
    // directories — a real user-facing bug closed here.
    if matches!(analysis, AnalysisName::Clones)
        && matches!(format, "csv" | "json" | "markdown" | "sarif")
    {
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };
        let rows =
            codelore_lib::cli_api::analyses::clones::run_clones(&opts).context("run clones")?;
        match format {
            "csv" => {
                codelore_lib::cli_api::output::csv::write_clones_csv(&rows, &mut out)
                    .context("write csv")?;
            }
            "json" => {
                codelore_lib::cli_api::output::json::write_json(&rows, &mut out)
                    .context("write json")?;
            }
            "markdown" => {
                codelore_lib::cli_api::output::markdown::write_clones_markdown(&rows, &mut out)
                    .context("write markdown")?;
            }
            "sarif" => {
                let repo_root = args.repo.display().to_string();
                codelore_lib::cli_api::output::sarif::write_clones_sarif(
                    &rows, &repo_root, &mut out,
                )
                .context("write sarif")?;
            }
            _ => unreachable!("format validated by outer matches!()"),
        }
        return Ok(());
    }

    // Pre-flight banner: opens the repo, runs cheap validity checks (path
    // exists, is a git repo, has at least one commit, --output parent dir
    // is writable), emits the boxed Style-B banner to stderr, and either
    // returns the opened `GixRepo` (Ready) or bails with a banner-shaped
    // error explaining what's wrong. Failure banners ALWAYS print even when
    // `--no-banner` was passed — error feedback is too important to swallow.
    let repo = {
        let _span = tracing::info_span!(target: "codelore::bench", "bench.open_repo").entered();
        preflight_and_open_repo(args, &opts, analysis_name, no_banner)?
    };

    // The persistent cache opens its DuckDB file read-only.
    // SQLite output requires `INSTALL sqlite; LOAD sqlite;` which writes
    // to the DuckDB extension registry on the connected database — that
    // cannot run on a read-only connection. Parquet output uses core
    // DuckDB's built-in `COPY ... TO file.parquet (FORMAT PARQUET)`,
    // which DOES work on read-only. So the bypass narrows from
    // `parquet|sqlite` (both) to `sqlite` only — parquet now benefits
    // from the cache speedup like csv/json/markdown/sarif.
    let needs_writable_db = format == "sqlite";

    let db = {
        let _span =
            tracing::info_span!(target: "codelore::bench", "bench.cache_or_ingest").entered();
        if args.no_cache || needs_writable_db {
            // --no-cache or sqlite output: always fresh in-memory. Still
            // honors `--temp-dir` so a very large repo spills instead of
            // OOM-ing on this bypass-the-cache path.
            let db = FactsDb::new_in_memory_with_temp_dir(opts.temp_dir.as_deref())
                .context("open fact store (in-memory)")?;
            db.ingest(&repo, &opts).context("ingest commits")?;
            db
        } else if let Some(cache_dir) = &args.cache_dir {
            // --cache-dir PATH: use a custom XDG root instead of the default.
            FactsDb::open_or_ingest_with_cache_root(&opts, &repo, cache_dir)
                .context("open or ingest (cache-dir)")?
        } else {
            // Default: use the XDG cache (read-only after first ingest).
            FactsDb::open_or_ingest(&opts, &repo).context("open or ingest")?
        }
    };

    // Parquet + SQLite write to file directly through DuckDB, not via Write trait.
    if format == "parquet" {
        let path = args.output.as_ref().expect("validated above");
        write_parquet(&db, &opts, analysis, path)?;
        write_provenance_sidecar(&db, &opts, analysis_name, path)?;
        return Ok(());
    }
    if format == "sqlite" {
        let path = args.output.as_ref().expect("validated above");
        codelore_lib::cli_api::output::sqlite::write_full_fact_store_sqlite(&db, &opts, path)
            .context("write sqlite")?;
        // No sidecar — provenance table lives inside the SQLite DB.
        return Ok(());
    }
    if format == "spa" {
        // Mirrors `--format sqlite` shape: bypasses the per-analysis
        // match because the SPA is a multi-analysis composite, not a
        // single row-type emission. Each new widget gets a new field
        // on `SpaDashboard` and a new analysis call inside
        // `build_spa_dashboard`.
        #[cfg(feature = "spa")]
        {
            // --output is optional for spa. When omitted, default to
            // `.codelore/spa.html` under the current working directory,
            // creating the `.codelore` dir if needed.
            let default_path = std::path::PathBuf::from(".codelore/spa.html");
            let path: &std::path::Path = if let Some(p) = args.output.as_deref() {
                p
            } else {
                if let Some(parent) = default_path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                &default_path
            };
            run_spa_dispatch(&db, &opts, &args.repo, path)?;
            return Ok(());
        }
        #[cfg(not(feature = "spa"))]
        {
            anyhow::bail!(
                "--format spa requires CodeLore to be built with the `spa` Cargo feature. \
                 Reinstall with `cargo install codelore --features spa`, build from source \
                 with `cargo build --features spa`, or use a prebuilt binary from \
                 https://github.com/emrecdr/codelore/releases (which ship with `spa` enabled)."
            );
        }
    }
    if format == "step-summary" {
        // Reuses the same multi-analysis dispatch as `--format spa` —
        // a step-summary IS a different rendering of the same
        // SpaDashboard, sized for GitHub's $GITHUB_STEP_SUMMARY 1 MB
        // cap. Streams to stdout by default so callers can
        // `>> $GITHUB_STEP_SUMMARY` directly.
        #[cfg(feature = "spa")]
        {
            run_step_summary_dispatch(&db, &opts, &args.repo, args.output.as_deref())?;
            return Ok(());
        }
        #[cfg(not(feature = "spa"))]
        {
            anyhow::bail!(
                "--format step-summary requires CodeLore to be built with the `spa` Cargo feature \
                 (the step-summary writer consumes the same SpaDashboard as the SPA HTML emitter). \
                 Reinstall with `cargo install codelore --features spa`, build from source \
                 with `cargo build --features spa`, or use a prebuilt binary from \
                 https://github.com/emrecdr/codelore/releases (which ship with `spa` enabled)."
            );
        }
    }

    // csv / json / sarif / markdown / html: stream through Write
    {
        let _span =
            tracing::info_span!(target: "codelore::bench", "bench.analyze_and_emit").entered();
        let mut out: Box<dyn Write> = match args.output.as_ref() {
            Some(path) => Box::new(std::fs::File::create(path)?),
            None => Box::new(std::io::stdout().lock()),
        };

        // SARIF needs the repo root; HTML needs the page title plus a
        // generated-at timestamp. Bundle them once and hand the same context
        // to every per-analysis dispatch fn so each can pick the bits its
        // wired formats require. The timestamp is computed unconditionally —
        // it is read only by the HTML page chrome, so it is inert for every
        // other format.
        let now = time::OffsetDateTime::now_utc();
        let generated_at = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second(),
        );
        let ctx = EmitCtx {
            repo_root: args.repo.display().to_string(),
            title: format!("CodeLore: {}", analysis.as_str()),
            generated_at,
            analysis_name: analysis.as_str(),
        };

        match &analysis {
            AnalysisName::Hotspots => dispatch_hotspots(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::HotspotVelocity => {
                dispatch_hotspot_velocity(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CodeHealth => dispatch_code_health(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::CodeAge => dispatch_code_age(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::AbsChurn => dispatch_abs_churn(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::AuthorChurn => {
                dispatch_author_churn(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::EntityChurn => {
                dispatch_entity_churn(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Communication => {
                dispatch_communication(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Ownership => dispatch_ownership(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Coupling => dispatch_coupling(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Summary => dispatch_summary(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Clones => dispatch_clones(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Revisions => dispatch_revisions(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Authors => dispatch_authors(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::TopCommitters => {
                dispatch_top_committers(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::GodClasses => dispatch_god_classes(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::ArchViolations => {
                dispatch_arch_violations(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::DependencyCycles => {
                dispatch_dependency_cycles(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ArchitectureRoles => {
                dispatch_architecture_roles(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Instability => {
                dispatch_instability(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CycleHealth => {
                dispatch_cycle_health(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::DefectValidation => {
                dispatch_defect_validation(&opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ArchitectureMetrics => {
                dispatch_architecture_metrics(&db, &opts, format, &ctx, &mut out)?;
            }
            // The only analysis that needs the Repo (it reads blobs at
            // historical revs), so it gets the opened `repo` directly
            // rather than the (db, opts) signature the rest share.
            AnalysisName::ArchitectureTrend => {
                dispatch_architecture_trend(&db, &repo, &opts, format, &ctx, &mut out)?;
            }
            // Also reads blobs at past revs — same special-case as ArchitectureTrend.
            AnalysisName::HealthTrend => {
                dispatch_health_trend(&db, &repo, &opts, format, &ctx, &mut out)?;
            }
            // Also needs the Repo (reads blobs at past revs while bisecting).
            AnalysisName::CycleOrigins => {
                dispatch_cycle_origins(&db, &repo, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ModularityViolations => {
                dispatch_modularity_violations(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::UnstableInterface => {
                dispatch_unstable_interface(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Crossing => dispatch_crossing(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::StaleCode => dispatch_stale_code(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::PairProgramming => {
                dispatch_pair_programming(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::LeadTime => dispatch_lead_time(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::BusFactor => dispatch_bus_factor(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::DeliveryFriction => {
                dispatch_delivery_friction(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::RefactoringTargets => {
                dispatch_refactoring_targets(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::KnowledgeIslands => {
                dispatch_knowledge_islands(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Soc => dispatch_soc(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Messages => dispatch_messages(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::MainDev => dispatch_main_dev(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::MainDevByRevs => {
                dispatch_main_dev_by_revs(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::MainDevByDeletions => {
                dispatch_main_dev_by_deletions(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::EntityEffort => {
                dispatch_entity_effort(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::EntityOwnership => {
                dispatch_entity_ownership(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CloneCoupling => {
                dispatch_clone_coupling(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::Centrality => dispatch_centrality(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::Communities => dispatch_communities(&db, &opts, format, &ctx, &mut out)?,
            AnalysisName::EffortExposure => {
                dispatch_effort_exposure(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CodeFamiliarity => {
                dispatch_code_familiarity(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::TeamComposition => {
                dispatch_team_composition(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::CoordinationNeeds => {
                dispatch_coordination_needs(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::MarginalOwnerRisk => {
                dispatch_marginal_owner_risk(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::ReleaseCadence => {
                dispatch_release_cadence(&repo, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::DeliveryMetrics => {
                dispatch_delivery_metrics(&db, &opts, format, &ctx, &mut out)?;
            }
            AnalysisName::FunctionXray => {
                let target = opts.target.as_deref().ok_or_else(|| {
                    CodeLoreError::Analysis(
                        "--target <path> is required for function-xray".to_string(),
                    )
                })?;
                dispatch_function_xray(&db, &repo, &opts, target, format, &ctx, &mut out)?;
            }
            AnalysisName::FunctionCoupling => {
                let target = opts.target.as_deref().ok_or_else(|| {
                    CodeLoreError::Analysis(
                        "--target <path> is required for function-coupling".to_string(),
                    )
                })?;
                dispatch_function_coupling(&db, &repo, &opts, target, format, &ctx, &mut out)?;
            }
            AnalysisName::FindingHotspotOverlap => {
                // Requires the external sidecar: open it read-only from the cache
                // dir. open_nonempty returns None when the sidecar is absent OR
                // present-but-empty — both mean "no findings ingested yet", and
                // both surface the same pre-condition error here. We handle the
                // None case at this layer so we never create an empty sidecar as
                // a side-effect of an analysis read.
                let cache_root = args
                    .cache_dir
                    .clone()
                    .unwrap_or_else(codelore_lib::cli_api::cache::default_cache_root);
                let store = codelore_lib::cli_api::external::ExternalStore::open_nonempty(
                    &cache_root,
                    &args.repo,
                )
                .context("open external findings store")?
                .ok_or_else(|| {
                    codelore_lib::cli_api::CodeLoreError::Analysis(
                        "finding-hotspot-overlap requires prior `codelore ingest-sarif` \
                         (no external findings found)"
                            .to_string(),
                    )
                })?;
                dispatch_finding_hotspot_overlap(&db, &opts, &store, format, &ctx, &mut out)?;
            }
        }
    } // out is dropped here, flushing any buffered writes

    if let Some(path) = args.output.as_ref() {
        write_provenance_sidecar(&db, &opts, analysis_name, path)?;
    }

    // Footer: closes the bracket opened by the pre-flight banner. Shows total
    // wall-clock time (humanised: "234ms" / "4.3s" / "2m 34s"). Suppressed
    // under the same conditions as the header — same `should_print` policy
    // ensures piped invocations stay clean.
    if banner::should_print(false, no_banner) {
        let footer = banner::Footer {
            analysis: analysis_name,
            elapsed: started_at.elapsed(),
            // Row counts plumbed through every (format, analysis) match arm
            // is a bigger refactor; deferred. The duration + analysis-name
            // line carries the bulk of the post-run UX value.
            rows: None,
        };
        eprint!("{}", footer.render(banner::should_color()));
    }

    Ok(())
}

/// Side-channel context every per-analysis dispatch fn needs beyond `db`,
/// `opts`, and `format`: SARIF wants the repo root; HTML wants the page title,
/// a generated-at timestamp, and the analysis name (for the not-wired message).
struct EmitCtx {
    repo_root: String,
    title: String,
    generated_at: String,
    analysis_name: &'static str,
}

/// The verbatim error for `--format html` on an analysis whose row type is not
/// yet wired through the generic HTML emitter. Shared so every non-HTML
/// dispatch fn reports the same coverage list.
fn html_not_wired(analysis_name: &str) -> anyhow::Error {
    CodeLoreError::Analysis(format!(
        "--format html for analysis `{analysis_name}` not yet wired (covered: hotspots, \
         code-health, knowledge-islands, clone-coupling, summary, revisions, \
         authors, top-committers — file an issue if you need another)"
    ))
    .into()
}

/// The verbatim error for an output `--format` a given analysis's dispatch
/// fn doesn't wire. Shared so every per-analysis fallback arm reports the
/// same `CodeLoreError::Analysis` shape — `"<analysis> analysis supports
/// <list>; got <fmt>"` — keeping the analysis-failure exit code uniform.
fn unsupported_format(analysis_name: &str, supported: &str, fmt: &str) -> anyhow::Error {
    CodeLoreError::Analysis(format!(
        "{analysis_name} analysis supports {supported}; got {fmt:?}"
    ))
    .into()
}

fn dispatch_revisions(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::csv::write_revisions_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::json::write_revisions_json(&rows, out)
                .context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::markdown::write_revisions_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::revisions::run_revisions(db, opts)
                .context("run revisions")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "revisions",
                "csv|json|markdown|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_hotspot_velocity(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::hotspot_velocity::run_hotspot_velocity(db, opts)
                    .context("run hotspot-velocity")?;
            codelore_lib::cli_api::output::csv::write_hotspot_velocity_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::hotspot_velocity::run_hotspot_velocity(db, opts)
                    .context("run hotspot-velocity")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::hotspot_velocity::run_hotspot_velocity(db, opts)
                    .context("run hotspot-velocity")?;
            codelore_lib::cli_api::output::markdown::write_hotspot_velocity_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "hotspot-velocity",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_hotspots(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::csv::write_hotspots_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::markdown::write_hotspots_markdown(&rows, out)
                .context("write markdown")?;
        }
        "sarif" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::sarif::write_hotspots_sarif(&rows, &ctx.repo_root, out)
                .context("write sarif")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "gha" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::gha::write_hotspots_gha(&rows, out)
                .context("write gha")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
                .context("run hotspots")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "hotspots",
                "csv|json|markdown|sarif|ndjson|gha|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_code_health(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    // Analyze has no --quiet; the notice self-suppresses off a TTY.
    notice_corpus_lens_absent(opts, false);
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::csv::write_code_health_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::markdown::write_code_health_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
                .context("run code-health")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "code-health",
                "csv|json|markdown|ndjson|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_code_age(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::code_age::run_code_age(db, opts)
                .context("run code-age")?;
            codelore_lib::cli_api::output::csv::write_code_age_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::code_age::run_code_age(db, opts)
                .context("run code-age")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::code_age::run_code_age(db, opts)
                .context("run code-age")?;
            codelore_lib::cli_api::output::markdown::write_code_age_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("code-age", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_abs_churn(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_abs_churn(db, opts)
                .context("run abs-churn")?;
            codelore_lib::cli_api::output::csv::write_abs_churn_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_abs_churn(db, opts)
                .context("run abs-churn")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_abs_churn(db, opts)
                .context("run abs-churn")?;
            codelore_lib::cli_api::output::markdown::write_abs_churn_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("abs-churn", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_author_churn(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_author_churn(db, opts)
                .context("run author-churn")?;
            codelore_lib::cli_api::output::csv::write_author_churn_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_author_churn(db, opts)
                .context("run author-churn")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_author_churn(db, opts)
                .context("run author-churn")?;
            codelore_lib::cli_api::output::markdown::write_author_churn_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("author-churn", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_entity_churn(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_entity_churn(db, opts)
                .context("run entity-churn")?;
            codelore_lib::cli_api::output::csv::write_entity_churn_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_entity_churn(db, opts)
                .context("run entity-churn")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::churn::run_entity_churn(db, opts)
                .context("run entity-churn")?;
            codelore_lib::cli_api::output::markdown::write_entity_churn_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("entity-churn", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_communication(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::communication::run_communication(db, opts)
                .context("run communication")?;
            codelore_lib::cli_api::output::csv::write_communication_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::communication::run_communication(db, opts)
                .context("run communication")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::communication::run_communication(db, opts)
                .context("run communication")?;
            codelore_lib::cli_api::output::markdown::write_communication_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "communication",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_ownership(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::ownership::run_ownership(db, opts)
                .context("run ownership")?;
            codelore_lib::cli_api::output::csv::write_ownership_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::ownership::run_ownership(db, opts)
                .context("run ownership")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::ownership::run_ownership(db, opts)
                .context("run ownership")?;
            codelore_lib::cli_api::output::markdown::write_ownership_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("ownership", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_coupling(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::csv::write_coupling_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::markdown::write_coupling_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
                .context("run coupling")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "coupling",
                "csv|json|markdown|ndjson",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_summary(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::csv::write_summary_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::markdown::write_summary_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
                .context("run summary")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => return Err(unsupported_format("summary", "csv|json|markdown|html", fmt)),
    }
    Ok(())
}

fn dispatch_clones(
    _db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    // csv | json | markdown | sarif are short-circuited before the repo opens
    // (a HEAD-only tree-sitter walk needs no history); the arms remain here so
    // the wired-format set stays the single source of truth for clones.
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::csv::write_clones_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::markdown::write_clones_markdown(&rows, out)
                .context("write markdown")?;
        }
        "sarif" => {
            let rows =
                codelore_lib::cli_api::analyses::clones::run_clones(opts).context("run clones")?;
            codelore_lib::cli_api::output::sarif::write_clones_sarif(&rows, &ctx.repo_root, out)
                .context("write sarif")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("clones", "csv|json|markdown|sarif", fmt)),
    }
    Ok(())
}

fn dispatch_authors(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::csv::write_authors_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::markdown::write_authors_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows = codelore_lib::cli_api::analyses::authors::run_authors(db, opts)
                .context("run authors")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => return Err(unsupported_format("authors", "csv|json|markdown|html", fmt)),
    }
    Ok(())
}

fn dispatch_top_committers(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::csv::write_top_committers_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::markdown::write_top_committers_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows =
                codelore_lib::cli_api::analyses::top_committers::run_top_committers(db, opts)
                    .context("run top-committers")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "top-committers",
                "csv|json|markdown|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_god_classes(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::god_classes::run_god_classes(db, opts)
                .context("run god-classes")?;
            codelore_lib::cli_api::output::csv::write_god_classes_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::god_classes::run_god_classes(db, opts)
                .context("run god-classes")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::god_classes::run_god_classes(db, opts)
                .context("run god-classes")?;
            codelore_lib::cli_api::output::markdown::write_god_classes_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("god-classes", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_arch_violations(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::arch_violations::run_arch_violations(db, opts)
                    .context("run architecture-violations")?;
            codelore_lib::cli_api::output::csv::write_arch_violations_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::arch_violations::run_arch_violations(db, opts)
                    .context("run architecture-violations")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::arch_violations::run_arch_violations(db, opts)
                    .context("run architecture-violations")?;
            codelore_lib::cli_api::output::markdown::write_arch_violations_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-violations",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_instability(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::instability::run_instability(db, opts)
                .context("run instability")?;
            codelore_lib::cli_api::output::csv::write_instability_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::instability::run_instability(db, opts)
                .context("run instability")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::instability::run_instability(db, opts)
                .context("run instability")?;
            codelore_lib::cli_api::output::markdown::write_instability_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("instability", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_cycle_origins(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::cycle_origins::run_cycle_origins(db, repo, opts)
                    .context("run cycle-origins")?;
            codelore_lib::cli_api::output::csv::write_cycle_origins_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::cycle_origins::run_cycle_origins(db, repo, opts)
                    .context("run cycle-origins")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::cycle_origins::run_cycle_origins(db, repo, opts)
                    .context("run cycle-origins")?;
            codelore_lib::cli_api::output::markdown::write_cycle_origins_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "cycle-origins",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_architecture_trend(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, repo, opts,
            )
            .context("run architecture-trend")?;
            codelore_lib::cli_api::output::csv::write_architecture_trend_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, repo, opts,
            )
            .context("run architecture-trend")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, repo, opts,
            )
            .context("run architecture-trend")?;
            codelore_lib::cli_api::output::markdown::write_architecture_trend_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-trend",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_health_trend(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)
                    .context("run health-trend")?;
            codelore_lib::cli_api::output::csv::write_health_trend_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)
                    .context("run health-trend")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::health_trend::run_health_trend(db, repo, opts)
                    .context("run health-trend")?;
            codelore_lib::cli_api::output::markdown::write_health_trend_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format("health-trend", "csv|json|markdown", fmt));
        }
    }
    Ok(())
}

fn dispatch_cycle_health(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::cycle_health::run_cycle_health(db, opts)
                .context("run cycle-health")?;
            codelore_lib::cli_api::output::csv::write_cycle_health_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::cycle_health::run_cycle_health(db, opts)
                .context("run cycle-health")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::cycle_health::run_cycle_health(db, opts)
                .context("run cycle-health")?;
            codelore_lib::cli_api::output::markdown::write_cycle_health_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format("cycle-health", "csv|json|markdown", fmt));
        }
    }
    Ok(())
}

/// Reads a defect-calibration artifact (never the fact store) and emits its
/// evidence rows, so it takes no `FactsDb` — unlike the other dispatchers.
fn dispatch_defect_validation(
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::defect_validation::run_defect_validation(opts)
                    .context("run defect-validation")?;
            codelore_lib::cli_api::output::csv::write_defect_validation_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::defect_validation::run_defect_validation(opts)
                    .context("run defect-validation")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::defect_validation::run_defect_validation(opts)
                    .context("run defect-validation")?;
            codelore_lib::cli_api::output::markdown::write_defect_validation_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "defect-validation",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_architecture_metrics(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                    db, opts,
                )
                .context("run architecture-metrics")?;
            codelore_lib::cli_api::output::csv::write_architecture_metrics_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                    db, opts,
                )
                .context("run architecture-metrics")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                    db, opts,
                )
                .context("run architecture-metrics")?;
            codelore_lib::cli_api::output::markdown::write_architecture_metrics_markdown(
                &rows, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-metrics",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_architecture_roles(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
                db, opts,
            )
            .context("run architecture-roles")?;
            codelore_lib::cli_api::output::csv::write_architecture_roles_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
                db, opts,
            )
            .context("run architecture-roles")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
                db, opts,
            )
            .context("run architecture-roles")?;
            codelore_lib::cli_api::output::markdown::write_architecture_roles_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "architecture-roles",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_dependency_cycles(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::dependency_cycles::run_dependency_cycles(db, opts)
                    .context("run dependency-cycles")?;
            codelore_lib::cli_api::output::csv::write_dependency_cycles_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::dependency_cycles::run_dependency_cycles(db, opts)
                    .context("run dependency-cycles")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::dependency_cycles::run_dependency_cycles(db, opts)
                    .context("run dependency-cycles")?;
            codelore_lib::cli_api::output::markdown::write_dependency_cycles_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "dependency-cycles",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_modularity_violations(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(
                    db, opts,
                )
                .context("run modularity-violations")?;
            codelore_lib::cli_api::output::csv::write_modularity_violations_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(
                    db, opts,
                )
                .context("run modularity-violations")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(
                    db, opts,
                )
                .context("run modularity-violations")?;
            codelore_lib::cli_api::output::markdown::write_modularity_violations_markdown(
                &rows, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "modularity-violations",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_crossing(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::crossing::run_crossing(db, opts)
                .context("run crossing")?;
            codelore_lib::cli_api::output::csv::write_crossing_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::crossing::run_crossing(db, opts)
                .context("run crossing")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::crossing::run_crossing(db, opts)
                .context("run crossing")?;
            codelore_lib::cli_api::output::markdown::write_crossing_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("crossing", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_unstable_interface(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(
                db, opts,
            )
            .context("run unstable-interface")?;
            codelore_lib::cli_api::output::csv::write_unstable_interface_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(
                db, opts,
            )
            .context("run unstable-interface")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(
                db, opts,
            )
            .context("run unstable-interface")?;
            codelore_lib::cli_api::output::markdown::write_unstable_interface_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "unstable-interface",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_stale_code(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::stale_code::run_stale_code(db, opts)
                .context("run stale-code")?;
            codelore_lib::cli_api::output::csv::write_stale_code_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::stale_code::run_stale_code(db, opts)
                .context("run stale-code")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::stale_code::run_stale_code(db, opts)
                .context("run stale-code")?;
            codelore_lib::cli_api::output::markdown::write_stale_code_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("stale-code", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_pair_programming(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::pair_programming::run_pair_programming(db, opts)
                    .context("run pair-programming")?;
            codelore_lib::cli_api::output::csv::write_pair_programming_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::pair_programming::run_pair_programming(db, opts)
                    .context("run pair-programming")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::pair_programming::run_pair_programming(db, opts)
                    .context("run pair-programming")?;
            codelore_lib::cli_api::output::markdown::write_pair_programming_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "pair-programming",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_lead_time(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::csv::write_lead_time_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::markdown::write_lead_time_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            let rows = codelore_lib::cli_api::analyses::lead_time::run_lead_time(db, opts)
                .context("run lead-time")?;
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "lead-time",
                "csv|json|ndjson|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_bus_factor(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::bus_factor::run_bus_factor(db, opts)
                .context("run bus-factor")?;
            codelore_lib::cli_api::output::csv::write_bus_factor_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::bus_factor::run_bus_factor(db, opts)
                .context("run bus-factor")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::bus_factor::run_bus_factor(db, opts)
                .context("run bus-factor")?;
            codelore_lib::cli_api::output::markdown::write_bus_factor_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("bus-factor", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_delivery_friction(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
                    .context("run delivery-friction")?;
            codelore_lib::cli_api::output::csv::write_delivery_friction_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
                    .context("run delivery-friction")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
                    .context("run delivery-friction")?;
            codelore_lib::cli_api::output::markdown::write_delivery_friction_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "delivery-friction",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_effort_exposure(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
                    .context("run effort-exposure")?;
            codelore_lib::cli_api::output::csv::write_effort_exposure_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
                    .context("run effort-exposure")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
                    .context("run effort-exposure")?;
            codelore_lib::cli_api::output::markdown::write_effort_exposure_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "effort-exposure",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_code_familiarity(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                    .context("run code-familiarity")?;
            codelore_lib::cli_api::output::csv::write_code_familiarity_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                    .context("run code-familiarity")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
                    .context("run code-familiarity")?;
            codelore_lib::cli_api::output::markdown::write_code_familiarity_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "code-familiarity",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_team_composition(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
                    .context("run team-composition")?;
            codelore_lib::cli_api::output::csv::write_team_composition_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
                    .context("run team-composition")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
                    .context("run team-composition")?;
            codelore_lib::cli_api::output::markdown::write_team_composition_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "team-composition",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_coordination_needs(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(
                db, opts,
            )
            .context("run coordination-needs")?;
            codelore_lib::cli_api::output::csv::write_coordination_needs_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(
                db, opts,
            )
            .context("run coordination-needs")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(
                db, opts,
            )
            .context("run coordination-needs")?;
            codelore_lib::cli_api::output::markdown::write_coordination_needs_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "coordination-needs",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_marginal_owner_risk(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(
                    db, opts,
                )
                .context("run marginal-owner-risk")?;
            codelore_lib::cli_api::output::csv::write_marginal_owner_risk_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(
                    db, opts,
                )
                .context("run marginal-owner-risk")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(
                    db, opts,
                )
                .context("run marginal-owner-risk")?;
            codelore_lib::cli_api::output::markdown::write_marginal_owner_risk_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "marginal-owner-risk",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_finding_hotspot_overlap(
    db: &FactsDb,
    opts: &Options,
    store: &codelore_lib::cli_api::external::ExternalStore,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    // Reject unsupported formats before running the (non-trivial) analysis, so
    // a bad format still surfaces the format error rather than an analysis error.
    match format {
        "csv" | "json" | "markdown" => {}
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "finding-hotspot-overlap",
                "csv|json|markdown",
                fmt,
            ));
        }
    }

    let rows =
        codelore_lib::cli_api::analyses::finding_hotspot_overlap::run_finding_hotspot_overlap(
            db, opts, store,
        )
        .context("run finding-hotspot-overlap")?;
    match format {
        "csv" => codelore_lib::cli_api::output::csv::write_finding_hotspot_overlap_csv(&rows, out)
            .context("write csv"),
        "json" => codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json"),
        // The format was validated above; only the three writer formats remain.
        _ => codelore_lib::cli_api::output::markdown::write_finding_hotspot_overlap_markdown(
            &rows, out,
        )
        .context("write markdown"),
    }
}

fn dispatch_release_cadence(
    repo: &GixRepo,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(repo, opts)
                    .context("run release-cadence")?;
            codelore_lib::cli_api::output::csv::write_release_cadence_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(repo, opts)
                    .context("run release-cadence")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(repo, opts)
                    .context("run release-cadence")?;
            codelore_lib::cli_api::output::markdown::write_release_cadence_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "release-cadence",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_delivery_metrics(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, opts)
                    .context("run delivery-metrics")?;
            codelore_lib::cli_api::output::csv::write_delivery_metrics_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, opts)
                    .context("run delivery-metrics")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, opts)
                    .context("run delivery-metrics")?;
            codelore_lib::cli_api::output::markdown::write_delivery_metrics_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "delivery-metrics",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_function_xray(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    target: &str,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                db, repo, opts, target,
            )
            .context("run function-xray")?;
            codelore_lib::cli_api::output::csv::write_function_xray_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                db, repo, opts, target,
            )
            .context("run function-xray")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                db, repo, opts, target,
            )
            .context("run function-xray")?;
            codelore_lib::cli_api::output::markdown::write_function_xray_markdown(
                &rows, target, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "function-xray",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_function_coupling(
    db: &FactsDb,
    repo: &GixRepo,
    opts: &Options,
    target: &str,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::function_coupling::run_function_coupling(
                db, repo, opts, target,
            )
            .context("run function-coupling")?;
            codelore_lib::cli_api::output::csv::write_function_coupling_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::function_coupling::run_function_coupling(
                db, repo, opts, target,
            )
            .context("run function-coupling")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::function_coupling::run_function_coupling(
                db, repo, opts, target,
            )
            .context("run function-coupling")?;
            codelore_lib::cli_api::output::markdown::write_function_coupling_markdown(
                &rows, target, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "function-coupling",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_refactoring_targets(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    let rows =
        codelore_lib::cli_api::analyses::refactoring_targets::run_refactoring_targets(db, opts)
            .context("run refactoring-targets")?;
    match format {
        "csv" => codelore_lib::cli_api::output::csv::write_refactoring_targets_csv(&rows, out)
            .context("write csv")?,
        "json" => {
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            codelore_lib::cli_api::output::markdown::write_refactoring_targets_markdown(&rows, out)
                .context("write markdown")?;
        }
        "ndjson" => {
            codelore_lib::cli_api::output::ndjson::write_ndjson(&rows, out)
                .context("write ndjson")?;
        }
        "html" => {
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "refactoring-targets",
                "csv|json|markdown|ndjson|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_knowledge_islands(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::csv::write_knowledge_islands_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::markdown::write_knowledge_islands_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => {
            let rows =
                codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
                    .context("run knowledge-islands")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "knowledge-islands",
                "csv|json|markdown|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_soc(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::soc::run_soc(db, opts).context("run soc")?;
            codelore_lib::cli_api::output::csv::write_soc_csv(&rows, out).context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::soc::run_soc(db, opts).context("run soc")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::soc::run_soc(db, opts).context("run soc")?;
            codelore_lib::cli_api::output::markdown::write_soc_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("soc", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_messages(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::messages::run_messages(db, opts)
                .context("run messages")?;
            codelore_lib::cli_api::output::csv::write_messages_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::messages::run_messages(db, opts)
                .context("run messages")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::messages::run_messages(db, opts)
                .context("run messages")?;
            codelore_lib::cli_api::output::markdown::write_messages_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("messages", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_main_dev(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev(db, opts)
                .context("run main-dev")?;
            codelore_lib::cli_api::output::csv::write_main_dev_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev(db, opts)
                .context("run main-dev")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev(db, opts)
                .context("run main-dev")?;
            codelore_lib::cli_api::output::markdown::write_main_dev_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("main-dev", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_main_dev_by_revs(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_revs(db, opts)
                .context("run main-dev-by-revs")?;
            codelore_lib::cli_api::output::csv::write_main_dev_by_revs_csv(
                &rows,
                out,
                opts.code_maat_compat,
            )
            .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_revs(db, opts)
                .context("run main-dev-by-revs")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_revs(db, opts)
                .context("run main-dev-by-revs")?;
            codelore_lib::cli_api::output::markdown::write_main_dev_by_revs_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "main-dev-by-revs",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_main_dev_by_deletions(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_deletions(db, opts)
                    .context("run main-dev-by-deletions")?;
            codelore_lib::cli_api::output::csv::write_main_dev_by_deletions_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_deletions(db, opts)
                    .context("run main-dev-by-deletions")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::main_dev::run_main_dev_by_deletions(db, opts)
                    .context("run main-dev-by-deletions")?;
            codelore_lib::cli_api::output::markdown::write_main_dev_by_deletions_markdown(
                &rows, out,
            )
            .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "main-dev-by-deletions",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_entity_effort(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::entity_effort::run_entity_effort(db, opts)
                .context("run entity-effort")?;
            codelore_lib::cli_api::output::csv::write_entity_effort_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::entity_effort::run_entity_effort(db, opts)
                .context("run entity-effort")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::entity_effort::run_entity_effort(db, opts)
                .context("run entity-effort")?;
            codelore_lib::cli_api::output::markdown::write_entity_effort_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "entity-effort",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_entity_ownership(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
                    .context("run entity-ownership")?;
            codelore_lib::cli_api::output::csv::write_entity_ownership_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
                    .context("run entity-ownership")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
                    .context("run entity-ownership")?;
            codelore_lib::cli_api::output::markdown::write_entity_ownership_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => {
            return Err(unsupported_format(
                "entity-ownership",
                "csv|json|markdown",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_clone_coupling(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::csv::write_clone_coupling_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::markdown::write_clone_coupling_markdown(&rows, out)
                .context("write markdown")?;
        }
        "sarif" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::sarif::write_clone_coupling_sarif(
                &rows,
                &ctx.repo_root,
                out,
            )
            .context("write sarif")?;
        }
        "html" => {
            let rows =
                codelore_lib::cli_api::analyses::clone_coupling::run_clone_coupling(db, opts)
                    .context("run clone-coupling")?;
            codelore_lib::cli_api::output::html::write_html(
                &rows,
                out,
                &ctx.title,
                &ctx.repo_root,
                &ctx.generated_at,
            )
            .context("write html")?;
        }
        fmt => {
            return Err(unsupported_format(
                "clone-coupling",
                "csv|json|markdown|sarif|html",
                fmt,
            ));
        }
    }
    Ok(())
}

fn dispatch_centrality(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let rows = codelore_lib::cli_api::analyses::centrality::run_centrality(db, opts)
                .context("run centrality")?;
            codelore_lib::cli_api::output::csv::write_centrality_csv(&rows, out)
                .context("write csv")?;
        }
        "json" => {
            let rows = codelore_lib::cli_api::analyses::centrality::run_centrality(db, opts)
                .context("run centrality")?;
            codelore_lib::cli_api::output::json::write_json(&rows, out).context("write json")?;
        }
        "markdown" => {
            let rows = codelore_lib::cli_api::analyses::centrality::run_centrality(db, opts)
                .context("run centrality")?;
            codelore_lib::cli_api::output::markdown::write_centrality_markdown(&rows, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("centrality", "csv|json|markdown", fmt)),
    }
    Ok(())
}

fn dispatch_communities(
    db: &FactsDb,
    opts: &Options,
    format: &str,
    ctx: &EmitCtx,
    out: &mut Box<dyn Write>,
) -> Result<()> {
    match format {
        "csv" => {
            let result = codelore_lib::cli_api::analyses::communities::run_communities(db, opts)
                .context("run communities")?;
            codelore_lib::cli_api::output::csv::write_communities_csv(&result, out)
                .context("write csv")?;
        }
        "json" => {
            let result = codelore_lib::cli_api::analyses::communities::run_communities(db, opts)
                .context("run communities")?;
            codelore_lib::cli_api::output::json::write_communities_json(&result, out)
                .context("write json")?;
        }
        "markdown" => {
            let result = codelore_lib::cli_api::analyses::communities::run_communities(db, opts)
                .context("run communities")?;
            codelore_lib::cli_api::output::markdown::write_communities_markdown(&result, out)
                .context("write markdown")?;
        }
        "html" => return Err(html_not_wired(ctx.analysis_name)),
        fmt => return Err(unsupported_format("communities", "csv|json|markdown", fmt)),
    }
    Ok(())
}

/// Pre-flight: cheap validations BEFORE the expensive ingest. Builds the
/// Style-B banner from `Options` + `GixRepo` state, prints to stderr (always
/// on failure, conditionally on success per `should_print`), and either
/// returns the opened repo (Ready) or bails with a clear error.
///
/// Checks run in order:
/// 1. `--repo` path exists on the filesystem
/// 2. Path opens as a git repository (gix-recognised)
/// 3. Repository has at least one commit (HEAD resolves)
/// 4. `--output` parent directory exists (catches typos before the 30s ingest)
fn preflight_and_open_repo(
    args: &AnalyzeArgs,
    opts: &Options,
    analysis_name: &str,
    no_banner: bool,
) -> Result<GixRepo> {
    use codelore_lib::cli_api::output::banner::{self, Banner, Preflight};
    use codelore_lib::cli_api::provenance::{DUCKDB_VERSION, GIX_VERSION};

    let repo_path_str = args.repo.display().to_string();
    let options_summary = format_options_summary(opts);
    let pkg_version = env!("CARGO_PKG_VERSION");

    // Helper: build a Banner with the given pre-flight state. Shared shell
    // keeps the failure renders consistent regardless of which check tripped.
    let make_banner =
        |branch: Option<String>, head_short: Option<String>, preflight: Preflight| Banner {
            codelore_version: pkg_version,
            gix_version: GIX_VERSION,
            duckdb_version: DUCKDB_VERSION,
            repo_path: repo_path_str.clone(),
            branch,
            head_short,
            analysis: analysis_name,
            options_summary: options_summary.clone(),
            preflight,
        };

    // Pre-flight failures MUST surface as typed `CodeLoreError` variants so
    // the spec §6.6 exit-code mapping in `main()` picks them up (Repo → 3,
    // Output → 5). A bare `anyhow::bail!` here would slip the chain and fall
    // through to the default exit code 1, breaking integration tests like
    // `invalid_repo_exits_with_code_3` and surprising any orchestrator that
    // dispatches on exit codes.

    // Step 1: does the path even exist on disk?
    if !args.repo.exists() {
        let b = make_banner(
            None,
            None,
            Preflight::RepoPathMissing {
                repo_path: repo_path_str.clone(),
            },
        );
        eprint!("{}", b.render(banner::should_color()));
        return Err(
            CodeLoreError::Repo(format!("--repo path does not exist: {repo_path_str}")).into(),
        );
    }

    // Step 2: open as git repo. gix returns an error for non-repo paths;
    // we translate into the `NotARepository` preflight variant for the user.
    // `GixRepo::open` already returns a `CodeLoreError::Repo`, so wrapping
    // with `anyhow::Error::new(e)` preserves the typed variant in the chain.
    let repo = match GixRepo::open(&args.repo) {
        Ok(r) => r,
        Err(e) => {
            let b = make_banner(
                None,
                None,
                Preflight::NotARepository {
                    repo_path: repo_path_str.clone(),
                },
            );
            eprint!("{}", b.render(banner::should_color()));
            return Err(anyhow::Error::new(e).context("open repo"));
        }
    };

    // Step 3: HEAD points to a commit. An empty repo (post `git init`, no
    // commits yet) makes every history-based analysis return nothing useful.
    let Ok(head_sha) = repo.head_sha() else {
        let b = make_banner(repo.head_branch_name(), None, Preflight::EmptyRepository);
        eprint!("{}", b.render(banner::should_color()));
        return Err(CodeLoreError::Repo("repository has no commits".to_string()).into());
    };
    let head_short: String = head_sha.chars().take(7).collect();
    let branch = repo.head_branch_name();

    // Step 4: `--output` parent dir exists. Fail-fast saves the user from
    // waiting 30s through an ingest only to discover the directory was a
    // typo. We don't actually try to open a write handle (that races with
    // legitimate file creation by the emitter); just check the parent dir.
    if let Some(out_path) = &args.output
        && let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        let b = make_banner(
            branch.clone(),
            Some(head_short.clone()),
            Preflight::OutputNotWritable {
                path: out_path.display().to_string(),
                reason: format!("parent directory does not exist: {}", parent.display()),
            },
        );
        eprint!("{}", b.render(banner::should_color()));
        return Err(CodeLoreError::Output(format!(
            "--output parent directory does not exist: {}",
            parent.display()
        ))
        .into());
    }

    // All green — print the Ready banner if conditions allow (TTY + not
    // suppressed), then hand the opened repo back to the caller.
    let b = make_banner(branch, Some(head_short), Preflight::Ready);
    if banner::should_print(false, no_banner) {
        eprint!("{}", b.render(banner::should_color()));
    }
    Ok(repo)
}

/// One-line summary of the tuning knobs that mattered for this run. Goes
/// into the banner's `Analysis:` row alongside the analysis name. We only
/// surface a small curated set — the full canonical Options JSON lives in
/// the provenance sidecar for reproducibility audits.
fn format_options_summary(opts: &Options) -> String {
    let mut parts = vec![format!("min-revs={}", opts.min_revs)];
    if let Some(n) = opts.rows_limit {
        parts.push(format!("rows={n}"));
    }
    if opts.include_merges {
        parts.push("merges=on".to_string());
    }
    if opts.code_maat_compat {
        parts.push("code-maat-compat".to_string());
    }
    if opts.explain {
        parts.push("explain".to_string());
    }
    parts.join(", ")
}

fn write_provenance_sidecar(
    db: &FactsDb,
    opts: &Options,
    analysis_name: &str,
    output_path: &std::path::Path,
) -> Result<()> {
    let manifest = codelore_lib::cli_api::provenance::Manifest::capture(db, opts, analysis_name)
        .context("capture provenance manifest")?;
    let json = manifest
        .to_json()
        .context("serialize provenance manifest")?;
    let sidecar = std::path::PathBuf::from(format!("{}.provenance.json", output_path.display()));

    // Atomic-rename pattern: write the sidecar payload to a temp file
    // first, fsync it, then atomically rename into place. Closes the
    // crash window where a half-written .provenance.json on disk would
    // be indistinguishable from a complete one. Downstream consumers
    // (SLSA verifiers, CI gates) can rely on "if the file exists,
    // its contents are complete and durable".
    //
    // The pid suffix lets concurrent codelore runs against the same
    // output path stay isolated (the last writer wins, and intermediate
    // writers don't trample each other's tmp files mid-write).
    //
    // Caveat: this only makes the SIDECAR atomic — the main output
    // (parquet, CSV, etc.) was already dropped by the caller before
    // we got here. Power-loss between the BufWriter drop and the
    // rename below can still leave a main output without a sidecar.
    // That's the residual gap; closing it would require restructuring
    // the output emitters to expose a sync_all hook on the main
    // handle, which is out of scope for the sidecar atomicity fix.
    let mut tmp_name = sidecar.as_os_str().to_owned();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp_path = std::path::PathBuf::from(tmp_name);

    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp_path).with_context(|| {
            format!(
                "create provenance sidecar tmp file at {}",
                tmp_path.display()
            )
        })?;
        f.write_all(json.as_bytes()).with_context(|| {
            format!("write provenance sidecar payload to {}", tmp_path.display())
        })?;
        f.sync_all().with_context(|| {
            format!(
                "fsync provenance sidecar tmp file at {}",
                tmp_path.display()
            )
        })?;
    } // File dropped → OS handle closed before rename.

    std::fs::rename(&tmp_path, &sidecar).with_context(|| {
        // On rename failure, clean up the orphan tmp file best-effort.
        // We can't surface a secondary error here because we're already
        // returning the rename's error; the orphan would be left for
        // the next process-id collision to clobber.
        let _ = std::fs::remove_file(&tmp_path);
        format!(
            "atomically rename {} -> {}",
            tmp_path.display(),
            sidecar.display()
        )
    })?;
    Ok(())
}

fn write_parquet(
    db: &FactsDb,
    opts: &Options,
    analysis: AnalysisName,
    path: &std::path::Path,
) -> Result<()> {
    match analysis {
        AnalysisName::Hotspots => {
            codelore_lib::cli_api::output::parquet::write_hotspots_parquet(db, opts, path)
                .context("write parquet")
        }
        AnalysisName::Revisions => {
            codelore_lib::cli_api::output::parquet::write_revisions_parquet(db, opts, path)
                .context("write parquet")
        }
        AnalysisName::Summary => {
            codelore_lib::cli_api::output::parquet::write_summary_parquet(db, opts, path)
                .context("write parquet")
        }
        other => anyhow::bail!(
            "--format parquet currently supports hotspots, revisions, summary only \
             (Plan 5 scope); got {other:?}"
        ),
    }
}

/// Run every analysis the dashboard consumes and assemble a
/// `SpaDashboard`. Shared by both `--format spa` (HTML emitter) and
/// `--format step-summary` (GFM markdown emitter) — they render the
/// same data into different output shapes.
#[cfg(feature = "spa")]
#[allow(clippy::too_many_lines)] // one run-each-analysis-then-assemble sequence
fn build_spa_dashboard(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    repo_path: &std::path::Path,
) -> anyhow::Result<codelore_lib::cli_api::output::spa::SpaDashboard> {
    use codelore_lib::cli_api::analyses::dashboard::{
        run_clone_summary, run_daily_commits, run_trends, run_xray,
    };
    use codelore_lib::cli_api::output::spa::SpaDashboard;

    // Each run_* call is an SQL query over the already-ingested fact
    // store, so the composite cost is bounded by the query mix
    // (typically <1s on mid-size repos). Optional widgets degrade
    // gracefully (warn + empty vec) on tiny fixtures or when the
    // analysis can't run.
    let hotspots = codelore_lib::cli_api::analyses::hotspots::run_hotspots(db, opts)
        .context("run hotspots for dashboard")?;
    let summary = codelore_lib::cli_api::analyses::summary::run_summary(db, opts)
        .context("run summary for dashboard")?;
    let code_health = codelore_lib::cli_api::analyses::code_health::run_code_health(db, opts)
        .context("run code-health for dashboard")?;
    let coupling = codelore_lib::cli_api::analyses::coupling::run_coupling(db, opts)
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: coupling analysis failed; skipping: {e}");
            Vec::new()
        });
    let knowledge_islands =
        codelore_lib::cli_api::analyses::knowledge_islands::run_knowledge_islands(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: knowledge-islands analysis failed; skipping: {e}");
                Vec::new()
            });
    let entity_ownership =
        codelore_lib::cli_api::analyses::entity_ownership::run_entity_ownership(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: entity-ownership analysis failed; skipping: {e}");
                Vec::new()
            });
    let xray = run_xray(db, 500).unwrap_or_else(|e| {
        tracing::warn!("dashboard: xray query failed; skipping: {e}");
        Vec::new()
    });
    let daily_commits = run_daily_commits(db).unwrap_or_else(|e| {
        tracing::warn!("dashboard: daily_commits query failed; skipping: {e}");
        Vec::new()
    });
    // Trends — restrict to the top-50 hotspot paths. The SPA's
    // Top-N selector defaults to 10 (the historical view) but lets
    // the user widen up to all 50 without a re-run. Frontend
    // slices `data.trends` reactively when the user changes the
    // selector — no backend round-trip needed.
    let top_paths: Vec<String> = hotspots.iter().take(50).map(|r| r.path.clone()).collect();
    let trends = run_trends(db, &top_paths).unwrap_or_else(|e| {
        tracing::warn!("dashboard: trends query failed; skipping: {e}");
        Vec::new()
    });
    let mi_rollup = Some(codelore_lib::cli_api::analyses::mi::MiRollup::from_hotspots(&hotspots));
    let coupling_density = compute_spa_coupling_density(db, opts, &coupling);
    let clones = run_clone_summary(db).unwrap_or_else(|e| {
        tracing::warn!("dashboard: clone summary query failed; skipping: {e}");
        Vec::new()
    });
    // Kamei JIT-SDP feature vector — pulled for the last-100
    // non-merge commits so the SPA's window selector can show 10,
    // 30, 60, or all-100 without a re-run. Default render is the
    // last 30 (historical view); the frontend slices reactively.
    let kamei_risk = codelore_lib::cli_api::analyses::dashboard::run_kamei_risk(db, 100)
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: kamei risk query failed; skipping: {e}");
            Vec::new()
        });
    // Resolved import edges for the architecture force-graph widget.
    // Empty when the resolver hasn't covered the repo's language mix
    // yet.
    let imports = codelore_lib::cli_api::analyses::dashboard::run_imports_for_arch_graph(db)
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: imports query failed; skipping: {e}");
            Vec::new()
        });
    // Structure×history fusion overlaid on the architecture graph.
    let modularity_violations =
        codelore_lib::cli_api::analyses::modularity_violations::run_modularity_violations(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: modularity-violations failed; skipping: {e}");
                Vec::new()
            });
    let unstable_interface =
        codelore_lib::cli_api::analyses::unstable_interface::run_unstable_interface(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: unstable-interface failed; skipping: {e}");
                Vec::new()
            });
    // Full per-file roles (no row cap) so the graph colours every module
    // and the propagation-cost caption sums over the whole graph.
    let architecture_roles =
        codelore_lib::cli_api::analyses::architecture_roles::run_architecture_roles(
            db,
            &opts.with_no_row_limit(),
        )
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: architecture-roles failed; skipping: {e}");
            Vec::new()
        });
    // Architecture-trend is the one dashboard input that needs the repo
    // (it re-reads source at sampled historical revs) and the only one
    // markedly heavier than an SQL query — so it opens its own repo
    // handle and degrades gracefully: any failure leaves the trend chart
    // empty rather than sinking the whole dashboard.
    let architecture_trend = codelore_lib::cli_api::repo::GixRepo::open(repo_path)
        .map_err(anyhow::Error::from)
        .and_then(|repo| {
            codelore_lib::cli_api::analyses::architecture_trend::run_architecture_trend(
                db, &repo, opts,
            )
            .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: architecture-trend failed; skipping: {e}");
            Vec::new()
        });
    // Repo health timeline + per-file series + transitions — like
    // architecture-trend, needs the repo to re-read source at sampled
    // historical revs. Opens its own handle and degrades gracefully.
    let (health_trend, file_health_series, health_transitions) =
        codelore_lib::cli_api::repo::GixRepo::open(repo_path)
            .map_err(anyhow::Error::from)
            .and_then(|repo| {
                codelore_lib::cli_api::analyses::health_trend::run_health_trend_detail(
                    db, &repo, opts,
                )
                .map_err(anyhow::Error::from)
            })
            .map_or_else(
                |e| {
                    tracing::warn!("dashboard: health-trend failed; skipping: {e}");
                    (Vec::new(), Vec::new(), Vec::new())
                },
                |d| (d.trend, d.file_series, d.transitions),
            );
    // Effort-exposure: LOC/commit/churn share per band over the trailing
    // window. Pure SQL over already-ingested facts; runs the same
    // code-health HEAD scan as the factor header (cheap, cached). Degrades
    // to empty on analysis failure so no widget is shown.
    let effort_exposure =
        codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: effort-exposure analysis failed; skipping: {e}");
                Vec::new()
            });
    // Marginal-owner risk: ownership concentration × code-health fusion.
    // Degrades to empty when no file meets the high/elevated threshold,
    // or when knowledge_shares is unavailable (e.g. tiny fixture repos).
    let marginal_owner_risk =
        codelore_lib::cli_api::analyses::marginal_owner_risk::run_marginal_owner_risk(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: marginal-owner-risk analysis failed; skipping: {e}");
                Vec::new()
            });
    // Delivery-metrics percentile distributions.  Requires include_merges;
    // degrades gracefully to empty so the tile and card are omitted.
    let delivery_metrics = {
        let mut dm_opts = opts.clone();
        dm_opts.include_merges = true;
        codelore_lib::cli_api::analyses::delivery_metrics::run_delivery_metrics(db, &dm_opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: delivery-metrics failed; skipping: {e}");
                Vec::new()
            })
    };
    // Release cadence — uses a separate Repo handle (same as architecture-trend).
    let release_cadence = codelore_lib::cli_api::repo::GixRepo::open(repo_path)
        .map_err(anyhow::Error::from)
        .and_then(|repo| {
            codelore_lib::cli_api::analyses::release_cadence::run_release_cadence(&repo, opts)
                .map_err(anyhow::Error::from)
        })
        .unwrap_or_else(|e| {
            tracing::warn!("dashboard: release-cadence failed; skipping: {e}");
            Vec::new()
        });
    // Delivery-friction — top 5 rows for the "where is friction" drill line.
    let delivery_friction =
        codelore_lib::cli_api::analyses::delivery_friction::run_delivery_friction(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: delivery-friction failed; skipping: {e}");
                Vec::new()
            })
            .into_iter()
            .take(5)
            .collect::<Vec<_>>();
    // Per-file function X-Ray for the top-10 hotspot paths. Each call
    // opens the HEAD blob via the gix repo handle (cheap; tree-sitter
    // spans only) and joins against already-ingested hunks. One failure
    // per path degrades gracefully to an empty rows vec for that path.
    let function_xray: Vec<codelore_lib::cli_api::output::spa::FileFunctionXray> =
        codelore_lib::cli_api::repo::GixRepo::open(repo_path).map_or_else(
            |e| {
                tracing::warn!("dashboard: could not open repo for function-xray; skipping: {e}");
                Vec::new()
            },
            |xray_repo| {
                hotspots
                    .iter()
                    .take(10)
                    .filter_map(|h| {
                        match codelore_lib::cli_api::analyses::function_xray::run_function_xray(
                            db, &xray_repo, opts, &h.path,
                        ) {
                            Ok(rows) if !rows.is_empty() => {
                                Some(codelore_lib::cli_api::output::spa::FileFunctionXray {
                                    path: h.path.clone(),
                                    rows,
                                })
                            }
                            Ok(_) => None,
                            Err(e) => {
                                tracing::debug!(
                                    "dashboard: function-xray skipped for {}: {e}",
                                    h.path
                                );
                                None
                            }
                        }
                    })
                    .collect()
            },
        );
    // Four-factor header tiles assembled from already-computed data.
    // Code + Architecture come from the health_trend series (zero extra
    // cost — the series is already in memory). Knowledge uses
    // code_familiarity when available, falling back to knowledge_islands.
    // Delivery uses delivery_metrics + release_cadence (degrades to no tile).
    let mut factors = codelore_lib::cli_api::analyses::factors::health_trend_factors(&health_trend);
    // Corpus-relative annotation on the Architecture tile: when an active
    // calibration artifact carries repo-level corpus pools, the
    // architecture-metrics rows include `corpus_percentile:propagation_cost`
    // and `corpus_n` — surface them on the tile's detail line. No active
    // artifact, no `repo_metrics` section, or no Architecture tile ⇒ the
    // detail line is unchanged.
    if let Some(arch_tile) = factors.iter_mut().find(|t| t.name == "Architecture") {
        let arch_metric_rows =
            codelore_lib::cli_api::analyses::architecture_metrics::run_architecture_metrics(
                db, opts,
            )
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: architecture-metrics for factor tile failed: {e}");
                Vec::new()
            });
        let metric_value = |name: &str| {
            arch_metric_rows
                .iter()
                .find(|r| r.metric == name)
                .map(|r| r.value.as_str())
        };
        if let (Some(p), Some(n)) = (
            metric_value("corpus_percentile:propagation_cost"),
            metric_value("corpus_n"),
        ) && let Ok(p) = p.parse::<f64>()
        {
            use std::fmt::Write as _;
            // The row's percentile is 0..1; the tile shows the conventional
            // 0..100 "P<nn>" reading.
            let _ = write!(arch_tile.detail, ", P{:.0} of {n} corpus repos", p * 100.0);
        }
    }
    // Knowledge card data — computed once, feeding both the factor tile and
    // the SPA payload. Degrades to empty on failure so the card is simply
    // absent when data is unavailable.
    let code_familiarity =
        codelore_lib::cli_api::analyses::code_familiarity::run_code_familiarity(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: code-familiarity for spa failed; skipping: {e}");
                Vec::new()
            });
    let knowledge_tile = code_familiarity
        .first()
        .map(|r| {
            codelore_lib::cli_api::analyses::factors::knowledge_factor_from_familiarity(
                r.familiarity_pct,
                r.islands_pct,
            )
        })
        .or_else(|| {
            // Prevalence denominator: all files live at HEAD (no `--min-revs`
            // gate). Degrades to a zero count on query failure, which the
            // factor constructor treats as "no denominator" → tile omitted.
            let total_live_files =
                codelore_lib::cli_api::analyses::knowledge_islands::count_live_files(db)
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            "dashboard: live-file count for knowledge tile failed; skipping: {e}"
                        );
                        0
                    });
            codelore_lib::cli_api::analyses::factors::knowledge_factor_from_islands(
                &knowledge_islands,
                total_live_files,
            )
        });
    if let Some(kt) = knowledge_tile {
        factors.push(kt);
    }
    if let Some(dt) = codelore_lib::cli_api::analyses::factors::delivery_factor_from_metrics(
        &delivery_metrics,
        &release_cadence,
    ) {
        factors.push(dt);
    }
    // Trim the SPA payload to what its consumers read: the delivery card
    // renders three of the five metrics, and only the cadence summary row
    // is consumed (per-tag rows are standalone-CLI output). The factor
    // tile above already read the full row sets.
    let delivery_metrics: Vec<_> = delivery_metrics
        .into_iter()
        .filter(|r| {
            matches!(
                r.metric.as_str(),
                "rework_pct" | "branch_duration_hours" | "lead_proxy_hours"
            )
        })
        .collect();
    let release_cadence: Vec<_> = release_cadence
        .into_iter()
        .filter(|r| r.tag == "__summary__")
        .collect();
    let team_composition =
        codelore_lib::cli_api::analyses::team_composition::run_team_composition(db, opts)
            .unwrap_or_else(|e| {
                tracing::warn!("dashboard: team-composition for spa failed; skipping: {e}");
                Vec::new()
            });
    // Coordination-needs: top 10 by tier desc then co-change entropy desc.
    // Tier order: high > medium > low > single (alphabetical inverse = correct
    // only by accident; sort explicitly).
    let coordination_needs = {
        let tier_rank = |t: &str| match t {
            "high" => 3u8,
            "medium" => 2,
            "low" => 1,
            _ => 0, // "single"
        };
        let mut cn =
            codelore_lib::cli_api::analyses::coordination_needs::run_coordination_needs(db, opts)
                .unwrap_or_else(|e| {
                    tracing::warn!("dashboard: coordination-needs for spa failed; skipping: {e}");
                    Vec::new()
                });
        cn.sort_by(|a, b| {
            tier_rank(&b.tier).cmp(&tier_rank(&a.tier)).then(
                b.cochange_entropy
                    .partial_cmp(&a.cochange_entropy)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        cn.truncate(10);
        cn
    };
    Ok(SpaDashboard {
        hotspots,
        summary,
        code_health,
        coupling,
        knowledge_islands,
        entity_ownership,
        xray,
        daily_commits,
        trends,
        mi_rollup,
        coupling_density,
        clones,
        kamei_risk,
        imports,
        modularity_violations,
        unstable_interface,
        architecture_roles,
        architecture_trend,
        health_trend,
        file_health_series,
        health_transitions,
        effort_exposure,
        marginal_owner_risk,
        code_familiarity,
        team_composition,
        coordination_needs,
        delivery_metrics,
        release_cadence,
        delivery_friction,
        function_xray,
        factors,
        options: codelore_lib::cli_api::output::spa::SpaOptionsSnapshot::from_options(opts),
    })
}

#[cfg(feature = "spa")]
fn run_spa_dispatch(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    repo_path: &std::path::Path,
    output: &std::path::Path,
) -> anyhow::Result<()> {
    use codelore_lib::cli_api::output::spa::write_spa;

    let dash = build_spa_dashboard(db, opts, repo_path)?;

    let now = time::OffsetDateTime::now_utc();
    let generated_at = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    );
    let title = "CodeLore Dashboard";
    let repo_display = repo_path.display().to_string();

    let mut out = std::fs::File::create(output)
        .with_context(|| format!("create spa output {}", output.display()))?;
    write_spa(&dash, title, &repo_display, &generated_at, &mut out)
        .context("write spa dashboard")?;
    // Drop the writer so file size is finalised on disk before the
    // stat() below.
    drop(out);

    // User feedback: --format spa is silent on success by default,
    // which makes the dashboard look like nothing happened. Print
    // the output path, size, and a clickable file:// URL for
    // terminals that linkify (iTerm2, modern macOS Terminal, most
    // Linux terminals). Single eprintln to stderr so it doesn't
    // pollute stdout when callers pipe.
    let size_bytes = std::fs::metadata(output).map_or(0, |m| m.len());
    #[allow(clippy::cast_precision_loss)]
    // size_bytes is formatted to one decimal place; display-precision loss on u64→f64 is imperceptible and intentional
    let size_human = if size_bytes >= 1_000_000 {
        format!("{:.1} MB", size_bytes as f64 / 1_000_000.0)
    } else if size_bytes >= 1_000 {
        format!("{:.1} kB", size_bytes as f64 / 1_000.0)
    } else {
        format!("{size_bytes} bytes")
    };
    let abs = output
        .canonicalize()
        .unwrap_or_else(|_| output.to_path_buf());
    eprintln!(
        "✓ spa dashboard written to {} ({})",
        output.display(),
        size_human
    );
    eprintln!("  open in browser: file://{}", abs.display());
    Ok(())
}

#[cfg(feature = "spa")]
fn run_step_summary_dispatch(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    repo_path: &std::path::Path,
    output: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    use codelore_lib::cli_api::output::step_summary::write_step_summary;

    let dash = build_spa_dashboard(db, opts, repo_path)?;
    let now = time::OffsetDateTime::now_utc();
    let generated_at = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    );
    let title = "CodeLore Analysis";
    let repo_display = repo_path.display().to_string();

    // step-summary streams to stdout by default so CI workflows can
    // `codelore ... --format step-summary >> $GITHUB_STEP_SUMMARY`
    // directly. `--output PATH` opt-in for local use / testing.
    if let Some(path) = output {
        let mut out = std::fs::File::create(path)
            .with_context(|| format!("create step-summary output {}", path.display()))?;
        write_step_summary(&dash, title, &repo_display, &generated_at, &mut out)
            .context("write step-summary")?;
        eprintln!("✓ step-summary written to {}", path.display());
    } else {
        let mut out = std::io::stdout().lock();
        write_step_summary(&dash, title, &repo_display, &generated_at, &mut out)
            .context("write step-summary")?;
    }
    Ok(())
}

/// Compute the behavioral coupling graph density for the SPA dashboard.
/// `coupling` is the Fisher-significant edge set already returned by
/// `run_coupling`. Skips the node-count query when the edge set is empty
/// (density would be `0` regardless) and converts node-count failures
/// into a warning + `None` so dashboard generation never blocks on a
/// secondary metric.
#[cfg(feature = "spa")]
fn compute_spa_coupling_density(
    db: &codelore_lib::cli_api::facts::FactsDb,
    opts: &codelore_lib::cli_api::Options,
    coupling: &[codelore_lib::cli_api::analyses::coupling::CouplingRow],
) -> Option<f64> {
    if coupling.is_empty() {
        return None;
    }
    match codelore_lib::cli_api::analyses::coupling::count_coupling_nodes(db, opts) {
        Ok(n) => Some(codelore_lib::cli_api::analyses::coupling::density(
            n,
            coupling.len(),
        )),
        Err(e) => {
            tracing::warn!("spa: coupling-density node count failed; skipping: {e}");
            None
        }
    }
}
