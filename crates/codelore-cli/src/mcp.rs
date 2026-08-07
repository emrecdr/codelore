//! MCP server for `CodeLore` (`codelore mcp`).
//!
//! Exposes `CodeLore` analyses as MCP tools over stdio. Each tool call opens
//! its own [`FactsDb`] via the warm-cache path so the `!Send + !Sync` `DuckDB`
//! connection never crosses thread or await boundaries.
//!
//! Every tool carries MCP annotations so a client can reason about it without
//! calling it. `read_only_hint` is scoped to the user's repository and files:
//! every tool may populate the persistent `DuckDB` cache on a cold call, which
//! does not count against the hint. `delta_health` is the one tool that is not
//! read-only — it checks revisions out into throwaway `git worktree`s.
//! `explain_file` is the one open-world tool, and only when the optional
//! `CODELORE_LLM_*` endpoint is configured.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rmcp::{
    handler::server::wrapper::Parameters, model::ErrorData, tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use codelore_lib::CodeLoreError;
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
        GateViolation, Gates, Thresholds, evaluate_clone_gate, evaluate_code_health_gate,
        evaluate_corpus_percentile_rows, evaluate_full_tree, evaluate_gate_thresholds,
        resolve_defect_calibration,
    },
    repo::{GixRepo, Repo as _},
};
use codelore_lib::complexity::Tier1Language;
use codelore_lib::defect_calibration;

/// Convert any displayable error to an MCP `ErrorData` internal error. Used for
/// genuinely internal failures (serialization, task-join, git process spawn)
/// that carry no `CodeLoreError` variant. Library calls go through
/// [`map_lib_err`], which routes caller-input errors to `invalid_params`.
fn internal(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Map a library error to the correct JSON-RPC error kind. The CLI's exit-2
/// bucket — [`CodeLoreError::InvalidOptions`] / `MalformedTeamMap`, i.e. bad
/// parameters or malformed user config — is a caller-input problem and becomes
/// `invalid_params` (-32602) so a client can tell it supplied bad input; every
/// other variant is a genuine internal/environment failure and stays
/// `internal_error` (-32603).
fn map_lib_err(e: &CodeLoreError) -> ErrorData {
    if e.exit_code() == 2 {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}

/// Default and hard-ceiling row caps for the listing read tools. An unbounded
/// listing can blow the caller's token budget, so every list tool caps its
/// output and discloses the suppressed remainder (see [`serialize_capped_rows`]).
const DEFAULT_ROW_CAP: usize = 50;
const MAX_ROW_CAP: usize = 500;

/// Resolve a caller-supplied `limit` into `1..=MAX_ROW_CAP`, defaulting to
/// `DEFAULT_ROW_CAP` when absent. A `0` clamps up to 1 (a listing tool always
/// returns at least the single worst row).
fn resolve_row_cap(limit: Option<u32>) -> usize {
    limit.map_or(DEFAULT_ROW_CAP, |n| (n as usize).clamp(1, MAX_ROW_CAP))
}

/// Serialize a rank-ordered row slice already truncated to its cap. When rows
/// were suppressed, a trailing `{omitted, total, note}` summary object is
/// appended to the JSON array so a caller sees the list is incomplete; an
/// untruncated list serializes as the bare array the tool has always returned,
/// so the absence of a summary object means the list is complete.
fn serialize_capped_rows<T: Serialize>(
    shown: &[T],
    total: usize,
    note: &str,
) -> std::result::Result<String, ErrorData> {
    let omitted = total.saturating_sub(shown.len());
    if omitted == 0 {
        return serde_json::to_string(shown).map_err(internal);
    }
    let mut arr = shown
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(internal)?;
    arr.push(serde_json::json!({
        "omitted": omitted,
        "total": total,
        "note": note,
    }));
    serde_json::to_string(&arr).map_err(internal)
}

/// Confirm `path` is in the repo's analyzed-file universe (tracked at HEAD).
/// Returns an actionable `invalid_params` error naming the path when it is not —
/// a typo or an absolute path where a repo-relative one is expected otherwise
/// reads as an empty result. Mirrors how `explain_file`'s fact sheet rejects an
/// unknown path.
fn require_tracked_path(repo: &GixRepo, path: &str) -> std::result::Result<(), ErrorData> {
    match repo
        .read_blob_at("HEAD", path)
        .map_err(|e| map_lib_err(&e))?
    {
        Some(_) => Ok(()),
        None => Err(ErrorData::invalid_params(
            format!(
                "path not found among files tracked at HEAD: {path:?} — paths are \
                 repo-relative; try repo_overview or hotspots to list analyzed files"
            ),
            None,
        )),
    }
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

/// Upper bound on distinct entries the result memo keeps alive. Tool outputs are
/// already row-capped (see [`serialize_capped_rows`]), so entries are small; this
/// bound guards only against an unbounded set of distinct `(tool, params)` reads
/// over a long-lived server. When full, the map is cleared wholesale before the
/// next insert — the coarsest bound, and safe because staleness is prevented by
/// the HEAD key, not by which entries happen to survive an eviction.
const MEMO_CAPACITY: usize = 512;

/// Compose a memo key from the tool name and a canonical parameter serialization.
/// `params_json` must be a stable serialization of the already-parsed parameter
/// struct — serializing the deserialized struct (never the raw request) makes the
/// key independent of JSON object key order, so `{"a":1,"b":2}` and `{"b":2,"a":1}`
/// resolve to one entry. The unit separator cannot appear in the tool name, so
/// `tool` and `params_json` can never run together into a colliding key.
fn memo_key(tool: &str, params_json: &str) -> String {
    format!("{tool}\u{1f}{params_json}")
}

/// A cheap cache-key fragment identifying the defect-calibration artifact's
/// current contents. The artifact path is fixed at server startup, but its
/// bytes can be regenerated (e.g. `codelore calibrate` rewriting it) without
/// moving HEAD — which the HEAD-scoped memo would otherwise not see, replaying
/// stale defect evidence. Stat the file for `(len, mtime)`: a rewrite changes at
/// least one, so folding this into the key makes a regenerated artifact miss the
/// prior entry. An absent path or a failed stat yields a stable sentinel (no
/// calibration influence to key on). This is a best-effort probe, not a content
/// hash — a rewrite preserving both length and mtime-to-the-nanosecond is not
/// distinguished, matching the "hint, not contract" stance of the merge probe.
fn calibration_key_fragment(path: Option<&Path>) -> String {
    let Some(md) = path.and_then(|p| std::fs::metadata(p).ok()) else {
        return "cal=none".to_string();
    };
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("cal={}:{mtime}", md.len())
}

/// Process-lifetime memo of committed-state MCP tool outputs (serialized JSON or
/// text — `String` is `Send`, unlike the `!Send` `DuckDB` connection that produced
/// it). Keyed by `(tool, canonical params)` and scoped to a single HEAD sha: the
/// first access at a new HEAD drops every entry, so a stored result can only ever
/// describe the commit that is currently HEAD. Working-tree-dependent tools
/// (`gate_changes`) and tools reading mutable out-of-tree inputs (`check_gates`'
/// thresholds file, `finding_hotspot_overlap`'s findings sidecar) never touch it.
///
/// Two committed-state inputs can change without moving HEAD: an in-progress
/// merge/rebase (which changes a briefing's leading note) and a regenerated
/// defect-calibration artifact (which changes defect evidence). `change_context`
/// folds both into its key, and `explain_file` folds the calibration identity in
/// (see [`calibration_key_fragment`]), so neither replays a stale entry after a
/// mid-process change at unchanged HEAD.
///
/// Concurrency is a single `Mutex` held only for a get or a put, never across the
/// `spawn_blocking` compute. Two concurrent calls that miss the same key both
/// compute and both insert; the values are identical, so last-write-wins is
/// harmless — duplicate compute is accepted rather than adding cross-call dedup.
#[derive(Default)]
struct ResultMemo {
    state: Mutex<MemoState>,
}

#[derive(Default)]
struct MemoState {
    /// The HEAD sha every stored entry was computed at. An access carrying a
    /// different sha clears `entries` before proceeding.
    head: String,
    entries: HashMap<String, String>,
}

impl ResultMemo {
    /// Return a memoized output for `key` valid at `head`, cloning it out under
    /// the lock. A `head` differing from the current scope means every entry
    /// describes a superseded commit: the map is dropped, the scope adopts the
    /// new `head`, and the lookup misses so the caller recomputes.
    fn get(&self, head: &str, key: &str) -> Option<String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.head != head {
            state.entries.clear();
            head.clone_into(&mut state.head);
            return None;
        }
        state.entries.get(key).cloned()
    }

    /// Store `value` under `key` for `head`. If a concurrent access advanced the
    /// scope to a different HEAD between this call's get and put, the result no
    /// longer applies to the current scope and is dropped. A full map is cleared
    /// before the insert (see [`MEMO_CAPACITY`]).
    fn put(&self, head: &str, key: String, value: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.head != head {
            return;
        }
        if state.entries.len() >= MEMO_CAPACITY && !state.entries.contains_key(&key) {
            state.entries.clear();
        }
        state.entries.insert(key, value);
    }
}

/// Run `compute` under the result memo. Opens the repo, resolves HEAD, and
/// returns a memoized output for `(tool, params_json)` when one exists at that
/// HEAD; otherwise runs `compute` (which receives the open repo and the resolved
/// HEAD sha), memoizes its `Ok` output, and returns it. Errors propagate without
/// being memoized. HEAD is resolved in-process via `gix` immediately before the
/// (skipped-on-hit) ingest, so a hit pays only the repo open plus a hash lookup.
fn memoized<F>(
    memo: &ResultMemo,
    repo_path: &Path,
    tool: &str,
    params_json: &str,
    compute: F,
) -> std::result::Result<String, ErrorData>
where
    F: FnOnce(&GixRepo, &str) -> std::result::Result<String, ErrorData>,
{
    let repo = GixRepo::open(repo_path).map_err(|e| map_lib_err(&e))?;
    let head = repo.head_sha().map_err(|e| map_lib_err(&e))?;
    let key = memo_key(tool, params_json);
    if let Some(hit) = memo.get(&head, &key) {
        return Ok(hit);
    }
    let out = compute(&repo, head.as_str())?;
    memo.put(&head, key, out.clone());
    Ok(out)
}

/// How many tool calls may occupy the blocking pool at once.
///
/// A tool call is not a cheap request: a cold one ingests the whole history
/// into `DuckDB` and fans out over `rayon`. Without a bound, an agent issuing
/// N calls in parallel starts N of those at once, each with its own connection
/// and its own thread pool, and they contend rather than finish. Four keeps a
/// burst of tool calls making progress while leaving the machine usable; the
/// rest queue and run as permits free up.
const MAX_CONCURRENT_CALLS: usize = 4;

/// MCP server state — the repo path and defect-calibration configuration
/// fixed at server startup, plus the process-lifetime result memo.
#[derive(Clone)]
pub struct CodeLoreServer {
    repo: PathBuf,
    defect_calibration: Option<PathBuf>,
    allow_foreign_calibration: bool,
    /// Shared across every concurrent tool call (hence `Arc`); see [`ResultMemo`].
    memo: Arc<ResultMemo>,
    /// Shared admission control for the blocking pool; see [`MAX_CONCURRENT_CALLS`].
    limit: Arc<Semaphore>,
}

impl CodeLoreServer {
    /// Run `work` on the blocking pool, holding one concurrency permit for its
    /// whole duration.
    ///
    /// Every tool body goes through here rather than calling `spawn_blocking`
    /// directly, so the bound cannot be forgotten by a tool added later. The
    /// permit is moved into the closure, so it is released when the blocking
    /// work finishes — including when it panics, since the release rides the
    /// unwind.
    async fn blocking<T, W>(&self, work: W) -> Result<T, ErrorData>
    where
        W: FnOnce() -> Result<T, ErrorData> + Send + 'static,
        T: Send + 'static,
    {
        let permit = Arc::clone(&self.limit)
            .acquire_owned()
            .await
            .map_err(internal)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(internal)?
    }
}

// ── Parameter structs (one per tool) ─────────────────────────────────────────

/// Parameters for the `repo_overview` tool (none required).
#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
pub struct RepoOverviewParams {}

/// Parameters for the `hotspots` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
pub struct HotspotsParams {
    /// Maximum rows to return (default 50, max 500).
    pub limit: Option<u32>,
}

/// Parameters for the `code_health` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
pub struct CodeHealthParams {
    /// Filter results to this file path (relative to repo root).
    /// Omit to return all files with complexity data.
    pub path: Option<String>,
    /// When listing (no `path`), the maximum rows to return, worst-health
    /// first (default: 50, clamped to 1..=500). A trailing summary object
    /// discloses any suppressed rows.
    pub limit: Option<u32>,
}

/// Parameters for the `delta_health` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DeltaHealthParams {
    /// Base revision (branch, tag, or full SHA). Must be resolvable by `git rev-parse`.
    pub base: String,
    /// Head revision (branch, tag, or full SHA). Must be resolvable by `git rev-parse`.
    pub head: String,
    /// Maximum per-function rows to return (default: 50, clamped to 1..=500).
    /// An `omitted_functions` count is added when rows are suppressed.
    pub limit: Option<u32>,
}

/// Parameters for the `refactoring_targets` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema, Default)]
pub struct RefactoringTargetsParams {
    /// Maximum rows to return, highest-priority first (default: 50, clamped to
    /// 1..=500). A trailing summary object discloses suppressed rows.
    pub limit: Option<u32>,
}

/// Parameters for the `function_xray` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FunctionXrayParams {
    /// File path (relative to repo root) to analyse.
    pub path: String,
}

