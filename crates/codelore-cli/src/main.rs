//! codelore — Behavioral Code Analyzer CLI.

mod analyze;
mod args;
mod calibrate;
mod calibrate_defects;
mod check;
mod diff;
mod diff_output;
mod explain;
mod gate;
mod mcp;

use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::cli_api::{AnalysisName, CodeLoreError, Options};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::args::{Cli, Command, DiffArgs, IngestSarifArgs, McpArgs};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        // Map CodeLoreError to its spec §6.6 exit code if present in the chain.
        // Falls back to 1 for non-CodeLoreError errors (e.g. clap parse errors).
        let code = e
            .chain()
            .find_map(|cause| cause.downcast_ref::<codelore_lib::cli_api::CodeLoreError>())
            .map_or(1, codelore_lib::cli_api::CodeLoreError::exit_code);
        std::process::exit(code);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Analyze(args) => analyze::analyze(&args, cli.no_banner),
        Command::Diff(args) => run_diff_cmd(&args),
        Command::Completions(args) => {
            run_completions_cmd(&args);
            Ok(())
        }
        Command::Explain(args) => explain::run_explain_cmd(&args),
        Command::Schema(args) => run_schema_cmd(&args),
        Command::Profile => run_profile_cmd(),
        Command::Docs => run_docs_cmd(),
        Command::Check(args) => check::run_check_cmd(&args),
        Command::Gate(args) => gate::run_gate_cmd(&args),
        Command::Mcp(args) => run_mcp_cmd(&args),
        Command::IngestSarif(args) => run_ingest_sarif_cmd(&args),
        Command::Calibrate(args) => calibrate::run_calibrate_cmd(&args),
        Command::CalibrateDefects(args) => calibrate_defects::run_calibrate_defects_cmd(&args),
    }
}

fn run_mcp_cmd(args: &McpArgs) -> Result<()> {
    mcp::run_mcp_server(
        args.repo.clone(),
        args.defect_calibration.clone(),
        args.allow_foreign_calibration,
    )
}

/// Ingest one or more SARIF files into the per-repo external-findings sidecar.
///
/// For each file, parses all SARIF runs, groups findings by engine, and
/// calls `ExternalStore::replace_engine` per engine so re-ingest is
/// idempotent. Prints a summary line to stdout on success.
fn run_ingest_sarif_cmd(args: &IngestSarifArgs) -> Result<()> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::external::{
        ExternalStore, group_findings_by_engine, parse_sarif_with_engines,
    };

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);

    let store = ExternalStore::open_or_create(&cache_root, &args.repo)
        .context("open external findings store")?;

    // Parse all input files into a flat vec, then group by engine so that
    // two SARIF files from the same engine are combined rather than
    // overwriting each other. Track every engine name present across all
    // inputs — including runs that produced zero results — so a clean re-scan
    // clears the engine's stale rows instead of leaving them behind.
    let mut all_findings = Vec::new();
    let mut all_engines: Vec<String> = Vec::new();
    for path in &args.file {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read SARIF file {}", path.display()))?;
        let (findings, engines) = parse_sarif_with_engines(&raw)
            .with_context(|| format!("parse SARIF file {}", path.display()))?;
        all_findings.extend(findings);
        for engine in engines {
            if !all_engines.contains(&engine) {
                all_engines.push(engine);
            }
        }
    }
    let mut by_engine = group_findings_by_engine(all_findings);
    // Seed empty batches for engines that ran but flagged nothing this pass.
    // `replace_engine` with an empty slice deletes that engine's prior rows,
    // keeping the stored count aligned with the current scanner run.
    for engine in all_engines {
        by_engine.entry(engine).or_default();
    }

    let engine_count = by_engine.len();
    let mut total_ingested: usize = 0;
    for (engine, findings) in &by_engine {
        let n = store
            .replace_engine(engine, findings)
            .with_context(|| format!("ingest findings for engine {engine}"))?;
        total_ingested += n;
    }

    println!(
        "ingested {} finding(s) from {} engine(s) → {}",
        total_ingested,
        engine_count,
        store.path().display(),
    );

    Ok(())
}

