//! Output emitters for `codelore diff`.
//!
//! Four formats:
//!   - `text`: human-readable terminal output (default)
//!   - `json`: full `DiffOutput` via serde
//!   - `markdown`: GFM tables designed for `$GITHUB_STEP_SUMMARY`
//!   - `sarif`: reuses `CODELORE-HOTSPOT` / `CODELORE-CLONE` /
//!     `CODELORE-LIVE-CLONE` from the analyze flow; results tagged with
//!     `properties.codelore/diff-classification` so CI tools can filter.

use std::io::Write;

use anyhow::{Context, Result};

use codelore_lib::cli_api::Options;
use codelore_lib::cli_api::analyses::delta_health::RiskClass;
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::output::sarif::{SARIF_SCHEMA_URL, TOOL_INFO_URI};

use crate::diff::DiffOutput;

/// Max delta-health function rows any emitter prints before truncating with
/// a "… and N more" trailer. One policy, shared by every format.
const DELTA_HEALTH_MAX_ROWS: usize = 20;

/// Number of evidence commits to include per finding in diff SARIF output.
const DIFF_EVIDENCE_N: u32 = 3;

/// Render an optional risk class for a delta-health cell (`∅` when absent).
fn risk_str(c: Option<RiskClass>) -> &'static str {
    c.map_or("\u{2205}", RiskClass::as_str)
}

/// Emit the diff output in `format`.
///
/// `db_opts` must be `Some` when `format == "sarif"` to populate evidence
/// chains; pass `None` for non-SARIF formats (ignored there). `repo_root` is
/// the user's real repository path (the `--repo` argument) — used for the
/// SARIF `primaryLocationLineHash` dedup key so it stays stable across runs and
/// matches the `check` recipe. It must NOT be the worktree path that
/// `db_opts`'s `Options` carries: that is a per-run tempdir already removed by
/// the time this runs.
pub fn emit(
    out: &mut dyn Write,
    output: &DiffOutput,
    format: &str,
    repo_root: &std::path::Path,
    db_opts: Option<(&FactsDb, &Options)>,
) -> Result<()> {
    match format {
        "text" => emit_text(out, output),
        "json" => emit_json(out, output),
        "markdown" => emit_markdown(out, output),
        "sarif" => emit_sarif(out, output, repo_root, db_opts),
        other => {
            anyhow::bail!("unknown --format {other:?} for diff; valid: text, json, markdown, sarif")
        }
    }
}

