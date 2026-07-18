//! Integration tests for `codelore mcp` — newline-delimited JSON-RPC 2.0 over stdio.
//!
//! The rmcp stdio transport uses newline-delimited JSON (one JSON object per line).
//! The test spawns the binary, exchanges the MCP initialize handshake, calls
//! `tools/list` and `tools/call`, and asserts the JSON shape.
//! The `tiny_repo` fixture is used for the initial tools/list + `repo_overview` smoke test;
//! `delivery_repo` (which has enough history for all analyses) is used for the 5 new tools.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use codelore_lib::test_support::{delivery_repo, tiny_repo};
use serde_json::{Value, json};

/// LLM environment variables the server reads at `explain_file` time. Cleared
/// on the spawned server so an ambient developer configuration can never make
/// the no-LLM `explain_file` assertions flaky.
const LLM_ENV_VARS: &[&str] = &[
    "CODELORE_LLM_PROVIDER",
    "CODELORE_LLM_BASE_URL",
    "CODELORE_LLM_API_KEY",
    "CODELORE_LLM_MODEL",
    "ANTHROPIC_API_KEY",
];

/// Serialize a JSON-RPC message as a newline-terminated line.
fn ndjson_line(msg: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(msg).unwrap();
    bytes.push(b'\n');
    bytes
}

/// Read one newline-delimited JSON message from the reader.
fn read_ndjson(reader: &mut BufReader<impl std::io::Read>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read JSON-RPC line");
    serde_json::from_str(line.trim()).expect("parse JSON-RPC line")
}

/// Spawn `codelore mcp`, complete the MCP handshake, and return (child, stdin, reader).
/// The caller owns stdin (write requests) and the reader (read responses).
fn spawn_mcp(
    repo_path: &str,
) -> (
    std::process::Child,
    std::process::ChildStdin,
    BufReader<std::process::ChildStdout>,
) {
    spawn_mcp_with_args(repo_path, &[])
}

/// Like [`spawn_mcp`], but with extra CLI args appended after `--repo`
/// (e.g. `--defect-calibration <path> --allow-foreign-calibration`).
fn spawn_mcp_with_args(
    repo_path: &str,
    extra_args: &[&str],
) -> (
    std::process::Child,
    std::process::ChildStdin,
    BufReader<std::process::ChildStdout>,
) {
    let bin = assert_cmd::cargo::cargo_bin("codelore");
    let mut builder = Command::new(&bin);
    builder
        .args(["mcp", "--repo", repo_path])
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for var in LLM_ENV_VARS {
        builder.env_remove(var);
    }
    let mut child = builder.spawn().expect("spawn codelore mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    // initialize handshake
    let init_req = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0.0.1" }
        }
    });
    stdin.write_all(&ndjson_line(&init_req)).unwrap();
    stdin.flush().unwrap();
    let init_resp = read_ndjson(&mut reader);
    // The initialize response must carry the server's positioning statement
    // (local-only, read-only, no network/account/telemetry) so MCP clients
    // can display it.
    let instructions = init_resp["result"]["instructions"]
        .as_str()
        .expect("initialize response carries an instructions string");
    assert!(
        instructions.contains("No network"),
        "instructions must state the local-only positioning, got: {instructions}"
    );

    // initialized notification
    let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    stdin.write_all(&ndjson_line(&notif)).unwrap();
    stdin.flush().unwrap();

    (child, stdin, reader)
}

/// Call a single tool and return the response Value.
fn call_tool(
    stdin: &mut std::process::ChildStdin,
    reader: &mut BufReader<std::process::ChildStdout>,
    id: u64,
    name: &str,
    arguments: &Value,
) -> Value {
    let req = json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    });
    stdin.write_all(&ndjson_line(&req)).unwrap();
    stdin.flush().unwrap();
    read_ndjson(reader)
}