/// Print the one-per-run notice that the corpus-percentile lens is inactive:
/// no `--calibration` artifact was passed and no world corpus is embedded, so
/// code-health rows carry no `corpus_percentile`. Called exactly once on each
/// code-health-producing path, so the "deduped per run" contract is structural.
/// Suppressed under `quiet` and when stderr is not a TTY (so redirected /
/// CI output stays clean), mirroring the pre-flight banner's print policy.
pub(crate) fn notice_corpus_lens_absent(opts: &Options, quiet: bool) {
    use std::io::IsTerminal as _;
    if quiet || !std::io::stderr().is_terminal() {
        return;
    }
    let embedded_absent = codelore_lib::cli_api::calibration::embedded_world().is_none();
    if opts.calibration.is_none() && embedded_absent {
        eprintln!(
            "note: corpus-percentile lens inactive — no calibration artifact (pass --calibration <path> or build one with `codelore calibrate`)."
        );
    }
}

/// Number of delta rows the text render shows; the rest fold into a
/// `(+n more files)` tail. The JSON document always carries every row.
pub(crate) const GATE_DELTA_TABLE_ROWS: usize = 10;

/// Number of advisory-finding rows the text render shows; the rest fold into
/// a `(+n more findings)` tail. Mirrors [`GATE_DELTA_TABLE_ROWS`]'s
/// render-only cap — `report.findings` (the JSON document, and the in-memory
/// `ChangeSetReport`) always carries every finding by design (spec §6); only
/// the rendered text is bounded, so a large coupling cluster or a big batch
/// of added files can never blow the token budget.
pub(crate) const GATE_FINDINGS_ROWS: usize = 10;

/// Write a single `key=value` line to `$GITHUB_OUTPUT` when the env
/// var is set. No-op outside GitHub Actions.
pub(crate) fn write_github_output(key: &str, value: &str) {
    if let Ok(path) = std::env::var("GITHUB_OUTPUT")
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{key}={value}");
    }
}

/// Operational telemetry. Prints what `CodeLore` ships under the
/// hood — schema version, pinned dependency versions, supported
/// analysis count, supported output format count. Useful for triage
/// when behaviour surprises a user.
#[allow(clippy::unnecessary_wraps)] // dispatcher uniformity — every arm returns Result<()>
fn run_profile_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    println!("# CodeLore profile\n");
    println!("**Version**: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "**Schema**: schema_v{} (`facts/schema_v1.sql`)",
        codelore_lib::cli_api::facts::schema::CURRENT_SCHEMA_VERSION
    );
    println!("**Analyses**: {} registered", AnalysisName::all().len());
    println!("**Output formats**: csv | json | sarif | markdown | parquet | sqlite | html | spa");
    println!(
        "**Pinned third-party**:\n  - gix {gix}\n  - DuckDB {duckdb}\n  - tree-sitter 0.25.x (Rust/Python/Java/JS/TS/TSX/C++)",
        gix = codelore_lib::cli_api::provenance::GIX_VERSION,
        duckdb = codelore_lib::cli_api::provenance::DUCKDB_VERSION,
    );
    println!("\n**Cache root**:");
    if let Some(dir) = dirs::cache_dir() {
        println!("  {}/codelore/", dir.display());
    } else {
        println!("  <unavailable on this platform>");
    }
    println!(
        "\n**SPA feature**: {}",
        if cfg!(feature = "spa") {
            "ENABLED"
        } else {
            "disabled (build with --features spa to opt in)"
        }
    );
    println!("\n_For per-analysis SQL + citations, run `codelore explain <topic>`._");
    Ok(())
}

