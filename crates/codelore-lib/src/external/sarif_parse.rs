//! SARIF 2.1.0 subset parser for external scanner findings.
//!
//! Implements the minimal robust parse subset from §VALIDATED:
//! - `rule_id`: `result.ruleId` else `run.tool.driver.rules[ruleIndex].id`
//! - `path`: `locations[0].physicalLocation.artifactLocation.uri`, with
//!   `file://` scheme and leading `./` stripped; `uriBaseId`-agnostic
//! - `start_line` / `end_line`: NULLABLE (absent in many dialects)
//! - `level`: result → rule `defaultConfiguration.level` → `"warning"`
//! - `fingerprint`: `partialFingerprints.primaryLocationLineHash` →
//!   any `partialFingerprints` value → any `fingerprints` value →
//!   self-hash `sha256(engine|rule_id|path|start_line)`
//! - `message.text`
//!
//! Unparseable individual results are skipped with [`tracing::warn`]; a
//! document that is not SARIF at all (missing `version` or `runs`) returns
//! [`Err`].

use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::{CodeLoreError, Result};

/// A single normalized external finding extracted from a SARIF document.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExternalFinding {
    /// `tool.driver.name`
    pub engine: String,
    /// `tool.driver.version`, empty string when absent
    pub engine_version: String,
    /// `result.ruleId` or resolved via `ruleIndex`
    pub rule_id: String,
    /// Normalized repo-relative path (no `file://`, no leading `./`)
    pub path: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    /// `"error"` | `"warning"` | `"note"` via the fallback chain
    pub level: String,
    /// Stable fingerprint; self-hash = `sha256(engine|rule_id|path|start_line)`
    pub fingerprint: String,
    pub message: String,
}

/// Parse a SARIF 2.1.0 document (all runs) into normalized findings.
///
/// Unparseable individual results are skipped with a [`tracing::warn`]; a
/// document that is not SARIF at all is an [`Err`].
///
/// # Errors
///
/// Returns [`CodeLoreError::Analysis`] when the input is not a SARIF document
/// (missing `version` field or `runs` array).
pub fn parse_sarif(raw: &str) -> Result<Vec<ExternalFinding>> {
    let doc: Value = serde_json::from_str(raw)
        .map_err(|e| CodeLoreError::Analysis(format!("SARIF parse: not valid JSON: {e}")))?;

    // A SARIF document MUST have a `version` field and a `runs` array.
    if doc.get("version").is_none() || doc.get("runs").is_none() {
        return Err(CodeLoreError::Analysis(
            "not a SARIF document: missing `version` or `runs`".into(),
        ));
    }

    let Some(runs) = doc["runs"].as_array() else {
        return Err(CodeLoreError::Analysis(
            "SARIF `runs` is not an array".into(),
        ));
    };

    let mut findings = Vec::new();

    for run in runs {
        let driver = &run["tool"]["driver"];
        let engine = driver["name"].as_str().unwrap_or("unknown").to_owned();
        let engine_version = driver["version"].as_str().unwrap_or("").to_owned();

        // Build a rule-id lookup table for ruleIndex indirection (CodeQL dialect).
        let rules: Vec<&Value> = driver["rules"]
            .as_array()
            .map(|v| v.iter().collect())
            .unwrap_or_default();

        let Some(results) = run["results"].as_array() else {
            continue;
        };

        for result in results {
            match parse_result(result, &engine, &engine_version, &rules) {
                Ok(finding) => findings.push(finding),
                Err(reason) => {
                    warn!("skipping unparseable SARIF result: {reason}");
                }
            }
        }
    }

    Ok(findings)
}

/// Parse one SARIF result object into an [`ExternalFinding`].
/// Returns `Err(reason)` — a plain string — for skip-worthy failures.
fn parse_result(
    result: &Value,
    engine: &str,
    engine_version: &str,
    rules: &[&Value],
) -> std::result::Result<ExternalFinding, String> {
    // ── rule_id ──────────────────────────────────────────────────────────────
    let rule_id = resolve_rule_id(result, rules)?;

    // ── location: path + region ──────────────────────────────────────────────
    let (path, start_line, end_line) = resolve_location(result)?;

    // ── level ────────────────────────────────────────────────────────────────
    let level = resolve_level(result, &rule_id, rules);

    // ── message ──────────────────────────────────────────────────────────────
    let message = result["message"]["text"].as_str().unwrap_or("").to_owned();

    // ── fingerprint ──────────────────────────────────────────────────────────
    let fingerprint = resolve_fingerprint(result, engine, &rule_id, &path, start_line);

    Ok(ExternalFinding {
        engine: engine.to_owned(),
        engine_version: engine_version.to_owned(),
        rule_id,
        path,
        start_line,
        end_line,
        level,
        fingerprint,
        message,
    })
}