/// Parameters for the `check_gates` tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct CheckGatesParams {
    /// Maximum violation rows to return (default: 50, clamped to 1..=500).
    /// `violation_count` always reports the true total, so a capped list never
    /// changes the verdict or the number an agent reports.
    pub limit: Option<u32>,
}

/// Parameters for the `finding_hotspot_overlap` tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct FindingHotspotOverlapParams {
    /// Maximum rows to return, highest-priority (`act-now`) first (default: 50,
    /// clamped to 1..=500). A trailing summary object discloses suppressed rows.
    pub limit: Option<u32>,
}

/// Parameters for the `explain_file` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ExplainFileParams {
    /// File path (relative to repo root) to build the evidence dossier for
    /// (e.g. "src/main.rs").
    pub path: String,
}

/// Parameters for the `change_context` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
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
    /// Configured gates that produced no verdict in this response, each with the
    /// reason — so an empty `violations` list is distinguishable from "did not
    /// run". Empty when nothing is skipped. See [`SkippedGate`].
    skipped_gates: Vec<SkippedGate>,
}

/// One gate `check_gates` produced no verdict for, paired with why. A skip is
/// either *structural* — a gate this tool never evaluates because its
/// committed-tree input (the corpus lens, the external-findings sidecar,
/// degraded-gate handling) is `codelore check`-only — or *runtime*: a
/// `[new_code]` gate with no pre-window baseline in this checkout. Carrying the
/// reason beside the name is what lets a caller tell an empty `violations` list
/// ("all gates passed") apart from "this gate did not run".
#[derive(Debug, Serialize)]
struct SkippedGate {
    /// The gate that produced no verdict.
    gate: &'static str,
    /// Why it produced none — a structural limit of this tool, or a runtime
    /// condition of this repository.
    reason: String,
}