/// Markdown dump of every supported analysis. Seeds the planned
/// full static-HTML doc site.
#[allow(clippy::unnecessary_wraps)] // dispatcher uniformity — every arm returns Result<()>
fn run_docs_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    println!("# CodeLore — Analysis catalogue\n");
    println!(
        "Auto-generated from `AnalysisName::all()`. Run `codelore explain <topic>` for per-analysis citations and formulas. The full citation chain lives in `docs/research-foundations.md`.\n"
    );
    println!("## Supported analyses\n");
    for analysis in AnalysisName::all() {
        println!("- `{}`", analysis.as_str());
    }
    println!("\n## Output formats\n");
    for fmt in &[
        ("csv", "code-maat-compatible flat tables"),
        ("json", "stable JSON shape per row type"),
        (
            "ndjson",
            "newline-delimited JSON — one row per line for stream consumers (LSP, `jq -c`, CI pipelines)",
        ),
        ("sarif", "SARIF 2.1.0 — surfaces in GitHub Code Scanning"),
        ("markdown", "GFM tables for `$GITHUB_STEP_SUMMARY`"),
        (
            "gha",
            "GitHub Actions workflow commands — `::error::` / `::warning::` / `::notice::` on stdout, surfaced as inline PR annotations",
        ),
        ("parquet", "columnar bulk export for analytical pipelines"),
        ("sqlite", "full DuckDB fact-store dump"),
        ("html", "self-contained per-analysis HTML report"),
        (
            "spa",
            "single-file interactive dashboard (opt-in via `spa` feature)",
        ),
    ] {
        println!("- `{}` — {}", fmt.0, fmt.1);
    }
    println!("\n## Conventions\n");
    println!("- Files alive at HEAD only (deleted files excluded from path-aggregating analyses)");
    println!("- Mailmap + `.codelore-teams` + `.codelorebots` consulted at ingest time");
    println!("- `.gitignore` / `.codeloreignore` honoured");
    println!("- `--time-bucket` supported on: hotspots, coupling, soc, code-health");
    println!(
        "\n## Reproducibility\n\nEvery file output is paired with a `.provenance.json` sidecar capturing the run's full `Options` shape. SQLite outputs embed the equivalent inside the `provenance` table."
    );
    println!(
        "\n_See also: `codelore profile` for operational telemetry, `codelore schema <type>` for row schemas, `docs/research-foundations.md` for citations._"
    );
    Ok(())
}

/// Emit shell-completion script for the given shell to stdout. The
/// `clap_complete` derive macro consumes our existing clap spec —
/// no hand-maintained completion files.
fn run_completions_cmd(args: &args::CompletionsArgs) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(args.shell, &mut cmd, bin_name, &mut std::io::stdout());
}

/// JSON Schema export. The CLI surfaces the row-type catalogue and
/// emits a minimal envelope per type today; full `JsonSchema` derive
/// on every row type is a planned enhancement (~80 LOC of
/// `#[derive(JsonSchema)]` adds across the analyses module) and
/// will populate the `items` shape once `schemars` derive lands.
fn run_schema_cmd(args: &args::SchemaArgs) -> Result<()> {
    let row_types: Vec<&str> = AnalysisName::all().iter().map(|a| a.as_str()).collect();
    match &args.row_type {
        None => {
            println!("Supported row types ({}):", row_types.len());
            for name in &row_types {
                println!("  {name}");
            }
            println!(
                "\nUsage: codelore schema <row-type>\n\nNote: today's emitter ships the row-type catalogue and a minimal envelope. The full JSON Schema documents populate the `items` shape once `schemars` derive is applied to every analyses/* row type."
            );
            Ok(())
        }
        Some(name) => {
            if row_types.contains(&name.as_str()) {
                println!(
                    "{{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"$id\": \"https://codelore.dev/schemas/{name}.json\",\n  \"title\": \"{name}\",\n  \"type\": \"array\",\n  \"items\": {{\n    \"$comment\": \"Full row-shape schema populates once schemars derive is applied.\"\n  }}\n}}"
                );
                Ok(())
            } else {
                Err(CodeLoreError::Analysis(format!(
                    "unknown row type `{name}` — run `codelore schema` (no arg) to list supported row types"
                ))
                .into())
            }
        }
    }
}

