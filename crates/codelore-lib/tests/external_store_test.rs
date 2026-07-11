//! Integration tests for `external::store::ExternalStore`.
//!
//! Covers:
//! - Ingest the three dialect SARIF fixtures → row counts match
//! - Re-ingest the same file → count unchanged (replace semantics)
//! - Two engines coexist independently

use codelore_lib::external::{ExternalFinding, group_findings_by_engine, parse_sarif_with_engines};
use codelore_lib::test_support::{finding_for, sarif_fixture, temp_external_store};

/// Parse a fixture/document and keep only the findings half.
fn parse_findings(raw: &str) -> Vec<ExternalFinding> {
    parse_sarif_with_engines(raw)
        .expect("SARIF document should parse")
        .0
}

// ─── count after ingesting each fixture ─────────────────────────────────────

#[test]
fn semgrep_fixture_ingests_two_findings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());
    let raw = sarif_fixture("semgrep.sarif.json");
    let findings = parse_findings(&raw);
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
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());
    let raw = sarif_fixture("clippy.sarif.json");
    let findings = parse_findings(&raw);
    assert_eq!(findings.len(), 2);
    let n = store
        .replace_engine("clippy", &findings)
        .expect("replace_engine");
    assert_eq!(n, 2);
    assert_eq!(store.count().expect("count"), 2);
}

#[test]
fn codeql_fixture_ingests_two_findings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());
    let raw = sarif_fixture("codeql.sarif.json");
    let findings = parse_findings(&raw);
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
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());
    let raw = sarif_fixture("semgrep.sarif.json");
    let findings = parse_findings(&raw);

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
/// The parser iterates all runs; findings from each run are keyed by that
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

    let findings = parse_findings(multi_run);
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

    // Group by engine (the real ingest path uses group_findings_by_engine).
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());
    let by_engine = group_findings_by_engine(findings);
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
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());

    // Two synthetic batches both labelled "semgrep" (as if from two files).
    // Distinct paths give distinct (engine, fingerprint) keys, so all three
    // rows coexist rather than colliding on the primary key.
    let batch_a = vec![finding_for("src/alpha.rs", "semgrep", "warning")];
    let batch_b = vec![
        finding_for("src/beta.rs", "semgrep", "error"),
        finding_for("src/gamma.rs", "semgrep", "note"),
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
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());

    // Parse two fixture files into a flat vec (mirrors the cmd's loop).
    let mut all = Vec::new();
    all.extend(parse_findings(&sarif_fixture("semgrep.sarif.json")));
    all.extend(parse_findings(&sarif_fixture("clippy.sarif.json")));

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
    let dir = tempfile::tempdir().expect("tempdir");
    let store = temp_external_store(dir.path());

    let semgrep_raw = sarif_fixture("semgrep.sarif.json");
    let semgrep = parse_findings(&semgrep_raw);
    store
        .replace_engine("semgrep", &semgrep)
        .expect("ingest semgrep");

    let clippy_raw = sarif_fixture("clippy.sarif.json");
    let clippy = parse_findings(&clippy_raw);
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
