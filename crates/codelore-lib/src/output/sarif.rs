//! SARIF 2.1.0 emitter for hotspot results — Behavioral SARIF taxonomy (spec §5.4).
//!
//! Rule `CODELORE-HOTSPOT`: properties.tags includes "behavioral" and "hotspot"
//! security-severity proxy: `(100 − cognitive_health) / 10`
//! partialFingerprints for stable identity across CI runs.

use crate::Result;
use crate::analyses::hotspots::HotspotRow;
use crate::hashing::sha256_prefixed;
use crate::quality_gates::GateViolation;
use crate::quality_gates::evidence::EvidenceCommit;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

/// SARIF 2.1.0 JSON schema URL. All emitters (including the diff emitter
/// in `codelore-cli`) reference this constant so that a future URL change
/// only needs to be made in one place.
pub const SARIF_SCHEMA_URL: &str = "https://json.schemastore.org/sarif-2.1.0.json";

/// Canonical project homepage. Surfaces in every SARIF report's
/// `tool.driver.informationUri` — GitHub Code Scanning links the
/// driver name in the tool-details panel here. Shared with the diff
/// SARIF emitter via `codelore_lib::output::sarif::TOOL_INFO_URI`.
pub const TOOL_INFO_URI: &str = "https://github.com/emrecdr/codelore";

const RULE_ID: &str = "CODELORE-HOTSPOT";
const AUTOMATION_ID_PREFIX: &str = "codelore/hotspots/run";

/// `helpUri` for `CODELORE-CLONE` / `CODELORE-LIVE-CLONE` rules.
/// `docs/research-foundations.md` carries the publishable rationale
/// for every behavioural analysis (Tornhill 2018, Kamei 2013, etc.);
/// the per-rule anchor mirrors the rule id in lowercase.
const CODELORE_RESEARCH_FOUNDATIONS_URL: &str =
    "https://github.com/emrecdr/codelore/blob/main/docs/research-foundations.md";

/// Per-RFC-3986 §2.3 "unreserved characters" set — alphanumeric plus
/// `-` / `.` / `_` / `~`. Everything else in a path segment is
/// percent-encoded so SARIF artifact URIs satisfy §4.1 (Code Scanning
/// rejects malformed URIs at upload time). We deliberately also
/// encode `/`-adjacent characters that are legal in URI paths but
/// confuse downstream tooling (e.g. `?`, `#`, `[`, `]`, `<`, `>`,
/// quotes, whitespace).
const URI_PATH_ENCODE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Encode a single path segment per [`URI_PATH_ENCODE`]. `/` is NOT
/// in the encode set so path separators pass through unchanged.
pub(super) fn percent_encode_path(p: &str) -> String {
    utf8_percent_encode(p, URI_PATH_ENCODE).to_string()
}

/// The `partialFingerprints.primaryLocationLineHash` value for a finding at
/// `path` under `repo_root`: `sha256("<repo_root>|<path>")`.
///
/// GitHub Code Scanning deduplicates alerts across uploads on this key, so
/// every emitter must compute it identically. Sharing one function keeps the
/// check emitter and the `codelore diff` emitter aligned: the same `repo_root`
/// and repo-relative `path` produce the same hash, so a file flagged by both
/// `check` and `diff` collapses to one alert. `path` is used raw (not
/// percent-encoded) — the URI encoding is a display concern, kept out of the
/// identity key so it stays stable if the encode set ever changes.
#[must_use]
pub fn primary_location_line_hash(repo_root: &str, path: &str) -> String {
    sha256_prefixed(&[repo_root, path])
}

/// The `partialFingerprints.diffFinding/v1` value for a `codelore diff`
/// result: `sha256("<rule>|<path>|<discriminant>")`.
///
/// The diff-domain identity key. Its parts are all stable across re-runs of the
/// same diff — the rule id, the repo-relative path, and a per-finding
/// discriminant (the diff classification, or the specific function/clone that
/// makes two results on one path distinct). It deliberately omits base/head
/// SHAs and numeric scores so the same finding keeps its identity as a PR's
/// commits and metrics move, letting GitHub coalesce re-uploads of an unchanged
/// finding.
#[must_use]
pub fn diff_finding_hash(rule: &str, path: &str, discriminant: &str) -> String {
    sha256_prefixed(&[rule, path, discriminant])
}

