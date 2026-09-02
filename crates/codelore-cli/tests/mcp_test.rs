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
    // Pin the NEGOTIATED revision, not the one we requested. Asserting only on
    // the request proves nothing about the server: an rmcp bump that shifted
    // the supported revision, or a downgrade during negotiation, would leave
    // every test here passing while clients pinned to the old revision broke.
    assert_eq!(
        init_resp["result"]["protocolVersion"].as_str(),
        Some("2024-11-05"),
        "negotiated MCP protocol revision changed; update every client \
         expectation deliberately rather than adopting the new value here"
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

/// Assert a tool response is not an error and return the first content text
/// verbatim. Unlike [`assert_tool_ok`], this does not parse the text as JSON —
/// `change_context` returns a plain-text briefing, not a JSON document.
fn assert_tool_ok_text(resp: &Value, tool_name: &str) -> String {
    assert_eq!(resp["jsonrpc"], "2.0", "{tool_name}: bad jsonrpc field");
    assert!(
        !resp["result"]["isError"].as_bool().unwrap_or(false),
        "{tool_name} returned MCP error: {resp}"
    );
    let content = resp["result"]["content"].as_array().expect("content array");
    assert!(!content.is_empty(), "{tool_name}: content array is empty");
    content[0]["text"].as_str().expect("text field").to_string()
}

/// Assert a response is a JSON-RPC error carrying `expected_code`. `-32602` is
/// `invalid_params` (a caller-input problem); `-32603` is `internal_error`.
fn assert_rpc_error_code(resp: &Value, expected_code: i64, ctx: &str) {
    let code = resp["error"]["code"].as_i64().unwrap_or_else(|| {
        panic!("{ctx}: expected a JSON-RPC error object with a numeric code, got: {resp}")
    });
    assert_eq!(
        code, expected_code,
        "{ctx}: wrong JSON-RPC error code: {resp}"
    );
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
        11,
        "expected exactly 11 tools, got {}: {:?}",
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
        "change_context",
        "gate_changes",
    ] {
        assert!(
            tool_names.contains(expected),
            "{expected} missing from tools/list: {tool_names:?}"
        );
    }

    // Every tool must carry an inputSchema object (MCP spec requirement) and
    // the annotations a client reasons about before calling it. A tool added
    // without annotations reads to an agent as "may modify anything, may reach
    // the network", so the hints are part of the contract, not decoration.
    for tool in tools {
        assert!(
            tool["inputSchema"].is_object(),
            "tool {:?} missing inputSchema: {tool}",
            tool["name"]
        );
        let ann = &tool["annotations"];
        assert!(
            ann["readOnlyHint"].is_boolean(),
            "tool {:?} missing annotations.readOnlyHint: {tool}",
            tool["name"]
        );
        assert!(
            ann["openWorldHint"].is_boolean(),
            "tool {:?} missing annotations.openWorldHint: {tool}",
            tool["name"]
        );

        // A `limit` description must state the cap the handler actually
        // applies. `refactoring_targets` advertised "(default: all)" while its
        // handler capped at 50 — an agent reading the schema had no reason to
        // pass `limit`, believed it received the whole population, and silently
        // reasoned over the top 50. This is asserted against the schema the
        // agent receives rather than the source text, because that string is
        // the contract; it recurred across three audit cycles as a doc fix
        // because nothing checked it.
        if let Some(desc) = tool["inputSchema"]["properties"]["limit"]["description"].as_str() {
            assert!(
                !desc.contains("default: all"),
                "tool {:?} advertises an uncapped `limit` default, but every \
                 capped handler resolves an absent limit to 50: {desc:?}",
                tool["name"]
            );
            assert!(
                desc.contains("50"),
                "tool {:?} `limit` description must name the default the handler \
                 applies (50) so an agent knows the list is capped: {desc:?}",
                tool["name"]
            );
        }
    }

    // The two tools whose hints are not the read-only/closed-world default.
    // delta_health checks revisions out into throwaway git worktrees;
    // explain_file calls the configured CODELORE_LLM_* endpoint when present.
    let hint = |name: &str, field: &str| -> Option<bool> {
        tools.iter().find(|t| t["name"] == name)?["annotations"][field].as_bool()
    };
    assert_eq!(
        hint("delta_health", "readOnlyHint"),
        Some(false),
        "delta_health writes git worktrees and must not claim to be read-only"
    );
    assert_eq!(
        hint("explain_file", "openWorldHint"),
        Some(true),
        "explain_file can call an external LLM endpoint and must declare an open world"
    );

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

/// `tools/call hotspots` with a `limit` caps the ranking at `limit` rows and
/// discloses the truncation, matching every sibling list tool: the array
/// carries the capped rows followed by a `{omitted, total, note}` summary.
///
/// The disclosure is the point. `hotspots` used to truncate into a bare array,
/// and the file's convention is that a bare array means the list is complete —
/// so an agent read a cut-off ranking as the whole population and could
/// conclude a file was not a hotspot when it simply fell past the cap.
#[test]
fn mcp_hotspots_returns_capped_rows() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "hotspots",
        &json!({ "limit": 1 }),
    );
    let parsed = assert_tool_ok(&resp, "hotspots");

    let entries = parsed.as_array().expect("hotspots: expected JSON array");
    assert!(
        !entries.is_empty(),
        "hotspots returned no rows for delivery_repo"
    );
    // The trailing summary object, when present, is the disclosure — not a row.
    let summary = entries.last().filter(|v| v.get("omitted").is_some());
    let rows: Vec<_> = entries[..entries.len() - usize::from(summary.is_some())].to_vec();
    assert!(
        rows.len() <= 1,
        "limit=1 must cap hotspots rows at one, got {}: {parsed}",
        rows.len()
    );
    if let Some(s) = summary {
        let omitted = s["omitted"].as_u64().expect("omitted must be a number");
        let total = s["total"].as_u64().expect("total must be a number");
        assert!(omitted > 0, "a summary object implies rows were cut: {s}");
        // Compare in u64 — the JSON values arrive as u64 and widening the
        // row count is lossless, whereas narrowing them would not be.
        assert_eq!(
            total,
            rows.len() as u64 + omitted,
            "total must equal shown + omitted: {s}"
        );
    }
    let first = &rows[0];
    assert!(
        first["path"].is_string(),
        "row missing `path` field: {first}"
    );
    assert!(
        first["revisions"].is_number(),
        "row missing numeric `revisions` field: {first}"
    );
    assert!(
        first["hotspot_score"].is_number(),
        "row missing numeric `hotspot_score` field: {first}"
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
    // Only an evaluated gate is explicitly configured — but `fail_on_degraded`
    // defaults to true and is check-only, so it is always disclosed as skipped
    // unless the config switches it off. Nothing else may appear.
    let skipped: Vec<&str> = parsed["skipped_gates"]
        .as_array()
        .expect("check_gates: skipped_gates array")
        .iter()
        .filter_map(|v| v["gate"].as_str())
        .collect();
    assert_eq!(
        skipped,
        vec!["fail_on_degraded"],
        "check_gates: only the default-on degraded semantics may be skipped when \
         every explicitly set gate is evaluated: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_check_gates_discloses_skipped_gates() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    // An evaluated gate (so the verdict is a real pass) plus a check-only gate
    // this tool does not evaluate on the committed-tree read path. Degraded
    // semantics are explicitly switched off to prove the disclosure honors it.
    std::fs::write(
        repo.dir.path().join(".codelore-thresholds.toml"),
        "[gates]\ncode_health_min = 0.0\nmax_findings_in_hot_files = 100\nhotspot_anchored_max = 9.0\nfail_on_degraded = false\n",
    )
    .unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "check_gates", &json!({}));
    let parsed = assert_tool_ok(&resp, "check_gates");

    let skipped: Vec<&str> = parsed["skipped_gates"]
        .as_array()
        .expect("check_gates: skipped_gates array")
        .iter()
        .filter_map(|v| v["gate"].as_str())
        .collect();
    assert!(
        skipped.contains(&"max_findings_in_hot_files"),
        "a configured check-only gate must be disclosed under skipped_gates: {parsed}"
    );
    assert!(
        skipped.contains(&"hotspot_anchored_max"),
        "the corpus-dependent anchored gate is check-only here and must be disclosed: {parsed}"
    );
    assert!(
        !skipped.contains(&"fail_on_degraded"),
        "explicitly disabled degraded semantics must not be disclosed as skipped: {parsed}"
    );
    // Every disclosed skip carries a non-empty reason string, so a caller can
    // tell an empty `violations` list ("all gates passed") from "did not run".
    assert!(
        parsed["skipped_gates"]
            .as_array()
            .expect("skipped_gates array")
            .iter()
            .all(|v| v["reason"].as_str().is_some_and(|r| !r.is_empty())),
        "each skipped gate must carry a reason: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// Capping the `violations` array must not move the number the agent reports.
///
/// `violation_count` is measured before truncation, so a capped response still
/// says how many gates failed — an agent that reads only the count reaches the
/// same conclusion at any `limit`. Without that, lowering `limit` would look
/// like fixing violations.
#[test]
fn mcp_check_gates_caps_violations_without_changing_the_count() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    // Two gates that both fail on this fixture's single scored file, so the
    // response carries more than one violation — `delivery_repo` has one
    // analyzable source file, so a single per-file gate cannot exercise a cap.
    std::fs::write(
        repo.dir.path().join(".codelore-thresholds.toml"),
        "[gates]\ncode_health_min = 100.0\ncorpus_percentile_max = 0.0\n",
    )
    .unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    let full = assert_tool_ok(
        &call_tool(&mut stdin, &mut reader, 1, "check_gates", &json!({})),
        "check_gates",
    );
    let full_count = full["violation_count"].as_u64().expect("violation_count");
    let full_rows = full["violations"].as_array().expect("violations").len();
    assert!(
        full_count >= 2,
        "fixture must produce at least two violations or this test cannot \
         exercise the cap, got {full_count}: {full}"
    );

    let capped = assert_tool_ok(
        &call_tool(
            &mut stdin,
            &mut reader,
            2,
            "check_gates",
            &json!({ "limit": 1 }),
        ),
        "check_gates",
    );
    let capped_rows = capped["violations"].as_array().expect("violations").len();

    assert_eq!(
        capped_rows, 1,
        "limit=1 must return exactly one violation row, got {capped_rows}: {capped}"
    );
    assert!(
        full_rows > capped_rows,
        "the uncapped call must return more rows than limit=1, else the cap is \
         untested: {full_rows} vs {capped_rows}"
    );
    assert_eq!(
        capped["violation_count"].as_u64(),
        Some(full_count),
        "violation_count must be the true total at any limit: {capped}"
    );
    assert_eq!(
        capped["verdict"], full["verdict"],
        "the verdict must not depend on how many rows were returned: {capped}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// `corpus_percentile_max` must be evaluated for real, not reported as
/// structurally skipped: `check_gates` already computes `code_health` (for
/// `code_health_min`), and the corpus lens fills `corpus_percentile` on those
/// rows whenever a calibration artifact is active — the embedded world
/// artifact by default, with no `--calibration` flag needed. A very low
/// ceiling must therefore fail with real violations, and the gate must never
/// appear in `skipped_gates` for this repo.
#[test]
fn mcp_check_gates_evaluates_corpus_percentile_max_for_real() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    std::fs::write(
        repo.dir.path().join(".codelore-thresholds.toml"),
        "[gates]\ncorpus_percentile_max = 0.0\n",
    )
    .unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "check_gates", &json!({}));
    let parsed = assert_tool_ok(&resp, "check_gates");

    assert_eq!(
        parsed["verdict"], "fail",
        "a 0.0 ceiling must fail against delivery_repo's real corpus percentiles: {parsed}"
    );
    let violations = parsed["violations"]
        .as_array()
        .expect("check_gates: violations array");
    assert!(
        violations
            .iter()
            .any(|v| v["gate"] == "corpus_percentile_max"),
        "corpus_percentile_max must produce real violations, not a skip: {parsed}"
    );
    let skipped: Vec<&str> = parsed["skipped_gates"]
        .as_array()
        .expect("check_gates: skipped_gates array")
        .iter()
        .filter_map(|v| v["gate"].as_str())
        .collect();
    assert!(
        !skipped.contains(&"corpus_percentile_max"),
        "corpus_percentile_max must not be disclosed as skipped when it was actually evaluated: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_function_xray_errors_on_unknown_path() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "function_xray",
        &json!({ "path": "src/does_not_exist.rs" }),
    );
    assert_eq!(resp["jsonrpc"], "2.0");
    // A typo path is caller input, not a file "with no functions": invalid_params
    // (-32602), and the message names the offending path.
    assert_rpc_error_code(&resp, -32602, "function_xray unknown path");
    assert!(
        resp.to_string().contains("src/does_not_exist.rs"),
        "the error must name the unknown path: {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_code_health_errors_on_unknown_path() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "code_health",
        &json!({ "path": "src/does_not_exist.rs" }),
    );
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_rpc_error_code(&resp, -32602, "code_health unknown path");
    assert!(
        resp.to_string().contains("src/does_not_exist.rs"),
        "the error must name the unknown path: {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_explain_file_errors_on_unknown_path() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "explain_file",
        &json!({ "path": "src/does_not_exist.rs" }),
    );
    assert_eq!(resp["jsonrpc"], "2.0");
    // A typo path is caller input, not an empty dossier: invalid_params (-32602),
    // so an agent re-reads params rather than retrying a transient -32603. The
    // message names the offending path.
    assert_rpc_error_code(&resp, -32602, "explain_file unknown path");
    assert!(
        resp.to_string().contains("src/does_not_exist.rs"),
        "the error must name the unknown path: {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_code_health_limit_is_honored() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    // delivery_repo has a single file above the default revision floor, so the
    // fixture cannot exceed the cap; assert the cap is honored and the bare-
    // array shape is preserved when nothing is omitted. The omitted-summary
    // contract on a >cap population is unit-tested in src/mcp.rs, which can
    // construct the population deterministically.
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "code_health",
        &json!({ "limit": 1 }),
    );
    let parsed = assert_tool_ok(&resp, "code_health");
    let arr = parsed
        .as_array()
        .expect("code_health returns an array when listing");
    let real_rows = arr.iter().filter(|v| v.get("path").is_some()).count();
    assert!(
        real_rows <= 1,
        "limit=1 must cap real rows at one: {parsed}"
    );
    assert!(
        arr.iter().all(|v| v.get("omitted").is_none()),
        "an untruncated list carries no omitted summary object: {parsed}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_delta_health_description_discloses_diff_subset() {
    let repo = tiny_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let list_req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    stdin.write_all(&ndjson_line(&list_req)).unwrap();
    stdin.flush().unwrap();
    let list_resp = read_ndjson(&mut reader);
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    let delta = tools
        .iter()
        .find(|t| t["name"] == "delta_health")
        .expect("delta_health tool present");
    let desc = delta["description"].as_str().unwrap_or_default();
    assert!(
        desc.contains("codelore diff"),
        "delta_health must disclose it is a subset of `codelore diff`: {desc}"
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
    // A bad rev is caller input, so it must surface as invalid_params (-32602),
    // not internal_error (-32603).
    assert_rpc_error_code(&resp, -32602, "delta_health bad rev");

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

/// Deleting a file must register as a removal.
///
/// The touched-file set was built from head-side rows alone, so a file with
/// no rows at head never entered it and its base rows were filtered out
/// before the comparison ran — deleting the worst file in a repository, the
/// single most decisive health improvement available, scored as no change.
/// The same omission made a rename read as pure addition, because the old
/// path's functions vanished with it.
#[test]
fn mcp_delta_health_sees_a_whole_file_deletion() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(p)
            .args(args)
            .status()
            .expect("spawn git")
            .success();
        assert!(ok, "git {args:?} failed");
    };
    git(&["init", "-b", "main", "--quiet"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "T"]);

    // A function big and branchy enough to classify, so its removal carries
    // weight rather than reading as a neutral no-op.
    let mut doomed = String::new();
    for i in 0..40 {
        use std::fmt::Write as _;
        let _ = writeln!(doomed, "    if x == {i} {{ return {i}; }}");
    }
    std::fs::write(
        p.join("doomed.rs"),
        format!(
            "pub fn doomed(x: i32) -> i32 {{
{doomed}    0
}}
"
        ),
    )
    .expect("write");
    std::fs::write(
        p.join("keep.rs"),
        "pub fn keep() -> i32 { 1 }
",
    )
    .expect("write");
    git(&["add", "."]);
    git(&["commit", "-m", "base: two files", "--quiet"]);

    std::fs::remove_file(p.join("doomed.rs")).expect("remove");
    git(&["add", "-A"]);
    git(&["commit", "-m", "head: delete the complex file", "--quiet"]);

    let (mut child, mut stdin, mut reader) = spawn_mcp(p.to_str().unwrap());
    let resp = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "delta_health",
        &json!({ "base": "HEAD~1", "head": "HEAD" }),
    );
    let parsed = assert_tool_ok(&resp, "delta_health");

    let removed = parsed["counts"]["removed"].as_u64().unwrap_or(0);
    assert!(
        removed >= 1,
        "deleting a file must register as a removal, got counts {}: {parsed}",
        parsed["counts"]
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
    use codelore_lib::defect_calibration::validate::default_weights;
    write_foreign_defect_artifact_with(dir, default_weights())
}

