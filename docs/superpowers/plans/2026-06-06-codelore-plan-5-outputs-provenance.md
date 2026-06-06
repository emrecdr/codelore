# CodeLore Plan 5: SARIF + Markdown + Parquet + SQLite Outputs + Provenance Manifest

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Plan 5 of 6 for v1 Spine release.** Builds on Plans 1–4.

**Goal:** Add the 5 remaining output formats (SARIF 2.1.0, JSON, Markdown for `$GITHUB_STEP_SUMMARY`, Parquet, SQLite) and emit a provenance manifest alongside every analysis. The Behavioral SARIF output is CodeLore's published wedge differentiator (no other tool emits SARIF for organizational signals; lands in GitHub Code Scanning UI alongside CodeQL alerts).

**Architecture:** New `output::sarif`, `output::json`, `output::markdown`, `output::parquet`, `output::sqlite` modules. Provenance manifest is a 1-row table + a JSON file emitted alongside every output. CLI `--format` flag accepts the 6 known formats; output detection falls through to file extension when `--output PATH` is given.

**Tech Stack additions:**
- `serde_json` (already in workspace for JSON emission)
- DuckDB native (already in workspace) for Parquet + SQLite ATTACH
- SARIF: hand-rolled JSON via serde_json structs (no sarif-rs dep — overkill)

**Out of scope:**
- MCP server mode (deferred to v1.5 per spec §8.1)
- Knowledge-graph output (v2)
- LLM-based commit classification (v2)
- AI-authorship full implementation (Plan 4 has stub)

**Definition of Done for Plan 5:**
- `codelore analyze --format sarif` emits SARIF 2.1.0 compatible JSON with hotspot findings
- `codelore analyze --format json` emits structured JSON
- `codelore analyze --format markdown` emits Markdown summary suitable for `$GITHUB_STEP_SUMMARY`
- `codelore analyze --format parquet --output file.parquet` works
- `codelore analyze --format sqlite --output file.db` works
- Every output is paired with a provenance manifest (sidecar `.provenance.json` for non-DB outputs; provenance table in DB for SQLite)
- All previous tests + Plan 5 tests pass; clippy/fmt/deny clean
- CHANGELOG + README updated

---

## §1 — Provenance manifest infrastructure (Phase 5.A)

### Task 1: Provenance manifest writer

**Files:**
- Create: `crates/codelore-lib/src/provenance/mod.rs`
- Modify: `crates/codelore-lib/src/facts/mod.rs` (add `provenance()` accessor)
- Modify: `crates/codelore-lib/src/lib.rs`
- Create: `crates/codelore-lib/tests/provenance_test.rs`

Provenance manifest documents every choice that affected the analysis: input range, thresholds, version pins, run timestamps. Per spec §3.2's provenance table + §5 differentiator.

```rust
// provenance/mod.rs
//! Provenance manifest emission.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub codelore_version: String,
    pub gix_version: String,
    pub arrow_version: String,
    pub duckdb_version: String,
    pub run_started_at: String,
    pub repo_path: String,
    pub after_date: Option<String>,
    pub before_date: Option<String>,
    pub analysis: String,
    pub min_revs: u32,
    pub min_shared_revs: u32,
    pub min_coupling_pct: u8,
    pub max_changeset_size: u32,
    pub fisher_significance: f64,
    pub include_merges: bool,
    pub age_time_now: Option<String>,
    pub merge_handling: String,
}

impl Manifest {
    pub fn capture(db: &FactsDb, opts: &Options, analysis: &str) -> Result<Self> {
        // Read existing provenance values from the DB + opts
        // ... query db.query_provenance("codelore_version") etc.
        Ok(Self {
            codelore_version: env!("CARGO_PKG_VERSION").to_string(),
            gix_version: "0.84.0".to_string(),
            arrow_version: crate::arrow_facade::ARROW_RUNTIME_VERSION.to_string(),
            duckdb_version: "1.10503.1".to_string(),
            run_started_at: format!("{}", time::OffsetDateTime::now_utc()),
            repo_path: opts.repo_path.display().to_string(),
            after_date: opts.after.map(|d| d.to_string()),
            before_date: opts.before.map(|d| d.to_string()),
            analysis: analysis.to_string(),
            min_revs: opts.min_revs,
            min_shared_revs: opts.min_shared_revs,
            min_coupling_pct: opts.min_coupling_pct,
            max_changeset_size: opts.max_changeset_size,
            fisher_significance: opts.fisher_significance,
            include_merges: opts.include_merges,
            age_time_now: opts.age_time_now.map(|d| d.to_string()),
            merge_handling: "exclude".to_string(),  // Plan 4 hardcoded; opts.include_merges flips it
        })
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| CodeLoreError::Output(format!("manifest json: {e}")))
    }
}
```