/// Per-run correlation suffix for SARIF `automationDetails.id`. SARIF
/// 2.1.0 §3.17.3 wants `<runGroupName>/<runName>/<correlationGuid>`;
/// without a per-run suffix, GitHub Code Scanning collapses every run
/// sharing the static id into a single timeline, defeating the
/// `partialFingerprints` work that the row-level emitters do.
///
/// We derive the suffix from system time (nanos-since-epoch, hashed
/// to 16 hex chars) instead of pulling in a `uuid` dep — uniqueness
/// per run is all that's required; cryptographic randomness isn't.
fn run_correlation_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    // Mix in the process id so two SARIF emissions in different
    // processes that happen at the same nanosecond (CI parallel
    // jobs on the same host) don't collide.
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

fn automation_id_for(prefix: &str) -> String {
    format!("{prefix}/{}", run_correlation_id())
}

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
    serde_json::to_writer_pretty(w, &doc).map_err(|e| super::serde_json_io_err("sarif", &e))?;
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
        "$schema": SARIF_SCHEMA_URL,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": {
                "id": automation_id_for(AUTOMATION_ID_PREFIX)
            },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": TOOL_INFO_URI,
                    "rules": [rule]
                }
            },
            "results": results
        }]
    })
}

fn build_result(row: &HotspotRow, repo_root: &str) -> serde_json::Value {
    use serde_json::json;

    // security-severity proxy: (100 - cognitive_health) / 10  (range 0.0–10.0)
    let security_severity = (100.0 - row.cognitive_health) / 10.0;

    // SARIF `level` derived from the same security-severity scale that
    // populates `properties.security-severity`. One source of truth —
    // mirrors build_live_clone_result. Was previously a separate threshold
    // on row.hotspot_score (a percentile-rank value bound to the current
    // run's repo), which produced inconsistent rendering: a small repo
    // where the top hotspot scored 0.4 emitted zero "warning" findings, a
    // larger repo emitted many. Severity-bands are absolute and align with
    // how SARIF consumers actually grade results.
    let level = if security_severity >= 7.0 {
        "error"
    } else if security_severity >= 4.0 {
        "warning"
    } else {
        "note"
    };

    // Stable fingerprint: sha256 of "<repo_root>|<path>"
    let fp = sha256_prefixed(&[repo_root, &row.path]);

    // Artifact URI: repo_root + "/" + percent-encoded path. The path
    // half is percent-encoded per RFC 3986 §4.1; GitHub Code Scanning
    // rejects malformed URIs at upload time and silently truncates at
    // `#` so inline annotations land on the wrong file.
    let artifact_uri = format!(
        "{}/{}",
        repo_root.trim_end_matches('/'),
        percent_encode_path(row.path.trim_start_matches('/'))
    );

    // Build the message text and properties bag with MI / band included only
    // when known. SARIF consumers (GitHub Code Scanning) prefer omitting
    // absent properties over surfacing `null` cells in their UI.
    let band_str = match row.mi_rank {
        Some(rank) if rank.is_finite() => {
            Some(crate::analyses::mi::MiBand::from_rank(rank).as_str())
        }
        _ => None,
    };
    let message_text = match (row.mi, band_str) {
        (Some(v), Some(band)) => format!(
            "Hotspot '{}': score={:.3}, cognitive_health={:.1}, revisions={}, cognitive={:.1}, mi={:.1} ({})",
            row.path,
            row.hotspot_score,
            row.cognitive_health,
            row.revisions,
            row.cognitive,
            v,
            band
        ),
        (Some(v), None) => format!(
            "Hotspot '{}': score={:.3}, cognitive_health={:.1}, revisions={}, cognitive={:.1}, mi={:.1}",
            row.path, row.hotspot_score, row.cognitive_health, row.revisions, row.cognitive, v
        ),
        _ => format!(
            "Hotspot '{}': score={:.3}, cognitive_health={:.1}, revisions={}, cognitive={:.1}",
            row.path, row.hotspot_score, row.cognitive_health, row.revisions, row.cognitive
        ),
    };

    let mut properties = json!({
        "security-severity": security_severity,
        "codelore/revs": row.revisions,
        "codelore/cognitive": row.cognitive,
        "codelore/cognitivehealth": row.cognitive_health,
        "codelore/score": row.hotspot_score,
        "tags": ["behavioral", "hotspot"]
    });
    if let Some(mi) = row.mi {
        properties["codelore/mi"] = json!(mi);
    }
    if let Some(rank) = row.mi_rank
        && rank.is_finite()
    {
        properties["codelore/mi-rank"] = json!(rank);
    }
    if let Some(band) = band_str {
        properties["codelore/mi-band"] = json!(band);
    }
    if let Some(pct) = row.ai_pct
        && pct.is_finite()
    {
        properties["codelore/ai-pct"] = json!(pct);
    }

    json!({
        "ruleId": RULE_ID,
        "level": level,
        "message": { "text": message_text },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": artifact_uri }
            }
        }],
        "partialFingerprints": {
            "primaryLocationLineHash": fp
        },
        "properties": properties
    })
}

