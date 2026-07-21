//! MCP server for `CodeLore` (`codelore mcp`).
//!
//! Exposes `CodeLore` analyses as MCP tools over stdio. All tools are read-only.
//! Each tool call opens its own [`FactsDb`] via the warm-cache path so the
//! `!Send + !Sync` `DuckDB` connection never crosses thread or await boundaries.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use rmcp::{
    handler::server::wrapper::Parameters, model::ErrorData, tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use codelore_lib::change_context;
use codelore_lib::change_set;
use codelore_lib::cli_api::{
    Options,
    analyses::{
        code_health,
        delta_health::{DeltaHealthSection, compute_delta_health, run_function_metrics},
        finding_hotspot_overlap, function_xray, hotspots, refactoring_targets, summary,
    },
    cache::default_cache_root,
    enrichment::{
        client::{LlmEnv, resolve_client},
        engine,
        fact_sheet::FileFactSheet,
        prompt::Lens,
    },
    external::ExternalStore,
    facts::FactsDb,
    quality_gates::{
        GateViolation, Thresholds, evaluate_clone_gate, evaluate_code_health_gate,
        evaluate_full_tree, evaluate_gate_thresholds, resolve_defect_calibration,
    },
    repo::{GixRepo, Repo as _},
};
use codelore_lib::defect_calibration;

/// Convert any displayable error to an MCP `ErrorData` internal error.
fn internal(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Resolve a revision string against `repo` via `git rev-parse`.
/// Returns the full 40-char SHA, or an `ErrorData` if the rev is unknown.
///
/// INVARIANT for every child process this server spawns: it must never
/// inherit the server's stdio. Stdout IS the JSON-RPC channel — a child
/// that writes to an inherited handle corrupts the protocol stream, and
/// if the client isn't draining it the child blocks on a full pipe and
/// deadlocks the tool call (deterministic on windows, whose pipe
/// buffers are small enough for git's checkout chatter to fill).
fn resolve_rev(repo: &Path, rev: &str) -> std::result::Result<String, ErrorData> {
    let out = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap_or("."),
            "rev-parse",
            "--verify",
            rev,
        ])
        .stdin(std::process::Stdio::null())
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
    let dir = tempfile::tempdir().map_err(|e| internal(format!("create temp dir: {e}")))?;
    let wt_path = dir.path().to_path_buf();
    let wt_path_str = wt_path.to_str().ok_or_else(|| {
        internal(format!(
            "worktree temp path is not valid UTF-8: {}",
            wt_path.display()
        ))
    })?;
    // Detached stdio per the module invariant (see `resolve_rev`);
    // stderr is captured so a failure carries git's own diagnosis.
    let out = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap_or("."),
            "worktree",
            "add",
            "--detach",
            "--quiet",
            wt_path_str,
            sha,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| internal(format!("git worktree add: {e}")))?;
    if !out.status.success() {
        return Err(internal(format!(
            "git worktree add failed for {sha}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok((
        wt_path,
        TempWorktree {
            repo: repo.to_path_buf(),
            dir,
        },
    ))
}

/// RAII guard that removes a git worktree when dropped.
struct TempWorktree {
    repo: PathBuf,
    dir: tempfile::TempDir,
}

impl Drop for TempWorktree {
    fn drop(&mut self) {
        let path = self.dir.path().to_str().unwrap_or("").to_string();
        // Best-effort cleanup; detached stdio per the module invariant
        // (see `resolve_rev`).
        let _ = Command::new("git")
            .args([
                "-C",
                self.repo.to_str().unwrap_or("."),
                "worktree",
                "remove",
                "--force",
                &path,
            ])
            .stdin(std::process::Stdio::null())
            .output();
    }
}

/// MCP server state — the repo path and defect-calibration configuration
/// fixed at server startup.
#[derive(Clone)]
pub struct CodeLoreServer {
    repo: PathBuf,
    defect_calibration: Option<PathBuf>,
    allow_foreign_calibration: bool,
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

/// Parameters for the `finding_hotspot_overlap` tool (none required).
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct FindingHotspotOverlapParams {}

/// Parameters for the `explain_file` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExplainFileParams {
    /// File path (relative to repo root) to build the evidence dossier for
    /// (e.g. "src/main.rs").
    pub path: String,
}

/// Parameters for the `change_context` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChangeContextParams {
    /// Repo-relative paths the caller intends to modify (1-20).
    pub paths: Vec<String>,
}

