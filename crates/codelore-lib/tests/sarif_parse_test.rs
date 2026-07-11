//! Table-driven tests for the SARIF 2.1.0 subset parser.
//!
//! Each dialect fixture exercises the SARIF 2.1.0 fallback chains:
//! - semgrep:  fingerprints.matchBasedId/v1 + uriBaseId %SRCROOT% + rule-default level
//! - clippy:   no fingerprints (self-hash) + relative uri + result-level
//! - `CodeQL`:   `partialFingerprints.primaryLocationLineHash` + `ruleIndex` + `file://` uri

use codelore_lib::external::sarif_parse::{
    ExternalFinding, parse_sarif_with_engines, self_hash_fingerprint,
};
use codelore_lib::test_support::sarif_fixture;

// ─── helpers ────────────────────────────────────────────────────────────────

/// Parse a document and keep only the findings half (engine list asserted
/// separately in the engine-specific tests).
fn parse_findings(raw: &str) -> Vec<ExternalFinding> {
    parse_sarif_with_engines(raw)
        .expect("SARIF document should parse")
        .0
}

// ─── semgrep dialect ────────────────────────────────────────────────────────

/// semgrep fixture exercises:
/// - `fingerprints.matchBasedId/v1` (NOT partialFingerprints) → used as fingerprint
/// - `uriBaseId: "%SRCROOT%"` → ignored; uri taken as-is
/// - `./src/api.py` → leading `./` stripped → `src/api.py`
/// - level from rule `defaultConfiguration.level` (results have no `level` field)
#[test]
fn semgrep_dialect_parses_correctly() {
    let raw = sarif_fixture("semgrep.sarif.json");
    let findings = parse_findings(&raw);

    // Hand-derived expected findings:
    // Result 1: src/db.py, lines 42-44, fingerprint from fingerprints.matchBasedId/v1
    // Result 2: ./src/api.py → src/api.py (leading ./ stripped), line 17, same fingerprints key
    let expected = vec![
        ExternalFinding {
            engine: "semgrep".into(),
            engine_version: "1.70.0".into(),
            rule_id: "python.flask.security.injection.tainted-sql-string".into(),
            path: "src/db.py".into(),
            start_line: Some(42),
            end_line: Some(44),
            level: "error".into(), // from rule defaultConfiguration.level
            fingerprint: "abc123def456".into(),
            message: "User-controlled data in a SQL statement".into(),
        },
        ExternalFinding {
            engine: "semgrep".into(),
            engine_version: "1.70.0".into(),
            rule_id: "python.flask.security.injection.tainted-sql-string".into(),
            path: "src/api.py".into(), // leading ./ stripped
            start_line: Some(17),
            end_line: None,
            level: "error".into(),
            fingerprint: "deadbeef0011".into(),
            message: "Another tainted SQL string".into(),
        },
    ];

    assert_eq!(findings, expected, "semgrep findings mismatch");
}

// ─── clippy dialect ──────────────────────────────────────────────────────────

/// clippy fixture exercises:
/// - NO fingerprints at all → self-hash fallback
/// - `level` from `result.level` directly
/// - relative URI with no scheme (pass-through)
/// - `endLine` present on first result, absent on second
#[test]
fn clippy_dialect_uses_self_hash_fingerprint() {
    let raw = sarif_fixture("clippy.sarif.json");
    let findings = parse_findings(&raw);

    assert_eq!(findings.len(), 2, "expected 2 clippy findings");

    let f1 = &findings[0];
    assert_eq!(f1.engine, "clippy");
    assert_eq!(f1.engine_version, "0.1.80");
    assert_eq!(f1.rule_id, "clippy::unwrap_used");
    assert_eq!(f1.path, "src/main.rs");
    assert_eq!(f1.start_line, Some(88));
    assert_eq!(f1.end_line, Some(88));
    assert_eq!(f1.level, "warning");
    // Self-hash: sha256(clippy|clippy::unwrap_used|src/main.rs|88)
    let expected_fp1 =
        self_hash_fingerprint("clippy", "clippy::unwrap_used", "src/main.rs", Some(88));
    assert_eq!(
        f1.fingerprint, expected_fp1,
        "result 1 fingerprint should be self-hash"
    );

    let f2 = &findings[1];
    assert_eq!(f2.rule_id, "clippy::cognitive_complexity");
    assert_eq!(f2.path, "src/parser.rs");
    assert_eq!(f2.start_line, Some(120));
    assert_eq!(f2.end_line, None);
    let expected_fp2 = self_hash_fingerprint(
        "clippy",
        "clippy::cognitive_complexity",
        "src/parser.rs",
        Some(120),
    );
    assert_eq!(
        f2.fingerprint, expected_fp2,
        "result 2 fingerprint should be self-hash"
    );
}

// ─── CodeQL dialect ──────────────────────────────────────────────────────────