/// A foreign artifact whose weights are a deliberate permutation of the
/// defaults — the value column reversed across the canonical smell order,
/// sum preserved — so any firing biomarker yields observably different
/// structural risk than the built-in weights do.
fn write_skewed_foreign_defect_artifact(dir: &std::path::Path) -> std::path::PathBuf {
    use codelore_lib::defect_calibration::validate::default_weights;
    let mut weights = default_weights();
    let mut values: Vec<f64> = weights.iter().map(|(_, value)| *value).collect();
    values.reverse();
    for (slot, value) in weights.iter_mut().zip(values) {
        slot.1 = value;
    }
    write_foreign_defect_artifact_with(dir, weights)
}

fn write_foreign_defect_artifact_with(
    dir: &std::path::Path,
    weights: Vec<(String, f64)>,
) -> std::path::PathBuf {
    use codelore_lib::defect_calibration::{
        DEFECT_FORMAT_VERSION, DefectArtifact, MiningStats, OracleConfig, TuningDecision,
        ValidationMetrics, save,
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
        weights,
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

/// Collect `path -> field` from a tool's row array.
fn scores_by_path(rows: &Value, field: &str) -> std::collections::BTreeMap<String, f64> {
    rows.as_array()
        .expect("tool returns a row array")
        .iter()
        .filter_map(|row| Some((row["path"].as_str()?.to_string(), row[field].as_f64()?)))
        .collect()
}

/// The server-level `--defect-calibration` must reach every tool whose result
/// embeds code-health scores, not just `check_gates`/`explain_file`. Two
/// servers over the same worsened fixture — one default, one with a
/// weight-permuted artifact — must agree on the file set but disagree on at
/// least one structural risk. Identical outputs would mean the artifact
/// silently never reached the analysis (the exact regression this pins).
#[test]
fn mcp_code_health_and_refactoring_targets_follow_the_server_defect_calibration() {
    let repo = delivery_repo::build();
    let root = repo.dir.path();
    let repo_path = root.to_str().unwrap();
    // Biomarkers are what the weights weigh: commit a fired biomarker (the
    // monster) plus the cohort the per-language rank needs, or both servers
    // would agree trivially at structural_risk = 0.
    worsen_file(root, "src/core.rs");
    commit_cohort_fillers(root, 10);

    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let artifact_path = write_skewed_foreign_defect_artifact(artifact_dir.path());

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let base_ch = assert_tool_ok(
        &call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({})),
        "code_health",
    );
    let base_rt = assert_tool_ok(
        &call_tool(
            &mut stdin,
            &mut reader,
            2,
            "refactoring_targets",
            &json!({}),
        ),
        "refactoring_targets",
    );
    drop(stdin);
    let _ = child.wait();

    let (mut child, mut stdin, mut reader) = spawn_mcp_with_args(
        repo_path,
        &[
            "--defect-calibration",
            artifact_path.to_str().unwrap(),
            "--allow-foreign-calibration",
        ],
    );
    let cal_ch = assert_tool_ok(
        &call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({})),
        "code_health",
    );
    let cal_rt = assert_tool_ok(
        &call_tool(
            &mut stdin,
            &mut reader,
            2,
            "refactoring_targets",
            &json!({}),
        ),
        "refactoring_targets",
    );
    drop(stdin);
    let _ = child.wait();

    for (tool, base_rows, cal_rows) in [
        ("code_health", &base_ch, &cal_ch),
        ("refactoring_targets", &base_rt, &cal_rt),
    ] {
        let base = scores_by_path(base_rows, "structural_risk");
        let calibrated = scores_by_path(cal_rows, "structural_risk");
        assert!(!base.is_empty(), "{tool}: baseline returned no rows");
        assert_eq!(
            base.keys().collect::<Vec<_>>(),
            calibrated.keys().collect::<Vec<_>>(),
            "{tool}: the artifact must reweight scores, not change the file set"
        );
        assert!(
            base.iter()
                .any(|(path, risk)| (calibrated[path] - risk).abs() > 1e-9),
            "{tool}: permuted smell weights moved no structural_risk — the \
             server's --defect-calibration never reached the analysis"
        );
    }
}