/// The `[gates]` gates configured in `thresholds` that `check_gates` does not
/// evaluate. `codelore check` evaluates every `[gates]` gate; this tool omits
/// the ones whose committed-tree inputs it does not carry (external findings,
/// the corpus lens), so the disclosure is (configured gates) − (the set
/// evaluated here). Returned in a stable declaration order; empty when nothing
/// is skipped.
fn skipped_check_gates(thresholds: &Thresholds) -> Vec<SkippedGate> {
    // Gates this tool evaluates, kept beside the evaluation calls in
    // `check_gates`. A configured `[gates]` gate absent from this set is
    // disclosed as skipped, so a gate added to the config surfaces here until
    // the tool is taught to evaluate it.
    const EVALUATED_HERE: &[&str] = &[
        "cognitive_max",
        "hotspot_score_max",
        "code_health_min",
        "disallow_clone_type_1",
        "max_dependency_cycles",
        "max_propagation_cost",
        "max_red_effort_pct",
        "code_familiarity_min",
        // Reuses the code-health rows already computed for `code_health_min` —
        // the corpus lens fills `corpus_percentile` on those rows whenever a
        // calibration artifact is active (the embedded world artifact by
        // default), so this tool needs no extra data to evaluate it for real.
        // See the runtime skip in `check_gates` for the no-artifact-active case.
        "corpus_percentile_max",
    ];
    // Exhaustive destructuring: adding a field to `Gates` fails to compile
    // here until the new gate is classified as evaluated or skipped.
    let Gates {
        cognitive_max,
        code_health_min,
        hotspot_score_max,
        hotspot_anchored_max,
        disallow_clone_type_1,
        max_dependency_cycles,
        max_propagation_cost,
        max_red_effort_pct,
        code_familiarity_min,
        max_findings_in_hot_files,
        corpus_percentile_max,
        fail_on_degraded,
        // Cross-surface exit-code policy (`codelore check` / `gate` / `diff`),
        // not a gate this tool applies — disclosed as skipped when the user
        // enables it, so a pass here is never mistaken for a full run's verdict.
        fail_on_skipped,
        // A modifier of max_red_effort_pct, not a standalone gate. This tool
        // DOES honor it (the effort-exposure gate above decomposes when set), so
        // it is neither reported as an evaluated gate nor as skipped.
        red_effort_exempt_improving: _,
    } = &thresholds.gates;
    let configured: [(&'static str, bool); 13] = [
        ("cognitive_max", cognitive_max.is_some()),
        ("hotspot_score_max", hotspot_score_max.is_some()),
        ("hotspot_anchored_max", hotspot_anchored_max.is_some()),
        ("code_health_min", code_health_min.is_some()),
        ("disallow_clone_type_1", *disallow_clone_type_1),
        ("max_dependency_cycles", max_dependency_cycles.is_some()),
        ("max_propagation_cost", max_propagation_cost.is_some()),
        ("max_red_effort_pct", max_red_effort_pct.is_some()),
        ("code_familiarity_min", code_familiarity_min.is_some()),
        (
            "max_findings_in_hot_files",
            max_findings_in_hot_files.is_some(),
        ),
        ("corpus_percentile_max", corpus_percentile_max.is_some()),
        // Defaults to true, so degraded-gate semantics are active in almost
        // every `codelore check` run while this tool never applies them —
        // disclosed whenever active.
        ("fail_on_degraded", *fail_on_degraded),
        // Opt-in exit-code policy this tool never applies — disclosed only when
        // the user sets it (defaults false).
        ("fail_on_skipped", *fail_on_skipped),
    ];
    configured
        .into_iter()
        .filter(|(name, set)| *set && !EVALUATED_HERE.contains(name))
        .map(|(gate, _)| SkippedGate {
            gate,
            reason: structural_skip_reason(gate).to_owned(),
        })
        .collect()
}

/// Why a configured `[gates]` gate is not evaluated by `check_gates`: its
/// committed-tree input is `codelore check`-only.
fn structural_skip_reason(gate: &str) -> &'static str {
    match gate {
        "max_findings_in_hot_files" => {
            "reads the external-findings sidecar, which is `codelore check`-only \
             (run `codelore ingest-sarif`, then `codelore check`)"
        }
        // `corpus_percentile_max` is NOT here — this tool evaluates it for
        // real (see `EVALUATED_HERE` above); its own runtime skip (no
        // calibration artifact active) is reported directly from `check_gates`.
        "hotspot_anchored_max" => {
            "depends on the calibration-corpus lens, which this tool does not carry \
             (it uses the plain, unanchored hotspot scan); `codelore check` is authoritative"
        }
        "fail_on_degraded" => "degraded-gate handling is `codelore check`-only",
        "fail_on_skipped" => "skipped-gate handling is `codelore check` / `gate` / `diff`-only",
        _ => "evaluated only by `codelore check`, which is authoritative for it",
    }
}

#[tool_router]
impl CodeLoreServer {
    // ── repo_overview ─────────────────────────────────────────────────────────