fn run_diff_cmd(args: &DiffArgs) -> Result<()> {
    let (output, head_db, head_opts) = diff::run_diff(args).context("codelore diff")?;

    // Advisory LLM narrative. Best-effort and format-scoped: it is produced only
    // for `text`/`markdown` with `--llm`, any failure degrades to a stderr
    // warning, and it never touches the deterministic output below or the
    // `should_fail` exit code.
    let format = args.format.as_str();
    let narrative: Option<(String, String)> = if args.llm {
        match format {
            "text" | "markdown" => diff_llm_narrative(args, &output),
            _ => {
                eprintln!("note: --llm applies to text/markdown output only; ignored for {format}");
                None
            }
        }
    } else {
        None
    };

    let mut out: Box<dyn Write> = match args.output.as_ref() {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    diff_output::emit(
        &mut out,
        &output,
        format,
        &args.repo,
        Some((&head_db, &head_opts)),
        narrative,
    )?;
    drop(out);

    if diff::should_fail(args, &output) {
        // Per spec §6.6: analysis-failure exit code is 4.
        std::process::exit(4);
    }
    Ok(())
}

/// Produce the advisory LLM narrative for a diff run, degrading gracefully.
///
/// The `DiffOutput` is flattened into a deterministic [`DiffFactSheet`], a chat
/// client is resolved from the `CODELORE_LLM_*` environment, and the narrative
/// is generated under [`Lens::DiffNarrative`] with a citation-check stamp. Any
/// failure — no endpoint configured, network error, or narration error — is
/// reported on stderr and yields `None`, so the caller's deterministic output
/// and exit code stay untouched. The narrative cache lives under the default
/// cache root (diff has no `--cache-dir`).
fn diff_llm_narrative(args: &DiffArgs, output: &diff::DiffOutput) -> Option<(String, String)> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::enrichment::client::{LlmEnv, resolve_client};
    use codelore_lib::cli_api::enrichment::engine;
    use codelore_lib::cli_api::enrichment::fact_sheet::DiffFactSheet;
    use codelore_lib::cli_api::enrichment::prompt::Lens;

    let sheet = DiffFactSheet::from_sections(diff_fact_sections(output));
    let canonical = sheet.to_canonical_text();
    let values = sheet.numeric_values();
    let cache_root = default_cache_root();

    let result = resolve_client(&LlmEnv::from_process_env()).and_then(|client| {
        engine::narrate(
            client.as_ref(),
            Lens::DiffNarrative,
            "diff",
            engine::SheetFacts {
                text: &canonical,
                values: &values,
            },
            &cache_root,
            &args.repo,
            args.llm_refresh,
        )
    });
    match result {
        Ok(result) => Some((result.narrative.clone(), engine::stamp(&result))),
        Err(e) => {
            eprintln!("warning: llm narrative unavailable: {e}");
            None
        }
    }
}

