//! Integration tests for `external::store::ExternalStore`.
//!
//! Covers:
//! - Ingest the three B1 SARIF fixtures → row counts match
//! - Re-ingest the same file → count unchanged (replace semantics)
//! - Two engines coexist independently

use std::path::Path;

use codelore_lib::external::{ExternalStore, parse_sarif};

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