    #[tool(
        name = "repo_overview",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Return a JSON object with `summary` (commit count, authors, files, date range) \
            and `options` (the active analysis options snapshot used for cache-keying). \
            First call on a cold cache triggers history ingest; warm-cache calls are fast."
    )]
    async fn repo_overview(
        &self,
        params: Parameters<RepoOverviewParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let memo = self.memo.clone();
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        self.blocking(move || {
            memoized(
                &memo,
                &repo_path,
                "repo_overview",
                &params_json,
                |repo, head| {
                    let opts = Options {
                        repo_path: repo_path.clone(),
                        ..Options::default()
                    };
                    let db =
                        FactsDb::open_or_ingest_with_cache_root(&opts, repo, &default_cache_root())
                            .map_err(|e| map_lib_err(&e))?;
                    // Same ingest witness as `check_gates`: a real HEAD over an
                    // empty commit store is a truncated checkout, not a genuinely
                    // empty repo.
                    db.ensure_ingest_witnessed(head)
                        .map_err(|e| map_lib_err(&e))?;
                    let rows = summary::run_summary(&db, &opts).map_err(|e| map_lib_err(&e))?;
                    let out = serde_json::json!({
                        "summary": rows,
                        "options": opts.canonical_json(),
                    });
                    serde_json::to_string(&out).map_err(internal)
                },
            )
        })
        .await
    }

    // ── hotspots ──────────────────────────────────────────────────────────────

    #[tool(
        name = "hotspots",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Return the top hotspot files ranked by revision count as JSON. \
            Pass `limit` to cap rows (default 50, max 500). \
            First call on a cold cache triggers history ingest."
    )]
    async fn hotspots(&self, params: Parameters<HotspotsParams>) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let memo = self.memo.clone();
        // Clamp through the shared row cap (1..=500) like every other list tool so
        // an oversized `limit` cannot blow the caller's token budget.
        let cap = resolve_row_cap(params.0.limit);
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        self.blocking(move || {
            memoized(&memo, &repo_path, "hotspots", &params_json, |repo, head| {
                let opts = Options {
                    repo_path: repo_path.clone(),
                    ..Options::default()
                };
                let db =
                    FactsDb::open_or_ingest_with_cache_root(&opts, repo, &default_cache_root())
                        .map_err(|e| map_lib_err(&e))?;
                // Same ingest witness as `check_gates`: a real HEAD over an
                // empty commit store is a truncated checkout, not a genuinely
                // empty repo.
                db.ensure_ingest_witnessed(head)
                    .map_err(|e| map_lib_err(&e))?;
                // Run unbounded (the ranking spans the full population either
                // way) and cap in-tool, matching the sibling list tools —
                // including their disclosure. The file's convention is that a
                // bare array means the list is complete, so truncating without
                // the `{omitted, total}` object let an agent read a cut-off
                // ranking as exhaustive.
                let rows = hotspots::run_hotspots(&db, &opts).map_err(|e| map_lib_err(&e))?;
                let total = rows.len();
                let shown = &rows[..total.min(cap)];
                serialize_capped_rows(
                    shown,
                    total,
                    "hotspot rows beyond the row cap were omitted; raise `limit` to see more",
                )
            })
        })
        .await
    }

    // ── code_health ───────────────────────────────────────────────────────────

    #[tool(
        name = "code_health",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Return per-file composite code-health scores (band: red/yellow/green, score 0–100) as JSON. \
            Pass `path` to filter to a single file; an unknown path returns an error naming it, \
            not an empty result. Otherwise the list is worst-health first, capped by `limit` \
            (default 50, max 500), with a trailing `{omitted, total, note}` summary object when rows \
            are suppressed. \
            First call on a cold cache triggers history ingest."
    )]
    async fn code_health(&self, params: Parameters<CodeHealthParams>) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let memo = self.memo.clone();
        let filter_path = params.0.path.clone();
        let cap = resolve_row_cap(params.0.limit);
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        self.blocking(move || {
            memoized(&memo, &repo_path, "code_health", &params_json, |repo, head| {
                let opts = Options {
                    repo_path: repo_path.clone(),
                    ..Options::default()
                };
                // A path outside the analyzed-file universe is a caller error, not
                // an empty single-file result — reject it before the ingest (the
                // error propagates un-memoized). A tracked file with no health row
                // (e.g. below the revision floor) legitimately returns [].
                if let Some(p) = &filter_path {
                    require_tracked_path(repo, p)?;
                }
                let db =
                    FactsDb::open_or_ingest_with_cache_root(&opts, repo, &default_cache_root())
                        .map_err(|e| map_lib_err(&e))?;
                // Same ingest witness as `check_gates`: a real HEAD over an empty
                // commit store is a truncated checkout, not a genuinely empty repo.
                db.ensure_ingest_witnessed(head).map_err(|e| map_lib_err(&e))?;
                let mut rows =
                    code_health::run_code_health(&db, &opts).map_err(|e| map_lib_err(&e))?;
                if let Some(p) = &filter_path {
                    rows.retain(|r| &r.path == p);
                    return serde_json::to_string(&rows).map_err(internal);
                }
                // Rows arrive worst-health first (score ascending); cap the tail.
                let total = rows.len();
                rows.truncate(cap);
                serialize_capped_rows(
                    &rows,
                    total,
                    "worst-health files first; raise limit (max 500) or pass a path for the rest",
                )
            })
        })
        .await
    }

    // ── delta_health ──────────────────────────────────────────────────────────

    #[tool(
        name = "delta_health",
        // Not read-only: this is the one tool that writes outside the cache,
        // checking each rev out into a throwaway `git worktree` it then
        // removes. Additive and repeatable, hence the two hints below.
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        description = "Return a function-level health delta between two revisions as JSON. \
            `base` and `head` are any rev-parse-able strings (branch, tag, SHA). \
            Returns verdict (improved/neutral/degraded), ratio, and per-function breakdown. \
            This is a simplified subset of `codelore diff`: clone-group membership and \
            base red-file context are not scored here — run `codelore diff` for the full report. \
            Pass `limit` to cap the per-function rows (default 50, max 500); an `omitted_functions` \
            count is added when rows are suppressed. \
            Cost: ingests history twice (once per rev); expect 5–30 s on a cold cache."
    )]
    async fn delta_health(
        &self,
        params: Parameters<DeltaHealthParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let memo = self.memo.clone();
        let base_rev = params.0.base.clone();
        let head_rev = params.0.head.clone();
        let cap = resolve_row_cap(params.0.limit);

        // Resolve revisions up front on the async thread (cheap git I/O). The
        // resolved SHAs — not the raw rev strings — key the memo, so a branch
        // name that later moves to a new commit misses the old entry.
        let base_sha = resolve_rev(&repo_path, &base_rev)?;
        let head_sha = resolve_rev(&repo_path, &head_rev)?;
        if base_sha == head_sha {
            return Err(ErrorData::invalid_params(
                format!("base and head both resolve to {base_sha}; nothing to diff"),
                None,
            ));
        }

        self.blocking(move || {
            // Scope the memo to the repo's current HEAD (uniform with the other
            // tools); the diff endpoints are the resolved base/head SHAs, so the
            // result is stable regardless of HEAD — a HEAD move only over-evicts.
            let main_repo = GixRepo::open(&repo_path).map_err(|e| map_lib_err(&e))?;
            let head = main_repo.head_sha().map_err(|e| map_lib_err(&e))?;
            let key = memo_key(
                "delta_health",
                &format!("base={base_sha}\u{1f}head={head_sha}\u{1f}limit={cap}"),
            );
            if let Some(hit) = memo.get(&head, &key) {
                return Ok(hit);
            }

            // Ingest each rev in its own worktree → in-memory DB.
            let (base_path, _base_wt) = temp_worktree(&repo_path, &base_sha)?;
            let (head_path, _head_wt) = temp_worktree(&repo_path, &head_sha)?;

            let ingest_at = |wt: &Path,
                             sha: &str|
             -> std::result::Result<
                Vec<codelore_lib::cli_api::analyses::delta_health::FunctionMetricRow>,
                ErrorData,
            > {
                let opts = Options {
                    repo_path: wt.to_path_buf(),
                    ..Options::default()
                };
                let repo = GixRepo::open(wt).map_err(|e| map_lib_err(&e))?;
                let db = FactsDb::new_in_memory().map_err(|e| map_lib_err(&e))?;
                db.ingest(&repo, &opts).map_err(|e| map_lib_err(&e))?;
                // Witness the populated store before deriving metrics: a detached
                // worktree over a shallow/truncated checkout can walk zero commits
                // and yield a well-formed but vacuous delta of plausible zeroes.
                // Error out (exit 3) rather than building and memoizing that.
                db.ensure_ingest_witnessed(sha)
                    .map_err(|e| map_lib_err(&e))?;
                run_function_metrics(&db).map_err(|e| map_lib_err(&e))
            };

            let base_fns = ingest_at(&base_path, &base_sha)?;
            let head_fns = ingest_at(&head_path, &head_sha)?;

            // Every path present at EITHER revision. Taking head alone looks
            // equivalent — head holds nearly every path — but it silently
            // drops the two cases that matter most:
            //
            // * A deleted file has no head rows, so its path never entered the
            //   set and its base rows were filtered out before the comparison
            //   ran. Deleting the worst file in the repository scored as
            //   no-change, which is the single most decisive health
            //   improvement a change can make.
            // * A renamed file holds its functions under the old path at base
            //   and the new path at head. With only the new path in the set,
            //   the base side vanished and every function read as an addition
            //   — so moving a large complex file unchanged produced a
            //   degrading verdict.
            //
            // The union is not "files the PR touched": `run_function_metrics`
            // returns every function at a revision. Change selection is done
            // by `compute_delta_health`, which skips rows identical on both
            // sides. This set only has to avoid hiding a side from it.
            //
            // A rename still reads as a removal plus an addition rather than a
            // paired move — pairing needs git's rename detection, which
            // `codelore diff` has and this path does not.
            let pr_files: HashSet<String> = base_fns
                .iter()
                .chain(head_fns.iter())
                .map(|r| r.path.clone())
                .collect();
            let clone_members: HashSet<(String, String)> = HashSet::new();
            let base_red: HashSet<String> = HashSet::new();

            let mut section: DeltaHealthSection =
                compute_delta_health(&base_fns, &head_fns, &pr_files, &clone_members, &base_red);

            // Bound the per-function rows so a large diff cannot blow the
            // caller's token budget. The rows are (path, name)-ordered, so a
            // truncation drops the lexicographically-last functions; the added
            // `omitted_functions` count discloses it and `codelore diff` gives
            // the full list.
            let total_fns = section.functions.len();
            let omitted_fns = total_fns.saturating_sub(cap);
            if omitted_fns > 0 {
                section.functions.truncate(cap);
            }
            let mut value = serde_json::to_value(&section).map_err(internal)?;
            if omitted_fns > 0 {
                value["omitted_functions"] = serde_json::json!(omitted_fns);
            }
            let out = serde_json::to_string(&value).map_err(internal)?;
            memo.put(&head, key, out.clone());
            Ok(out)
        })
        .await
    }

    // ── refactoring_targets ───────────────────────────────────────────────────

    #[tool(
        name = "refactoring_targets",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Return the highest-priority refactoring candidates ranked by risk÷LOC as JSON. \
            Pass `limit` to cap rows (default 50, max 500); a trailing `{omitted, total, note}` \
            summary object discloses any suppressed rows. \
            First call on a cold cache triggers history ingest."
    )]
    async fn refactoring_targets(
        &self,
        params: Parameters<RefactoringTargetsParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let memo = self.memo.clone();
        let cap = resolve_row_cap(params.0.limit);
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        self.blocking(move || {
            memoized(
                &memo,
                &repo_path,
                "refactoring_targets",
                &params_json,
                |repo, head| {
                    let opts = Options {
                        repo_path: repo_path.clone(),
                        ..Options::default()
                    };
                    let db =
                        FactsDb::open_or_ingest_with_cache_root(&opts, repo, &default_cache_root())
                            .map_err(|e| map_lib_err(&e))?;
                    // Same ingest witness as `check_gates`: a real HEAD over an
                    // empty commit store is a truncated checkout, not a genuinely
                    // empty repo.
                    db.ensure_ingest_witnessed(head).map_err(|e| map_lib_err(&e))?;
                    // Run unbounded so the true total is known, then cap in-tool
                    // with the omitted disclosure (the analysis ranks over the
                    // full set either way).
                    let mut rows = refactoring_targets::run_refactoring_targets(&db, &opts)
                        .map_err(|e| map_lib_err(&e))?;
                    let total = rows.len();
                    rows.truncate(cap);
                    serialize_capped_rows(
                        &rows,
                        total,
                        "highest-priority refactor targets first; raise limit (max 500) for the rest",
                    )
                },
            )
        })
        .await
    }

    // ── function_xray ─────────────────────────────────────────────────────────

    #[tool(
        name = "function_xray",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Return per-function change-frequency and complexity for a file as a JSON array. \
            `path` is the file path relative to the repo root (e.g. \"src/main.rs\"). \
            A path not tracked at HEAD returns an error naming it, not an empty array; \
            a tracked file in a language without function analysis returns a `{note}` object; \
            a tracked source file with no parsed functions returns []. \
            First call on a cold cache triggers history ingest."
    )]
    async fn function_xray(
        &self,
        params: Parameters<FunctionXrayParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let memo = self.memo.clone();
        let target = params.0.path.clone();
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        self.blocking(move || {
            memoized(&memo, &repo_path, "function_xray", &params_json, |repo, head| {
                let opts = Options {
                    repo_path: repo_path.clone(),
                    ..Options::default()
                };
                // Resolve the path against the analyzed-file universe first: a
                // typo or absolute path is a caller error (propagated
                // un-memoized), not a file "with no functions".
                require_tracked_path(repo, &target)?;
                // A tracked file the function analyser does not support (not a
                // Tier-1 language) legitimately yields no functions — say that,
                // rather than returning a bare [] that reads like an empty result.
                if Tier1Language::from_path(&target).is_none() {
                    let note = serde_json::json!({
                        "functions": [],
                        "note": format!(
                            "{target} is tracked but not a Tier-1 source file (function analysis \
                             covers Rust, Python, Java, JavaScript, TypeScript); no per-function \
                             breakdown available"
                        ),
                    });
                    return serde_json::to_string(&note).map_err(internal);
                }
                let db =
                    FactsDb::open_or_ingest_with_cache_root(&opts, repo, &default_cache_root())
                        .map_err(|e| map_lib_err(&e))?;
                // Same ingest witness as `check_gates`: a real HEAD over an empty
                // commit store is a truncated checkout, not a genuinely empty repo.
                db.ensure_ingest_witnessed(head).map_err(|e| map_lib_err(&e))?;
                let rows = function_xray::run_function_xray(&db, repo, &opts, &target)
                    .map_err(|e| map_lib_err(&e))?;
                serde_json::to_string(&rows).map_err(internal)
            })
        })
        .await
    }

    // ── check_gates ───────────────────────────────────────────────────────────

    #[tool(
        name = "check_gates",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Evaluate `.codelore-thresholds.toml` quality gates at HEAD and return a JSON \
            summary with verdict (pass/fail/no_thresholds), violation count, individual violations, \
            and a `skipped_gates` array of `{gate, reason}` for every configured gate that produced \
            no verdict — so an empty `violations` list is distinguishable from a gate that did not run. \
            This tool evaluates a subset of `codelore check`: the `max_findings_in_hot_files` and \
            `hotspot_anchored_max` gates, `--ratchet`, and degraded-gate handling remain check-only, \
            so a config using those can make this verdict diverge — `codelore check` is authoritative. \
            A configured `[new_code]` gate that finds no pre-window baseline (a young repository, or a \
            shallow fetch-depth checkout) is reported here too, with a reason that names fetch-depth \
            when the checkout is shallow. \
            Returns `no_thresholds` verdict when no config file is found. \
            The `violations` array is capped (default: 50, raise `limit` for more) while \
            `violation_count` always reports the true total, so the verdict and the count \
            are never affected by the cap. \
            First call on a cold cache triggers history ingest."
    )]
    async fn check_gates(&self, params: Parameters<CheckGatesParams>) -> Result<String, ErrorData> {
        let cap = resolve_row_cap(params.0.limit);
        let repo_path = self.repo.clone();
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        // Deliberately not memoized: the verdict depends on the current contents
        // of `.codelore-thresholds.toml`, an on-disk config re-discovered every
        // call that can be edited (or created) without a commit, so a HEAD-keyed
        // entry could serve a stale verdict after a threshold edit.
        self.blocking(move || {
            let thresholds = Thresholds::discover(&repo_path).map_err(|e| map_lib_err(&e))?;
            if thresholds.is_empty() {
                let summary = GateSummary {
                    verdict: "no_thresholds".into(),
                    violation_count: 0,
                    violations: Vec::<GateViolation>::new(),
                    skipped_gates: Vec::new(),
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
            let repo = GixRepo::open(&repo_path).map_err(|e| map_lib_err(&e))?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(|e| map_lib_err(&e))?;
            // Same ingest witness as `codelore check`: a real HEAD over an empty
            // commit store is a truncated checkout on which every gate passes over
            // no data. An agent must get a hard error, never a spurious green.
            let head_sha = repo.head_sha().map_err(|e| map_lib_err(&e))?;
            db.ensure_ingest_witnessed(&head_sha)
                .map_err(|e| map_lib_err(&e))?;

            let mut violations: Vec<GateViolation> = Vec::new();

            // hotspot-scoped gates (cognitive_max, hotspot_score_max)
            let hs = hotspots::run_hotspots(&db, &opts).map_err(|e| map_lib_err(&e))?;
            violations.extend(evaluate_full_tree(&thresholds, &hs));

            // code_health_min gate
            let ch = code_health::run_code_health(&db, &opts).map_err(|e| map_lib_err(&e))?;
            violations.extend(evaluate_code_health_gate(&thresholds, &ch));

            // corpus_percentile_max gate — reuses `ch`: `run_code_health` always
            // runs the corpus lens (a no-op without an active artifact), so
            // `corpus_percentile` is already populated on these rows whenever a
            // calibration artifact is active (the embedded world artifact by
            // default, or `--calibration`). Mirrors `codelore check`'s own
            // `evaluate_corpus_percentile_rows` call and its skip convention
            // (`check.rs`): a SKIP, not a pass, when no artifact is active at all.
            let mut corpus_skip: Option<SkippedGate> = None;
            if let Some(max) = thresholds.gates.corpus_percentile_max {
                let has_calibration = ch.iter().any(|r| r.corpus_percentile.is_some());
                if has_calibration {
                    violations.extend(evaluate_corpus_percentile_rows(max, &ch));
                } else {
                    corpus_skip = Some(SkippedGate {
                        gate: "corpus_percentile_max",
                        reason: crate::CORPUS_PERCENTILE_SKIP_REASON.into(),
                    });
                }
            }

            // clone gate
            violations.extend(evaluate_clone_gate(&thresholds, &db).map_err(|e| map_lib_err(&e))?);

            // effort-exposure gate — reuses the code-health rows computed for
            // code_health_min so the heaviest analysis runs once per call. With
            // the improving-churn exemption on, the red band is decomposed (a
            // scoped window-start parse of the red files via the repo) so the
            // gate compares the degrading share; off, the base rows are used.
            if let Some(max) = thresholds.gates.max_red_effort_pct {
                use codelore_lib::cli_api::analyses::effort_exposure;
                let exempt = thresholds.gates.red_effort_exempt_improving;
                let no_limit = opts.with_no_row_limit();
                let rows = if exempt {
                    effort_exposure::run_effort_exposure_decomposed(&db, &repo, &no_limit, &ch)
                } else {
                    effort_exposure::run_effort_exposure_with_health(&db, &no_limit, &ch)
                }
                .map_err(|e| map_lib_err(&e))?;
                violations.extend(
                    codelore_lib::cli_api::quality_gates::evaluate_effort_exposure_rows_exempt(
                        max, exempt, &rows,
                    ),
                );
            }

            // [new_code] two-band period gate — mirrors `codelore check`: reuses
            // the code-health rows and the effort-exposure window-start
            // machinery, evaluating the born + touched bands over the committed
            // HEAD. A run whose history is shallower than the window is disclosed
            // as a runtime skip in `skipped_gates` below, with a reason that names
            // fetch-depth when the checkout is shallow — so an empty violations
            // list is never mistaken for "the new-code gate passed".
            let mut new_code_skip: Option<SkippedGate> = None;
            if let Some(nc) = &thresholds.new_code {
                use codelore_lib::cli_api::analyses::new_code;
                let scope = new_code::run_new_code_scope(&db, &repo, &opts, nc.window_days, &ch)
                    .map_err(|e| map_lib_err(&e))?;
                if scope.window_start_present {
                    violations.extend(
                        codelore_lib::cli_api::quality_gates::evaluate_new_code_rows(nc, &scope),
                    );
                } else {
                    new_code_skip = Some(SkippedGate {
                        gate: "new_code",
                        reason: crate::new_code_skip_reason(
                            f64::from(nc.window_days),
                            repo.is_shallow(),
                        ),
                    });
                }
            }

            // architecture + familiarity gates. This tool evaluates a subset
            // of `codelore check`: the `max_findings_in_hot_files` and
            // `hotspot_anchored_max` gates, degraded-gate semantics, and
            // `--ratchet` remain check-only — `skipped_gates` (below) names any
            // that this config configured, so a client sees where this verdict
            // can diverge from a CI run. `codelore check` is authoritative.
            violations.extend(
                codelore_lib::cli_api::quality_gates::evaluate_architecture_gate(&thresholds, &db)
                    .map_err(|e| map_lib_err(&e))?,
            );
            violations.extend(
                codelore_lib::cli_api::quality_gates::evaluate_familiarity_gate(
                    &thresholds,
                    &db,
                    &opts,
                )
                .map_err(|e| map_lib_err(&e))?,
            );

            let verdict = if violations.is_empty() {
                "pass"
            } else {
                "fail"
            };
            let mut skipped_gates = skipped_check_gates(&thresholds);
            skipped_gates.extend(new_code_skip);
            skipped_gates.extend(corpus_skip);
            // `violation_count` is measured BEFORE truncation and stays the true
            // total, so the verdict and the count an agent reports are never
            // affected by the cap — only how many rows it reads. A wide refactor
            // against a tight gate otherwise emitted one row per violating file
            // into the agent's context with no bound.
            //
            // Capped here, unlike the `codelore gate` JSON document which carries
            // every row by design (see GATE_FINDINGS_ROWS): that document is a
            // file artifact, this response is context-window budget.
            let violation_count = violations.len();
            violations.truncate(cap);
            let summary = GateSummary {
                verdict: verdict.into(),
                violation_count,
                violations,
                skipped_gates,
            };
            serde_json::to_string(&summary).map_err(internal)
        })
        .await
    }

    // ── finding_hotspot_overlap ───────────────────────────────────────────────

    #[tool(
        name = "finding_hotspot_overlap",
        annotations(read_only_hint = true, open_world_hint = false),
        description = "Return the behavioral×static fusion table: external scanner findings \
            joined with hotspot rank and code-health band, producing an `act-now` / `plan` / `note` \
            priority for each flagged file. Requires a prior `codelore ingest-sarif` run to populate \
            the external findings sidecar; returns a structured note when the sidecar is absent or empty. \
            Rows are highest-priority first, capped by `limit` (default 50, max 500), with a trailing \
            `{omitted, total, note}` summary object when rows are suppressed. \
            Cost: warm-cache fast after ingest; does not trigger history re-ingest."
    )]
    async fn finding_hotspot_overlap(
        &self,
        params: Parameters<FindingHotspotOverlapParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let cap = resolve_row_cap(params.0.limit);
        // Deliberately not memoized: the fusion joins the external-findings
        // sidecar, an on-disk store rewritten by `codelore ingest-sarif` with no
        // change to HEAD, and the DuckDB-backed sidecar exposes no cheap content
        // digest to key on (its mtime is unreliable under read-time journaling),
        // so a HEAD-only key could serve stale fusion after a re-ingest.
        self.blocking(move || {
            let cache_root = default_cache_root();

            // open_nonempty returns None when the sidecar is absent OR
            // present-but-empty — the MCP tool never creates it; that is
            // ingest-sarif's job. Both cases return the structured "run
            // ingest-sarif first" note.
            let Some(store) = ExternalStore::open_nonempty(&cache_root, &repo_path)
                .map_err(|e| map_lib_err(&e))?
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
            let repo = GixRepo::open(&repo_path).map_err(|e| map_lib_err(&e))?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
                .map_err(|e| map_lib_err(&e))?;
            // Same ingest witness as `check_gates`: a real HEAD over an empty
            // commit store is a truncated checkout, not a genuinely empty repo.
            let head_sha = repo.head_sha().map_err(|e| map_lib_err(&e))?;
            db.ensure_ingest_witnessed(&head_sha)
                .map_err(|e| map_lib_err(&e))?;

            let mut rows = finding_hotspot_overlap::run_finding_hotspot_overlap(&db, &opts, &store)
                .map_err(|e| map_lib_err(&e))?;
            let total = rows.len();
            rows.truncate(cap);
            serialize_capped_rows(
                &rows,
                total,
                "act-now findings first; raise limit (max 500) for the rest",
            )
        })
        .await
    }

    // ── explain_file ──────────────────────────────────────────────────────────

    #[tool(
        name = "explain_file",
        // Open-world: the advisory narrative is off by default, but when
        // CODELORE_LLM_* is configured this tool calls that endpoint.
        annotations(read_only_hint = true, open_world_hint = true),
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
        let memo = self.memo.clone();
        let target = params.0.path.clone();
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        self.blocking(move || {
            let repo = GixRepo::open(&repo_path).map_err(|e| map_lib_err(&e))?;
            let head = repo.head_sha().map_err(|e| map_lib_err(&e))?;
            // Fold the calibration-artifact identity into the key: the dossier's
            // defect-evidence section reads that artifact, which can be
            // regenerated without moving HEAD (see [`calibration_key_fragment`]).
            let key = memo_key(
                "explain_file",
                &format!(
                    "{params_json}\u{1f}{}",
                    calibration_key_fragment(defect_calibration.as_deref())
                ),
            );

            // The advisory narrative is an external, potentially-nondeterministic
            // LLM call, so explain_file is memoized only in its deterministic
            // no-LLM form: when the process has an LLM configured the memo is
            // bypassed end to end and the narrative is produced fresh every call.
            // The LLM configuration is read from the process environment, fixed
            // for the server's lifetime.
            let client = resolve_client(&LlmEnv::from_process_env());
            let memoizable = client.is_err();
            if memoizable && let Some(hit) = memo.get(&head, &key) {
                return Ok(hit);
            }

            // A path outside the analyzed-file universe is a caller error, not an
            // empty dossier — reject it before the ingest so the response is an
            // actionable invalid_params naming the path (propagated un-memoized),
            // mirroring code_health and function_xray.
            require_tracked_path(&repo, &target)?;

            // min_revs = 1 so any single named file resolves in its own dossier,
            // matching the `explain <path>` CLI surface.
            let opts = Options {
                repo_path: repo_path.clone(),
                min_revs: 1,
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(|e| map_lib_err(&e))?;
            // Same ingest witness as `check_gates`: a real HEAD over an empty
            // commit store is a truncated checkout, not a genuinely empty repo.
            db.ensure_ingest_witnessed(&head)
                .map_err(|e| map_lib_err(&e))?;
            let sheet =
                FileFactSheet::build(&db, &repo, &opts, &target).map_err(|e| map_lib_err(&e))?;

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
            let out = match client {
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
            let serialized = serde_json::to_string(&out).map_err(internal)?;
            if memoizable {
                memo.put(&head, key, serialized.clone());
            }
            Ok(serialized)
        })
        .await
    }

    // ── change_context ────────────────────────────────────────────────────────

    #[tool(
        name = "change_context",
        annotations(read_only_hint = true, open_world_hint = false),
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
        let memo = self.memo.clone();
        let defect_calibration = self.defect_calibration.clone();
        let allow_foreign_calibration = self.allow_foreign_calibration;
        let paths = params.0.paths.clone();
        let params_json = serde_json::to_string(&params.0).map_err(internal)?;
        // Hand-rolled memo (not the generic `memoized`) so the key can carry two
        // committed-state inputs that change without moving HEAD: an in-progress
        // merge/rebase (which the briefing's leading note reflects) and a
        // regenerated calibration artifact (which its defect evidence reflects).
        self.blocking(move || {
            let repo = GixRepo::open(&repo_path).map_err(|e| map_lib_err(&e))?;
            let head = repo.head_sha().map_err(|e| map_lib_err(&e))?;
            // Both probes are cheap (a git-dir stat and a file stat) and run
            // before the lookup so a mid-process change misses the prior entry.
            let merge = repo.merge_or_rebase_in_progress();
            let cal = calibration_key_fragment(defect_calibration.as_deref());
            let key = memo_key(
                "change_context",
                &format!("{params_json}\u{1f}merge={merge}\u{1f}{cal}"),
            );
            if let Some(hit) = memo.get(&head, &key) {
                return Ok(hit);
            }
            // min_revs = 1 so any single named file resolves in its briefing,
            // matching the `change_context` lib contract.
            let opts = Options {
                repo_path: repo_path.clone(),
                min_revs: 1,
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(|e| map_lib_err(&e))?;
            // Same ingest witness as `check_gates`: a real HEAD over an empty
            // commit store is a truncated checkout, not a genuinely empty repo.
            db.ensure_ingest_witnessed(&head)
                .map_err(|e| map_lib_err(&e))?;
            // An empty or oversized path list surfaces as `InvalidOptions` (the
            // CLI's exit-2 config/param bucket), which `map_lib_err` routes to a
            // JSON-RPC `invalid_params` so the client sees it as bad input rather
            // than an internal failure — and, being an error, it is never memoized.
            let out = change_context::build_change_context(&db, &repo, &opts, &paths)
                .map_err(|e| map_lib_err(&e))?;
            memo.put(&head, key, out.clone());
            Ok(out)
        })
        .await
    }

    // ── gate_changes ──────────────────────────────────────────────────────────

    #[tool(
        name = "gate_changes",
        annotations(read_only_hint = true, open_world_hint = false),
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
        // Deliberately not memoized: this is a working-tree tool — it projects the
        // uncommitted edits in `worktree_changes()` against HEAD, so its output
        // changes with every unstaged keystroke and is never a function of the
        // committed state alone.
        self.blocking(move || {
            let opts = Options {
                repo_path: repo_path.clone(),
                defect_calibration,
                allow_foreign_calibration,
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(|e| map_lib_err(&e))?;
            let changes = repo.worktree_changes().map_err(|e| map_lib_err(&e))?;
            if changes.is_empty() {
                return Ok("PASS (no working-tree changes to gate)".to_string());
            }
            let cache_root = default_cache_root();
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
                .map_err(|e| map_lib_err(&e))?;
            // Same ingest witness as `check_gates`: a shallow checkout WITH
            // uncommitted edits reaches this point (the empty-tree early return
            // above only guards a clean shallow checkout), so a real HEAD over an
            // empty commit store must still be caught here rather than reported
            // as a confident PASS over no history.
            let head_sha = repo.head_sha().map_err(|e| map_lib_err(&e))?;
            db.ensure_ingest_witnessed(&head_sha)
                .map_err(|e| map_lib_err(&e))?;
            let report = change_set::build_change_set_report(&db, &repo, &opts, &cache_root)
                .map_err(|e| map_lib_err(&e))?;
            // Thresholds are re-evaluated on every call — a warm sidecar hit
            // returns measured data only, never a stored verdict.
            let thresholds = Thresholds::discover(&repo_path).map_err(|e| map_lib_err(&e))?;
            let violations = if thresholds.is_empty() {
                None
            } else {
                Some(evaluate_gate_thresholds(&thresholds, &report))
            };
            Ok(render_gate_changes(&report, violations.as_deref()))
        })
        .await
    }
}

