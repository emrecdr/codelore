//! HTTP round-trip tests for the two-dialect chat client.
//!
//! Each test spins a one-shot `TcpListener` on `127.0.0.1:0` that serves a
//! single canned HTTP response, points a client at it, and asserts on both the
//! request the client sent (path, headers, body shape) and the response it
//! parsed. Nothing touches an external network.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use codelore_lib::enrichment::client::{AnthropicClient, ChatClient, OpenAiCompatClient};

/// A request captured by the test server: the request line, its headers, and
/// the raw body bytes as a string.
struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Spawn a one-shot server that replies `HTTP 200` with `response_body`.
fn serve_once(response_body: &'static str) -> (String, mpsc::Receiver<CapturedRequest>) {
    serve_once_status(200, "OK", response_body)
}

/// Spawn a one-shot HTTP server on an ephemeral localhost port. It accepts one
/// connection, reads the whole request, hands it back over the returned
/// channel, and replies with `status` and `response_body` as JSON. Returns the
/// bound base URL (`http://127.0.0.1:<port>`).
fn serve_once_status(
    status: u16,
    reason: &'static str,
    response_body: &'static str,
) -> (String, mpsc::Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base = format!("http://{addr}");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let captured = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
        stream.flush().ok();
        // The error tests drop the receiver without reading; ignore the result.
        let _ = tx.send(captured);
    });
    (base, rx)
}

/// Read a complete HTTP request: headers up to the blank line, then exactly
/// `Content-Length` body bytes.
fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        let n = stream.read(&mut chunk).expect("read request headers");
        if n == 0 {
            break buf.len();
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).expect("read request body");
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }

    CapturedRequest {
        request_line,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

/// Find the first index of `needle` in `haystack`.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Whether `headers` carries `name: value` (case-insensitive name).
fn has_header(headers: &[(String, String)], name: &str, value: &str) -> bool {
    headers
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case(name) && v == value)
}

#[test]
fn anthropic_dialect_round_trip() {
    let canned = r#"{"content":[{"type":"text","text":"Anthropic narrative."}]}"#;
    let (base, rx) = serve_once(canned);

    let client = AnthropicClient::new("secret-key".to_string(), "claude-test".to_string(), base);
    let out = client
        .complete("the system prompt", "the user prompt")
        .expect("completion succeeds");
    assert_eq!(out, "Anthropic narrative.");

    let req = rx.recv().expect("captured request");
    assert!(
        req.request_line.starts_with("POST /v1/messages HTTP/1.1"),
        "request line was: {}",
        req.request_line
    );
    assert!(has_header(&req.headers, "x-api-key", "secret-key"));
    assert!(has_header(&req.headers, "anthropic-version", "2023-06-01"));

    let body: serde_json::Value = serde_json::from_str(&req.body).expect("request body is JSON");
    assert_eq!(body["model"], "claude-test");
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["system"], "the system prompt");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "the user prompt");
}

#[test]
fn openai_compat_dialect_round_trip() {
    let canned = r#"{"choices":[{"message":{"role":"assistant","content":"OpenAI narrative."}}]}"#;
    let (base, rx) = serve_once(canned);

    let client =
        OpenAiCompatClient::new(base, Some("bearer-token".to_string()), "llama3".to_string());
    let out = client
        .complete("the system prompt", "the user prompt")
        .expect("completion succeeds");
    assert_eq!(out, "OpenAI narrative.");

    let req = rx.recv().expect("captured request");
    assert!(
        req.request_line
            .starts_with("POST /chat/completions HTTP/1.1"),
        "request line was: {}",
        req.request_line
    );
    assert!(has_header(
        &req.headers,
        "authorization",
        "Bearer bearer-token"
    ));

    let body: serde_json::Value = serde_json::from_str(&req.body).expect("request body is JSON");
    assert_eq!(body["model"], "llama3");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], "the system prompt");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["messages"][1]["content"], "the user prompt");
}

#[test]
fn openai_compat_without_api_key_omits_authorization() {
    let canned = r#"{"choices":[{"message":{"content":"no-auth narrative."}}]}"#;
    let (base, rx) = serve_once(canned);

    let client = OpenAiCompatClient::new(base, None, "llama3".to_string());
    let out = client.complete("sys", "usr").expect("completion succeeds");
    assert_eq!(out, "no-auth narrative.");

    let req = rx.recv().expect("captured request");
    assert!(
        !req.headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization")),
        "no Authorization header should be sent without an api key: {:?}",
        req.headers
    );
}

#[test]
fn non_success_status_becomes_analysis_error_with_status_and_body() {
    let canned = r#"{"error":"model \"ghost\" not found — pull it first"}"#;
    let (base, _rx) = serve_once_status(404, "Not Found", canned);

    let client = OpenAiCompatClient::new(base, None, "ghost".to_string());
    let err = client
        .complete("sys", "usr")
        .expect_err("a 404 must be an error");
    let message = err.to_string();
    assert!(
        message.contains("404"),
        "error should carry the status: {message}"
    );
    assert!(
        message.contains("model") && message.contains("not found"),
        "error should carry the response body: {message}"
    );
}