#[allow(clippy::too_many_lines)] // long but linear: renders gate/hotspot/coupling/clone sections in fixed order; splitting would scatter the section sequence
fn emit_text(out: &mut dyn Write, output: &DiffOutput) -> Result<()> {
    writeln!(
        out,
        "CodeLore diff — {} {} {} ({} files changed)",
        &output.base_sha[..8.min(output.base_sha.len())],
        if output.merge_base_used {
            "...→"
        } else {
            ".."
        },
        &output.head_sha[..8.min(output.head_sha.len())],
        output.hotspots.pr_touched_existing.len()
            + output.coupling_absences.len()
            + output.clones.pr_touched_existing.len(),
    )?;
    writeln!(out)?;

    if !output.gate_violations.is_empty() {
        writeln!(
            out,
            "✗ {} quality-gate VIOLATION(s) (from [diff] section)",
            output.gate_violations.len()
        )?;
        for v in &output.gate_violations {
            writeln!(
                out,
                "    {} actual={} threshold={}",
                v.gate, v.actual, v.threshold
            )?;
        }
        writeln!(out)?;
    }

    if let Some(dh) = &output.delta_health {
        match dh.ratio {
            Some(ratio) => writeln!(
                out,
                "Delta health: {ratio:.1}/100 — {} ({} added, {} modified, {} removed, {} files skipped)",
                dh.verdict,
                dh.counts.added,
                dh.counts.modified,
                dh.counts.removed,
                dh.counts.skipped
            )?,
            None => writeln!(
                out,
                "Delta health: {} (no analyzable code changed)",
                dh.verdict
            )?,
        }
        for f in dh.functions.iter().take(DELTA_HEALTH_MAX_ROWS) {
            writeln!(
                out,
                "    [{}] {}::{} {} \u{2192} {}{}",
                f.outcome.as_str(),
                f.path,
                f.function,
                risk_str(f.before),
                risk_str(f.after),
                if f.in_red_file { " (red file)" } else { "" },
            )?;
        }
        if dh.functions.len() > DELTA_HEALTH_MAX_ROWS {
            writeln!(
                out,
                "    \u{2026} and {} more",
                dh.functions.len() - DELTA_HEALTH_MAX_ROWS
            )?;
        }
        writeln!(out)?;
    }

    if !output.hotspots.rank_entrants.is_empty() {
        writeln!(
            out,
            "▶ {} NEW hotspot(s) entered the top-N (rank-entrant)",
            output.hotspots.rank_entrants.len()
        )?;
        for h in &output.hotspots.rank_entrants {
            writeln!(
                out,
                "    {:<60} score={:.4} revs={} cognitive={:.1}",
                h.path, h.hotspot_score, h.revisions, h.cognitive
            )?;
        }
        writeln!(out)?;
    }

    if !output.hotspots.score_increased.is_empty() {
        writeln!(
            out,
            "▶ {} hotspot(s) got WORSE (score increased)",
            output.hotspots.score_increased.len()
        )?;
        for s in &output.hotspots.score_increased {
            writeln!(
                out,
                "    {:<60} {:.4} → {:.4} (+{:.4})",
                s.path, s.base_score, s.head_score, s.delta
            )?;
        }
        writeln!(out)?;
    }

    if !output.coupling_absences.is_empty() {
        writeln!(
            out,
            "▶ {} historically-coupled file(s) NOT modified together",
            output.coupling_absences.len()
        )?;
        for a in &output.coupling_absences {
            writeln!(out, "    touched: {}", a.touched_file)?;
            writeln!(
                out,
                "    expected: {} (historical {:.0}% coupling over {} shared commits, p={:.4})",
                a.expected_partner, a.historical_coupling, a.historical_shared_revs, a.fisher_p,
            )?;
        }
        writeln!(out)?;
    }

    if !output.clones.new_families.is_empty() {
        writeln!(
            out,
            "▶ {} NEW clone family member(s) introduced",
            output.clones.new_families.len()
        )?;
        for c in &output.clones.new_families {
            writeln!(
                out,
                "    group={} {}:{}:{} ({} nodes)",
                c.clone_group_id, c.entity, c.function, c.start_line, c.node_count
            )?;
        }
        writeln!(out)?;
    }

    if !output.hotspots.pr_touched_existing.is_empty() {
        writeln!(
            out,
            "ℹ {} PR-touched file(s) are already in the top-N hotspot list",
            output.hotspots.pr_touched_existing.len()
        )?;
        for h in &output.hotspots.pr_touched_existing {
            writeln!(out, "    {:<60} score={:.4}", h.path, h.hotspot_score)?;
        }
    }

    Ok(())
}

fn emit_json(out: &mut dyn Write, output: &DiffOutput) -> Result<()> {
    serde_json::to_writer_pretty(out, output).context("diff json")?;
    Ok(())
}