/// Flatten a `DiffOutput` into ordered fact-sheet sections for the advisory diff
/// narrative. Only sections with data are emitted, and every numeric value is
/// rendered through the shared [`fmt_num`] formatter so the narrative's citation
/// check can match each quoted number back to a fact.
fn diff_fact_sections(output: &diff::DiffOutput) -> Vec<(String, Vec<(String, String)>)> {
    use codelore_lib::cli_api::enrichment::fact_sheet::fmt_num;

    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();

    // verdict — change-level health ratio, verdict, and change counts.
    if let Some(dh) = &output.delta_health {
        let mut facts = vec![("verdict".to_string(), dh.verdict.clone())];
        if let Some(ratio) = dh.ratio {
            facts.push(("ratio".to_string(), fmt_num(ratio)));
        }
        facts.push(("added".to_string(), dh.counts.added.to_string()));
        facts.push(("modified".to_string(), dh.counts.modified.to_string()));
        facts.push(("removed".to_string(), dh.counts.removed.to_string()));
        facts.push(("skipped".to_string(), dh.counts.skipped.to_string()));
        sections.push(("verdict".to_string(), facts));
    }

    // gates — [diff] quality-gate violations.
    if !output.gate_violations.is_empty() {
        let mut facts = Vec::new();
        for (i, v) in output.gate_violations.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.gate"), v.gate.clone()));
            facts.push((format!("{n}.path"), v.path.clone()));
            facts.push((format!("{n}.actual"), v.actual.clone()));
            facts.push((format!("{n}.threshold"), v.threshold.clone()));
        }
        sections.push(("gates".to_string(), facts));
    }

    // entrants — files newly entering the top-N hotspot list.
    if !output.hotspots.rank_entrants.is_empty() {
        let mut facts = Vec::new();
        for (i, h) in output.hotspots.rank_entrants.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.path"), h.path.clone()));
            facts.push((format!("{n}.hotspot_score"), fmt_num(h.hotspot_score)));
            facts.push((format!("{n}.revisions"), h.revisions.to_string()));
            facts.push((format!("{n}.cognitive"), fmt_num(h.cognitive)));
            facts.push((format!("{n}.cognitive_health"), fmt_num(h.cognitive_health)));
        }
        sections.push(("entrants".to_string(), facts));
    }

    // score-increased — existing hotspots whose score grew past the threshold.
    if !output.hotspots.score_increased.is_empty() {
        let mut facts = Vec::new();
        for (i, s) in output.hotspots.score_increased.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.path"), s.path.clone()));
            facts.push((format!("{n}.base_score"), fmt_num(s.base_score)));
            facts.push((format!("{n}.head_score"), fmt_num(s.head_score)));
            facts.push((format!("{n}.delta"), fmt_num(s.delta)));
        }
        sections.push(("score-increased".to_string(), facts));
    }

    // absences — historically-coupled files omitted from the PR.
    if !output.coupling_absences.is_empty() {
        let mut facts = Vec::new();
        for (i, a) in output.coupling_absences.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.touched_file"), a.touched_file.clone()));
            facts.push((format!("{n}.expected_partner"), a.expected_partner.clone()));
            facts.push((
                format!("{n}.historical_coupling"),
                fmt_num(a.historical_coupling),
            ));
            facts.push((format!("{n}.fisher_p"), fmt_num(a.fisher_p)));
            facts.push((
                format!("{n}.historical_shared_revs"),
                a.historical_shared_revs.to_string(),
            ));
        }
        sections.push(("absences".to_string(), facts));
    }

    // clones — new clone-family members introduced by the PR.
    if !output.clones.new_families.is_empty() {
        let mut facts = Vec::new();
        for (i, c) in output.clones.new_families.iter().enumerate() {
            let n = i + 1;
            facts.push((format!("{n}.clone_group_id"), c.clone_group_id.to_string()));
            facts.push((format!("{n}.entity"), c.entity.clone()));
            facts.push((format!("{n}.function"), c.function.clone()));
            facts.push((format!("{n}.start_line"), c.start_line.to_string()));
            facts.push((format!("{n}.end_line"), c.end_line.to_string()));
            facts.push((format!("{n}.node_count"), c.node_count.to_string()));
        }
        sections.push(("clones".to_string(), facts));
    }

    sections
}

fn init_logging(verbose: bool) {
    let filter = if verbose {
        EnvFilter::new("info,codelore=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    // Emit a span-close event with elapsed time whenever a span exits.
    // Enables `RUST_LOG=codelore::bench=info codelore analyze …` to print
    // per-stage timing — no `--bench` flag needed. The CLOSE event is
    // suppressed by default at WARN level, so this has zero overhead for
    // normal runs.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_span_events(FmtSpan::CLOSE)
        .init();
}