/// Render the `gate_changes` text document, mirroring `codelore gate`'s text
/// forms: a verdict line (`PASS` / `FAIL — n violation(s)` / the advisory-only
/// disclosure when `violations` is `None` because no thresholds are
/// configured), the merge-in-progress note, violation rows in `codelore
/// check`'s exact form, one line per advisory finding capped at the CLI's row
/// limit with a `(+n more findings)` tail, and the per-file delta table
/// capped at the CLI's row limit with a `(+n more files)` tail.
/// Append `(+n more <noun>)` when a rendered section was truncated.
///
/// `render_gate_changes` caps three sections independently, and each cap is
/// only honest if the reader is told rows were dropped. Keeping the take and
/// the disclosure as separate hand-written statements is what let the
/// violations list ship uncapped while its two siblings were bounded, so the
/// tail is a call rather than a pattern to remember.
fn push_truncation_tail(lines: &mut Vec<String>, total: usize, cap: usize, noun: &str) {
    let hidden = total.saturating_sub(cap);
    if hidden > 0 {
        lines.push(format!("(+{hidden} more {noun})"));
    }
}

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
    let violation_rows = violations.unwrap_or_default();
    for v in violation_rows.iter().take(crate::GATE_VIOLATION_ROWS) {
        lines.push(format!(
            "  - {gate}: {path} — actual {actual} vs threshold {threshold}",
            gate = v.gate,
            path = v.path,
            actual = v.actual,
            threshold = v.threshold,
        ));
    }
    push_truncation_tail(
        &mut lines,
        violation_rows.len(),
        crate::GATE_VIOLATION_ROWS,
        "violations",
    );
    for f in report.findings.iter().take(crate::GATE_FINDINGS_ROWS) {
        lines.push(format!("[{}] {}: {}", f.kind, f.path, f.detail));
    }
    push_truncation_tail(
        &mut lines,
        report.findings.len(),
        crate::GATE_FINDINGS_ROWS,
        "findings",
    );
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
    push_truncation_tail(
        &mut lines,
        report.health.deltas.len(),
        crate::GATE_DELTA_TABLE_ROWS,
        "files",
    );
    if let Some(action) = gate_changes_action(report, violations) {
        lines.push(action);
    }
    lines.join("\n")
}

