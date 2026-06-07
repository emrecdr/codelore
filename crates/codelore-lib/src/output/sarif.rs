//! SARIF 2.1.0 emitter for hotspot results — Behavioral SARIF taxonomy (spec §5.4).
//!
//! Rule `CODELORE-HOTSPOT`: properties.tags includes "behavioral" and "hotspot"
//! security-severity proxy: `(100 − code_health) / 10`
//! partialFingerprints for stable identity across CI runs.

use crate::analyses::hotspots::HotspotRow;
use crate::{CodeLoreError, Result};
use sha2::{Digest, Sha256};
use std::io::Write;

const SARIF_SCHEMA: &str = "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0.json";
const RULE_ID: &str = "CODELORE-HOTSPOT";
const AUTOMATION_ID: &str = "codelore/hotspots/run";

/// Emit a SARIF 2.1.0 document for `rows` to `w`.
///
/// * `repo_root` — used as the URI prefix for artifact locations
///   (e.g. `"https://github.com/example/repo"`).
pub fn write_hotspots_sarif<W: Write>(
    rows: &[HotspotRow],
    repo_root: &str,
    w: &mut W,
) -> Result<()> {
    let doc = build_sarif(rows, repo_root);
    serde_json::to_writer_pretty(w, &doc)
        .map_err(|e| CodeLoreError::Output(format!("sarif: {e}")))?;
    Ok(())
}

fn build_sarif(rows: &[HotspotRow], repo_root: &str) -> serde_json::Value {
    use serde_json::{Value, json};

    let rule = json!({
        "id": RULE_ID,
        "shortDescription": {
            "text": "Behavioral hotspot: high churn × high complexity"
        },
        "helpUri": "https://codescene.com/docs/guides/technical/hotspots.html",
        "properties": {
            "tags": ["behavioral", "hotspot"]
        }
    });

    let results: Vec<Value> = rows
        .iter()
        .map(|row| build_result(row, repo_root))
        .collect();

    json!({
        "$schema": SARIF_SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": {
                "id": AUTOMATION_ID
            },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/emre/codescene",
                    "rules": [rule]
                }
            },
            "results": results
        }]
    })
}

fn build_result(row: &HotspotRow, repo_root: &str) -> serde_json::Value {
    use serde_json::json;

    let level = if row.hotspot_score >= 0.5 {
        "warning"
    } else {
        "note"
    };

    // Stable fingerprint: sha256 of "<repo_root>|<path>"
    let fp = {
        let mut hasher = Sha256::new();
        hasher.update(repo_root.as_bytes());
        hasher.update(b"|");
        hasher.update(row.path.as_bytes());
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };

    // security-severity proxy: (100 - code_health) / 10  (range 0.0–10.0)
    let security_severity = (100.0 - row.code_health) / 10.0;

    // Artifact URI: repo_root + "/" + path (strip leading slash from path if any)
    let artifact_uri = format!(
        "{}/{}",
        repo_root.trim_end_matches('/'),
        row.path.trim_start_matches('/')
    );

    json!({
        "ruleId": RULE_ID,
        "level": level,
        "message": {
            "text": format!(
                "Hotspot '{}': score={:.3}, code_health={:.1}, revisions={}, cognitive={:.1}",
                row.path, row.hotspot_score, row.code_health, row.revisions, row.cognitive
            )
        },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": {
                    "uri": artifact_uri
                }
            }
        }],
        "partialFingerprints": {
            "primaryLocationLineHash": fp
        },
        "properties": {
            "security-severity": security_severity,
            "codelore/revs": row.revisions,
            "codelore/cognitive": row.cognitive,
            "codelore/codehealth": row.code_health,
            "codelore/score": row.hotspot_score,
            "tags": ["behavioral", "hotspot"]
        }
    })
}

// =============================================================================
// CODELORE-CLONE (Plan 8 §2 Task 10)
//
// Plain clone-detection findings (the live-clone variant — CODELORE-LIVE-CLONE
// — lands in Plan 8 §6 Task 21). One SARIF result per clone family with
// `locations[]` listing every family member (one physicalLocation per file +
// line range). `partialFingerprints.cloneGroupFingerprint/v1` = the AST
// digest from the clones row, so results are stable across CI runs even when
// family sizes fluctuate.
// =============================================================================

use crate::analyses::clones::ClonesRow;

const CLONE_RULE_ID: &str = "CODELORE-CLONE";
const CLONE_AUTOMATION_ID: &str = "codelore/clones/run";