/// Parameters for the `gate_changes` tool (none required — the change set is
/// discovered from the working tree).
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct GateChangesParams {}

// ── Output type for check_gates ───────────────────────────────────────────────

/// Summary returned by the `check_gates` tool.
#[derive(Debug, Serialize)]
struct GateSummary {
    /// `"pass"`, `"fail"`, or `"no_thresholds"`.
    verdict: String,
    /// Number of violations found.
    violation_count: usize,
    /// Individual gate violations, if any.
    violations: Vec<GateViolation>,
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
            let opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
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
            let mut opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            opts.rows_limit = Some(limit);
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
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
    async fn code_health(&self, params: Parameters<CodeHealthParams>) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let filter_path = params.0.path.clone();
        tokio::task::spawn_blocking(move || {
            let opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
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

            let ingest_at = |wt: &Path| -> std::result::Result<
                Vec<codelore_lib::cli_api::analyses::delta_health::FunctionMetricRow>,
                ErrorData,
            > {
                let opts = Options {
                    repo_path: wt.to_path_buf(),
                    ..Options::default()
                };
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

            let section: DeltaHealthSection =
                compute_delta_health(&base_fns, &head_fns, &pr_files, &clone_members, &base_red);

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
            let mut opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            if let Some(n) = limit {
                opts.rows_limit = Some(n);
            }
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
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
            let opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
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
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        tokio::task::spawn_blocking(move || {
            let thresholds = Thresholds::discover(&repo_path).map_err(internal)?;
            if thresholds.is_empty() {
                let summary = GateSummary {
                    verdict: "no_thresholds".into(),
                    violation_count: 0,
                    violations: Vec::<GateViolation>::new(),
                };
                return serde_json::to_string(&summary).map_err(internal);
            }

            // The server-resolved calibration threads into the analyses so
            // this verdict matches a `codelore check` run under the same
            // repo `[calibration]` section or startup flag.
            let opts = Options {
                repo_path: repo_path.clone(),
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
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

            // effort-exposure gate — reuses the code-health rows computed for
            // code_health_min so the heaviest analysis runs once per call.
            if let Some(max) = thresholds.gates.max_red_effort_pct {
                let rows =
                    codelore_lib::cli_api::analyses::effort_exposure::run_effort_exposure_with_health(
                        &db,
                        &opts.with_no_row_limit(),
                        &ch,
                    )
                    .map_err(internal)?;
                violations.extend(
                    codelore_lib::cli_api::quality_gates::evaluate_effort_exposure_rows(max, &rows),
                );
            }

            // architecture + familiarity gates. This tool evaluates a subset
            // of `codelore check`: the `max_findings_in_hot_files` and
            // `corpus_percentile_max` gates, degraded-gate semantics, and
            // `--ratchet` remain check-only, so a config using those gates can
            // make this verdict diverge from a CI run — `codelore check` is
            // authoritative.
            violations.extend(
                codelore_lib::cli_api::quality_gates::evaluate_architecture_gate(&thresholds, &db)
                    .map_err(internal)?,
            );
            violations.extend(
                codelore_lib::cli_api::quality_gates::evaluate_familiarity_gate(
                    &thresholds,
                    &db,
                    &opts,
                )
                .map_err(internal)?,
            );

            let verdict = if violations.is_empty() {
                "pass"
            } else {
                "fail"
            };
            let summary = GateSummary {
                verdict: verdict.into(),
                violation_count: violations.len(),
                violations,
            };
            serde_json::to_string(&summary).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── finding_hotspot_overlap ───────────────────────────────────────────────

    #[tool(
        name = "finding_hotspot_overlap",
        description = "Return the behavioral×static fusion table: external scanner findings \
            joined with hotspot rank and code-health band, producing an `act-now` / `plan` / `note` \
            priority for each flagged file. Requires a prior `codelore ingest-sarif` run to populate \
            the external findings sidecar; returns a structured note when the sidecar is absent or empty. \
            Cost: warm-cache fast after ingest; does not trigger history re-ingest."
    )]
    async fn finding_hotspot_overlap(
        &self,
        _params: Parameters<FindingHotspotOverlapParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        tokio::task::spawn_blocking(move || {
            let cache_root = default_cache_root();

            // open_nonempty returns None when the sidecar is absent OR
            // present-but-empty — the MCP tool never creates it; that is
            // ingest-sarif's job. Both cases return the structured "run
            // ingest-sarif first" note.
            let Some(store) =
                ExternalStore::open_nonempty(&cache_root, &repo_path).map_err(internal)?
            else {
                let note = serde_json::json!({
                    "findings": [],
                    "note": "run codelore ingest-sarif first"
                });
                return serde_json::to_string(&note).map_err(internal);
            };

            let opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
                .map_err(internal)?;

            let rows = finding_hotspot_overlap::run_finding_hotspot_overlap(&db, &opts, &store)
                .map_err(internal)?;
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── explain_file ──────────────────────────────────────────────────────────

    #[tool(
        name = "explain_file",
        description = "Return a deterministic per-file evidence dossier for one repo-relative file \
            path. `fact_sheet` is always present: the ordered analysis sections (code-health, \
            biomarkers, hotspots, coupling, ownership, functions, and import cycles) as \
            structured JSON. When the server was started with `--defect-calibration`, the fact \
            sheet also carries a `defect-evidence` section. When the server environment has an \
            LLM configured (the `CODELORE_LLM_*` variables), the response also carries a grounded \
            advisory `narrative` with its `model` and a `grounded` citation-check verdict; when it \
            does not, `narrative_error` is returned instead. The fact sheet is always returned and \
            the call never fails because the LLM is unavailable. \
            First call on a cold cache triggers history ingest."
    )]
    async fn explain_file(
        &self,
        params: Parameters<ExplainFileParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let target = params.0.path.clone();
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        tokio::task::spawn_blocking(move || {
            // min_revs = 1 so any single named file resolves in its own dossier,
            // matching the `explain <path>` CLI surface.
            let opts = Options {
                repo_path: repo_path.clone(),
                min_revs: 1,
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(internal)?;
            let sheet = FileFactSheet::build(&db, &repo, &opts, &target).map_err(internal)?;

            // The structured fact sheet: an ordered array of {section, facts}
            // objects, preserving the builder's section and key order.
            let fact_sheet: Vec<serde_json::Value> = sheet
                .sections
                .iter()
                .map(|(name, facts)| {
                    let facts_obj: serde_json::Map<String, serde_json::Value> = facts
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect();
                    serde_json::json!({ "section": name, "facts": facts_obj })
                })
                .collect();

            // Advisory narrative — best-effort. The tool never fails because the
            // LLM is unavailable: a resolution or narration error becomes
            // `narrative_error` alongside the always-present fact sheet.
            let out = match resolve_client(&LlmEnv::from_process_env()) {
                Ok(client) => {
                    let canonical = sheet.to_canonical_text();
                    let values = sheet.numeric_values();
                    match engine::narrate(
                        client.as_ref(),
                        Lens::FileDiagnosis,
                        &target,
                        engine::SheetFacts {
                            text: &canonical,
                            values: &values,
                        },
                        &default_cache_root(),
                        &repo_path,
                        false,
                    ) {
                        Ok(result) => serde_json::json!({
                            "fact_sheet": fact_sheet,
                            "narrative": result.narrative,
                            "grounded": result.grounded,
                            "model": result.model,
                        }),
                        Err(e) => serde_json::json!({
                            "fact_sheet": fact_sheet,
                            "narrative_error": e.to_string(),
                        }),
                    }
                }
                Err(e) => serde_json::json!({
                    "fact_sheet": fact_sheet,
                    "narrative_error": e.to_string(),
                }),
            };
            serde_json::to_string(&out).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── change_context ────────────────────────────────────────────────────────

    #[tool(
        name = "change_context",
        description = "Temporal pre-write briefing for files you are about to modify: \
            code-health band, hotspot standing, historically co-changed partners \
            (edit those too), owner concentration incl. a departed-owner flag, \
            calibrated structural risk, and recent churn — compact text, \
            ~150 tokens per file. 1-20 paths. Committed-history view; for \
            gate evaluation of the committed tree use `check_gates`. \
            First call on a cold cache triggers history ingest."
    )]
    async fn change_context(
        &self,
        params: Parameters<ChangeContextParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        let paths = params.0.paths.clone();
        tokio::task::spawn_blocking(move || {
            // min_revs = 1 so any single named file resolves in its briefing,
            // matching the `change_context` lib contract.
            let opts = Options {
                repo_path: repo_path.clone(),
                min_revs: 1,
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(internal)?;
            change_context::build_change_context(&db, &repo, &opts, &paths).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    // ── gate_changes ──────────────────────────────────────────────────────────

    #[tool(
        name = "gate_changes",
        description = "Working-tree quality verdict for the agent loop: projects what the \
            current uncommitted edits do to code health and the import graph vs HEAD, \
            evaluates the repo's working-tree `[diff]` gates against the projection, \
            and returns compact text — verdict line, violations, advisory findings, \
            and a per-file delta table. With no thresholds configured the verdict \
            line reads `no thresholds configured — advisory only` and the advisory \
            sections still render; a clean tree returns \
            `PASS (no working-tree changes to gate)`. Reads the working tree; the \
            committed-tree counterpart is `check_gates`. \
            First call on a cold cache triggers history ingest."
    )]
    async fn gate_changes(
        &self,
        _params: Parameters<GateChangesParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        tokio::task::spawn_blocking(move || {
            let opts = Options {
                repo_path: repo_path.clone(),
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let changes = repo.worktree_changes().map_err(internal)?;
            if changes.is_empty() {
                return Ok("PASS (no working-tree changes to gate)".to_string());
            }
            let cache_root = default_cache_root();
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
                .map_err(internal)?;
            let report = change_set::build_change_set_report(&db, &repo, &opts, &cache_root)
                .map_err(internal)?;
            // Thresholds are re-evaluated on every call — a warm sidecar hit
            // returns measured data only, never a stored verdict.
            let thresholds = Thresholds::discover(&repo_path).map_err(internal)?;
            let violations = if thresholds.is_empty() {
                None
            } else {
                Some(evaluate_gate_thresholds(&thresholds, &report))
            };
            Ok(render_gate_changes(&report, violations.as_deref()))
        })
        .await
        .map_err(internal)?
    }
}

/// Render the `gate_changes` text document, mirroring `codelore gate`'s text
/// forms: a verdict line (`PASS` / `FAIL — n violation(s)` / the advisory-only
/// disclosure when `violations` is `None` because no thresholds are
/// configured), the merge-in-progress note, violation rows in `codelore
/// check`'s exact form, one line per advisory finding capped at the CLI's row
/// limit with a `(+n more findings)` tail, and the per-file delta table
/// capped at the CLI's row limit with a `(+n more files)` tail.
fn render_gate_changes(
    report: &change_set::ChangeSetReport,
    violations: Option<&[GateViolation]>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    match violations {
        None => lines.push("no thresholds configured — advisory only".to_string()),
        Some([]) => lines.push("PASS".to_string()),
        Some(v) => lines.push(format!("FAIL — {} violation(s)", v.len())),
    }
    if report.merge_in_progress {
        lines.push(
            "note: merge/rebase in progress — projection reflects committed HEAD history"
                .to_string(),
        );
    }
    for v in violations.unwrap_or_default() {
        lines.push(format!(
            "  - {gate}: {path} — actual {actual} vs threshold {threshold}",
            gate = v.gate,
            path = v.path,
            actual = v.actual,
            threshold = v.threshold,
        ));
    }
    for f in report.findings.iter().take(crate::GATE_FINDINGS_ROWS) {
        lines.push(format!("[{}] {}: {}", f.kind, f.path, f.detail));
    }
    let hidden_findings = report
        .findings
        .len()
        .saturating_sub(crate::GATE_FINDINGS_ROWS);
    if hidden_findings > 0 {
        lines.push(format!("(+{hidden_findings} more findings)"));
    }
    for d in report
        .health
        .deltas
        .iter()
        .take(crate::GATE_DELTA_TABLE_ROWS)
    {
        match (d.baseline_score, d.projected_score, d.delta) {
            (Some(b), Some(p), Some(delta)) => {
                lines.push(format!("{}  {b:.1} → {p:.1}  ({delta:+.1})", d.path));
            }
            _ => lines.push(format!(
                "{}  — {}",
                d.path,
                d.reason.as_deref().unwrap_or("not scored")
            )),
        }
    }
    let hidden = report
        .health
        .deltas
        .len()
        .saturating_sub(crate::GATE_DELTA_TABLE_ROWS);
    if hidden > 0 {
        lines.push(format!("(+{hidden} more files)"));
    }
    lines.join("\n")
}

/// Wire the tool router into the MCP `ServerHandler` trait and set the
/// server's `instructions` field — clients display this to the operator
/// so the "local-only / read-only / no telemetry" positioning is
/// communicated at the protocol layer, not only in `--help`.
#[tool_handler(
    instructions = "Local-only behavioral analysis of the git repository configured at startup. \
        Read-only. No network, no account, no telemetry — beyond the optional CODELORE_LLM_* \
        endpoint you configure for explain_file's advisory narrative (off by default, and \
        local-first when enabled). \
        First call on a cold cache pays a one-time history ingest (5–30 s for typical repos); \
        subsequent calls within the same server session are fast."
)]
impl rmcp::handler::server::ServerHandler for CodeLoreServer {}

/// Start the MCP server and block until the client closes the connection.
///
/// When `defect_calibration` is set, the artifact is loaded and its
/// repo-identity checked before the server starts serving — a bad path or a
/// foreign artifact (without `allow_foreign_calibration`) is a startup error,
/// not a failure surfaced on the first `explain_file` call. The loaded
/// artifact is discarded here; each `explain_file` call loads it again itself
/// via `Options`.
///
/// When no startup flag is given, a `[calibration]` section in the repo's
/// thresholds file fills the artifact path instead — validated fail-fast
/// here identically to the flag path, so a malformed thresholds file fails
/// server startup rather than surfacing on the first tool call.
pub fn run_mcp_server(
    repo: PathBuf,
    defect_calibration: Option<PathBuf>,
    allow_foreign_calibration: bool,
) -> Result<()> {
    let defect_calibration = if defect_calibration.is_some() {
        defect_calibration
    } else {
        resolve_defect_calibration(None, &repo)?
    };
    if let Some(path) = &defect_calibration {
        let artifact = defect_calibration::load(path)?;
        defect_calibration::check_repo_identity(&artifact, &repo, allow_foreign_calibration)?;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let server = CodeLoreServer {
                repo,
                defect_calibration,
                allow_foreign_calibration,
            };
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
