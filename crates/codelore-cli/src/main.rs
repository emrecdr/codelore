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
mod suggest;

use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;
use codelore_lib::cli_api::{AnalysisName, CodeLoreError, Options};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::args::{Cli, Command, DiffArgs, IngestSarifArgs, McpArgs};

fn main() {
    if let Err(e) = run() {
        // A reader closing our stdout early — the classic `codelore … | head`,
        // or a pager quit — surfaces as a BrokenPipe I/O error on the next
        // write. That is a normal way to consume partial output, not a failure,
        // so exit 0 silently (no error line), matching conventional CLI
        // behaviour. The workspace forbids `unsafe`, so we cannot restore the
        // default SIGPIPE disposition; recognising the error on the way out is
        // the mechanism.
        if is_broken_pipe(&e) {
            std::process::exit(0);
        }
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

/// True when `err`'s cause chain carries a `BrokenPipe` I/O error — i.e. the
/// process reading our stdout closed the pipe before we finished writing.
///
/// Output emitters surface this in two shapes: a bare `std::io::Error` (the
/// CSV/Markdown writers propagate it via `map_err(CodeLoreError::Io)`, and the
/// serde-based JSON/NDJSON/SARIF writers rebuild it as `Io` so the kind is
/// preserved), and a `CodeLoreError::Io`/`RepoIo` wrapping one directly. Walk
/// the whole chain and match either.
fn is_broken_pipe(err: &anyhow::Error) -> bool {
    use std::io::ErrorKind::BrokenPipe;
    err.chain().any(|cause| {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.kind() == BrokenPipe
        } else if let Some(cle) = cause.downcast_ref::<CodeLoreError>() {
            matches!(
                cle,
                CodeLoreError::Io(io) | CodeLoreError::RepoIo(io) if io.kind() == BrokenPipe
            )
        } else {
            false
        }
    })
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
/// var is set. No-op outside GitHub Actions. An open or write failure
/// logs a `tracing::warn!` rather than failing silently.
pub(crate) fn write_github_output(key: &str, value: &str) {
    let Ok(path) = std::env::var("GITHUB_OUTPUT") else {
        // Unset is the normal local/non-CI case — not an error, no warning.
        return;
    };
    let mut f = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("write_github_output: could not open {path}: {e}");
            return;
        }
    };
    // Pre-assemble the whole `key=value\n` line and emit it with one
    // `write_all`, matching the gate-run ledger's single-write append.
    let line = format!("{key}={value}\n");
    if let Err(e) = f.write_all(line.as_bytes()) {
        tracing::warn!("write_github_output: write failed for key={key}: {e}");
    }
}

/// The advisory `check` and `gate` print when no thresholds file exists.
/// Both exit 0 in that state, so without naming what is missing the line is
/// indistinguishable from a gate that ran and found nothing — which for
/// someone wiring CI is a green build measuring nothing. Names gates worth
/// starting from, the way the ratchet's initialization message does.
/// `command` is the subcommand's own name so the line names what was run.
pub(crate) fn vacuous_pass_notice(command: &str) -> String {
    format!(
        "codelore {command}: no thresholds configured (no `.codelore-thresholds.toml` at repo \
         root); vacuously passing. Add a [gates] section with code_health_min / \
         max_dependency_cycles / max_red_effort_pct to make it bind on regressions."
    )
}

/// Map the `fail_on_skipped` policy onto a run's ledger records: when the policy
/// is on, every gate recorded `"skipped"` becomes a synthetic [`GateViolation`]
/// so the surface's normal violation-exit path fails the run; off (the default)
/// yields none, so a skipped gate stays exit-0. The ledger keeps the honest
/// `"skipped"` verdict — only the exit-facing violation set gains a row, tagged
/// with the `(skipped)` pseudo-path so the SARIF/GHA emitters skip the
/// commit-evidence lookup on it. Shared by `codelore check` and `codelore gate`;
/// `codelore diff` has no ledger and honours the policy through
/// [`diff::should_fail`] instead.
pub(crate) fn skipped_gate_violations(
    records: &[codelore_lib::cli_api::quality_gates::ledger::GateRunRecord],
    fail_on_skipped: bool,
) -> Vec<codelore_lib::cli_api::quality_gates::GateViolation> {
    use codelore_lib::cli_api::quality_gates::GateViolation;
    if !fail_on_skipped {
        return Vec::new();
    }
    records
        .iter()
        .filter(|r| r.verdict == "skipped")
        .map(|r| GateViolation {
            gate: r.gate.clone(),
            path: "(skipped)".into(),
            actual: "skipped".into(),
            threshold: "fail_on_skipped".into(),
        })
        .collect()
}