/// Resolve `rule_id`: `result.ruleId` else `rules[ruleIndex].id`.
fn resolve_rule_id(result: &Value, rules: &[&Value]) -> std::result::Result<String, String> {
    if let Some(id) = result["ruleId"].as_str() {
        return Ok(id.to_owned());
    }
    if let Some(idx) = result["ruleIndex"].as_u64() {
        let idx = usize::try_from(idx).unwrap_or(usize::MAX);
        if let Some(rule) = rules.get(idx)
            && let Some(id) = rule["id"].as_str()
        {
            return Ok(id.to_owned());
        }
        return Err(format!("ruleIndex {idx} out of bounds or rule has no id"));
    }
    // ruleId is technically optional in SARIF 2.1.0; use a placeholder.
    Ok(String::new())
}

/// Resolve the primary location: (repo-relative path, `start_line`, `end_line`).
fn resolve_location(
    result: &Value,
) -> std::result::Result<(String, Option<u32>, Option<u32>), String> {
    let loc = result["locations"]
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| "result has no locations".to_owned())?;

    let uri = loc["physicalLocation"]["artifactLocation"]["uri"]
        .as_str()
        .ok_or_else(|| "location missing uri".to_owned())?;

    let path = normalize_path(uri);

    let region = &loc["physicalLocation"]["region"];
    let start_line = region["startLine"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok());
    let end_line = region["endLine"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok());

    Ok((path, start_line, end_line))
}

/// Strip `file://` scheme (including any authority) and leading `./`.
/// The rest is left as-is — callers receive the path exactly as the
/// scanner encoded it after scheme removal.
///
/// Examples:
/// - `"file:///home/runner/work/repo/src/main.rs"` →
///   `"/home/runner/work/repo/src/main.rs"` (absolute path; scheme stripped,
///   leading `/` kept — `CodeQL` emits absolute URIs and the absolute form
///   is stored as-is in the sidecar)
/// - `"./src/db.py"` → `"src/db.py"` (leading `./` stripped)
/// - `"src/db.py"` → `"src/db.py"` (unchanged)
/// - `"file://src/db.py"` → `"src/db.py"` (relative after scheme)
fn normalize_path(uri: &str) -> String {
    // Strip file:// scheme — may be followed by an absolute host+path
    // (file:///abs) or a relative path (file://rel).
    let without_scheme = if let Some(rest) = uri.strip_prefix("file://") {
        // file:///home/runner/... → /home/runner/... → strip the repo root later;
        // for now just drop the scheme so the caller sees an absolute path.
        // file://src/... (no leading /) → src/...
        rest
    } else {
        uri
    };

    // Strip leading `./`
    let without_dotslash = without_scheme.strip_prefix("./").unwrap_or(without_scheme);

    without_dotslash.to_owned()
}

/// Resolve `level` via fallback chain:
/// result.level → rule.defaultConfiguration.level → `"warning"`.
fn resolve_level(result: &Value, rule_id: &str, rules: &[&Value]) -> String {
    if let Some(lvl) = result["level"].as_str() {
        return normalize_level(lvl);
    }
    // Look up the rule default by matching rule id (ruleIndex may also be used,
    // but we have the resolved rule_id so we match on id to avoid double-lookup).
    for rule in rules {
        if rule["id"].as_str() == Some(rule_id)
            && let Some(lvl) = rule["defaultConfiguration"]["level"].as_str()
        {
            return normalize_level(lvl);
        }
    }
    "warning".to_owned()
}

/// Normalize SARIF level values to the canonical set.
fn normalize_level(s: &str) -> String {
    match s {
        "error" | "warning" | "note" | "none" => s.to_owned(),
        _ => "warning".to_owned(),
    }
}

/// Resolve fingerprint per §VALIDATED fallback chain:
/// 1. `partialFingerprints.primaryLocationLineHash`
/// 2. any other value in `partialFingerprints`
/// 3. any value in `fingerprints` (semgrep stores `matchBasedId/v1` here)
/// 4. self-hash: `sha256(engine|rule_id|path|start_line)`
fn resolve_fingerprint(
    result: &Value,
    engine: &str,
    rule_id: &str,
    path: &str,
    start_line: Option<u32>,
) -> String {
    // 1. partialFingerprints.primaryLocationLineHash
    if let Some(v) = result["partialFingerprints"]["primaryLocationLineHash"].as_str()
        && !v.is_empty()
    {
        return v.to_owned();
    }

    // 2. any other partialFingerprints value
    if let Some(map) = result["partialFingerprints"].as_object() {
        for (_key, val) in map {
            if let Some(s) = val.as_str()
                && !s.is_empty()
            {
                return s.to_owned();
            }
        }
    }

    // 3. any fingerprints value (semgrep uses fingerprints.matchBasedId/v1)
    if let Some(map) = result["fingerprints"].as_object() {
        for (_key, val) in map {
            if let Some(s) = val.as_str()
                && !s.is_empty()
            {
                return s.to_owned();
            }
        }
    }

    // 4. self-hash: sha256(engine|rule_id|path|start_line)
    let mut hasher = Sha256::new();
    hasher.update(engine.as_bytes());
    hasher.update(b"|");
    hasher.update(rule_id.as_bytes());
    hasher.update(b"|");
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    let sl = start_line.map(|n| n.to_string()).unwrap_or_default();
    hasher.update(sl.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
