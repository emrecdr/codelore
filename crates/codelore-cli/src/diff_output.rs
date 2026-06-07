//! Plan 8 §7 T28 — output emitters for `codelore diff`.
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

use crate::diff::DiffOutput;

pub fn emit(out: &mut dyn Write, output: &DiffOutput, format: &str) -> Result<()> {
    match format {
        "text" => emit_text(out, output),
        "json" => emit_json(out, output),
        "markdown" => emit_markdown(out, output),
        "sarif" => emit_sarif(out, output),
        other => {
            anyhow::bail!("unknown --format {other:?} for diff; valid: text, json, markdown, sarif")
        }
    }
}

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

#[allow(clippy::too_many_lines)]
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
                h.path, h.hotspot_score, h.revisions, h.cognitive
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
                s.path, s.base_score, s.head_score, s.delta
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
                a.touched_file,
                a.expected_partner,
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
                c.clone_group_id, c.entity, c.function, c.start_line, c.end_line, c.node_count
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

#[allow(clippy::too_many_lines)]
fn emit_sarif(out: &mut dyn Write, output: &DiffOutput) -> Result<()> {
    // Build a SARIF document that mixes rank-entrant + score-increase
    // findings into CODELORE-HOTSPOT results with a `diff-classification`
    // property tagging each. This way the existing GitHub Code Scanning
    // hotspot rule renders the diff findings inline on the PR.
    use serde_json::json;

    let mut hotspot_results: Vec<serde_json::Value> = Vec::new();
    for h in &output.hotspots.rank_entrants {
        hotspot_results.push(json!({
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
        }));
    }
    for s in &output.hotspots.score_increased {
        hotspot_results.push(json!({
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
        }));
    }
    for c in &output.clones.new_families {
        hotspot_results.push(json!({
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
        }));
    }

    let doc = json!({
        "$schema": "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "automationDetails": { "id": "codelore/diff/run" },
            "tool": {
                "driver": {
                    "name": "codelore",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/emre/codescene",
                    "rules": [
                        {
                            "id": "CODELORE-HOTSPOT",
                            "shortDescription": { "text": "Behavioral hotspot diff" },
                            "properties": { "tags": ["behavioral", "hotspot", "pr-diff"] }
                        },
                        {
                            "id": "CODELORE-CLONE",
                            "shortDescription": { "text": "New clone family in PR" },
                            "properties": { "tags": ["behavioral", "clone", "pr-diff"] }
                        }
                    ]
                }
            },
            "results": hotspot_results
        }]
    });

    serde_json::to_writer_pretty(out, &doc).context("diff sarif")?;
    Ok(())
}