#[allow(clippy::too_many_lines)] // long but linear: renders each markdown section in fixed order; splitting would break the section structure
fn emit_markdown(out: &mut dyn Write, output: &DiffOutput) -> Result<()> {
    writeln!(out, "# CodeLore PR Analysis")?;
    writeln!(out)?;
    writeln!(
        out,
        "**Range:** `{}` {} `{}`{}",
        &output.base_sha[..8.min(output.base_sha.len())],
        if output.merge_base_used {
            "...→"
        } else {
            ".."
        },
        &output.head_sha[..8.min(output.head_sha.len())],
        if output.merge_base_used {
            " (merge-base resolved)"
        } else {
            ""
        }
    )?;
    writeln!(out)?;

    if !output.gate_violations.is_empty() {
        writeln!(
            out,
            "## ❌ Quality-gate violations ({})",
            output.gate_violations.len()
        )?;
        writeln!(out)?;
        writeln!(out, "| Gate | Actual | Threshold |")?;
        writeln!(out, "|---|---:|---:|")?;
        for v in &output.gate_violations {
            writeln!(
                out,
                "| `{}` | `{}` | `{}` |",
                codelore_lib::cli_api::output::markdown::escape_md_cell(&v.gate),
                codelore_lib::cli_api::output::markdown::escape_md_cell(&v.actual),
                codelore_lib::cli_api::output::markdown::escape_md_cell(&v.threshold),
            )?;
        }
        writeln!(out)?;
    }

    if let Some(dh) = &output.delta_health {
        writeln!(out, "## Delta health")?;
        writeln!(out)?;
        match dh.ratio {
            Some(ratio) => writeln!(out, "**{ratio:.1}/100 — {}**", dh.verdict)?,
            None => writeln!(out, "**{}** — no analyzable code changed", dh.verdict)?,
        }
        writeln!(out)?;
        if !dh.functions.is_empty() {
            writeln!(out, "| Function | Before | After | Outcome | Reasons |")?;
            writeln!(out, "|---|---|---|---|---|")?;
            for f in dh.functions.iter().take(DELTA_HEALTH_MAX_ROWS) {
                writeln!(
                    out,
                    "| `{}::{}`{} | {} | {} | {} | {} |",
                    f.path,
                    f.function,
                    if f.in_red_file { " \u{1F534}" } else { "" },
                    risk_str(f.before),
                    risk_str(f.after),
                    f.outcome.as_str(),
                    codelore_lib::cli_api::output::markdown::escape_md_cell(&f.reasons.join("; ")),
                )?;
            }
            if dh.functions.len() > DELTA_HEALTH_MAX_ROWS {
                writeln!(out)?;
                writeln!(
                    out,
                    "\u{2026} and {} more",
                    dh.functions.len() - DELTA_HEALTH_MAX_ROWS
                )?;
            }
        }
        writeln!(out)?;
    }

    if !output.hotspots.rank_entrants.is_empty() {
        writeln!(
            out,
            "## ⚠️ New hotspots ({})",
            output.hotspots.rank_entrants.len()
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "Files newly entering the top-{} hotspot list:",
            output.hotspots.rank_entrants.len()
        )?;
        writeln!(out)?;
        writeln!(out, "| File | Score | Revisions | Cognitive |")?;
        writeln!(out, "|---|---:|---:|---:|")?;
        for h in &output.hotspots.rank_entrants {
            writeln!(
                out,
                "| `{}` | {:.4} | {} | {:.1} |",
                codelore_lib::cli_api::output::markdown::escape_md_cell(&h.path),
                h.hotspot_score,
                h.revisions,
                h.cognitive
            )?;
        }
        writeln!(out)?;
    }

    if !output.hotspots.score_increased.is_empty() {
        writeln!(
            out,
            "## 📈 Hotspots worsened ({})",
            output.hotspots.score_increased.len()
        )?;
        writeln!(out)?;
        writeln!(out, "| File | Base score | Head score | Δ |")?;
        writeln!(out, "|---|---:|---:|---:|")?;
        for s in &output.hotspots.score_increased {
            writeln!(
                out,
                "| `{}` | {:.4} | {:.4} | +{:.4} |",
                codelore_lib::cli_api::output::markdown::escape_md_cell(&s.path),
                s.base_score,
                s.head_score,
                s.delta
            )?;
        }
        writeln!(out)?;
    }

    if !output.coupling_absences.is_empty() {
        writeln!(
            out,
            "## 🔗 Missing co-changes ({})",
            output.coupling_absences.len()
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "Files in this PR historically change with another file that's NOT in the PR:"
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "| Touched | Expected partner | Historical coupling | Shared revs | Fisher p |"
        )?;
        writeln!(out, "|---|---|---:|---:|---:|")?;
        for a in &output.coupling_absences {
            writeln!(
                out,
                "| `{}` | `{}` | {:.1}% | {} | {:.4} |",
                codelore_lib::cli_api::output::markdown::escape_md_cell(&a.touched_file),
                codelore_lib::cli_api::output::markdown::escape_md_cell(&a.expected_partner),
                a.historical_coupling,
                a.historical_shared_revs,
                a.fisher_p,
            )?;
        }
        writeln!(out)?;
    }

    if !output.clones.new_families.is_empty() {
        writeln!(
            out,
            "## 🌱 New clones ({})",
            output.clones.new_families.len()
        )?;
        writeln!(out)?;
        writeln!(out, "| Group | Entity | Function | Lines | Nodes |")?;
        writeln!(out, "|---|---|---|---|---:|")?;
        for c in &output.clones.new_families {
            writeln!(
                out,
                "| {} | `{}` | `{}` | {}-{} | {} |",
                c.clone_group_id,
                codelore_lib::cli_api::output::markdown::escape_md_cell(&c.entity),
                codelore_lib::cli_api::output::markdown::escape_md_cell(&c.function),
                c.start_line,
                c.end_line,
                c.node_count
            )?;
        }
        writeln!(out)?;
    }

    if output.hotspots.rank_entrants.is_empty()
        && output.hotspots.score_increased.is_empty()
        && output.coupling_absences.is_empty()
        && output.clones.new_families.is_empty()
    {
        writeln!(out, "✅ No new behavioral findings.")?;
        writeln!(out)?;
    }

    Ok(())
}