/// Emit a SARIF 2.1.0 document for clone families to `w`.
///
/// Members of the same `clone_group_id` are aggregated into a single SARIF
/// `result` with multiple `locations[]`. Severity scales mildly with family
/// size (more copies = more drift risk) but caps at 6 — these are noisier
/// than hotspots, and live-clones (CODELORE-LIVE-CLONE, Plan 8 §6) carry
/// the higher severity.
pub fn write_clones_sarif<W: Write>(rows: &[ClonesRow], repo_root: &str, w: &mut W) -> Result<()> {
    let doc = build_clones_sarif(rows, repo_root);
    serde_json::to_writer_pretty(w, &doc)
        .map_err(|e| CodeLoreError::Output(format!("clones sarif: {e}")))?;
    Ok(())
}

fn build_clones_sarif(rows: &[ClonesRow], repo_root: &str) -> serde_json::Value {
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    // Group rows by clone_group_id so each family becomes one SARIF result.
    let mut families: BTreeMap<u32, Vec<&ClonesRow>> = BTreeMap::new();
    for row in rows {
        families.entry(row.clone_group_id).or_default().push(row);
    }

    let rule = json!({
        "id": CLONE_RULE_ID,
        "shortDescription": {
            "text": "Code clone family (Type 1 + Type 2 via AST structural hashing)"
        },
        "fullDescription": {
            "text": "A group of functions whose AST structure is identical after \
                     normalizing identifiers and literals. Type 1 = exact; Type 2 = \
                     renamed/parameterized. See CODELORE-LIVE-CLONE for the \
                     higher-severity intersection with change-coupling."
        },
        "helpUri": "https://github.com/emre/codescene/blob/main/docs/superpowers/plans/2026-06-07-codelore-plan-7-clone-detection.md",
        "properties": {
            "precision": "medium",
            "tags": ["behavioral", "clone", "type-1", "type-2"]
        }
    });

    let results: Vec<Value> = families
        .into_iter()
        .map(|(group_id, members)| build_clones_result(group_id, &members, repo_root))
        .collect();

    json!({
        "$schema": SARIF_SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": { "id": CLONE_AUTOMATION_ID },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/emre/codescene",
                    "rules": [rule]
                }
            },
            "results": results
        }]
    })
}

fn build_clones_result(
    group_id: u32,
    members: &[&ClonesRow],
    repo_root: &str,
) -> serde_json::Value {
    use serde_json::{Value, json};

    let family_size = members.len();
    // Severity scales mildly with family size but caps at 6.0; live-clones
    // (Plan 8 §6 Task 21) carry the higher severity. usize → f64 cast is safe
    // here because family_size is bounded by available memory and never
    // approaches 2^52.
    #[allow(clippy::cast_precision_loss)]
    let security_severity = (3.0_f64 + family_size as f64).min(6.0);
    let level = if family_size >= 5 { "warning" } else { "note" };

    let fingerprint = members.first().map_or("", |m| m.fingerprint.as_str());

    // Each family member → one physicalLocation. SARIF consumers (e.g. GitHub
    // Code Scanning) render the first location as the primary; the rest become
    // relatedLocations conceptually. We put them all in `locations[]` so any
    // SARIF-compliant consumer can iterate them.
    let locations: Vec<Value> = members
        .iter()
        .map(|m| {
            let artifact_uri = format!(
                "{}/{}",
                repo_root.trim_end_matches('/'),
                m.entity.trim_start_matches('/')
            );
            json!({
                "physicalLocation": {
                    "artifactLocation": { "uri": artifact_uri },
                    "region": {
                        "startLine": m.start_line,
                        "endLine": m.end_line
                    }
                },
                "message": { "text": format!("function: {}", m.function) }
            })
        })
        .collect();

    let names = members
        .iter()
        .map(|m| m.function.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    json!({
        "ruleId": CLONE_RULE_ID,
        "level": level,
        "message": {
            "text": format!(
                "Clone family of {} functions (similarity {:.2}, {} structural nodes): {}",
                family_size,
                members.first().map_or(1.0, |m| m.similarity),
                members.first().map_or(0, |m| m.node_count),
                names
            )
        },
        "locations": locations,
        "partialFingerprints": {
            // Versioned key per Plan 8 §6 research brief.
            "cloneGroupFingerprint/v1": fingerprint,
            "cloneGroupId/v1": format!("{group_id}")
        },
        "properties": {
            "security-severity": security_severity,
            "codelore/clone-group-id": group_id,
            "codelore/family-size": family_size,
            "codelore/similarity": members.first().map_or(1.0, |m| m.similarity),
            "codelore/node-count": members.first().map_or(0, |m| m.node_count),
            "tags": ["behavioral", "clone", "type-1", "type-2"]
        }
    })
}