/// One next-action line for `gate_changes`, derived only from data already in
/// `report` — no new analysis. On FAIL it names the first violated gate and the
/// changed file whose projected health delta is worst (the one to fix first);
/// on a pass (or advisory-only) run that still carries findings it names the
/// first finding to review. `None` when there is nothing actionable to add, so
/// no filler line is rendered. Compact by construction — it renders inside the
/// tool's existing token budget.
fn gate_changes_action(
    report: &change_set::ChangeSetReport,
    violations: Option<&[GateViolation]>,
) -> Option<String> {
    match violations {
        Some(v) if !v.is_empty() => {
            let gate = &v[0].gate;
            // Worst projected delta among scored files (most negative).
            let worst = report
                .health
                .deltas
                .iter()
                .filter_map(|d| d.delta.map(|delta| (delta, d.path.as_str())))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            Some(match worst {
                Some((delta, path)) => format!(
                    "→ fix {path} first (health delta {delta:+.1}) — it drives the {gate} violation"
                ),
                None => format!("→ address the {gate} violation — see the rows above"),
            })
        }
        // PASS or advisory-only with advisory findings: point at the first one.
        _ => report
            .findings
            .first()
            .map(|f| format!("→ review {} ({}) before committing", f.path, f.kind)),
    }
}

/// Wire the tool router into the MCP `ServerHandler` trait and set the
/// server's `instructions` field — clients display this to the operator
/// so the "local-only / read-only / no telemetry" positioning is
/// communicated at the protocol layer, not only in `--help`.
#[tool_handler(
    instructions = "Local-only behavioral analysis of the git repository configured at startup. \
        No tool modifies tracked content; delta_health creates and removes throwaway git \
        worktrees to read two revisions, and every tool may populate the local cache. \
        No network, no account, no telemetry — beyond the optional CODELORE_LLM_* \
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
                memo: Arc::new(ResultMemo::default()),
                limit: Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS)),
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