/// Assert a tool response is not an error and return the first content text as a parsed JSON Value.
fn assert_tool_ok(resp: &Value, tool_name: &str) -> Value {
    assert_eq!(resp["jsonrpc"], "2.0", "{tool_name}: bad jsonrpc field");
    assert!(
        !resp["result"]["isError"].as_bool().unwrap_or(false),
        "{tool_name} returned MCP error: {resp}"
    );
    let content = resp["result"]["content"].as_array().expect("content array");
    assert!(!content.is_empty(), "{tool_name}: content array is empty");
    let text = content[0]["text"].as_str().expect("text field");
    serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("{tool_name}: content text is not valid JSON ({e}): {text}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn mcp_tools_list_and_repo_overview() {
    let repo = tiny_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    // tools/list
    let list_req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    stdin.write_all(&ndjson_line(&list_req)).unwrap();
    stdin.flush().unwrap();
    let list_resp = read_ndjson(&mut reader);
    assert_eq!(list_resp["id"], 1);
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");

    // Exact count — catches both missing tools and accidental extras.
    assert_eq!(
        tools.len(),
        9,
        "expected exactly 9 tools, got {}: {:?}",
        tools.len(),
        tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect::<Vec<_>>()
    );

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in &[
        "repo_overview",
        "hotspots",
        "code_health",
        "delta_health",
        "refactoring_targets",
        "function_xray",
        "check_gates",
        "finding_hotspot_overlap",
        "explain_file",
    ] {
        assert!(
            tool_names.contains(expected),
            "{expected} missing from tools/list: {tool_names:?}"
        );
    }

    // Every tool must carry an inputSchema object (MCP spec requirement).
    for tool in tools {
        assert!(
            tool["inputSchema"].is_object(),
            "tool {:?} missing inputSchema: {tool}",
            tool["name"]
        );
    }

    // tools/call repo_overview — now returns {summary: [...], options: {...}}
    let resp = call_tool(&mut stdin, &mut reader, 2, "repo_overview", &json!({}));
    let parsed = assert_tool_ok(&resp, "repo_overview");
    assert!(
        parsed["summary"].is_array(),
        "expected `summary` array in repo_overview response: {parsed}"
    );
    assert!(
        parsed["options"].is_object(),
        "expected `options` object in repo_overview response: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_code_health_returns_scored_rows() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({}));
    let parsed = assert_tool_ok(&resp, "code_health");

    let rows = parsed.as_array().expect("code_health: expected JSON array");
    // delivery_repo has Rust source — at least one file should have complexity data.
    assert!(
        !rows.is_empty(),
        "code_health returned no rows for delivery_repo"
    );
    // Each row must have a `band` and a numeric `score`.
    let first = &rows[0];
    assert!(
        first["band"].is_string(),
        "row missing `band` field: {first}"
    );
    assert!(
        first["score"].is_number(),
        "row missing numeric `score` field: {first}"
    );
    // The embedded world corpus covers Rust, so `delivery_repo`'s Rust rows
    // carry `corpus_percentile` — this exercises the serde propagation of the
    // corpus lens through MCP. At least one row must be populated, and every
    // present value must be a well-formed float in `0..=1` (with `beyond_corpus`
    // a bool when present). A row whose files fall outside the corpus stays
    // absent (serde skip_serializing_if = Option::is_none) — also valid.
    let mut any_corpus = false;
    for row in rows {
        if let Some(cp) = row.get("corpus_percentile") {
            let p = cp
                .as_f64()
                .unwrap_or_else(|| panic!("corpus_percentile must be a number: {row}"));
            assert!(
                (0.0..=1.0).contains(&p),
                "corpus_percentile must be in 0..=1: {row}"
            );
            any_corpus = true;
            if let Some(beyond) = row.get("beyond_corpus") {
                assert!(
                    beyond.is_boolean(),
                    "beyond_corpus must be a bool when present: {row}"
                );
            }
        } else {
            // Absent percentile → beyond_corpus must also be absent (falsy-skip).
            assert!(
                row.get("beyond_corpus").is_none(),
                "beyond_corpus must be absent when corpus_percentile is: {row}"
            );
        }
    }
    assert!(
        any_corpus,
        "the embedded world corpus covers Rust, so at least one delivery_repo row \
         must carry corpus_percentile"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_refactoring_targets_returns_array() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "refactoring_targets",
        &json!({ "limit": 5 }),
    );
    let parsed = assert_tool_ok(&resp, "refactoring_targets");
    assert!(
        parsed.is_array(),
        "refactoring_targets: expected JSON array, got: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_function_xray_returns_rows_for_valid_path() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    // delivery_repo seeds src/core.rs with a `core()` function.
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "function_xray",
        &json!({ "path": "src/core.rs" }),
    );
    let parsed = assert_tool_ok(&resp, "function_xray");
    // Returns an array (possibly empty if tree-sitter doesn't parse the minimal fixture).
    assert!(
        parsed.is_array(),
        "function_xray: expected JSON array, got: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_check_gates_returns_verdict() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    // Write a thresholds file that is guaranteed to pass on delivery_repo.
    let thresholds_path = repo.dir.path().join(".codelore-thresholds.toml");
    std::fs::write(&thresholds_path, "[gates]\ncode_health_min = 0.0\n").unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "check_gates", &json!({}));
    let parsed = assert_tool_ok(&resp, "check_gates");

    assert!(
        parsed["verdict"].is_string(),
        "check_gates: expected `verdict` string: {parsed}"
    );
    assert!(
        parsed["violation_count"].is_number(),
        "check_gates: expected `violation_count` number: {parsed}"
    );
    assert!(
        parsed["violations"].is_array(),
        "check_gates: expected `violations` array: {parsed}"
    );
    // A threshold of 0.0 means any score passes; expect no violations.
    assert_eq!(
        parsed["verdict"], "pass",
        "check_gates: expected pass verdict with permissive threshold, got: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_delta_health_rejects_bad_rev() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "delta_health",
        &json!({ "base": "nonexistent-branch-xyz", "head": "HEAD" }),
    );
    // An invalid rev must return either a JSON-RPC error or isError=true at the tool level.
    // In practice, resolve_rev returns ErrorData::invalid_params which rmcp surfaces as a
    // JSON-RPC -32602 error rather than a tool-call result.
    assert_eq!(resp["jsonrpc"], "2.0");
    let is_rpc_error = resp["error"].is_object();
    let is_tool_error = resp["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        is_rpc_error || is_tool_error,
        "delta_health with bad rev should return an error, got: {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_delta_health_returns_section_for_valid_revs() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    // Use HEAD~1..HEAD as base..head — delivery_repo has multiple commits.
    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "delta_health",
        &json!({ "base": "HEAD~1", "head": "HEAD" }),
    );
    let parsed = assert_tool_ok(&resp, "delta_health");

    // DeltaHealthSection fields: verdict, ratio (nullable), counts, functions.
    assert!(
        parsed["verdict"].is_string(),
        "delta_health: expected `verdict` string: {parsed}"
    );
    assert!(
        parsed["counts"].is_object(),
        "delta_health: expected `counts` object: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_finding_hotspot_overlap_returns_note_when_sidecar_absent() {
    // On a fresh tiny_repo the sidecar is never created, so the tool must
    // return the structured note JSON (not a tool error).
    let repo = tiny_repo::build();
    let repo_path_buf = repo.dir.path().to_path_buf();
    let repo_path = repo_path_buf.to_str().unwrap();

    // Compute the sidecar path the MCP tool would use — same derivation as
    // open_existing in mcp.rs (default_cache_root + repo_cache_dir).
    let cache_root = codelore_lib::cli_api::cache::default_cache_root();
    let sidecar_path = codelore_lib::cli_api::cache::repo_cache_dir(&cache_root, &repo_path_buf)
        .join("external-findings.duckdb-ext");

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "finding_hotspot_overlap",
        &json!({}),
    );
    let parsed = assert_tool_ok(&resp, "finding_hotspot_overlap");

    // The sidecar is absent → structured note, not an error result.
    assert!(
        parsed["findings"].is_array(),
        "finding_hotspot_overlap: expected `findings` array in note response: {parsed}"
    );
    assert_eq!(
        parsed["findings"].as_array().unwrap().len(),
        0,
        "finding_hotspot_overlap: note response `findings` must be empty: {parsed}"
    );
    assert!(
        parsed["note"].is_string(),
        "finding_hotspot_overlap: expected `note` string in response: {parsed}"
    );
    assert!(
        parsed["note"].as_str().unwrap().contains("ingest-sarif"),
        "finding_hotspot_overlap: note must mention ingest-sarif: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();

    // The MCP read path must never create the sidecar as a side-effect.
    assert!(
        !sidecar_path.exists(),
        "MCP read must not create the sidecar: {}",
        sidecar_path.display()
    );
}

#[test]
fn mcp_explain_file_returns_fact_sheet_and_narrative_error_without_llm() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    // Derive a target from the `code_health` tool so `explain_file` is guaranteed
    // a file with a code-health row (its mandatory section). A path present at
    // the default `min_revs` is a superset of what `explain_file` sees at
    // `min_revs = 1`.
    let ch = call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({}));
    let ch_rows = assert_tool_ok(&ch, "code_health");
    let target = ch_rows
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["path"].as_str())
        .expect("code_health yields at least one file for delivery_repo")
        .to_string();

    let resp = call_tool(
        &mut stdin,
        &mut reader,
        2,
        "explain_file",
        &json!({ "path": target }),
    );
    let parsed = assert_tool_ok(&resp, "explain_file");

    // The fact sheet is always present, and it is the structured sections array.
    let sections = parsed["fact_sheet"]
        .as_array()
        .expect("explain_file always returns a fact_sheet array");
    assert!(
        !sections.is_empty(),
        "the fact sheet carries at least the mandatory code-health section: {parsed}"
    );
    assert!(
        sections
            .iter()
            .any(|s| s["section"] == "code-health" && s["facts"].is_object()),
        "the fact sheet includes a structured code-health section: {parsed}"
    );

    // The server env carries no LLM configuration (spawn_mcp clears it), so the
    // narrative degrades to a narrative_error and no narrative is produced.
    assert!(
        parsed["narrative_error"].is_string(),
        "without an LLM configured, explain_file sets narrative_error: {parsed}"
    );
    assert!(
        parsed.get("narrative").is_none(),
        "no narrative may be present when the LLM is unavailable: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// A syntactically valid defect-calibration artifact with a deliberately
/// foreign `repo_identity` — proves `--allow-foreign-calibration` is what lets
/// the server start with it, not merely that the flag parses. `weights` are
/// the built-in smell defaults in canonical order, matching what
/// `active_weights` (consulted by the dossier's code-health section)
/// requires of a well-formed artifact.
fn write_foreign_defect_artifact(dir: &std::path::Path) -> std::path::PathBuf {
    use codelore_lib::defect_calibration::{
        DEFECT_FORMAT_VERSION, DefectArtifact, MiningStats, OracleConfig, TuningDecision,
        ValidationMetrics, save, validate::default_weights,
    };
    let artifact = DefectArtifact {
        format_version: DEFECT_FORMAT_VERSION,
        repo_identity: "0".repeat(64),
        head_at_mining: "0".repeat(40),
        vintage: "defects-2026-07-17".to_string(),
        generated_at: "2026-07-17T00:00:00Z".to_string(),
        oracle: OracleConfig::default(),
        mining: MiningStats::default(),
        validation: ValidationMetrics {
            band_table: vec![("red".to_string(), 5, 1.0)],
            auc_default: None,
            precision_at_10: None,
            precision_at_red: None,
            implicated_files: 3,
            linked_defects: 5,
            sample_dates: vec!["2026-01-01".to_string()],
            excluded_no_data: 0,
        },
        weights: default_weights(),
        tuning: TuningDecision::DefaultsKept {
            reason: "insufficient evidence for weight tuning".to_string(),
            auc_validation_default: None,
            auc_validation_tuned: None,
        },
    };
    let path = dir.join("defects.calib.json");
    save(&artifact, &path).expect("save artifact");
    path
}

#[test]
fn mcp_explain_file_defect_calibration_adds_defect_evidence_section() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let artifact_path = write_foreign_defect_artifact(artifact_dir.path());

    let (mut child, mut stdin, mut reader) = spawn_mcp_with_args(
        repo_path,
        &[
            "--defect-calibration",
            artifact_path.to_str().unwrap(),
            "--allow-foreign-calibration",
        ],
    );

    let ch = call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({}));
    let ch_rows = assert_tool_ok(&ch, "code_health");
    let target = ch_rows
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["path"].as_str())
        .expect("code_health yields at least one file for delivery_repo")
        .to_string();

    let resp = call_tool(
        &mut stdin,
        &mut reader,
        2,
        "explain_file",
        &json!({ "path": target }),
    );
    let parsed = assert_tool_ok(&resp, "explain_file");

    let sections = parsed["fact_sheet"]
        .as_array()
        .expect("explain_file always returns a fact_sheet array");
    assert!(
        sections.iter().any(|s| s["section"] == "defect-evidence"),
        "the fact sheet must carry a defect-evidence section when the server \
         was started with --defect-calibration: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// MCP process must exit immediately with code 4 when started with a
/// foreign defect-calibration artifact and no `--allow-foreign-calibration`
/// override. The foreign artifact is rejected before the tokio runtime starts,
/// so the process never reads stdin (attempting an MCP handshake would hang).
/// This test captures stderr to verify the error message names both the
/// identity mismatch and the override flag.
#[test]
fn mcp_refuses_to_start_on_foreign_artifact_without_override() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let artifact_path = write_foreign_defect_artifact(artifact_dir.path());

    // Build the Command directly, mirroring spawn_mcp_with_args's construction
    // but with stderr(Stdio::piped()) to capture the error. We do NOT use
    // spawn_mcp_with_args because its read_ndjson handshake would panic on a
    // child that exits before writing the initialize response.
    let bin = assert_cmd::cargo::cargo_bin("codelore");
    let mut builder = Command::new(&bin);
    builder
        .args([
            "mcp",
            "--repo",
            repo_path,
            "--defect-calibration",
            artifact_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in LLM_ENV_VARS {
        builder.env_remove(var);
    }

    let output = builder.output().expect("spawn codelore mcp");

    // The foreign artifact without --allow-foreign-calibration must cause
    // the process to exit with code 4.
    assert_eq!(
        output.status.code(),
        Some(4),
        "expected exit code 4 for foreign artifact without override, got: {:?}",
        output.status.code()
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mined from a different repository"),
        "stderr must mention 'mined from a different repository', got: {stderr}"
    );
    assert!(
        stderr.contains("--allow-foreign-calibration"),
        "stderr must mention '--allow-foreign-calibration', got: {stderr}"
    );
}