Test: capture a manifest from FactsDb after ingest, assert fields are non-empty.

Commit: `feat(lib): provenance manifest infrastructure (Manifest + capture + to_json)`.

---

## §2 — Output emitters (Phase 5.B)

### Task 2: JSON output emitter

**Files:**
- Create: `crates/codelore-lib/src/output/json.rs`
- Modify: `crates/codelore-lib/src/output/mod.rs`
- Create: `crates/codelore-lib/tests/output_json_test.rs`

For each analysis row type, derive `Serialize` (if not already) and write JSON array via `serde_json::to_writer_pretty`. Pattern:

```rust
pub fn write_hotspots_json<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    serde_json::to_writer_pretty(w, rows)
        .map_err(|e| CodeLoreError::Output(format!("json: {e}")))
}
```

Repeat for the 11 analysis row types.

Each row struct in analyses/ needs `#[derive(Serialize)]`. Add now.

Commit: `feat(lib): JSON output emitter for all 11 analyses`.

---

### Task 3: SARIF 2.1.0 emitter — THE DIFFERENTIATOR

**Files:**
- Create: `crates/codelore-lib/src/output/sarif.rs`
- Create: `crates/codelore-lib/tests/output_sarif_test.rs`

Per spec §5.4 Behavioral SARIF taxonomy. Schema docs: https://docs.oasis-open.org/sarif/sarif/v2.1.0/

Hand-roll the JSON via serde_json — no sarif-rs dep needed for our limited use.

```rust
// output/sarif.rs
use serde::Serialize;

#[derive(Serialize)]
pub struct SarifLog {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub version: String,
    pub runs: Vec<SarifRun>,
}

#[derive(Serialize)]
pub struct SarifRun {
    pub tool: SarifTool,
    #[serde(rename = "automationDetails")]
    pub automation_details: SarifAutomation,
    pub results: Vec<SarifResult>,
}

#[derive(Serialize)]
pub struct SarifTool {
    pub driver: SarifDriver,
}

#[derive(Serialize)]
pub struct SarifDriver {
    pub name: String,
    pub version: String,
    #[serde(rename = "informationUri")]
    pub information_uri: String,
    pub rules: Vec<SarifRule>,
}

#[derive(Serialize)]
pub struct SarifRule {
    pub id: String,
    pub name: String,
    #[serde(rename = "shortDescription")]
    pub short_description: SarifText,
    #[serde(rename = "helpUri")]
    pub help_uri: String,
    pub properties: SarifRuleProps,
}

#[derive(Serialize)]
pub struct SarifText {
    pub text: String,
}

#[derive(Serialize)]
pub struct SarifRuleProps {
    pub tags: Vec<String>,
}

#[derive(Serialize)]
pub struct SarifAutomation {
    pub id: String,
}

#[derive(Serialize)]
pub struct SarifResult {
    #[serde(rename = "ruleId")]
    pub rule_id: String,
    pub level: String,
    pub message: SarifText,
    pub locations: Vec<SarifLocation>,
    #[serde(rename = "partialFingerprints")]
    pub partial_fingerprints: std::collections::BTreeMap<String, String>,
    pub properties: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
pub struct SarifLocation {
    #[serde(rename = "physicalLocation")]
    pub physical_location: SarifPhysical,
}

#[derive(Serialize)]
pub struct SarifPhysical {
    #[serde(rename = "artifactLocation")]
    pub artifact_location: SarifArtifact,
}

#[derive(Serialize)]
pub struct SarifArtifact {
    pub uri: String,
}

pub fn write_hotspots_sarif<W: std::io::Write>(
    rows: &[HotspotRow],
    repo_root: &str,
    w: &mut W,
) -> Result<()> {
    let rules = vec![
        SarifRule {
            id: "CODELORE-HOTSPOT".into(),
            name: "Hotspot".into(),
            short_description: SarifText { text: "Behavioral hotspot — high revisions × complexity".into() },
            help_uri: "https://github.com/.../codelore#hotspots".into(),
            properties: SarifRuleProps { tags: vec!["behavioral".into(), "hotspot".into()] },
        },
    ];

    let results: Vec<SarifResult> = rows.iter().map(|r| {
        let mut props: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
        props.insert("tags".into(), serde_json::json!(["behavioral", "hotspot"]));
        props.insert("security-severity".into(),
            serde_json::json!(format!("{:.1}", (100.0 - r.code_health) / 10.0)));
        props.insert("codelore/revs".into(), serde_json::json!(r.revisions));
        props.insert("codelore/cognitive".into(), serde_json::json!(r.cognitive));
        props.insert("codelore/codehealth".into(), serde_json::json!(r.code_health));
        props.insert("codelore/score".into(), serde_json::json!(r.hotspot_score));

        let mut fingerprints = std::collections::BTreeMap::new();
        fingerprints.insert("primaryLocationLineHash".into(),
            format!("sha256:{}", hex::encode(sha256_str(&r.path))));

        SarifResult {
            rule_id: "CODELORE-HOTSPOT".into(),
            level: if r.hotspot_score >= 0.5 { "warning".into() } else { "note".into() },
            message: SarifText {
                text: format!("Hotspot: {} touched {} times across high-complexity code (cognitive={}, code-health={})",
                    r.path, r.revisions, r.cognitive, r.code_health),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysical {
                    artifact_location: SarifArtifact {
                        uri: format!("{}/{}", repo_root.trim_end_matches('/'), r.path),
                    },
                },
            }],
            partial_fingerprints: fingerprints,
            properties: props,
        }
    }).collect();

    let log = SarifLog {
        schema: "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0.json".into(),
        version: "2.1.0".into(),
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "codelore".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    information_uri: "https://github.com/.../codelore".into(),
                    rules,
                },
            },
            automation_details: SarifAutomation { id: "codelore/hotspots/run".into() },
            results,
        }],
    };

    serde_json::to_writer_pretty(w, &log)
        .map_err(|e| CodeLoreError::Output(format!("sarif: {e}")))
}

fn sha256_str(s: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().to_vec()
}
```