/// The discriminating reason a `[new_code]` gate skipped at runtime. A shallow
/// checkout (fetch-depth truncated the pre-window history) and a genuinely young
/// repository both leave the window with no pre-window baseline and read
/// identically at the window-start query — but they are different causes with
/// different fixes, so the reason names fetch-depth when the checkout is shallow.
/// Shared between `codelore check`'s `eprintln!` notice and the MCP `[new_code]`
/// gate's `reason` field so the wording never drifts between the two surfaces.
pub(crate) fn new_code_skip_reason(window_days: f64, shallow_checkout: bool) -> String {
    if shallow_checkout {
        format!(
            "the checkout is shallow (fetch-depth): its history is truncated to within the \
             {window_days:.0}-day window, so there is no pre-window baseline to contrast the \
             working set against. This is the checkout, not the repository — re-run against \
             full history (fetch-depth: 0)."
        )
    } else {
        format!(
            "the repository's history is shallower than the {window_days:.0}-day window (a \
             genuinely young repository), so there is no pre-window baseline to contrast the \
             working set against."
        )
    }
}

/// The reason the `corpus_percentile_max` gate skipped: no calibration artifact
/// is active, or no analyzed file resolved a corpus percentile. Shared between
/// `codelore check`'s `eprintln!` notice and the MCP `check_gates` tool's
/// `SkippedGate.reason` field so the wording never drifts between the two
/// surfaces — the same one-definition guarantee as `new_code_skip_reason`.
pub(crate) const CORPUS_PERCENTILE_SKIP_REASON: &str = "no corpus percentile data (no calibration artifact active, or no analyzed file resolved a percentile)";

/// Operational telemetry. Prints what `CodeLore` ships under the
/// hood — schema version, pinned dependency versions, supported
/// analysis count, supported output format count. Useful for triage
/// when behaviour surprises a user.
fn run_profile_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    // Write through a locked stdout handle with propagating `writeln!` rather
    // than `println!`: a reader closing the pipe part-way through this dump
    // (`codelore profile | head`) then routes the BrokenPipe up to `main`'s
    // quiet-exit arm instead of panicking inside the print macro.
    let mut out = std::io::stdout().lock();
    writeln!(out, "# CodeLore profile\n")?;
    writeln!(out, "**Version**: {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(
        out,
        "**Schema**: schema_v{} (`facts/schema_v1.sql`)",
        codelore_lib::cli_api::facts::schema::CURRENT_SCHEMA_VERSION
    )?;
    writeln!(
        out,
        "**Analyses**: {} registered",
        AnalysisName::all().len()
    )?;
    writeln!(
        out,
        "**Output formats**: {}",
        args::analyze_format_names().join(" | ")
    )?;
    writeln!(
        out,
        "**Pinned third-party**:\n  - gix {gix}\n  - DuckDB {duckdb}\n  - tree-sitter 0.25.x (Rust/Python/Java/JS/TS/TSX/C++)",
        gix = codelore_lib::cli_api::provenance::GIX_VERSION,
        duckdb = codelore_lib::cli_api::provenance::DUCKDB_VERSION,
    )?;
    writeln!(out, "\n**Cache root**:")?;
    if let Some(dir) = dirs::cache_dir() {
        writeln!(out, "  {}/codelore/", dir.display())?;
    } else {
        writeln!(out, "  <unavailable on this platform>")?;
    }
    writeln!(
        out,
        "\n**SPA feature**: {}",
        if cfg!(feature = "spa") {
            "ENABLED"
        } else {
            "disabled (build with --features spa to opt in)"
        }
    )?;
    writeln!(
        out,
        "\n_For per-analysis SQL + citations, run `codelore explain <topic>`._"
    )?;
    Ok(())
}

