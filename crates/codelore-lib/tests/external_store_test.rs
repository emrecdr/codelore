//! Integration tests for `external::store::ExternalStore`.
//!
//! Covers:
//! - Ingest the three dialect SARIF fixtures → row counts match
//! - Re-ingest the same file → count unchanged (replace semantics)
//! - Two engines coexist independently

use std::path::Path;

use codelore_lib::external::{
    ExternalFinding, ExternalStore, group_findings_by_engine, parse_sarif,
};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sarif")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read fixture {name}: {e}"))
}

/// Open a fresh store in a tempdir.
fn temp_store() -> (tempfile::TempDir, ExternalStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store =
        ExternalStore::open_or_create(dir.path(), Path::new("/test/repo")).expect("open_or_create");
    (dir, store)
}

// ─── count after ingesting each fixture ─────────────────────────────────────

#[test]
fn semgrep_fixture_ingests_two_findings() {
    let (_dir, store) = temp_store();
    let raw = fixture("semgrep.sarif.json");
    let findings = parse_sarif(&raw).expect("parse semgrep fixture");
    // Two findings in the semgrep fixture.
    assert_eq!(findings.len(), 2);
    let n = store
        .replace_engine("semgrep", &findings)
        .expect("replace_engine");
    assert_eq!(n, 2);
    assert_eq!(store.count().expect("count"), 2);
}

#[test]
fn clippy_fixture_ingests_two_findings() {
    let (_dir, store) = temp_store();
    let raw = fixture("clippy.sarif.json");
    let findings = parse_sarif(&raw).expect("parse clippy fixture");
    assert_eq!(findings.len(), 2);
    let n = store
        .replace_engine("clippy", &findings)
        .expect("replace_engine");
    assert_eq!(n, 2);
    assert_eq!(store.count().expect("count"), 2);
}

#[test]
fn codeql_fixture_ingests_two_findings() {
    let (_dir, store) = temp_store();
    let raw = fixture("codeql.sarif.json");
    let findings = parse_sarif(&raw).expect("parse codeql fixture");
    assert_eq!(findings.len(), 2);
    let n = store
        .replace_engine("CodeQL", &findings)
        .expect("replace_engine");
    assert_eq!(n, 2);
    assert_eq!(store.count().expect("count"), 2);
}

// ─── replace semantics ───────────────────────────────────────────────────────

/// Re-ingesting the same file must leave the count unchanged.
#[test]
fn reingest_same_engine_is_idempotent() {
    let (_dir, store) = temp_store();
    let raw = fixture("semgrep.sarif.json");
    let findings = parse_sarif(&raw).expect("parse");

    store
        .replace_engine("semgrep", &findings)
        .expect("first ingest");
    assert_eq!(store.count().expect("count"), 2);

    // Re-ingest — count must stay at 2.
    store
        .replace_engine("semgrep", &findings)
        .expect("second ingest");
    assert_eq!(store.count().expect("count after re-ingest"), 2);
}

// ─── multi-run SARIF document ────────────────────────────────────────────────

/// A single SARIF document may contain multiple runs from different engines.
/// `parse_sarif` iterates all runs; findings from each run are keyed by that
/// run's `tool.driver.name`. Ingesting the result groups them by engine so
/// `replace_engine` applies the correct per-engine replace semantics.
#[test]
fn multi_run_sarif_produces_findings_from_both_engines() {
    let multi_run = r#"{
        "version": "2.1.0",
        "runs": [
            {
                "tool": { "driver": { "name": "engine-a", "version": "1.0" } },
                "results": [
                    {
                        "ruleId": "a/rule1",
                        "level": "warning",
                        "message": { "text": "finding from engine-a" },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": "src/foo.rs" },
                                "region": { "startLine": 10 }
                            }
                        }]
                    }
                ]
            },
            {
                "tool": { "driver": { "name": "engine-b", "version": "2.0" } },
                "results": [
                    {
                        "ruleId": "b/rule1",
                        "level": "error",
                        "message": { "text": "finding from engine-b" },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": "src/bar.rs" },
                                "region": { "startLine": 20 }
                            }
                        }]
                    },
                    {
                        "ruleId": "b/rule2",
                        "level": "warning",
                        "message": { "text": "second finding from engine-b" },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": "src/baz.rs" },
                                "region": { "startLine": 5 }
                            }
                        }]
                    }
                ]
            }
        ]
    }"#;

    let findings = parse_sarif(multi_run).expect("parse multi-run SARIF");
    // 1 from engine-a + 2 from engine-b = 3 total.
    assert_eq!(findings.len(), 3);
    assert_eq!(
        findings.iter().filter(|f| f.engine == "engine-a").count(),
        1
    );
    assert_eq!(
        findings.iter().filter(|f| f.engine == "engine-b").count(),
        2
    );

    // Group by engine (mirrors what run_ingest_sarif_cmd does).
    let (_dir, store) = temp_store();
    let mut by_engine: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for f in findings {
        by_engine.entry(f.engine.clone()).or_default().push(f);
    }
    for (engine, batch) in &by_engine {
        store.replace_engine(engine, batch).expect("replace_engine");
    }

    // All 3 findings must be in the store under their respective engines.
    assert_eq!(store.count().expect("count"), 3);
}