// =============================================================================
// CODELORE-CLONE
//
// Plain clone-detection findings (the live-clone variant is
// CODELORE-LIVE-CLONE). One SARIF result per clone family with
// `locations[]` listing every family member (one physicalLocation per file +
// line range). `partialFingerprints.cloneGroupFingerprint/v1` = the AST
// digest from the clones row, so results are stable across CI runs even when
// family sizes fluctuate.
// =============================================================================

use crate::analyses::clones::ClonesRow;

const CLONE_RULE_ID: &str = "CODELORE-CLONE";
const CLONE_AUTOMATION_ID_PREFIX: &str = "codelore/clones/run";

/// Emit a SARIF 2.1.0 document for clone families to `w`.
///
/// Members of the same `clone_group_id` are aggregated into a single SARIF
/// `result` with multiple `locations[]`. Severity scales mildly with family
/// size (more copies = more drift risk) but caps at 6 — these are noisier
/// than hotspots, and live-clones (CODELORE-LIVE-CLONE) carry
/// the higher severity.
pub fn write_clones_sarif<W: Write>(rows: &[ClonesRow], repo_root: &str, w: &mut W) -> Result<()> {
    let doc = build_clones_sarif(rows, repo_root);
    serde_json::to_writer_pretty(w, &doc)
        .map_err(|e| super::serde_json_io_err("clones sarif", &e))?;
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
        "helpUri": CODELORE_RESEARCH_FOUNDATIONS_URL,
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
        "$schema": SARIF_SCHEMA_URL,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": { "id": automation_id_for(CLONE_AUTOMATION_ID_PREFIX) },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": TOOL_INFO_URI,
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
    // carry the higher severity. usize → f64 cast is safe
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
                percent_encode_path(m.entity.trim_start_matches('/'))
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
            // Versioned key per the research brief.
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
// CODELORE-LIVE-CLONE
//
// The high-severity intersection of clones × Fisher-significant change-coupling.
// One SARIF result per (clone_group_id, file_a, file_b) — see research brief
// a0a6cf3534a65a643. Designed to surface inline in GitHub Code Scanning on PRs
// that touch any participating file.
// =============================================================================

use crate::analyses::clone_coupling::CloneCouplingRow;

const LIVE_CLONE_RULE_ID: &str = "CODELORE-LIVE-CLONE";
const LIVE_CLONE_AUTOMATION_ID_PREFIX: &str = "codelore/clone-coupling/run";

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
        .map_err(|e| super::serde_json_io_err("clone-coupling sarif", &e))?;
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
        "helpUri": CODELORE_RESEARCH_FOUNDATIONS_URL,
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
        "$schema": SARIF_SCHEMA_URL,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": { "id": automation_id_for(LIVE_CLONE_AUTOMATION_ID_PREFIX) },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": TOOL_INFO_URI,
                    "rules": [rule]
                }
            },
            "results": results
        }]
    })
}