/// `CodeQL` fixture exercises:
/// - `ruleIndex` indirection (no `ruleId` on results)
/// - `partialFingerprints.primaryLocationLineHash` → used as fingerprint
/// - `file://` URI scheme → stripped, leaving the path after the authority
/// - level fallback: result 1 has `level` on result; result 2 has no
///   result.level and the rule has no defaultConfiguration → falls back to "warning"
#[test]
fn codeql_dialect_resolves_rule_index_and_strips_file_uri() {
    let raw = sarif_fixture("codeql.sarif.json");
    let findings = parse_findings(&raw);

    assert_eq!(findings.len(), 2, "expected 2 codeql findings");

    let f1 = &findings[0];
    assert_eq!(f1.engine, "CodeQL");
    assert_eq!(f1.engine_version, "2.17.0");
    // rule_id resolved via ruleIndex=0 → rules[0].id
    assert_eq!(f1.rule_id, "java/sql-injection");
    // file:// stripped — absolute path remainder (including leading /)
    assert_eq!(
        f1.path,
        "/home/runner/work/repo/src/main/java/com/example/Dao.java"
    );
    assert_eq!(f1.start_line, Some(55));
    assert_eq!(f1.end_line, Some(58));
    assert_eq!(f1.level, "error"); // from result.level
    // partialFingerprints.primaryLocationLineHash takes priority
    assert_eq!(f1.fingerprint, "1a2b3c4d5e6f7890");

    let f2 = &findings[1];
    assert_eq!(f2.rule_id, "java/xss"); // rules[1].id via ruleIndex=1
    assert_eq!(
        f2.path,
        "/home/runner/work/repo/src/main/java/com/example/Controller.java"
    );
    assert_eq!(f2.start_line, Some(30));
    assert_eq!(f2.end_line, None);
    // No result.level, no rule defaultConfiguration.level → "warning"
    assert_eq!(f2.level, "warning");
    assert_eq!(f2.fingerprint, "aabbccddeeff0011");
}

// ─── error + skip cases ─────────────────────────────────────────────────────

/// A non-SARIF JSON document (missing `version` and `runs`) should return Err.
#[test]
fn non_sarif_json_returns_err() {
    let not_sarif = r#"{"kind": "SomeOtherTool", "results": []}"#;
    let err = parse_sarif_with_engines(not_sarif);
    assert!(err.is_err(), "non-SARIF JSON should return Err");
}

/// Completely invalid JSON should return Err.
#[test]
fn invalid_json_returns_err() {
    let err = parse_sarif_with_engines("not json at all {{{");
    assert!(err.is_err(), "invalid JSON should return Err");
}

/// A valid SARIF document where one result is malformed (no `locations`)
/// should skip that result but parse the valid ones.
#[test]
fn malformed_result_inside_valid_doc_is_skipped() {
    // One result has no locations (→ skipped); the second is valid.
    let raw = r#"{
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": { "name": "test-scanner", "version": "1.0" } },
            "results": [
                {
                    "ruleId": "test/bad",
                    "message": { "text": "no locations here" }
                },
                {
                    "ruleId": "test/good",
                    "level": "warning",
                    "message": { "text": "valid finding" },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": "src/good.rs" },
                            "region": { "startLine": 10 }
                        }
                    }]
                }
            ]
        }]
    }"#;

    let findings = parse_findings(raw);
    // The malformed result (no locations) is skipped; only the valid one survives.
    assert_eq!(
        findings.len(),
        1,
        "expected exactly 1 finding (malformed skipped)"
    );
    assert_eq!(findings[0].rule_id, "test/good");
    assert_eq!(findings[0].path, "src/good.rs");
    assert_eq!(findings[0].start_line, Some(10));
    // No fingerprints → self-hash
    let expected_fp = self_hash_fingerprint("test-scanner", "test/good", "src/good.rs", Some(10));
    assert_eq!(findings[0].fingerprint, expected_fp);
}

/// A finding with no region (nullable start/end lines) is valid.
#[test]
fn finding_with_no_region_is_valid() {
    let raw = r#"{
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": { "name": "scanner", "version": "0.1" } },
            "results": [{
                "ruleId": "check/no-region",
                "level": "note",
                "message": { "text": "no region" },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": "src/lib.rs" }
                    }
                }]
            }]
        }]
    }"#;

    let findings = parse_findings(raw);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].start_line, None);
    assert_eq!(findings[0].end_line, None);
    assert_eq!(findings[0].level, "note");
}

/// The engine list surfaces an engine even when its run flagged nothing — the
/// signal ingest needs to clear that engine's stale rows on a clean re-scan.
#[test]
fn parse_sarif_engines_includes_zero_result_runs() {
    let raw = r#"{
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": { "name": "semgrep", "version": "1.0" } },
            "results": []
        }]
    }"#;

    // No findings, but the engine name is still surfaced.
    let (findings, engines) = parse_sarif_with_engines(raw).expect("parse");
    assert!(findings.is_empty());
    assert_eq!(engines, vec!["semgrep"]);
}

/// Engines are deduplicated and non-SARIF input errors, matching the findings
/// side of the same parse.
#[test]
fn parse_sarif_engines_dedupes_and_rejects_non_sarif() {
    let two_runs_same_engine = r#"{
        "version": "2.1.0",
        "runs": [
            { "tool": { "driver": { "name": "clippy" } }, "results": [] },
            { "tool": { "driver": { "name": "clippy" } }, "results": [] }
        ]
    }"#;
    let (_findings, engines) = parse_sarif_with_engines(two_runs_same_engine).expect("engines");
    assert_eq!(engines, vec!["clippy"]);

    // Missing `runs` is not a SARIF document.
    assert!(parse_sarif_with_engines(r#"{"version":"2.1.0"}"#).is_err());
}
