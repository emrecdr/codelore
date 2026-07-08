//! Integration tests for `codelore mcp` — newline-delimited JSON-RPC 2.0 over stdio.
//!
//! The rmcp stdio transport uses newline-delimited JSON (one JSON object per line).
//! The test spawns the binary, exchanges the MCP initialize handshake, calls
//! `tools/list` and `tools/call` (repo_overview), and asserts the JSON shape.
//! Uses the `tiny_repo` fixture so the ingest is fast.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use codelore_lib::test_support::tiny_repo;
use serde_json::{Value, json};

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

#[test]
fn mcp_tools_list_and_repo_overview() {
    let repo = tiny_repo::build();
    let repo_path = repo.dir.path().to_str().unwrap();

    let bin = assert_cmd::cargo::cargo_bin("codelore");

    let mut child = Command::new(&bin)
        .args(["mcp", "--repo", repo_path])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn codelore mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    // 1. initialize request.
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0.0.1" }
        }
    });
    stdin.write_all(&ndjson_line(&init_req)).unwrap();
    stdin.flush().unwrap();

    let init_resp = read_ndjson(&mut reader);
    assert_eq!(init_resp["jsonrpc"], "2.0");
    assert_eq!(init_resp["id"], 1);
    assert!(
        init_resp["result"]["capabilities"].is_object(),
        "expected capabilities object, got: {init_resp}"
    );

    // 2. initialized notification (required by MCP spec before calling tools).
    let initialized_notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    stdin.write_all(&ndjson_line(&initialized_notif)).unwrap();
    stdin.flush().unwrap();

    // 3. tools/list.
    let list_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    stdin.write_all(&ndjson_line(&list_req)).unwrap();
    stdin.flush().unwrap();

    let list_resp = read_ndjson(&mut reader);
    assert_eq!(list_resp["jsonrpc"], "2.0");
    assert_eq!(list_resp["id"], 2);
    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        tool_names.contains(&"repo_overview"),
        "repo_overview missing from tools: {tool_names:?}"
    );
    assert!(
        tool_names.contains(&"hotspots"),
        "hotspots missing from tools: {tool_names:?}"
    );

    // 4. tools/call repo_overview.
    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "repo_overview",
            "arguments": {}
        }
    });
    stdin.write_all(&ndjson_line(&call_req)).unwrap();
    stdin.flush().unwrap();

    let call_resp = read_ndjson(&mut reader);
    assert_eq!(call_resp["jsonrpc"], "2.0");
    assert_eq!(call_resp["id"], 3);
    assert!(
        call_resp["result"].is_object(),
        "expected result object: {call_resp}"
    );
    assert!(
        !call_resp["result"]["isError"].as_bool().unwrap_or(false),
        "repo_overview returned an error: {call_resp}"
    );
    // The content array should contain at least one text element whose
    // text is valid JSON (a serialized Vec<SummaryRow>).
    let content = call_resp["result"]["content"]
        .as_array()
        .expect("content array");
    assert!(!content.is_empty(), "content array is empty");
    let text = content[0]["text"].as_str().expect("text field");
    let parsed: Value = serde_json::from_str(text).expect("content text is valid JSON");
    assert!(
        parsed.is_array(),
        "expected JSON array of summary rows: {text}"
    );

    // Shut down cleanly.
    drop(stdin);
    let _ = child.wait();
}