// ─── multi-file same-engine grouping ────────────────────────────────────────

/// When two SARIF files share the same engine name (e.g. two semgrep runs on
/// different parts of a monorepo), the caller groups findings by engine before
/// calling `replace_engine` once per engine. The combined count must equal the
/// sum of both files — not just the second file's count.
///
/// This is the core correctness property of the per-engine replace design.
#[test]
fn multi_file_same_engine_combined_count_is_sum() {
    let (_dir, store) = temp_store();

    // Two synthetic batches both labelled "semgrep" (as if from two files).
    let batch_a: Vec<ExternalFinding> = vec![ExternalFinding {
        engine: "semgrep".to_string(),
        engine_version: "1.0.0".to_string(),
        rule_id: "rule/a1".to_string(),
        path: "src/alpha.rs".to_string(),
        start_line: Some(1),
        end_line: None,
        level: "warning".to_string(),
        fingerprint: "semgrep/v1/a1/alpha".to_string(),
        message: "finding a1".to_string(),
    }];

    let batch_b: Vec<ExternalFinding> = vec![
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule/b1".to_string(),
            path: "src/beta.rs".to_string(),
            start_line: Some(2),
            end_line: None,
            level: "error".to_string(),
            fingerprint: "semgrep/v1/b1/beta".to_string(),
            message: "finding b1".to_string(),
        },
        ExternalFinding {
            engine: "semgrep".to_string(),
            engine_version: "1.0.0".to_string(),
            rule_id: "rule/b2".to_string(),
            path: "src/gamma.rs".to_string(),
            start_line: Some(3),
            end_line: None,
            level: "note".to_string(),
            fingerprint: "semgrep/v1/b2/gamma".to_string(),
            message: "finding b2".to_string(),
        },
    ];

    // Caller groups both batches into one combined batch (mirrors run_ingest_sarif_cmd).
    let mut combined = batch_a;
    combined.extend(batch_b);

    let n = store
        .replace_engine("semgrep", &combined)
        .expect("replace_engine combined");

    // 1 from batch_a + 2 from batch_b = 3 total — not just 2 (file2-overwrites-file1).
    assert_eq!(n, 3, "replace_engine must return count of inserted rows");
    assert_eq!(
        store.count().expect("count"),
        3,
        "store must hold all findings from both files"
    );
}

// ─── group_findings_by_engine pins the real ingest code path ────────────────

/// Simulates `run_ingest_sarif_cmd` passing two fixture files: parse each,
/// extend a flat vec, call `group_findings_by_engine`, then `replace_engine`
/// per engine. This exercises the extracted lib fn on real fixture data so
/// the actual ingest code path is under test, not just the store contract.
///
/// Semgrep fixture: 2 findings (engine "semgrep").
/// Clippy fixture:  2 findings (engine "clippy-sarif").
/// Expected after grouping + ingesting: 4 total across 2 engine keys.
#[test]
fn group_findings_by_engine_combines_two_fixture_files() {
    let (_dir, store) = temp_store();

    // Parse two fixture files into a flat vec (mirrors the cmd's loop).
    let mut all = Vec::new();
    all.extend(parse_sarif(&fixture("semgrep.sarif.json")).expect("parse semgrep"));
    all.extend(parse_sarif(&fixture("clippy.sarif.json")).expect("parse clippy"));

    // Group and ingest — this is the extracted code path.
    let by_engine = group_findings_by_engine(all);
    for (engine, findings) in &by_engine {
        store
            .replace_engine(engine, findings)
            .unwrap_or_else(|e| panic!("replace_engine {engine}: {e}"));
    }

    assert_eq!(
        store.count().expect("count"),
        4,
        "2 semgrep + 2 clippy findings must all be stored"
    );
    assert_eq!(by_engine.len(), 2, "two distinct engine keys");
}

// ─── two engines coexist ─────────────────────────────────────────────────────

/// Ingesting two different engines must accumulate findings (not replace each other).
#[test]
fn two_engines_coexist() {
    let (_dir, store) = temp_store();

    let semgrep_raw = fixture("semgrep.sarif.json");
    let semgrep = parse_sarif(&semgrep_raw).expect("parse semgrep");
    store
        .replace_engine("semgrep", &semgrep)
        .expect("ingest semgrep");

    let clippy_raw = fixture("clippy.sarif.json");
    let clippy = parse_sarif(&clippy_raw).expect("parse clippy");
    store
        .replace_engine("clippy", &clippy)
        .expect("ingest clippy");

    // Both engines' findings must be present (2 + 2 = 4).
    assert_eq!(store.count().expect("count"), 4);

    // Re-ingesting semgrep must not disturb clippy findings.
    store
        .replace_engine("semgrep", &semgrep)
        .expect("re-ingest semgrep");
    assert_eq!(store.count().expect("count after semgrep re-ingest"), 4);
}