/// A one-language corpus artifact whose single `cyclomatic` pool is a linear
/// ramp to one million — any real file sits at a vanishing percentile, so its
/// lens output is observably different from the embedded world corpus.
fn write_corpus_artifact(dir: &std::path::Path) -> std::path::PathBuf {
    use codelore_lib::calibration::{
        CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MetricQuantiles, Stratum,
    };
    let quantiles: Vec<f64> = (0..=1000).map(|i| f64::from(i) * 1000.0).collect();
    let artifact = CalibrationArtifact {
        format_version: CALIBRATION_FORMAT_VERSION,
        corpus_vintage: "test-mcp-lens".to_string(),
        generated_at: "2026-09-01T00:00:00Z".to_string(),
        repos_included: 3,
        repos_attempted: 3,
        languages: vec![LanguageTable {
            language: "rust".to_string(),
            sample_functions: 4_000,
            strata: vec![Stratum {
                sloc_min: 0,
                sloc_max: u64::MAX,
                metrics: vec![MetricQuantiles {
                    metric: "cyclomatic".to_string(),
                    quantiles,
                }],
            }],
        }],
        repo_metrics: None,
    };
    let path = dir.join("world.calib.json");
    std::fs::write(
        &path,
        serde_json::to_string(&artifact).expect("serialize artifact"),
    )
    .expect("write artifact");
    path
}

