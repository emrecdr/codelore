//! MCP server for `CodeLore` (`codelore mcp`).
//!
//! Exposes `CodeLore` analyses as MCP tools over stdio. All tools are read-only.
//! Each tool call opens its own [`FactsDb`] via the warm-cache path so the
//! `!Send + !Sync` `DuckDB` connection never crosses thread or await boundaries.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use rmcp::{handler::server::wrapper::Parameters, model::ErrorData, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use codelore_lib::cli_api::{
    Options,
    analyses::{
        code_health,
        delta_health::{DeltaHealthSection, compute_delta_health, run_function_metrics},
        function_xray,
        hotspots,
        refactoring_targets,
        summary,
    },
    cache::default_cache_root,
    facts::FactsDb,
    quality_gates::{
        GateViolation, Thresholds, evaluate_clone_gate, evaluate_code_health_gate,
        evaluate_effort_exposure_gate, evaluate_full_tree,
    },
    repo::GixRepo,
};

/// Serializable mirror of [`GateViolation`] for JSON output.
/// `GateViolation` itself does not derive `Serialize`; we map at the boundary.
#[derive(Debug, Serialize)]
struct ViolationRecord {
    gate: String,
    path: String,
    actual: String,
    threshold: String,
}

impl From<GateViolation> for ViolationRecord {
    fn from(v: GateViolation) -> Self {
        Self { gate: v.gate, path: v.path, actual: v.actual, threshold: v.threshold }
    }
}

/// Convert any displayable error to an MCP `ErrorData` internal error.
fn internal(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Resolve a revision string against `repo` via `git rev-parse`.
/// Returns the full 40-char SHA, or an `ErrorData` if the rev is unknown.
fn resolve_rev(repo: &Path, rev: &str) -> std::result::Result<String, ErrorData> {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap_or("."), "rev-parse", "--verify", rev])
        .output()
        .map_err(|e| ErrorData::internal_error(format!("git rev-parse: {e}"), None))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(ErrorData::invalid_params(
            format!("revision {rev:?} not found in this repository"),
            None,
        ))
    }
}

/// Create a temporary git worktree detached at `sha`, returning its path and a cleanup guard.
/// The caller is responsible for dropping the guard to remove the worktree.
fn temp_worktree(
    repo: &Path,
    sha: &str,
) -> std::result::Result<(PathBuf, TempWorktree), ErrorData> {
    let dir =
        tempfile::tempdir().map_err(|e| internal(format!("create temp dir: {e}")))?;
    let wt_path = dir.path().to_path_buf();
    let status = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap_or("."),
            "worktree",
            "add",
            "--detach",
            "--quiet",
            wt_path.to_str().unwrap(),
            sha,
        ])
        .status()
        .map_err(|e| internal(format!("git worktree add: {e}")))?;
    if !status.success() {
        return Err(internal(format!("git worktree add failed for {sha}")));
    }
    Ok((wt_path, TempWorktree { repo: repo.to_path_buf(), dir }))
}

/// RAII guard that removes a git worktree when dropped.
struct TempWorktree {
    repo: PathBuf,
    dir: tempfile::TempDir,
}

impl Drop for TempWorktree {
    fn drop(&mut self) {
        let path = self.dir.path().to_str().unwrap_or("").to_string();
        let _ = Command::new("git")
            .args([
                "-C",
                self.repo.to_str().unwrap_or("."),
                "worktree",
                "remove",
                "--force",
                &path,
            ])
            .status();
    }
}

/// MCP server state — the repo path fixed at server startup.
#[derive(Clone)]
pub struct CodeLoreServer {
    repo: PathBuf,
}

// ── Parameter structs (one per tool) ─────────────────────────────────────────

/// Parameters for the `repo_overview` tool (none required).
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct RepoOverviewParams {}

/// Parameters for the `hotspots` tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct HotspotsParams {
    /// Maximum rows to return (default: 20).
    pub limit: Option<u32>,
}

/// Parameters for the `code_health` tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct CodeHealthParams {
    /// Filter results to this file path (relative to repo root).
    /// Omit to return all files with complexity data.
    pub path: Option<String>,
}

/// Parameters for the `delta_health` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeltaHealthParams {
    /// Base revision (branch, tag, or full SHA). Must be resolvable by `git rev-parse`.
    pub base: String,
    /// Head revision (branch, tag, or full SHA). Must be resolvable by `git rev-parse`.
    pub head: String,
}

/// Parameters for the `refactoring_targets` tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct RefactoringTargetsParams {
    /// Maximum rows to return (default: all).
    pub limit: Option<u32>,
}

/// Parameters for the `function_xray` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionXrayParams {
    /// File path (relative to repo root) to analyse.
    pub path: String,
}

/// Parameters for the `check_gates` tool (none required).
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct CheckGatesParams {}

