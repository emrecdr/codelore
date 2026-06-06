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