/// The server-level `--calibration` must reach every lens-consuming tool: the
/// embedded world corpus and a custom artifact must disagree on
/// `corpus_percentile` somewhere while agreeing on which files carry the
/// annotation. Identical outputs would mean the startup flag silently never
/// reached the analysis — the corpus-lens twin of the defect-calibration
/// regression above.
#[test]
fn mcp_code_health_lens_follows_the_server_calibration() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let artifact_path = write_corpus_artifact(artifact_dir.path());

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let base = assert_tool_ok(
        &call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({})),
        "code_health",
    );
    drop(stdin);
    let _ = child.wait();

    let (mut child, mut stdin, mut reader) = spawn_mcp_with_args(
        repo_path,
        &["--calibration", artifact_path.to_str().unwrap()],
    );
    let lensed = assert_tool_ok(
        &call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({})),
        "code_health",
    );
    drop(stdin);
    let _ = child.wait();

    let base = scores_by_path(&base, "corpus_percentile");
    let lensed = scores_by_path(&lensed, "corpus_percentile");
    assert!(
        !base.is_empty(),
        "the embedded world corpus must annotate at least one rust file"
    );
    assert_eq!(
        base.keys().collect::<Vec<_>>(),
        lensed.keys().collect::<Vec<_>>(),
        "the artifact must re-rank the lens, not change which files carry it"
    );
    assert!(
        base.iter()
            .any(|(path, pct)| (lensed[path] - pct).abs() > 1e-9),
        "a ramp-to-a-million corpus moved no corpus_percentile — the server's \
         --calibration never reached the analysis"
    );
}