You'll need to add `sha2 = "0.10"` and `hex = "0.4"` deps to codelore-lib.

Test: emit a SARIF blob, parse it back via serde_json, assert `runs[0].results[0].ruleId == "CODELORE-HOTSPOT"`.

Plan 5 ships SARIF for `hotspots` only. Other rules (CODELORE-COUPLING, CODELORE-OWNERSHIP-RISK, CODELORE-CODE-HEALTH) defer to Plan 5.x or v1.5.

Commit: `feat(lib): SARIF 2.1.0 emitter for hotspots — Behavioral SARIF differentiator`.

---

### Task 4: Markdown output emitter

**Files:**
- Create: `crates/codelore-lib/src/output/markdown.rs`
- Create: `crates/codelore-lib/tests/output_markdown_test.rs`

Markdown output is designed for `$GITHUB_STEP_SUMMARY` (CI table summaries). Pattern:

```rust
pub fn write_hotspots_markdown<W: Write>(rows: &[HotspotRow], w: &mut W) -> Result<()> {
    writeln!(w, "# CodeLore hotspots").map_err(CodeLoreError::Io)?;
    writeln!(w).map_err(CodeLoreError::Io)?;
    writeln!(w, "| Entity | Revisions | Cognitive | Code Health | Score |").map_err(CodeLoreError::Io)?;
    writeln!(w, "|---|---|---|---|---|").map_err(CodeLoreError::Io)?;
    for row in rows {
        writeln!(w, "| `{}` | {} | {:.2} | {:.2} | {:.4} |",
            row.path, row.revisions, row.cognitive, row.code_health, row.hotspot_score)
            .map_err(CodeLoreError::Io)?;
    }
    Ok(())
}
```

Repeat for the 11 analyses.

Commit: `feat(lib): Markdown output emitter for all 11 analyses`.

---

### Task 5: Parquet + SQLite outputs via DuckDB

**Files:**
- Create: `crates/codelore-lib/src/output/parquet.rs`
- Create: `crates/codelore-lib/src/output/sqlite.rs`
- Tests

DuckDB's `COPY ... TO 'file.parquet' (FORMAT PARQUET)` does Parquet natively. Pattern:

```rust
pub fn write_revisions_parquet(db: &FactsDb, opts: &Options, path: &Path) -> Result<()> {
    let sql = format!(
        "COPY (SELECT path AS entity, COUNT(DISTINCT rev) AS n_revs
               FROM changes GROUP BY path HAVING n_revs >= {}
               ORDER BY n_revs DESC)
         TO '{}' (FORMAT PARQUET);",
        opts.min_revs,
        path.display(),
    );
    db.conn().execute(&sql, [])
        .map_err(|e| CodeLoreError::Output(format!("parquet: {e}")))?;
    Ok(())
}
```