/// Markdown dump of every supported analysis. Seeds the planned
/// full static-HTML doc site.
fn run_docs_cmd() -> Result<()> {
    use codelore_lib::cli_api::analysis::AnalysisName;
    // Propagating `writeln!` over a locked stdout (see `run_profile_cmd`): this
    // multi-line catalogue is a natural `codelore docs | head` target, so an
    // early pipe close must reach `main`'s quiet-exit arm, not panic.
    let mut out = std::io::stdout().lock();
    writeln!(out, "# CodeLore — Analysis catalogue\n")?;
    writeln!(
        out,
        "Auto-generated from `AnalysisName::all()`. Run `codelore explain <topic>` for per-analysis citations and formulas. The full citation chain lives in `docs/research-foundations.md`.\n"
    )?;
    writeln!(out, "## Supported analyses\n")?;
    for analysis in AnalysisName::all() {
        writeln!(out, "- `{}`", analysis.as_str())?;
    }
    writeln!(out, "\n## Output formats\n")?;
    for (name, description) in args::ANALYZE_FORMATS {
        writeln!(out, "- `{name}` — {description}")?;
    }
    writeln!(out, "\n## Conventions\n")?;
    writeln!(
        out,
        "- Files alive at HEAD only (deleted files excluded from path-aggregating analyses)"
    )?;
    writeln!(
        out,
        "- Mailmap + `.codelore-teams` + `.codelorebots` consulted at ingest time"
    )?;
    writeln!(out, "- `.gitignore` / `.codeloreignore` honoured")?;
    writeln!(
        out,
        "- `--time-bucket` supported on: hotspots, coupling, soc, code-health"
    )?;
    writeln!(
        out,
        "\n## Reproducibility\n\nEvery file output is paired with a `.provenance.json` sidecar capturing the run's full `Options` shape. SQLite outputs embed the equivalent inside the `provenance` table."
    )?;
    writeln!(
        out,
        "\n_See also: `codelore profile` for operational telemetry, `codelore schema <type>` for row schemas, `docs/research-foundations.md` for citations._"
    )?;
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
    // Locked stdout + propagating `writeln!` (see `run_profile_cmd`) so the
    // row-type catalogue survives `codelore schema | head` as a quiet exit.
    let mut out = std::io::stdout().lock();
    match &args.row_type {
        None => {
            writeln!(out, "Supported row types ({}):", row_types.len())?;
            for name in &row_types {
                writeln!(out, "  {name}")?;
            }
            writeln!(
                out,
                "\nUsage: codelore schema <row-type>\n\nNote: today's emitter ships the row-type catalogue and a minimal envelope. The full JSON Schema documents populate the `items` shape once `schemars` derive is applied to every analyses/* row type."
            )?;
            Ok(())
        }
        Some(name) => {
            if row_types.contains(&name.as_str()) {
                writeln!(
                    out,
                    "{{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"$id\": \"https://codelore.dev/schemas/{name}.json\",\n  \"title\": \"{name}\",\n  \"type\": \"array\",\n  \"items\": {{\n    \"$comment\": \"Full row-shape schema populates once schemars derive is applied.\"\n  }}\n}}"
                )?;
                Ok(())
            } else {
                let hint = suggest::nearest(name, row_types.iter().copied())
                    .map(|s| format!(" (did you mean `{s}`?)"))
                    .unwrap_or_default();
                Err(CodeLoreError::Analysis(format!(
                    "unknown row type `{name}`{hint} — run `codelore schema` (no arg) to list supported row types"
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
        // A `[diff]` gate violation (or a skip failed under `fail_on_skipped`)
        // is a gate failure, not an analysis crash — exit 1, matching the
        // `bail!` path in `check`/`gate`.
        std::process::exit(1);
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

#[cfg(test)]
mod tests {
    use super::is_broken_pipe;
    use anyhow::Context as _;
    use codelore_lib::cli_api::CodeLoreError;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn detects_bare_broken_pipe() {
        let err = anyhow::Error::new(IoError::from(ErrorKind::BrokenPipe));
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn detects_broken_pipe_behind_context() {
        // The shape the CSV/Markdown writers produce: an io error carried up
        // through one or more `.context(...)` frames.
        let err = std::result::Result::<(), _>::Err(IoError::from(ErrorKind::BrokenPipe))
            .context("write csv")
            .unwrap_err();
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn detects_broken_pipe_wrapped_in_codelore_io() {
        let err = anyhow::Error::new(CodeLoreError::Io(IoError::from(ErrorKind::BrokenPipe)));
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn ignores_other_io_kinds() {
        let err = anyhow::Error::new(CodeLoreError::Io(IoError::from(
            ErrorKind::PermissionDenied,
        )));
        assert!(!is_broken_pipe(&err));
    }

    #[test]
    fn ignores_unrelated_errors() {
        let err = anyhow::Error::new(CodeLoreError::Analysis("boom".into()));
        assert!(!is_broken_pipe(&err));
    }
}