/// A malformed `--calibration` file must be a startup error (exit 4, before
/// the tokio runtime starts and stdin is read), not a failure surfaced on the
/// first lens-consuming call — the corpus twin of the foreign-defect startup
/// guard below.
#[test]
fn mcp_refuses_to_start_on_a_malformed_calibration_artifact() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let bad = artifact_dir.path().join("world.calib.json");
    std::fs::write(&bad, b"{ this is not a calibration artifact").expect("write bad artifact");

    let bin = assert_cmd::cargo::cargo_bin("codelore");
    let mut builder = Command::new(&bin);
    builder
        .args([
            "mcp",
            "--repo",
            repo_path,
            "--calibration",
            bad.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in LLM_ENV_VARS {
        builder.env_remove(var);
    }
    let output = builder.output().expect("spawn codelore mcp");

    assert_eq!(
        output.status.code(),
        Some(4),
        "expected exit 4 for a malformed --calibration artifact, got {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
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

/// `check_gates` must evaluate under the calibration the server resolved at
/// startup — here via the thresholds `[calibration]` section. The artifact is
/// valid when the server starts (startup validation passes) and is deleted
/// before the tool call, so the call can only fail if the per-call `Options`
/// actually carries the calibration path into the analyses. A server that
/// ignored the section here would return a verdict instead of an error.
#[test]
fn check_gates_honors_calibration_section() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let artifact_path = write_foreign_defect_artifact(artifact_dir.path());

    // A gate that must be evaluated (non-empty thresholds) plus the
    // calibration section naming the artifact.
    // A TOML *literal* (single-quoted) string: a Windows absolute path
    // contains backslashes, which a double-quoted TOML string would read as
    // escape sequences (e.g. `\U`), making the thresholds file unparseable and
    // the server refuse to start. A literal string takes the path verbatim.
    std::fs::write(
        repo.dir.path().join(".codelore-thresholds.toml"),
        format!(
            "[gates]\ncode_health_min = 0.0\n\n[calibration]\ndefect_artifact = '{}'\n",
            artifact_path.display()
        ),
    )
    .unwrap();

    // The artifact is foreign to the fixture repo; the override lets startup
    // validation pass so the failure below can only come from the tool call.
    let (mut child, mut stdin, mut reader) =
        spawn_mcp_with_args(repo_path, &["--allow-foreign-calibration"]);

    std::fs::remove_file(&artifact_path).expect("delete artifact after startup");

    let resp = call_tool(&mut stdin, &mut reader, 1, "check_gates", &json!({}));
    let is_error =
        resp["error"].is_object() || resp["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        is_error,
        "check_gates must fail when the configured calibration artifact cannot be loaded: {resp}"
    );
    assert!(
        resp.to_string().contains("defects.calib.json"),
        "the error must name the missing artifact: {resp}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_change_context_returns_briefing_for_known_path() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    // Derive a target from `code_health` so the briefing is guaranteed a file
    // with recorded history. A path present at the default `min_revs` is a
    // superset of what the briefing sees at `min_revs = 1`.
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
        "change_context",
        &json!({ "paths": [target.clone()] }),
    );
    let text = assert_tool_ok_text(&resp, "change_context");

    // The briefing is plain text: the requested path heads its block, and the
    // health + owner lines are present (in honest-absence form if data is
    // missing, but the labels are always there for a path with history).
    assert!(
        text.contains(&target),
        "briefing must name the requested path: {text}"
    );
    assert!(
        text.contains("health "),
        "briefing must carry a health line: {text}"
    );
    assert!(
        text.contains("owner:"),
        "briefing must carry an owner line: {text}"
    );
    // The output is a fixed-format text briefing, never a JSON document and
    // never a renderer that leaked an `undefined` sentinel.
    assert!(
        !text.contains("undefined"),
        "briefing must not contain the literal 'undefined': {text}"
    );
    assert!(
        !text.contains('{') && !text.contains('}'),
        "briefing is plain text, not JSON: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// Resolve `HEAD` to a full SHA via git — used to write a realistic
/// `MERGE_HEAD` marker into the fixture's git dir.
fn head_sha(repo_path: &str) -> String {
    let out = Command::new("git")
        .args(["-C", repo_path, "rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse HEAD");
    assert!(out.status.success(), "git rev-parse HEAD failed");
    String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}

#[test]
fn mcp_change_context_reflects_mid_session_merge_state() {
    // A merge/rebase started or aborted at unchanged HEAD must not replay a
    // stale briefing from the process-lifetime memo. The merge-in-progress
    // marker changes `merge_or_rebase_in_progress()` (and thus the leading
    // note) without moving HEAD, so it is folded into the memo key: the note
    // appears when a merge begins and disappears when it is aborted, across
    // three calls to one long-lived server process.
    const NOTE: &str = "merge/rebase in progress";
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    let merge_head = repo.dir.path().join(".git").join("MERGE_HEAD");

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    // A file guaranteed to have history, mirroring the sibling briefing test.
    let ch = call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({}));
    let ch_rows = assert_tool_ok(&ch, "code_health");
    let target = ch_rows
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["path"].as_str())
        .expect("code_health yields at least one file")
        .to_string();
    let params = json!({ "paths": [target] });

    // 1) Clean tree: no note. This populates the memo at the merge=false key.
    let clean = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 2, "change_context", &params),
        "change_context",
    );
    assert!(
        !clean.contains(NOTE),
        "clean tree must carry no merge note: {clean}"
    );

    // 2) Merge in progress at the SAME HEAD: the note must appear, proving the
    // second call did not serve the memoized merge=false briefing.
    std::fs::write(&merge_head, format!("{}\n", head_sha(repo_path))).expect("write MERGE_HEAD");
    let merging = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 3, "change_context", &params),
        "change_context",
    );
    assert!(
        merging.contains(NOTE),
        "a merge started at unchanged HEAD must surface the note, not a memoized-stale briefing: {merging}"
    );

    // 3) Merge aborted: the note must be gone again.
    std::fs::remove_file(&merge_head).expect("remove MERGE_HEAD");
    let aborted = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 4, "change_context", &params),
        "change_context",
    );
    assert!(
        !aborted.contains(NOTE),
        "an aborted merge at unchanged HEAD must drop the note: {aborted}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_change_context_rejects_empty_and_oversized_path_lists() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    // 0 paths — below the 1-path floor.
    let empty = call_tool(
        &mut stdin,
        &mut reader,
        1,
        "change_context",
        &json!({ "paths": [] }),
    );
    assert_eq!(empty["jsonrpc"], "2.0");
    let empty_is_error =
        empty["error"].is_object() || empty["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        empty_is_error,
        "an empty path list must be an error, got: {empty}"
    );
    assert!(
        empty.to_string().contains("20"),
        "the empty-list error must name the 20-path limit: {empty}"
    );
    // A bad path list is caller input → invalid_params (-32602).
    assert_rpc_error_code(&empty, -32602, "change_context empty paths");

    // 21 paths — above the 20-path ceiling.
    let too_many: Vec<String> = (0..21).map(|i| format!("src/file{i}.rs")).collect();
    let oversized = call_tool(
        &mut stdin,
        &mut reader,
        2,
        "change_context",
        &json!({ "paths": too_many }),
    );
    assert_eq!(oversized["jsonrpc"], "2.0");
    let oversized_is_error =
        oversized["error"].is_object() || oversized["result"]["isError"].as_bool().unwrap_or(false);
    assert!(
        oversized_is_error,
        "an oversized path list must be an error, got: {oversized}"
    );
    assert!(
        oversized.to_string().contains("20"),
        "the oversized-list error must name the 20-path limit: {oversized}"
    );
    assert_rpc_error_code(&oversized, -32602, "change_context oversized paths");

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_change_context_stays_within_token_budget() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    // Two real paths from the code-health rows; if the repo has only one, reuse
    // it (the budget is per requested path, so a duplicate still exercises two
    // blocks).
    let ch = call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({}));
    let ch_rows = assert_tool_ok(&ch, "code_health");
    let rows = ch_rows
        .as_array()
        .expect("code_health returns an array for delivery_repo");
    let first = rows
        .first()
        .and_then(|row| row["path"].as_str())
        .expect("code_health yields at least one file for delivery_repo")
        .to_string();
    let second = rows
        .get(1)
        .and_then(|row| row["path"].as_str())
        .unwrap_or(&first)
        .to_string();

    let resp = call_tool(
        &mut stdin,
        &mut reader,
        2,
        "change_context",
        &json!({ "paths": [first, second] }),
    );
    let text = assert_tool_ok_text(&resp, "change_context");

    // Budget is 150 whitespace-split tokens per requested path (spec §6).
    let tokens = text.split_whitespace().count();
    assert!(
        tokens <= 300,
        "budget is 150 tokens/path; got {tokens} for 2 paths: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// A deeply nested, high-complexity function appended to a tracked fixture
/// file so its projected code-health score lands strictly below its HEAD
/// baseline (the population-relative smell ranks make the appended monster the
/// per-language maximum on every raw metric).
const GATE_MONSTER_FN: &str = r"
fn monster(x: i32) -> i32 {
    let mut acc = 0;
    for a in 0..x {
        if a % 2 == 0 && a % 3 == 0 || a % 5 == 0 {
            for b in 0..a {
                if b > 1 {
                    match b % 4 {
                        0 => { if b > 10 { acc += 1; } else { acc += 2; } }
                        1 => { while acc < 100 { acc += 1; if acc % 7 == 0 { break; } } }
                        2 => { for c in 0..b { if c > 3 && c < 9 || c == 5 { acc += c; } } }
                        _ => { if a > b { acc -= 1; } else { acc += 1; } }
                    }
                }
            }
        }
    }
    acc
}
";

/// Append `GATE_MONSTER_FN` to a tracked file in the fixture clone.
fn worsen_file(repo_root: &std::path::Path, rel_path: &str) {
    let path = repo_root.join(rel_path);
    let mut content = std::fs::read_to_string(&path).expect("read fixture file");
    content.push_str(GATE_MONSTER_FN);
    std::fs::write(&path, content).expect("write fixture file");
}

/// Commit `n` trivial single-function Rust files into the fixture clone so its
/// Rust language cohort clears the code-health per-language structural-biomarker
/// floor. `delivery_repo` ships five Rust files; the per-language percent-rank
/// only ranks a cohort at or above the floor, so below it an appended monster
/// moves no biomarker and the projected health never drops.
fn commit_cohort_fillers(repo_root: &std::path::Path, n: usize) {
    for i in 0..n {
        std::fs::write(
            repo_root.join(format!("src/cohort_filler_{i}.rs")),
            format!("pub fn cohort_filler_{i}() -> i32 {{ {i} }}\n"),
        )
        .expect("write cohort filler");
    }
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["add", "."]);
    // Cloned bundles carry no committer identity, so pin one for this commit.
    git(&[
        "-c",
        "user.email=cohort@example.com",
        "-c",
        "user.name=Cohort",
        "commit",
        "--quiet",
        "-m",
        "cohort fillers",
    ]);
}