#[allow(clippy::too_many_lines)] // long but linear JSON construction for a live-clone SARIF result; splitting would fragment the primary/secondary location pairing
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
            percent_encode_path(p.trim_start_matches('/'))
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
    let mut pair = [row.file_a.as_str(), row.file_b.as_str()];
    pair.sort_unstable();
    let file_pair_hash = sha256_prefixed(&pair);

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

// =============================================================================
// CODELORE CHECK — quality-gate SARIF with evidence chains
//
// One rule per distinct gate name; one result per GateViolation.
// Per-file violations get relatedLocations + a codeFlow (≤5 evidence commits).
// Repo-wide violations (path == "(repo-wide)" or "(degraded)") use uri "."
// with no region.
// =============================================================================

const CHECK_AUTOMATION_ID_PREFIX: &str = "codelore/check/run";

/// Emit a SARIF 2.1.0 document for quality-gate violations to `w`.
///
/// `evidence` maps a repo-relative path to its top-N [`EvidenceCommit`]s;
/// it is only populated for per-file violated paths (never for `"(repo-wide)"`
/// or `"(degraded)"`).  `repo_root` and `head_sha` are used for stable
/// `partialFingerprints`.
///
/// # Errors
///
/// Returns [`crate::CodeLoreError::Io`] when the output sink fails mid-write
/// (e.g. a reader closed the pipe early) and [`crate::CodeLoreError::Output`]
/// on a genuine JSON serialization fault.
pub fn write_check_sarif<W: Write, S: std::hash::BuildHasher>(
    violations: &[GateViolation],
    evidence: &HashMap<String, Vec<EvidenceCommit>, S>,
    repo_root: &Path,
    head_sha: &str,
    w: &mut W,
) -> Result<()> {
    let doc = build_check_sarif(violations, evidence, repo_root, head_sha);
    serde_json::to_writer_pretty(w, &doc)
        .map_err(|e| super::serde_json_io_err("check sarif", &e))?;
    Ok(())
}

fn build_check_sarif<S: std::hash::BuildHasher>(
    violations: &[GateViolation],
    evidence: &HashMap<String, Vec<EvidenceCommit>, S>,
    repo_root: &Path,
    head_sha: &str,
) -> serde_json::Value {
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    // One ReportingDescriptor per distinct gate (deduped, stable order).
    let distinct_gates: BTreeSet<&str> = violations.iter().map(|v| v.gate.as_str()).collect();
    let rules: Vec<Value> = distinct_gates
        .iter()
        .map(|gate| {
            json!({
                "id": gate,
                "shortDescription": {
                    "text": format!("Quality gate: {gate}")
                },
                "helpUri": "https://github.com/emrecdr/codelore/blob/main/docs/advanced-usage.md#check-gates"
            })
        })
        .collect();

    let repo_root_str = repo_root.to_string_lossy();
    let results: Vec<Value> = violations
        .iter()
        .map(|v| build_check_result(v, evidence, &repo_root_str, head_sha))
        .collect();

    json!({
        "$schema": SARIF_SCHEMA_URL,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": {
                "id": automation_id_for(CHECK_AUTOMATION_ID_PREFIX)
            },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": TOOL_INFO_URI,
                    "rules": rules
                }
            },
            "results": results
        }]
    })
}

/// Wrap pre-built evidence location objects into the two SARIF attachment
/// shapes GitHub reads: `codeFlows` (each location wrapped in `{"location":
/// …}` inside a single threadFlow) and `relatedLocations` (the plain location
/// array). Returns `(codeFlows, relatedLocations)`.
///
/// Callers build their own location objects — each emitter owns its message
/// format and uri handling; this helper only supplies the shared structural
/// wrapping so the check and diff emitters stay in sync. `codeFlows`:
/// SARIF §3.36; `relatedLocations`: SARIF §3.27.22.
#[must_use]
pub fn evidence_attachments(
    locs: Vec<serde_json::Value>,
) -> (serde_json::Value, serde_json::Value) {
    use serde_json::json;

    let thread_flow_locs: Vec<serde_json::Value> =
        locs.iter().map(|loc| json!({ "location": loc })).collect();
    let code_flows = json!([{ "threadFlows": [{ "locations": thread_flow_locs }] }]);
    (code_flows, serde_json::Value::Array(locs))
}