// ── Output type for check_gates ───────────────────────────────────────────────

/// Summary returned by the `check_gates` tool.
#[derive(Debug, Serialize)]
struct GateSummary {
    /// `"pass"`, `"fail"`, or `"no_thresholds"`.
    verdict: String,
    /// Number of violations found.
    violation_count: usize,
    /// Individual gate violations, if any.
    violations: Vec<ViolationRecord>,
}

#[tool_router]
impl CodeLoreServer {
    // ── repo_overview ─────────────────────────────────────────────────────────

    #[tool(
        name = "repo_overview",
        description = "Return a JSON object with `summary` (commit count, authors, files, date range) \
            and `options` (the active analysis options snapshot used for cache-keying). \
            First call on a cold cache triggers history ingest; warm-cache calls are fast."
    )]
    async fn repo_overview(
        &self,
        _params: Parameters<RepoOverviewParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        tokio::task::spawn_blocking(move || {
            let opts = Options { repo_path: repo_path.clone(), ..Options::default() };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db =
                FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                    .map_err(internal)?;
            let rows = summary::run_summary(&db, &opts).map_err(internal)?;
            let out = serde_json::json!({
                "summary": rows,
                "options": opts.canonical_json(),
            });
            serde_json::to_string(&out).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── hotspots ──────────────────────────────────────────────────────────────

    #[tool(
        name = "hotspots",
        description = "Return the top hotspot files ranked by revision count as JSON. \
            Pass `limit` to cap rows (default: 20). \
            First call on a cold cache triggers history ingest."
    )]
    async fn hotspots(&self, params: Parameters<HotspotsParams>) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let limit = params.0.limit.unwrap_or(20);
        tokio::task::spawn_blocking(move || {
            let mut opts = Options { repo_path: repo_path.clone(), ..Options::default() };
            opts.rows_limit = Some(limit);
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db =
                FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                    .map_err(internal)?;
            let rows = hotspots::run_hotspots(&db, &opts).map_err(internal)?;
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── code_health ───────────────────────────────────────────────────────────

    #[tool(
        name = "code_health",
        description = "Return per-file composite code-health scores (band: red/yellow/green, score 0–100) as JSON. \
            Pass `path` to filter to a single file. \
            First call on a cold cache triggers history ingest."
    )]
    async fn code_health(
        &self,
        params: Parameters<CodeHealthParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let filter_path = params.0.path.clone();
        tokio::task::spawn_blocking(move || {
            let opts = Options { repo_path: repo_path.clone(), ..Options::default() };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db =
                FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                    .map_err(internal)?;
            let mut rows = code_health::run_code_health(&db, &opts).map_err(internal)?;
            if let Some(p) = filter_path {
                rows.retain(|r| r.path == p);
            }
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── delta_health ──────────────────────────────────────────────────────────

    #[tool(
        name = "delta_health",
        description = "Return a function-level health delta between two revisions as JSON. \
            `base` and `head` are any rev-parse-able strings (branch, tag, SHA). \
            Returns verdict (improved/neutral/degraded), ratio, and per-function breakdown. \
            Cost: ingests history twice (once per rev); expect 5–30 s on a cold cache."
    )]
    async fn delta_health(
        &self,
        params: Parameters<DeltaHealthParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let base_rev = params.0.base.clone();
        let head_rev = params.0.head.clone();

        // Resolve revisions up front on the async thread (cheap git I/O).
        let base_sha = resolve_rev(&repo_path, &base_rev)?;
        let head_sha = resolve_rev(&repo_path, &head_rev)?;
        if base_sha == head_sha {
            return Err(ErrorData::invalid_params(
                format!("base and head both resolve to {base_sha}; nothing to diff"),
                None,
            ));
        }

        tokio::task::spawn_blocking(move || {
            // Ingest each rev in its own worktree → in-memory DB.
            let (base_path, _base_wt) = temp_worktree(&repo_path, &base_sha)?;
            let (head_path, _head_wt) = temp_worktree(&repo_path, &head_sha)?;

            let ingest_at = |wt: &Path| -> std::result::Result<Vec<codelore_lib::cli_api::analyses::delta_health::FunctionMetricRow>, ErrorData> {
                let opts = Options { repo_path: wt.to_path_buf(), ..Options::default() };
                let repo = GixRepo::open(wt).map_err(internal)?;
                let db = FactsDb::new_in_memory().map_err(internal)?;
                db.ingest(&repo, &opts).map_err(internal)?;
                run_function_metrics(&db).map_err(internal)
            };

            let base_fns = ingest_at(&base_path)?;
            let head_fns = ingest_at(&head_path)?;

            // All files touched between the two revs count as "PR files".
            let pr_files: HashSet<String> = head_fns.iter().map(|r| r.path.clone()).collect();
            let clone_members: HashSet<(String, String)> = HashSet::new();
            let base_red: HashSet<String> = HashSet::new();

            let section: DeltaHealthSection = compute_delta_health(
                &base_fns,
                &head_fns,
                &pr_files,
                &clone_members,
                &base_red,
            );

            serde_json::to_string(&section).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── refactoring_targets ───────────────────────────────────────────────────

    #[tool(
        name = "refactoring_targets",
        description = "Return the highest-priority refactoring candidates ranked by risk÷LOC as JSON. \
            Pass `limit` to cap rows (default: all). \
            First call on a cold cache triggers history ingest."
    )]
    async fn refactoring_targets(
        &self,
        params: Parameters<RefactoringTargetsParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let limit = params.0.limit;
        tokio::task::spawn_blocking(move || {
            let mut opts = Options { repo_path: repo_path.clone(), ..Options::default() };
            if let Some(n) = limit {
                opts.rows_limit = Some(n);
            }
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db =
                FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                    .map_err(internal)?;
            let rows =
                refactoring_targets::run_refactoring_targets(&db, &opts).map_err(internal)?;
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── function_xray ─────────────────────────────────────────────────────────

    #[tool(
        name = "function_xray",
        description = "Return per-function change-frequency and complexity for a file as JSON. \
            `path` is the file path relative to the repo root (e.g. \"src/main.rs\"). \
            First call on a cold cache triggers history ingest."
    )]
    async fn function_xray(
        &self,
        params: Parameters<FunctionXrayParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let target = params.0.path.clone();
        tokio::task::spawn_blocking(move || {
            let opts = Options { repo_path: repo_path.clone(), ..Options::default() };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db =
                FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                    .map_err(internal)?;
            let rows =
                function_xray::run_function_xray(&db, &repo, &opts, &target).map_err(internal)?;
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── check_gates ───────────────────────────────────────────────────────────

    #[tool(
        name = "check_gates",
        description = "Evaluate `.codelore-thresholds.toml` quality gates at HEAD and return a JSON \
            summary with verdict (pass/fail/no_thresholds), violation count, and individual violations. \
            Returns `no_thresholds` verdict when no config file is found. \
            First call on a cold cache triggers history ingest."
    )]
    async fn check_gates(
        &self,
        _params: Parameters<CheckGatesParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        tokio::task::spawn_blocking(move || {
            let thresholds = Thresholds::discover(&repo_path).map_err(internal)?;
            if thresholds.is_empty() {
                let summary = GateSummary {
                    verdict: "no_thresholds".into(),
                    violation_count: 0,
                    violations: Vec::<ViolationRecord>::new(),
                };
                return serde_json::to_string(&summary).map_err(internal);
            }

            let opts = Options { repo_path: repo_path.clone(), ..Options::default() };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db =
                FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                    .map_err(internal)?;

            let mut violations: Vec<GateViolation> = Vec::new();

            // hotspot-scoped gates (cognitive_max, hotspot_score_max)
            let hs = hotspots::run_hotspots(&db, &opts).map_err(internal)?;
            violations.extend(evaluate_full_tree(&thresholds, &hs));

            // code_health_min gate
            let ch = code_health::run_code_health(&db, &opts).map_err(internal)?;
            violations.extend(evaluate_code_health_gate(&thresholds, &ch));

            // clone gate
            violations.extend(evaluate_clone_gate(&thresholds, &db).map_err(internal)?);

            // effort-exposure gate
            violations
                .extend(evaluate_effort_exposure_gate(&thresholds, &db, &opts).map_err(internal)?);

            let verdict = if violations.is_empty() { "pass" } else { "fail" };
            let records: Vec<ViolationRecord> = violations.into_iter().map(Into::into).collect();
            let summary = GateSummary {
                verdict: verdict.into(),
                violation_count: records.len(),
                violations: records,
            };
            serde_json::to_string(&summary).map_err(internal)
        })
        .await
        .map_err(internal)?
    }
}

/// Wire the tool router into the MCP `ServerHandler` trait and set the
/// server's `instructions` field — clients display this to the operator
/// so the "local-only / read-only / no telemetry" positioning is
/// communicated at the protocol layer, not only in `--help`.
#[tool_handler(
    instructions = "Local-only behavioral analysis of the git repository configured at startup. \
        Read-only. No network, no account, no telemetry. \
        First call on a cold cache pays a one-time history ingest (5–30 s for typical repos); \
        subsequent calls within the same server session are fast."
)]
impl rmcp::handler::server::ServerHandler for CodeLoreServer {}

/// Start the MCP server and block until the client closes the connection.
pub fn run_mcp_server(repo: PathBuf) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let server = CodeLoreServer { repo };
            let transport = rmcp::transport::io::stdio();
            let running = rmcp::service::serve_server(server, transport)
                .await
                .map_err(|e| anyhow::anyhow!("MCP init error: {e}"))?;
            running
                .waiting()
                .await
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))
        })
}