#[test]
fn gate_changes_reports_clean_tree() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "gate_changes", &json!({}));
    let text = assert_tool_ok_text(&resp, "gate_changes");

    assert!(
        text.contains("no working-tree changes"),
        "a fresh clone has nothing to gate: {text}"
    );
    assert!(
        text.starts_with("PASS"),
        "a clean tree is an explicit pass, not a skipped evaluation: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn gate_changes_flags_working_tree_edit() {
    let repo = delivery_repo::build();
    // delivery_repo ships five Rust files; commit more so the language cohort
    // clears the code-health per-language biomarker floor and the appended
    // monster can actually lower src/core.rs's projected health.
    commit_cohort_fillers(repo.dir.path(), 9);
    let repo_path = repo.dir.path().to_str().unwrap();

    // A per-file floor of 0.0 (no changed file may lower its own health) at
    // the repo root — discovered by the server, no startup flag involved. The
    // thresholds file is untracked, so it never enters the change set itself.
    std::fs::write(
        repo.dir.path().join(".codelore-thresholds.toml"),
        "[diff]\ndelta_code_health_min_per_file = 0.0\n",
    )
    .unwrap();
    worsen_file(repo.dir.path(), "src/core.rs");

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "gate_changes", &json!({}));
    let text = assert_tool_ok_text(&resp, "gate_changes");

    assert!(
        text.starts_with("FAIL — "),
        "line 1 must be the FAIL verdict: {text}"
    );
    assert!(
        text.contains("  - delta_code_health_min_per_file: src/core.rs — actual "),
        "the violation row must use check's exact form and name the file: {text}"
    );
    assert!(
        text.contains("[health-drop] src/core.rs:"),
        "the health-drop finding must name the file: {text}"
    );
    assert!(
        text.contains(" → "),
        "the delta table must render baseline → projected: {text}"
    );
    // The FAIL next-action line names the worst-delta file and the driving gate.
    assert!(
        text.contains("\u{2192} fix src/core.rs first"),
        "the FAIL action line must name the worst-delta file to fix first: {text}"
    );
    assert!(
        text.contains("drives the delta_code_health_min_per_file violation"),
        "the FAIL action line must name the driving gate: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn gate_changes_token_budget_holds() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    // Multi-finding scenario with no thresholds configured: two worsened
    // tracked files (one health-drop finding each) plus one staged new file
    // (a new-file finding), exercising the advisory-only verdict form.
    worsen_file(repo.dir.path(), "src/core.rs");
    worsen_file(repo.dir.path(), "src/stable.rs");
    std::fs::write(
        repo.dir.path().join("src/fresh.rs"),
        "pub fn fresh() -> u32 { 1 }\n",
    )
    .unwrap();
    let add = Command::new("git")
        .args(["-C", repo_path, "add", "src/fresh.rs"])
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed: {add:?}");

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "gate_changes", &json!({}));
    let text = assert_tool_ok_text(&resp, "gate_changes");

    assert!(
        text.starts_with("no thresholds configured — advisory only"),
        "without thresholds the verdict line discloses advisory-only: {text}"
    );

    // Finding lines are the only lines that open with '['.
    let findings = text.lines().filter(|l| l.starts_with('[')).count();
    assert!(
        findings >= 3,
        "two worsened files plus a staged new file must produce at least \
         three findings, got {findings}: {text}"
    );
    // Budget pinned by the plan: ≤ 80 whitespace tokens base, ≤ 40 per finding.
    let tokens = text.split_whitespace().count();
    assert!(
        tokens <= 80 + 40 * findings,
        "budget is 80 + 40·findings tokens; got {tokens} for {findings} findings: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn gate_changes_findings_render_capped_with_more_tail() {
    // Regression: a change set large enough to produce more findings
    // than the render cap must still stay within the token budget — the
    // per-finding budget formula alone doesn't protect against this because
    // it scales WITH the finding count. 13 newly-added files (each its own
    // "new-file" finding) exceed the 10-row render cap, so a "(+n more
    // findings)" tail must appear and the render must not grow past it.
    const ADDED: usize = 13;
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    for i in 0..ADDED {
        std::fs::write(
            repo.dir.path().join(format!("src/gate_extra_{i}.rs")),
            format!("pub fn extra_{i}() -> u32 {{ {i} }}\n"),
        )
        .unwrap();
    }
    let add = Command::new("git")
        .args(["-C", repo_path, "add", "-A"])
        .output()
        .expect("git add");
    assert!(add.status.success(), "git add failed: {add:?}");

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let resp = call_tool(&mut stdin, &mut reader, 1, "gate_changes", &json!({}));
    let text = assert_tool_ok_text(&resp, "gate_changes");

    let finding_lines = text.lines().filter(|l| l.starts_with("[new-file]")).count();
    assert_eq!(
        finding_lines, 10,
        "the findings render must cap at 10 rows: {text}"
    );
    assert!(
        text.contains(&format!("(+{} more findings)", ADDED - 10)),
        "a '(+n more findings)' tail must disclose the hidden rows: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// Commit every staged/tracked change in `repo_root`, supplying a committer
/// identity explicitly — a fresh fixture clone carries none (cross-platform).
fn commit_all(repo_root: &std::path::Path, message: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["-c", "user.email=codelore-test@example.com"])
        .args(["-c", "user.name=CodeLore Test"])
        .args(["commit", "-aqm", message])
        .status()
        .expect("spawn git commit");
    assert!(status.success(), "git commit must succeed");
}

#[test]
fn mcp_committed_state_read_repeat_is_byte_identical() {
    // A repeated identical committed-state read is served from the process
    // memo, so its serialized payload must be byte-for-byte the same.
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let first = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({})),
        "code_health",
    );
    let second = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 2, "code_health", &json!({})),
        "code_health",
    );
    assert_eq!(
        first, second,
        "a repeated committed-state read must return the memoized payload verbatim"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_committed_state_read_refreshes_after_new_commit() {
    // The memo is keyed by HEAD: a new commit between two reads must yield a
    // fresh result, never the pre-commit payload.
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let before = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 1, "code_health", &json!({})),
        "code_health",
    );

    // Worsen a tracked file that already carries a health row, then commit it so
    // HEAD advances (the memo's invalidation trigger).
    worsen_file(repo.dir.path(), "src/core.rs");
    commit_all(repo.dir.path(), "worsen core.rs");

    let after = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 2, "code_health", &json!({})),
        "code_health",
    );
    assert_ne!(
        before, after,
        "a read after a new commit must be recomputed for the new HEAD, not served \
         from the pre-commit memo"
    );

    // The new HEAD's result is itself memoized — an immediate repeat matches it.
    let after_again = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 3, "code_health", &json!({})),
        "code_health",
    );
    assert_eq!(
        after, after_again,
        "the post-commit result must be memoized under the new HEAD"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn mcp_gate_changes_is_not_memoized_across_worktree_edit() {
    // gate_changes reads the working tree, so it must never be memoized: an
    // uncommitted edit (HEAD unchanged) must change its verdict between two
    // calls in the same server session.
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);
    let clean = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 1, "gate_changes", &json!({})),
        "gate_changes",
    );
    assert!(
        clean.contains("no working-tree changes"),
        "a clean tree reports nothing to gate: {clean}"
    );

    // Edit the working tree WITHOUT committing: HEAD is unchanged, so a
    // HEAD-keyed memo would wrongly replay the clean-tree verdict.
    worsen_file(repo.dir.path(), "src/core.rs");
    let dirty = assert_tool_ok_text(
        &call_tool(&mut stdin, &mut reader, 2, "gate_changes", &json!({})),
        "gate_changes",
    );
    assert_ne!(
        clean, dirty,
        "an uncommitted edit must change gate_changes output — it is never memoized"
    );
    assert!(
        !dirty.contains("no working-tree changes"),
        "the second call must observe the uncommitted edit: {dirty}"
    );
    assert!(
        dirty.contains("src/core.rs"),
        "the worsened file must surface in the second call: {dirty}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// `check_gates` is the one tool that declares an output schema, so an agent
/// can validate its verdict instead of parsing a text blob.
///
/// Two halves, and the second is the one that would break quietly: declaring
/// `outputSchema` obliges the server to return `structuredContent` conforming
/// to it, and it must keep emitting the text block so clients that never read
/// structured content are unaffected. A regression in either half still looks
/// like a working tool from one side.
#[test]
fn mcp_check_gates_declares_output_schema_and_returns_structured_content() {
    let repo = delivery_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();
    std::fs::write(
        repo.dir.path().join(".codelore-thresholds.toml"),
        "[gates]\ncode_health_min = 100.0\n",
    )
    .unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp(repo_path);

    let list_req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    stdin.write_all(&ndjson_line(&list_req)).unwrap();
    stdin.flush().unwrap();
    let listed = read_ndjson(&mut reader);
    let tools = listed["result"]["tools"].as_array().expect("tools array");
    let cg = tools
        .iter()
        .find(|t| t["name"] == "check_gates")
        .expect("check_gates present");
    let schema = &cg["outputSchema"];
    assert!(
        schema.is_object(),
        "check_gates must declare an outputSchema: {cg}"
    );
    let props = schema["properties"]
        .as_object()
        .expect("outputSchema.properties");
    for key in ["verdict", "violation_count", "violations", "skipped_gates"] {
        assert!(
            props.contains_key(key),
            "outputSchema must describe `{key}`: {schema}"
        );
    }
    assert_eq!(
        schema["properties"]["violations"]["type"], "array",
        "`violations` must be typed as an array, not left opaque: {schema}"
    );

    let resp = call_tool(&mut stdin, &mut reader, 2, "check_gates", &json!({}));
    let structured = &resp["result"]["structuredContent"];
    assert!(
        structured.is_object(),
        "declaring an outputSchema obliges the tool to return structuredContent: {resp}"
    );
    assert_eq!(
        structured["verdict"], "fail",
        "a 100.0 floor must fail, and the verdict must arrive as structured data: {structured}"
    );

    // The text block stays, byte-equal to the structured payload, so a client
    // that only reads `content` sees exactly what it always did.
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content block");
    let reparsed: serde_json::Value = serde_json::from_str(text).expect("text block is JSON");
    assert_eq!(
        &reparsed, structured,
        "the text block and structuredContent must carry the same document"
    );

    drop(stdin);
    let _ = child.wait();
}

/// The repo path is the server's most common misconfiguration (a typo'd
/// path in a client config): the server must refuse to START, naming the
/// fix, rather than reporting healthy and failing on every tool call.
#[test]
fn mcp_refuses_to_serve_a_nonexistent_repo() {
    let bin = assert_cmd::cargo::cargo_bin("codelore");
    let out = Command::new(&bin)
        .args(["mcp", "--repo", "/definitely/not/a/repository"])
        .stdin(Stdio::null())
        .output()
        .expect("spawn codelore mcp");
    // Pinned to the exact code, not merely "nonzero": a bare failure check
    // accepts a panic (the workspace unwinds, so a panic escaping main exits
    // 101) and a signal kill just as readily as the intended error, and it
    // accepted the wrong code here for a whole release — the startup guard
    // was raising an untyped error, which main's chain-walk could not map,
    // so a repository misconfiguration exited 1 instead of the spec's 3.
    assert_eq!(
        out.status.code(),
        Some(3),
        "a nonexistent repo is a repository error (spec exit 3), got {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot serve MCP"),
        "the error must name the misconfiguration: {stderr}"
    );
}

/// `--cache-dir` must reach every fact-store open: after one tool call, the
/// persistent cache lands under the override, not the XDG default.
#[test]
fn mcp_cache_dir_override_places_the_fact_store() {
    let tiny = tiny_repo::build();
    let cache = tempfile::tempdir().expect("tempdir");
    let (mut child, mut stdin, mut reader) = spawn_mcp_with_args(
        tiny.dir.path().to_str().unwrap(),
        &["--cache-dir", cache.path().to_str().unwrap()],
    );
    let resp = call_tool(&mut stdin, &mut reader, 1, "repo_overview", &json!({}));
    assert_tool_ok(&resp, "repo_overview");
    drop(stdin);
    let _ = child.wait();

    let mut found = false;
    if let Ok(entries) = std::fs::read_dir(cache.path().join("codelore")) {
        for repo_dir in entries.flatten() {
            if let Ok(files) = std::fs::read_dir(repo_dir.path()) {
                found |= files
                    .flatten()
                    .any(|f| f.path().extension().is_some_and(|e| e == "duckdb"));
            }
        }
    }
    assert!(
        found,
        "one tool call must place a .duckdb fact store under the --cache-dir override"
    );
}