SQLite via DuckDB ATTACH:

```rust
pub fn write_analysis_sqlite(db: &FactsDb, _opts: &Options, path: &Path) -> Result<()> {
    let sql = format!(
        "ATTACH '{}' AS sink (TYPE SQLITE);
         CREATE TABLE sink.commits AS SELECT * FROM commits;
         CREATE TABLE sink.changes AS SELECT * FROM changes;
         CREATE TABLE sink.entities AS SELECT * FROM entities;
         CREATE TABLE sink.complexity_metrics AS SELECT * FROM complexity_metrics;
         CREATE TABLE sink.provenance AS SELECT * FROM provenance;
         DETACH sink;",
        path.display(),
    );
    db.conn().execute_batch(&sql)
        .map_err(|e| CodeLoreError::Output(format!("sqlite: {e}")))?;
    Ok(())
}
```

Test: write a Parquet file, read it back via DuckDB `SELECT * FROM 'file.parquet'`, assert row count.

Commit: `feat(lib): Parquet + SQLite output via DuckDB COPY + ATTACH`.

---

## §3 — CLI dispatch (Phase 5.C)

### Task 6: Wire all formats into CLI

**Files:**
- Modify: `crates/codelore-cli/src/main.rs`
- Modify: `crates/codelore-cli/src/args.rs` (already accepts --format as String)
- Modify: `crates/codelore-cli/tests/cli_test.rs`

Update `analyze()` to dispatch on `(analysis, format)` pair. For Plan 5, support: csv, json, sarif, markdown, parquet, sqlite. Add validation: parquet and sqlite require `--output PATH`.

For each format × analysis combination, call the matching emitter. There's no need to ship every combination right away; Plan 5 ships:
- csv: all 11 analyses (already in Plan 4)
- json: all 11 analyses
- sarif: hotspots only (Plan 5 scope; Plan 6 may add coupling SARIF)
- markdown: all 11 analyses
- parquet: hotspots, revisions, summary (others can be added later)
- sqlite: all (dumps the whole DB)

Add a few CLI tests:

```rust
#[test]
fn analyze_hotspots_emits_sarif() {
    let tiny = codelore_lib::test_support::tiny_repo::build();
    Command::cargo_bin("codelore")
        .unwrap()
        .args([
            "analyze", "--analysis", "hotspots",
            "--repo", tiny.dir.path().to_str().unwrap(),
            "--format", "sarif",
            "--min-revs", "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("CODELORE-HOTSPOT"))
        .stdout(predicate::str::contains("schemastore.azurewebsites.net"));
}
```

Commit: `feat(cli): dispatch all output formats (csv | json | sarif | markdown | parquet | sqlite)`.

---

## §4 — Provenance sidecar (Phase 5.D)

### Task 7: Emit provenance sidecar alongside every analysis output

**Files:**
- Modify: `crates/codelore-cli/src/main.rs`

When `--output PATH` is set, after writing the analysis output, also write `PATH.provenance.json` containing the captured `Manifest` JSON.

When output goes to stdout (no `--output` flag), skip the sidecar (or print to stderr — your choice).

For SQLite output, the provenance table is already in the DB (replicated via ATTACH); no sidecar needed.

Test: assert sidecar exists after `codelore analyze --output /tmp/x.csv` and contains expected fields.

Commit: `feat(cli): emit provenance manifest sidecar alongside file outputs`.

---

## §5 — Docs (Phase 5.E)

### Task 8: CHANGELOG + README

Update CHANGELOG with Plan 5 section (5 new output formats + provenance manifest). README shows usage examples for sarif, parquet, sqlite. Mark Plan 5 ✅ in roadmap.

Commit: `docs: CHANGELOG + README for Plan 5 output formats + provenance`.

---

## Plan 5 Definition of Done

- [ ] `codelore analyze --format sarif` emits SARIF 2.1.0 with hotspot findings
- [ ] `codelore analyze --format json` emits structured JSON
- [ ] `codelore analyze --format markdown` emits Markdown summary
- [ ] `codelore analyze --format parquet --output file.parquet` works
- [ ] `codelore analyze --format sqlite --output file.db` works
- [ ] Provenance manifest sidecar for non-DB outputs
- [ ] All previous tests + Plan 5 tests pass
- [ ] clippy/fmt/deny clean
- [ ] CHANGELOG + README updated

After Plan 5: author **Plan 6** (differential testing + perf benchmarks + release infra). After Plan 6: v1.0 ships.

---

*End of Plan 5.*