#[cfg(test)]
mod tests {
    use super::{
        CodeHealthParams, DEFAULT_ROW_CAP, DeltaHealthParams, MAX_ROW_CAP, MEMO_CAPACITY,
        ResultMemo, map_lib_err, memo_key, resolve_row_cap, serialize_capped_rows,
        skipped_check_gates,
    };
    use crate::new_code_skip_reason;
    use codelore_lib::CodeLoreError;
    use codelore_lib::cli_api::quality_gates::Thresholds;
    use serde_json::{Value, json};

    /// Canonical key for a parameter struct the same way the tools build it:
    /// serialize the already-parsed struct, so JSON object key order never leaks
    /// into the key.
    fn key_for<T: serde::Serialize>(tool: &str, params: &T) -> String {
        memo_key(tool, &serde_json::to_string(params).unwrap())
    }

    #[test]
    fn memo_key_is_independent_of_json_field_order() {
        // Two orderings of the same single-field params (code_health keys off
        // exactly this serialization).
        let ch_ab: CodeHealthParams =
            serde_json::from_str(r#"{"path":"src/a.rs","limit":5}"#).unwrap();
        let ch_ba: CodeHealthParams =
            serde_json::from_str(r#"{"limit":5,"path":"src/a.rs"}"#).unwrap();
        assert_eq!(
            key_for("code_health", &ch_ab),
            key_for("code_health", &ch_ba)
        );

        // A multi-field struct exercises reordering across several keys.
        let delta_ordered: DeltaHealthParams =
            serde_json::from_str(r#"{"base":"x","head":"y","limit":3}"#).unwrap();
        let delta_shuffled: DeltaHealthParams =
            serde_json::from_str(r#"{"limit":3,"head":"y","base":"x"}"#).unwrap();
        assert_eq!(
            key_for("delta_health", &delta_ordered),
            key_for("delta_health", &delta_shuffled),
        );

        // Different params (or different tools) must NOT collide.
        let ch_other: CodeHealthParams =
            serde_json::from_str(r#"{"path":"src/a.rs","limit":6}"#).unwrap();
        assert_ne!(
            key_for("code_health", &ch_ab),
            key_for("code_health", &ch_other)
        );
        assert_ne!(key_for("code_health", &ch_ab), key_for("hotspots", &ch_ab));
    }

    #[test]
    fn memo_serves_hit_at_same_head_and_clears_on_head_change() {
        let memo = ResultMemo::default();
        // First access at a head is always a miss and adopts that head as scope.
        assert!(memo.get("head1", "k").is_none());
        memo.put("head1", "k".to_string(), "v1".to_string());
        assert_eq!(memo.get("head1", "k").as_deref(), Some("v1"));

        // A different head drops every entry, so the same key now misses; the
        // new head becomes the scope and a fresh value can be stored.
        assert!(memo.get("head2", "k").is_none());
        memo.put("head2", "k".to_string(), "v2".to_string());
        assert_eq!(memo.get("head2", "k").as_deref(), Some("v2"));

        // Returning to the old head cannot resurrect its cleared entry.
        assert!(memo.get("head1", "k").is_none());
    }

    #[test]
    fn memo_put_is_dropped_when_scope_advanced_during_compute() {
        // Models a HEAD move landing between one call's get (miss at h1) and its
        // put: a concurrent call advanced the scope to h2, so the late h1 result
        // must not be stored under the h2 scope.
        let memo = ResultMemo::default();
        assert!(memo.get("h1", "k").is_none()); // scope = h1
        assert!(memo.get("h2", "k").is_none()); // scope = h2 (concurrent advance)
        memo.put("h1", "k".to_string(), "stale".to_string());
        assert!(
            memo.get("h2", "k").is_none(),
            "a put for a superseded head must not land in the current scope"
        );
    }

    #[test]
    fn memo_is_bounded_and_clears_when_full() {
        let memo = ResultMemo::default();
        assert!(memo.get("h", "seed").is_none()); // adopt scope
        for i in 0..=MEMO_CAPACITY {
            memo.put("h", format!("key{i}"), "v".to_string());
        }
        // The insert that overflowed the cap cleared the map first, so the
        // earliest key is gone and only the overflowing key survives.
        assert!(
            memo.get("h", "key0").is_none(),
            "the oldest entry must be evicted once the cap is exceeded"
        );
        assert_eq!(
            memo.get("h", &format!("key{MEMO_CAPACITY}")).as_deref(),
            Some("v"),
            "the entry that triggered the clear must itself be retained"
        );
    }

    #[test]
    fn row_cap_defaults_and_clamps() {
        assert_eq!(resolve_row_cap(None), DEFAULT_ROW_CAP);
        assert_eq!(resolve_row_cap(Some(10)), 10);
        // 0 clamps up to 1 (a list tool always returns at least one row); an
        // oversized request clamps down to the hard ceiling.
        assert_eq!(resolve_row_cap(Some(0)), 1);
        assert_eq!(resolve_row_cap(Some(10_000)), MAX_ROW_CAP);
    }

    #[test]
    fn capped_rows_are_a_bare_array_when_complete() {
        let rows = vec![json!({ "path": "a" }), json!({ "path": "b" })];
        let out = serialize_capped_rows(&rows, rows.len(), "note").unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();
        let arr = parsed.as_array().expect("bare array");
        assert_eq!(
            arr.len(),
            2,
            "no summary object when nothing omitted: {out}"
        );
        assert!(
            arr.iter().all(|v| v.get("omitted").is_none()),
            "an untruncated list carries no omitted summary: {out}"
        );
    }

    #[test]
    fn capped_rows_append_omitted_summary_when_truncated() {
        // Two of five rows shown → a trailing {omitted, total, note} object.
        let shown = vec![json!({ "path": "a" }), json!({ "path": "b" })];
        let out = serialize_capped_rows(&shown, 5, "worst first").unwrap();
        let parsed: Value = serde_json::from_str(&out).unwrap();
        let arr = parsed.as_array().expect("array");
        assert_eq!(arr.len(), 3, "two rows plus one summary object: {out}");
        let summary = arr.last().unwrap();
        assert_eq!(
            summary["omitted"], 3,
            "5 total − 2 shown = 3 omitted: {out}"
        );
        assert_eq!(summary["total"], 5);
        assert_eq!(summary["note"], "worst first");
        assert!(
            summary.get("path").is_none(),
            "the summary object is distinguishable from a row (no path): {out}"
        );
    }

    #[test]
    fn skipped_gates_lists_configured_but_unevaluated_gates() {
        // A config mixing an evaluated gate with the two check-only gates:
        // the check-only ones are disclosed as skipped, and so is the
        // default-on degraded handling (check-only, active unless disabled).
        // `corpus_percentile_max` is deliberately NOT one of the check-only
        // examples here — this tool evaluates it for real (see
        // `EVALUATED_HERE`); `hotspot_anchored_max` remains check-only because
        // this tool's hotspot scan is the plain, unanchored variant.
        let thresholds = Thresholds::from_text(
            "[gates]\ncode_health_min = 50.0\nmax_findings_in_hot_files = 5\nhotspot_anchored_max = 9.0\n",
        )
        .expect("parse thresholds");
        let skipped = skipped_check_gates(&thresholds);
        assert_eq!(
            skipped.iter().map(|s| s.gate).collect::<Vec<_>>(),
            vec![
                "hotspot_anchored_max",
                "max_findings_in_hot_files",
                "fail_on_degraded"
            ],
        );
        // Every skip carries a non-empty reason, so an empty violation list is
        // distinguishable from "did not run".
        assert!(
            skipped.iter().all(|s| !s.reason.is_empty()),
            "each skipped gate must disclose a reason"
        );
    }

    #[test]
    fn skipped_gates_empty_when_all_configured_gates_are_evaluated() {
        // Degraded handling defaults to on and is check-only, so an empty
        // disclosure requires explicitly switching it off.
        let thresholds = Thresholds::from_text(
            "[gates]\ncode_health_min = 50.0\ncognitive_max = 30.0\nfail_on_degraded = false\n",
        )
        .expect("parse thresholds");
        assert!(
            skipped_check_gates(&thresholds).is_empty(),
            "gates this tool evaluates are not disclosed as skipped"
        );
    }

    #[test]
    fn skipped_gates_disclose_default_on_degraded_handling() {
        // With no explicit setting, `fail_on_degraded` is active (defaults to
        // true) and this tool never applies it — it must be disclosed so a
        // pass here is never mistaken for a full `codelore check` pass.
        let thresholds =
            Thresholds::from_text("[gates]\ncode_health_min = 50.0\n").expect("parse thresholds");
        let skipped = skipped_check_gates(&thresholds);
        assert_eq!(
            skipped.iter().map(|s| s.gate).collect::<Vec<_>>(),
            vec!["fail_on_degraded"]
        );
    }

    #[test]
    fn new_code_skip_reason_names_fetch_depth_only_when_shallow() {
        // A shallow checkout and a genuinely young repository both leave the
        // window without a baseline, but the disclosure must tell them apart:
        // only the shallow case is about the checkout (fetch-depth), and only
        // it should advise re-fetching full history.
        let shallow = new_code_skip_reason(90.0, true);
        assert!(
            shallow.contains("fetch-depth") && shallow.contains("shallow"),
            "shallow-checkout reason must name fetch-depth: {shallow}"
        );
        let young = new_code_skip_reason(90.0, false);
        assert!(
            young.contains("young repository") && !young.contains("fetch-depth"),
            "young-repository reason must not blame fetch-depth: {young}"
        );
    }

    #[test]
    fn lib_error_kind_follows_the_exit_code_bucket() {
        // exit-2 (config/param) → invalid_params (-32602); everything else →
        // internal_error (-32603). ErrorData exposes the numeric code.
        let params = map_lib_err(&CodeLoreError::InvalidOptions("bad".into()));
        assert_eq!(
            params.code.0, -32602,
            "InvalidOptions maps to invalid_params"
        );
        let internal = map_lib_err(&CodeLoreError::Analysis("boom".into()));
        assert_eq!(internal.code.0, -32603, "Analysis maps to internal_error");
    }
}