/// Build one evidence-commit location object: a `physicalLocation` whose
/// `artifactLocation.uri` is the percent-encoded `path` (leading `/` trimmed,
/// per RFC 3986 §4.1 so Code Scanning accepts the URI), plus the caller's
/// `message`. When `with_region` is set the location carries `region.startLine
/// = 1` — the diff emitter anchors evidence at line 1 because it has no
/// per-commit line span; the check emitter omits the region entirely.
///
/// Both the check and diff SARIF emitters own their (deliberately different)
/// message wording and call this helper only for the shared object shape, so
/// the two evidence encodings can never silently drift apart.
#[must_use]
pub fn evidence_location(path: &str, message: &str, with_region: bool) -> serde_json::Value {
    use serde_json::json;

    let uri = percent_encode_path(path.trim_start_matches('/'));
    let physical = if with_region {
        json!({
            "artifactLocation": { "uri": uri },
            "region": { "startLine": 1 }
        })
    } else {
        json!({ "artifactLocation": { "uri": uri } })
    };
    json!({
        "physicalLocation": physical,
        "message": { "text": message }
    })
}

/// Build one SARIF result for a single [`GateViolation`].
fn build_check_result<S: std::hash::BuildHasher>(
    v: &GateViolation,
    evidence: &HashMap<String, Vec<EvidenceCommit>, S>,
    repo_root: &str,
    head_sha: &str,
) -> serde_json::Value {
    use serde_json::json;

    // Repo-wide gates (e.g. clone count) emit a synthetic "." uri with no
    // region. Per-file gates emit the real path.
    let is_repo_wide = v.path == "(repo-wide)" || v.path == "(degraded)";
    let uri = if is_repo_wide {
        ".".to_owned()
    } else {
        percent_encode_path(v.path.trim_start_matches('/'))
    };

    let location = json!({ "physicalLocation": { "artifactLocation": { "uri": uri } } });

    // Stable fingerprints:
    // 1. gateFinding/v1 — our versioned key; includes head_sha so it
    //    differentiates the same gate breached at two different commits.
    // 2. primaryLocationLineHash — the same repo_root|path shape the hotspots
    //    emitter uses; GitHub deduplicates on this key.
    let gate_fp = sha256_prefixed(&[&v.gate, &v.path, head_sha]);
    let primary_fp = primary_location_line_hash(repo_root, &v.path);

    let message_text = format!(
        "{gate}: {actual} vs threshold {threshold}",
        gate = v.gate,
        actual = v.actual,
        threshold = v.threshold,
    );

    // Evidence chain — only for per-file violations.
    let chain: &[EvidenceCommit] = if is_repo_wide {
        &[]
    } else {
        evidence.get(&v.path).map_or(&[], Vec::as_slice)
    };

    let evidence_locs: Vec<serde_json::Value> = chain
        .iter()
        .map(|c| {
            let message = format!(
                "{date} {author}: {msg} (+{churn} lines)",
                date = c.date,
                author = c.author,
                msg = c.message_head,
                churn = c.churn,
            );
            // Check evidence carries no region — a file-level anchor is all the
            // gate has; the diff emitter passes `true` to anchor at line 1.
            evidence_location(&v.path, &message, false)
        })
        .collect();

    let mut result = json!({
        "ruleId": v.gate,
        "level": "error",
        "message": { "text": message_text },
        "locations": [location],
        "partialFingerprints": {
            "gateFinding/v1": gate_fp,
            "primaryLocationLineHash": primary_fp
        }
    });

    if !evidence_locs.is_empty() {
        let (code_flows, related_locations) = evidence_attachments(evidence_locs);
        result["relatedLocations"] = related_locations;
        result["codeFlows"] = code_flows;
    }

    result
}
