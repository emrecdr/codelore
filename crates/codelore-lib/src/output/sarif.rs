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

// =============================================================================
// CODELORE-LIVE-CLONE (Plan 8 §6 Task 21)
//
// The high-severity intersection of clones × Fisher-significant change-coupling.
// One SARIF result per (clone_group_id, file_a, file_b) — see research brief
// a0a6cf3534a65a643. Designed to surface inline in GitHub Code Scanning on PRs
// that touch any participating file.
// =============================================================================

use crate::analyses::clone_coupling::CloneCouplingRow;

const LIVE_CLONE_RULE_ID: &str = "CODELORE-LIVE-CLONE";
const LIVE_CLONE_AUTOMATION_ID: &str = "codelore/clone-coupling/run";

/// Emit a SARIF 2.1.0 document for live clone-coupling findings.
///
/// Schema per the research brief:
///
/// - One SARIF `result` per (`clone_group_id`, `file_a`, `file_b`) pair.
/// - `locations[0]` = higher-`support_a` partner (the more-frequently-changed
///   file); `locations[1]` = lower partner. Matches GitHub Code Scanning's
///   "first location is primary" rendering convention.
/// - `partialFingerprints` keys: `cloneGroupFingerprint/v1` (AST digest) +
///   `filePairHash/v1` (`sha256` of sorted file pair).
///   - `properties.security-severity` derived from `combined_score * 10`
///     (0-10 scale per SARIF spec §3.27.17). Live clones get higher severity
///     than the bare CODELORE-CLONE rule because the co-change signal proves
///     this is real debt, not dead lookalike code.
pub fn write_clone_coupling_sarif<W: Write>(
    rows: &[CloneCouplingRow],
    repo_root: &str,
    w: &mut W,
) -> Result<()> {
    let doc = build_clone_coupling_sarif(rows, repo_root);
    serde_json::to_writer_pretty(w, &doc)
        .map_err(|e| CodeLoreError::Output(format!("clone-coupling sarif: {e}")))?;
    Ok(())
}

fn build_clone_coupling_sarif(rows: &[CloneCouplingRow], repo_root: &str) -> serde_json::Value {
    use serde_json::{Value, json};

    let rule = json!({
        "id": LIVE_CLONE_RULE_ID,
        "shortDescription": {
            "text": "Live clone: cloned function whose copies co-change at Fisher-significant rates"
        },
        "fullDescription": {
            "text": "A pair of cloned functions whose containing files are also \
                     coupled at Fisher-exact p < 0.05. The combined_score \
                     (similarity × coupling_degree × (1 − p_value)) ranks how \
                     actionable the finding is. Live clones are real technical \
                     debt; dead clones (filtered out) are noise."
        },
        "helpUri": "https://github.com/emre/codescene/blob/main/docs/superpowers/plans/2026-06-07-codelore-plan-8-v1.x-readiness.md",
        "properties": {
            "precision": "high",
            "tags": ["behavioral", "clone", "live-clone", "co-change", "x-ray"]
        }
    });

    let results: Vec<Value> = rows
        .iter()
        .map(|row| build_live_clone_result(row, repo_root))
        .collect();

    json!({
        "$schema": SARIF_SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": { "id": LIVE_CLONE_AUTOMATION_ID },
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

fn build_live_clone_result(row: &CloneCouplingRow, repo_root: &str) -> serde_json::Value {
    use serde_json::json;

    // Higher support → primary location. Ties broken alphabetically so the
    // SARIF output is deterministic across runs.
    let (
        primary_file,
        primary_entity,
        primary_start,
        primary_end,
        secondary_file,
        secondary_entity,
        secondary_start,
        secondary_end,
    ) = if (row.support_a, &row.file_a) >= (row.support_b, &row.file_b) {
        (
            &row.file_a,
            &row.entity_a,
            row.start_line_a,
            row.end_line_a,
            &row.file_b,
            &row.entity_b,
            row.start_line_b,
            row.end_line_b,
        )
    } else {
        (
            &row.file_b,
            &row.entity_b,
            row.start_line_b,
            row.end_line_b,
            &row.file_a,
            &row.entity_a,
            row.start_line_a,
            row.end_line_a,
        )
    };

    let mk_uri = |p: &str| {
        format!(
            "{}/{}",
            repo_root.trim_end_matches('/'),
            p.trim_start_matches('/')
        )
    };

    // security-severity = combined_score * 10, clamped to [0, 10] per SARIF spec.
    let security_severity = (row.combined_score * 10.0).clamp(0.0, 10.0);
    let level = if security_severity >= 7.0 {
        "error"
    } else if security_severity >= 4.0 {
        "warning"
    } else {
        "note"
    };

    // partialFingerprints — versioned keys per research brief for stable
    // cross-run identity even when family sizes fluctuate.
    let mut file_pair_hasher = Sha256::new();
    let mut pair = [row.file_a.as_str(), row.file_b.as_str()];
    pair.sort_unstable();
    file_pair_hasher.update(pair[0].as_bytes());
    file_pair_hasher.update(b"|");
    file_pair_hasher.update(pair[1].as_bytes());
    let file_pair_hash = format!("sha256:{}", hex::encode(file_pair_hasher.finalize()));

    json!({
        "ruleId": LIVE_CLONE_RULE_ID,
        "level": level,
        "message": {
            "text": format!(
                "Live clone family {} — {} ({}:{}) and {} ({}:{}) co-change at \
                 {:.0}% degree (combined_score {:.3}; similarity {:.2}, {} shared revs)",
                row.clone_group_id,
                primary_entity,
                primary_file, primary_start,
                secondary_entity,
                secondary_file, secondary_start,
                row.degree_pct * 100.0,
                row.combined_score,
                row.similarity,
                row.shared_revs,
            )
        },
        "locations": [
            {
                "physicalLocation": {
                    "artifactLocation": { "uri": mk_uri(primary_file) },
                    "region": { "startLine": primary_start, "endLine": primary_end }
                },
                "message": { "text": format!("primary: {primary_entity}") }
            },
            {
                "physicalLocation": {
                    "artifactLocation": { "uri": mk_uri(secondary_file) },
                    "region": { "startLine": secondary_start, "endLine": secondary_end }
                },
                "message": { "text": format!("partner: {secondary_entity}") }
            }
        ],
        "partialFingerprints": {
            "cloneGroupFingerprint/v1": row.fingerprint,
            "filePairHash/v1": file_pair_hash,
            "cloneGroupId/v1": format!("{}", row.clone_group_id)
        },
        "properties": {
            "security-severity": security_severity,
            "codelore/clone-group-id": row.clone_group_id,
            "codelore/similarity": row.similarity,
            "codelore/shared-revs": row.shared_revs,
            "codelore/degree-pct": row.degree_pct,
            "codelore/p-value": row.p_value,
            "codelore/combined-score": row.combined_score,
            "tags": ["behavioral", "clone", "live-clone", "co-change", "x-ray"]
        }
    })
}