#[allow(clippy::too_many_lines)] // long but linear: constructs a complete SARIF document (rules + results for hotspot/coupling/clone) in one pass; splitting would fragment the schema structure
fn emit_sarif(
    out: &mut dyn Write,
    output: &DiffOutput,
    repo_root: &std::path::Path,
    db_opts: Option<(&FactsDb, &Options)>,
) -> Result<()> {
    // Build a SARIF document that mixes rank-entrant + score-increase
    // findings into CODELORE-HOTSPOT results with a `diff-classification`
    // property tagging each. This way the existing GitHub Code Scanning
    // hotspot rule renders the diff findings inline on the PR.
    use codelore_lib::cli_api::quality_gates::evidence::evidence_for_path;
    use serde_json::json;

    // A failed evidence query degrades a result to chainless (no codeFlows /
    // relatedLocations) rather than failing the whole diff. Such a failure is
    // systemic — it repeats for every path — so warn at most once per run,
    // matching the ⚠-prefixed stderr convention used by `check`.
    let evidence_warned = std::cell::Cell::new(false);

    // Build codeFlows + relatedLocations from evidence commits for a given path.
    // Returns None when no db is available, the query fails, or no evidence was
    // found. codeFlows: SARIF §3.36 — threadFlowLocations wrap the location in
    //   {"location": …}; relatedLocations: SARIF §3.27.22 — plain location array.
    // Both point to the same commits; GitHub reads relatedLocations for the
    // inline annotation panel and codeFlows for the "Show paths" flows tab.
    let evidence_for = |path: &str| -> Option<(serde_json::Value, serde_json::Value)> {
        let (db, opts) = db_opts?;
        let commits = match evidence_for_path(db, opts, path, DIFF_EVIDENCE_N) {
            Ok(commits) => commits,
            Err(e) => {
                if !evidence_warned.replace(true) {
                    eprintln!(
                        "  ⚠ diff: evidence lookup failed ({e}); results will be emitted without commit chains"
                    );
                }
                return None;
            }
        };
        if commits.is_empty() {
            return None;
        }
        // Each emitter owns its message wording; the shared helper builds the
        // location object shape (percent-encoded uri + region) so the check and
        // diff evidence encodings stay aligned. Diff anchors evidence at line 1
        // (with_region = true) — it has no per-commit line span.
        let related: Vec<serde_json::Value> = commits
            .iter()
            .map(|ev| {
                let message = format!("{} {} {} (churn={})", ev.date, ev.rev, ev.author, ev.churn);
                codelore_lib::cli_api::output::sarif::evidence_location(path, &message, true)
            })
            .collect();
        let (code_flows, related_locations) =
            codelore_lib::cli_api::output::sarif::evidence_attachments(related);
        Some((code_flows, related_locations))
    };

    // Canonicalized repo root for the `primaryLocationLineHash` key. Derived
    // from the user's real repo path exactly as the check emitter derives it
    // (canonicalize, fall back to the raw path) so a file flagged by both
    // `check` and `diff` produces the same hash and GitHub coalesces the two
    // alerts. Must not use the worktree path in `db_opts` — that is a per-run
    // tempdir, already removed, so it would be neither stable nor check-aligned.
    let repo_root: String = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
        .to_string_lossy()
        .into_owned();

    // Attach the shared dedup fingerprints to a result, merging into any
    // `partialFingerprints` already present (e.g. the coupling-pair key).
    // `primaryLocationLineHash` mirrors the check recipe (repo_root|path);
    // `diffFinding/v1` is the stable diff-domain identity (rule|path|discriminant).
    let add_fingerprints = |result: &mut serde_json::Value, rule: &str, path: &str, disc: &str| {
        use codelore_lib::cli_api::output::sarif::{diff_finding_hash, primary_location_line_hash};
        let fps = result
            .as_object_mut()
            .expect("result is a JSON object")
            .entry("partialFingerprints")
            .or_insert_with(|| json!({}));
        let fps = fps
            .as_object_mut()
            .expect("partialFingerprints is an object");
        fps.insert(
            "diffFinding/v1".into(),
            json!(diff_finding_hash(rule, path, disc)),
        );
        fps.insert(
            "primaryLocationLineHash".into(),
            json!(primary_location_line_hash(&repo_root, path)),
        );
    };

    let mut hotspot_results: Vec<serde_json::Value> = Vec::new();
    for h in &output.hotspots.rank_entrants {
        let evidence = evidence_for(&h.path);
        let mut result = json!({
            "ruleId": "CODELORE-HOTSPOT",
            "level": "warning",
            "message": {
                "text": format!(
                    "New hotspot in PR: '{}' (score={:.3}, revisions={}, cognitive={:.1})",
                    h.path, h.hotspot_score, h.revisions, h.cognitive
                )
            },
            "locations": [{
                "physicalLocation": {
                    "artifactLocation": { "uri": h.path },
                    "region": { "startLine": 1 }
                }
            }],
            "properties": {
                "security-severity": ((100.0 - h.code_health) / 10.0).clamp(0.0, 10.0),
                "codelore/diff-classification": "rank-entrant",
                "codelore/score": h.hotspot_score,
                "tags": ["behavioral", "hotspot", "pr-diff"]
            }
        });
        add_fingerprints(&mut result, "CODELORE-HOTSPOT", &h.path, "rank-entrant");
        if let Some((code_flows, related_locs)) = evidence {
            result["codeFlows"] = code_flows;
            result["relatedLocations"] = related_locs;
        }
        hotspot_results.push(result);
    }
    for s in &output.hotspots.score_increased {
        let evidence = evidence_for(&s.path);
        let mut result = json!({
            "ruleId": "CODELORE-HOTSPOT",
            "level": "warning",
            "message": {
                "text": format!(
                    "Hotspot worsened in PR: '{}' (score {:.3} → {:.3}, Δ +{:.3})",
                    s.path, s.base_score, s.head_score, s.delta
                )
            },
            "locations": [{
                "physicalLocation": {
                    "artifactLocation": { "uri": s.path },
                    "region": { "startLine": 1 }
                }
            }],
            "properties": {
                "codelore/diff-classification": "score-increase",
                "codelore/base-score": s.base_score,
                "codelore/head-score": s.head_score,
                "codelore/score-delta": s.delta,
                "tags": ["behavioral", "hotspot", "pr-diff"]
            }
        });
        add_fingerprints(&mut result, "CODELORE-HOTSPOT", &s.path, "score-increase");
        if let Some((code_flows, related_locs)) = evidence {
            result["codeFlows"] = code_flows;
            result["relatedLocations"] = related_locs;
        }
        hotspot_results.push(result);
    }
    for c in &output.clones.new_families {
        let evidence = evidence_for(&c.entity);
        let mut result = json!({
            "ruleId": "CODELORE-CLONE",
            "level": "note",
            "message": {
                "text": format!(
                    "New clone introduced in PR: group {} '{}' ({} nodes)",
                    c.clone_group_id, c.function, c.node_count
                )
            },
            "locations": [{
                "physicalLocation": {
                    "artifactLocation": { "uri": c.entity },
                    "region": { "startLine": c.start_line, "endLine": c.end_line }
                }
            }],
            "properties": {
                "codelore/diff-classification": "new-clone-family",
                "codelore/clone-group-id": c.clone_group_id,
                "tags": ["behavioral", "clone", "pr-diff"]
            }
        });
        // The clone AST fingerprint is the stable discriminant — clone_group_id
        // is positional and can be reassigned across runs.
        add_fingerprints(&mut result, "CODELORE-CLONE", &c.entity, &c.fingerprint);
        if let Some((code_flows, related_locs)) = evidence {
            result["codeFlows"] = code_flows;
            result["relatedLocations"] = related_locs;
        }
        hotspot_results.push(result);
    }

    // delta_health degrading results: one CODELORE-DELTA-HEALTH result per
    // unique degrading path (deduplicated; evidence_for_path is file-scoped).
    if let Some(dh) = output.delta_health.as_ref() {
        use codelore_lib::cli_api::analyses::delta_health::Outcome;
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for f in &dh.functions {
            if f.outcome != Outcome::Bad {
                continue;
            }
            if seen.contains(f.path.as_str()) {
                continue;
            }
            seen.insert(&f.path);
            let evidence = evidence_for(&f.path);
            let mut result = json!({
                "ruleId": "CODELORE-DELTA-HEALTH",
                "level": "warning",
                "message": {
                    "text": format!(
                        "Function health degraded in PR: '{}' in '{}' \
                         (risk {} → {})",
                        f.function, f.path,
                        f.before.map_or("none", RiskClass::as_str),
                        f.after.map_or("none", RiskClass::as_str),
                    )
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": f.path },
                        "region": { "startLine": 1 }
                    }
                }],
                "properties": {
                    "codelore/diff-classification": "delta-health-degraded",
                    "codelore/function": f.function.clone(),
                    "tags": ["behavioral", "health", "pr-diff"]
                }
            });
            add_fingerprints(
                &mut result,
                "CODELORE-DELTA-HEALTH",
                &f.path,
                "delta-health-degraded",
            );
            if let Some((code_flows, related_locs)) = evidence {
                result["codeFlows"] = code_flows;
                result["relatedLocations"] = related_locs;
            }
            hotspot_results.push(result);
        }
    }

    // CodeScene-signature "absent change pattern": a file historically and
    // significantly coupled with the file touched in this PR was NOT
    // touched. Often indicates a forgotten test, migration, or doc update.
    for a in &output.coupling_absences {
        // Canonical fingerprint pairs files alphabetically so the same pair
        // gets the same fingerprint regardless of which side was the
        // touched_file in any given PR.
        let (lo, hi) = if a.touched_file <= a.expected_partner {
            (&a.touched_file, &a.expected_partner)
        } else {
            (&a.expected_partner, &a.touched_file)
        };
        let mut result = json!({
            "ruleId": "CODELORE-MISSING-COCHANGE",
            "level": "note",
            "message": {
                "text": format!(
                    "PR modifies '{}' but historically '{}' co-changes with it \
                     (shared_revs={}, degree={:.1}%, fisher_p={:.4}). \
                     Verify the absence is intentional.",
                    a.touched_file,
                    a.expected_partner,
                    a.historical_shared_revs,
                    a.historical_coupling,
                    a.fisher_p,
                )
            },
            "locations": [{
                "physicalLocation": {
                    "artifactLocation": { "uri": a.touched_file },
                    "region": { "startLine": 1 }
                }
            }],
            "partialFingerprints": {
                "couplingPair/v1": format!("{lo}::{hi}")
            },
            "properties": {
                "codelore/diff-classification": "missing-cochange",
                "codelore/missing-partner": a.expected_partner.clone(),
                "codelore/historical-coupling-pct": a.historical_coupling,
                "codelore/historical-shared-revs": a.historical_shared_revs,
                "codelore/fisher-p": a.fisher_p,
                "tags": ["behavioral", "coupling", "absent-change-pattern", "pr-diff"]
            }
        });
        // The canonical pair is the stable discriminant; merges alongside the
        // existing couplingPair/v1 key.
        add_fingerprints(
            &mut result,
            "CODELORE-MISSING-COCHANGE",
            &a.touched_file,
            &format!("{lo}::{hi}"),
        );
        hotspot_results.push(result);
    }

    // CODELORE-DELTA-HEALTH rule is only included in the driver when the
    // delta_health section is present (avoids declaring a rule with zero results).
    let mut rules = vec![
        json!({
            "id": "CODELORE-HOTSPOT",
            "shortDescription": { "text": "Behavioral hotspot diff" },
            "properties": { "tags": ["behavioral", "hotspot", "pr-diff"] }
        }),
        json!({
            "id": "CODELORE-CLONE",
            "shortDescription": { "text": "New clone family in PR" },
            "properties": { "tags": ["behavioral", "clone", "pr-diff"] }
        }),
        json!({
            "id": "CODELORE-MISSING-COCHANGE",
            "name": "MissingCoChange",
            "shortDescription": { "text": "Coupled file omitted from PR" },
            "fullDescription": { "text": "A file historically and significantly coupled (Fisher-significant) with the file modified in this PR was NOT modified. The 'absent change pattern' often indicates a forgotten test, migration script, or documentation update." },
            "defaultConfiguration": { "level": "note" },
            "properties": { "tags": ["behavioral", "coupling", "absent-change-pattern", "pr-diff"] }
        }),
    ];
    if output.delta_health.is_some() {
        rules.push(json!({
            "id": "CODELORE-DELTA-HEALTH",
            "shortDescription": { "text": "Function health degraded in PR" },
            "properties": { "tags": ["behavioral", "health", "pr-diff"] }
        }));
    }

    let doc = json!({
        "$schema": SARIF_SCHEMA_URL,
        "version": "2.1.0",
        "runs": [{
            "automationDetails": { "id": "codelore/diff/run" },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": TOOL_INFO_URI,
                    "rules": rules
                }
            },
            "results": hotspot_results
        }]
    });

    serde_json::to_writer_pretty(out, &doc).context("diff sarif")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{CouplingAbsence, DiffOutput};

    #[test]
    fn emit_sarif_includes_missing_cochange_rule_and_results() {
        let output = DiffOutput {
            base_sha: "deadbeef".into(),
            head_sha: "cafef00d".into(),
            merge_base_used: false,
            coupling_absences: vec![CouplingAbsence {
                touched_file: "src/auth/login.rs".into(),
                expected_partner: "src/auth/session.rs".into(),
                historical_coupling: 87.5,
                fisher_p: 0.001,
                historical_shared_revs: 12,
            }],
            ..DiffOutput::default()
        };

        let mut buf = Vec::new();
        let repo_root = std::path::Path::new("/repo");
        emit_sarif(&mut buf, &output, repo_root, None).expect("emit_sarif");
        let sarif: serde_json::Value = serde_json::from_slice(&buf).expect("valid SARIF JSON");

        // Rule must be declared in the tool driver registry.
        let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .expect("rules array present");
        assert!(
            rules.iter().any(|r| r["id"] == "CODELORE-MISSING-COCHANGE"),
            "CODELORE-MISSING-COCHANGE rule must be declared, got: {rules:?}"
        );

        // And a corresponding result must be emitted for the absence.
        let results = sarif["runs"][0]["results"]
            .as_array()
            .expect("results array present");
        let absence_results: Vec<_> = results
            .iter()
            .filter(|r| r["ruleId"] == "CODELORE-MISSING-COCHANGE")
            .collect();
        assert_eq!(
            absence_results.len(),
            1,
            "exactly one CODELORE-MISSING-COCHANGE result expected for one absence"
        );

        let r = absence_results[0];
        // Message mentions both files
        let msg = r["message"]["text"].as_str().expect("message present");
        assert!(msg.contains("src/auth/login.rs"));
        assert!(msg.contains("src/auth/session.rs"));

        // Location uses the touched file
        assert_eq!(
            r["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "src/auth/login.rs"
        );

        // Versioned partialFingerprint for stable cross-run identity, pair is
        // alphabetical regardless of which side touched_file was
        let fp = r["partialFingerprints"]["couplingPair/v1"]
            .as_str()
            .expect("fp present");
        assert_eq!(fp, "src/auth/login.rs::src/auth/session.rs");

        // The shared dedup keys are added alongside couplingPair/v1.
        let diff_fp = r["partialFingerprints"]["diffFinding/v1"]
            .as_str()
            .expect("diffFinding/v1 present");
        assert_eq!(
            diff_fp,
            codelore_lib::cli_api::output::sarif::diff_finding_hash(
                "CODELORE-MISSING-COCHANGE",
                "src/auth/login.rs",
                "src/auth/login.rs::src/auth/session.rs",
            )
        );
        // primaryLocationLineHash mirrors the check recipe sha256(repo_root|path)
        // for the touched file, using the repo_root passed to emit_sarif.
        let primary_fp = r["partialFingerprints"]["primaryLocationLineHash"]
            .as_str()
            .expect("primaryLocationLineHash present");
        assert_eq!(
            primary_fp,
            codelore_lib::cli_api::output::sarif::primary_location_line_hash(
                "/repo",
                "src/auth/login.rs",
            )
        );

        // Properties expose the missing partner + numeric thresholds for CI consumers
        assert_eq!(
            r["properties"]["codelore/diff-classification"],
            "missing-cochange"
        );
        assert_eq!(
            r["properties"]["codelore/missing-partner"],
            "src/auth/session.rs"
        );
        assert_eq!(r["properties"]["codelore/historical-shared-revs"], 12);
    }

    #[test]
    fn emit_sarif_zero_absences_zero_results() {
        // Empty input must not emit a CODELORE-MISSING-COCHANGE result
        // (the rule definition still appears in the tool driver, but
        // results section is empty for that rule).
        let output = DiffOutput::default();
        let mut buf = Vec::new();
        emit_sarif(&mut buf, &output, std::path::Path::new("/repo"), None).expect("emit_sarif");
        let sarif: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        let results = sarif["runs"][0]["results"].as_array().unwrap();
        assert_eq!(
            results
                .iter()
                .filter(|r| r["ruleId"] == "CODELORE-MISSING-COCHANGE")
                .count(),
            0
        );
    }
}
