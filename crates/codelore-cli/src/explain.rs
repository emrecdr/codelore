//! `codelore explain` — metric catalogue and per-file evidence dossiers.
//!
//! With no argument, lists every supported topic. With a topic name, prints its
//! citation, formula, and source. With an argument that names no topic, resolves
//! it to a repo-relative source file and prints that file's deterministic
//! evidence dossier — and, with `--llm`, an advisory grounded narrative.

use anyhow::{Context, Result};
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::GixRepo;
use codelore_lib::cli_api::{CodeLoreError, Options};

use crate::args;

/// Print formula + citation + SQL for the named metric or analysis.
/// With no topic, lists every supported topic. Makes the auditable-
/// formulas brand promise tactile on the CLI side — `codelore
/// explain hotspot-score` is the answer to "why does this file have
/// score 8.4?".
#[allow(clippy::too_many_lines)] // catalogue table — splitting would hurt readability
pub(crate) fn run_explain_cmd(args: &args::ExplainArgs) -> Result<()> {
    let topics: &[(&str, &str, &str, &str)] = &[
        (
            "hotspot-score",
            "Tornhill 2018 — Software Design X-Rays",
            "percentile_rank(revisions) × percentile_rank(cognitive) × (100 − code_health) / 4. Range [0, 10].",
            "See analyses/hotspots.rs::SQL (file_revs + file_complexity + joined CTEs).",
        ),
        (
            "code-health",
            "code-health composite: biomarker structural risk (Complex/Large Method, God Class, DRY, Shotgun Surgery, Deep Nesting, Many Args, Complex Conditional) fused with behavioral signal (Nagappan & Ball 2005 churn + Mockus & Herbsleb 2002 ownership); coupling centrality enters once via the Shotgun Surgery biomarker (Tornhill 2018); self-relative percentile banding (Alves/Ypma/Visser 2010) plus an additive corpus-relative percentile when a calibration artifact is active",
            "100 × (1 − 0.50·structural_risk − 0.30·churn − 0.20·ownership_fv), where structural_risk is a weighted sum of biomarker intensities (complex-method 0.22, god-class 0.18, large-method 0.12, dry 0.12, shotgun-surgery 0.12, deep-nesting 0.10, many-args 0.07, complex-conditional 0.07); band from structural_risk thresholds (≥0.55 red, ≥0.28 yellow, else green); percentile = per-language PERCENT_RANK of structural_risk.",
            "See analyses/code_health.rs.",
        ),
        (
            "refactoring-targets",
            "effort-aware refactoring priority: (code-health structural_risk × hotspot_score) ÷ inspection effort, with a ManualUp baseline (Popt / PofB20 framing)",
            "priority = (structural_risk × hotspot_score) / max(loc, 25). Ranked DESC. `manual_up_rank` ranks the same files by ascending LOC (the \"inspect the small dense files first\" baseline the composite is meant to beat); `dominant_type` is the file's highest-intensity biomarker.",
            "See analyses/refactoring_targets.rs.",
        ),
        (
            "mi",
            "Coleman 1994 + SEI 1997",
            "171 − 5.2·log₂(V) − 0.23·CC − 16.2·log₂(SLOC) + 50·sin(√(2.4·comments%)). file-level `kind='unit'` entry.",
            "Surfaced by rust-code-analysis via codelore-rca.",
        ),
        (
            "coupling-density",
            "Newman 2010 §6.10 — graph density",
            "edges / (V·(V−1)/2) where V is the candidate node set (files with revs ≥ min_revs) and edges are Fisher-significant coupling pairs.",
            "See analyses/coupling.rs::density.",
        ),
        (
            "hotspots",
            "Tornhill 2015 + Bird et al. 2011",
            "Per-file behavioural risk surface: revisions × max(cognitive) × code-health composite. The flagship CodeLore analysis.",
            "See analyses/hotspots.rs.",
        ),
        (
            "hotspot-velocity",
            "Change-acceleration early warning (recent vs baseline churn)",
            "Per file: acceleration = recent_per_week − baseline_per_week over a 30-day recent window vs the 90 days before it, anchored at MAX(commits.date). Positive = heating up (becoming a hotspot before its all-time count shows it); negative = cooling down.",
            "See analyses/hotspot_velocity.rs.",
        ),
        (
            "god-classes",
            "Brown et al. 1998 *AntiPatterns* §3.1 + Riel 1996 *Object-Oriented Design Heuristics*",
            "(cognitive / 100.0) × (fan_in + fan_out). Ranks files where all three pull up.",
            "See analyses/god_classes.rs.",
        ),
        (
            "architecture-roles",
            "Baldwin, MacCormack & Rusnak 2014 — Hidden Structure (Research Policy 43:8)",
            "Per-file Core/Shared/Control/Periphery from transitive visibility fan-in (vfi) / fan-out (vfo) on the import graph: Core = the largest cyclic group; Shared = vfi≥core, vfo<core; Control = vfi<core, vfo≥core; Periphery = both below. reach_pct = vfo/n×100; mean(vfo/n) = MacCormack propagation cost.",
            "See analyses/architecture_roles.rs + analyses/import_graph.rs::reachability.",
        ),
        (
            "instability",
            "Martin 1994 — OO Design Quality Metrics (Clean Architecture 2017)",
            "Per file: afferent coupling ca (files importing it / in-degree), efferent coupling ce (files it imports / out-degree), instability I = ce/(ca+ce) in [0,1]. 0 = stable (depended-on, depends on nothing), 1 = unstable. Resolved import graph; Abstractness/Distance need symbol data and are out of scope.",
            "See analyses/instability.rs.",
        ),
        (
            "cycle-health",
            "behavioral heat + extraction candidate per import cycle",
            "Per non-trivial SCC of the resolved import graph: heat_pct = the cycle \
             members' share of repo LOC churn over the trailing --window-days window \
             (same anchoring as effort-exposure); verdict = live when at least one \
             member appears in a window commit (a zero-LOC touch still counts), fossil \
             otherwise; extract_candidate = the member whose trial removal minimises \
             the largest surviving SCC of the induced subgraph (Tarjan per member, ties \
             by fewest surviving cyclic nodes then lexicographic path); \
             predicted_pc_drop = whole-graph MacCormack propagation-cost delta if the \
             candidate were extracted. Trial removal and the prediction run only for \
             cycles of ≤ 64 members; above that bound the prediction is absent and the \
             candidate falls back to the highest in-cycle degree.",
            "See analyses/cycle_health.rs.",
        ),
        (
            "defect-validation",
            "Śliwerski, Zimmermann & Zeller 2005 (SZZ) + Kim et al. 2006 (AG-SZZ)",
            "Reads an own-repo defect-calibration artifact (built by `codelore \
             calibrate-defects`) and reports its evidence as flat (metric, value) \
             rows: the band table (share of defect-introducing changes that landed \
             in files red / yellow / green at the time), AUC and precision@k of \
             HEAD structural_risk against the defect-implicated file labels, mining \
             tallies, and the weight-tuning decision with both validation AUCs. \
             Association, not causation — a defect touching a red file is evidence \
             the score ranks it high, not proof the score caused the defect. Reads \
             the artifact only; without one it emits zero rows and a stderr hint.",
            "See analyses/defect_validation.rs + defect_calibration/.",
        ),
        (
            "architecture-metrics",
            "Lakos 1996 (CCD/ACD/NCCD) + MacCormack/Rusnak/Baldwin 2006/2014",
            "Repo-level (metric, value) rows: propagation_cost = density of the transitive-closure matrix; acd = mean transitive dependency set size; nccd = CCD / balanced-binary-tree CCD (<1 flat, >1 layered, >2 likely cyclic); dependency_cycles, largest_cycle; architecture_type = hierarchical / core-periphery / multi-core.",
            "See analyses/architecture_metrics.rs.",
        ),
        (
            "architecture-trend",
            "Architectural decay over the commit sequence",
            "Recomputes propagation cost, dependency-cycle count and largest tangle at up to 12 historical revs (evenly spaced across history), rebuilding the import graph in memory at each by reading + resolving source blobs at that rev. Shows whether structure is decaying and roughly when it started. Heavier than the SQL-only analyses (it re-parses source per sample); computed on demand, never cached.",
            "See analyses/architecture_trend.rs.",
        ),
        (
            "health-trend",
            "Repo health (architectural + code + combined) over the commit sequence",
            "Computes three 0-100 scores at up to 12 historical revs (evenly spaced): \
             architectural health (structural — propagation cost + dependency tangle), \
             code health (the rev-parameterized code-health engine with duplication \
             excluded, averaged over files), and their equal blend. Bands: green >= 70, \
             yellow 40-69, red < 40. Rebuilds the import graph + re-scans complexity per \
             sample, so it is heavier than SQL-only analyses; computed on demand, never \
             cached.",
            "See analyses/health_trend.rs.",
        ),
        (
            "effort-exposure",
            "Engineering effort distribution across code-health bands",
            "For each code-health band (red / yellow / green) reports the percentage of \
             files, SLOC, trailing-window commits, and LOC churn in that band. Answers \
             the hero KPI question: are we spending most effort fighting fires in red code \
             or extending healthy green code? Commit-share Wilson 95% CI is included per \
             band. Window anchors to the repo's last commit date (not wall-clock) via \
             --window-days (default 90).",
            "See analyses/effort_exposure.rs.",
        ),
        (
            "code-familiarity",
            "Decayed-knowledge familiarity score for the active team",
            "Computes what fraction of SLOC is actively known by current contributors \
             (authors with ≥1 commit in the trailing window). Uses exponentially-decayed \
             knowledge shares (Jabrayilzade et al., ICSE-SEIP 2022). Also reports islands \
             percentage: SLOC in files where one person holds ≥80% of knowledge with no \
             meaningful backup. Low familiarity or high islands percentage signals knowledge \
             risk. Verdict threshold configurable via [gates] code_familiarity_min in \
             .codelore-thresholds.toml (default 70.0).",
            "See analyses/code_familiarity.rs.",
        ),
        (
            "team-composition",
            "Contribution-span tenure buckets with behavioral veteran gate and onboarding velocity",
            "Buckets each author by contribution span (last − first commit): onboarded \
             (<90 d), experienced (90–364 d), veteran (≥365 d). Veterans who have not \
             touched a breadth of files comparable to the current 80%-core set are capped \
             at 'experienced' (veteran_breadth_ok = false). Also reports onboarding_weeks: \
             how many weeks from an author's first commit to their first week in the weekly \
             80%-core set. Authors whose first commit falls within the project's first 12 \
             weeks (founders) receive NULL for onboarding_weeks.",
            "See analyses/team_composition.rs.",
        ),
        (
            "coordination-needs",
            "Per-file coordination overhead: fragmentation, interleave, co-change entropy",
            "For each file reports: knowledge fragmentation (1 − HHI, 0 = single owner, \
             near 1 = evenly spread knowledge); author-switch interleave between adjacent \
             commits (0 = always same author, 1 = always alternating); and co-change graph \
             entropy contribution (EASE 2025, arXiv 2504.18511; window-scoped, commits \
             touching >30 files excluded). Tier: single (1 author) | low (frag<0.25) | \
             medium | high (frag≥0.50 AND interleave≥0.50). Joined with code-health band \
             so high-fragmentation files in the red band surface first.",
            "See analyses/coordination_needs.rs.",
        ),
        (
            "marginal-owner-risk",
            "Ownership concentration × code-health fusion: files where active authors have shallow familiarity",
            "For each file in the yellow or red health band, reports the maximum knowledge \
             share held by any author who committed within window_days. Risk tiers: high \
             (red band AND top active share <0.10); elevated ((red AND <0.30) OR (yellow \
             AND <0.10)). Rows that do not meet either threshold are excluded. The \
             ownership × code-quality interaction is correlational, not causal \
             (Palomba et al., EASE 2023, arXiv 2304.11636).",
            "See analyses/marginal_owner_risk.rs.",
        ),
        (
            "release-cadence",
            "Inter-release gap statistics from git tags (median, IQR, trend)",
            "Filters tags by --release-tag-glob (default 'v*'), then computes the \
             number of days between consecutive release tags. Emits one row per \
             matched tag (date, days_since_prev) plus a synthetic '__summary__' \
             row carrying the median gap, IQR, and a trend label: 'accelerating' \
             (negative OLS slope < −0.1 d/release), 'slowing' (slope > +0.1), or \
             'stable' (within ±0.1). Tags are proxies for releases, not \
             deployments; cadence reflects tagging discipline as much as actual \
             release velocity. First tag always has no predecessor gap.",
            "See analyses/release_cadence.rs.",
        ),
        (
            "delivery-metrics",
            "Repo-level delivery flow distributions: batch size, branch duration, rework, and lead-proxy (p50/p75/p90)",
            "Five percentile distributions over merge units and commits: batch_size_files \
             (distinct paths per merge), batch_size_loc (LOC churn per merge), \
             branch_duration_hours (merge date − earliest branch-side author date), \
             rework_pct (hunk-overlap within --rework-window-days, approximate), and \
             lead_proxy_hours (author→committer date gap, positive only, non-merge commits). \
             Requires commit_parents table (schema v4) and merges ingested with \
             include_merges=true. Branch metrics are unreliable on squash/rebase workflows \
             (emits a warning when merge count < 3 and commit count > 50).",
            "See analyses/delivery_metrics.rs.",
        ),
        (
            "function-xray",
            "Per-function change frequency for a single target file (Gall et al. ICSM 2003 HistoryFinder)",
            "Requires --target <repo-relative-path>. For each function/method alive at HEAD \
             in the target file, counts revisions where at least one hunk overlapped the \
             function's line span. Hunk-overlap attribution is more accurate than file-level \
             blame: it uses the span at change time. Pure deletions (new_lines=0) are attributed \
             to the function whose span contained the deleted anchor line. Sorted by change_freq \
             DESC.",
            "See analyses/function_xray.rs.",
        ),
        (
            "function-coupling",
            "Per-function-pair co-change frequency with Fisher significance for a single target file",
            "Requires --target <repo-relative-path>. For each pair of HEAD-alive functions in the \
             target file that co-changed (both touched in the same revision) in ≥2 revisions, \
             emits the pair with co-change count, per-function change counts, confidence \
             (co/min(a,b)), and two-tailed Fisher exact p-value. \
             Sorted by p-value ASC. Research: Adams et al. ICSM 2006.",
            "See analyses/function_coupling.rs.",
        ),
        (
            "cycle-origins",
            "Commit-level archaeology for dependency cycles",
            "For each dependency cycle at HEAD, binary-searches history (reading + resolving source at past revisions) to find the earliest commit where that cycle existed — the commit that closed the loop. Reports the forming commit's SHA + date per cycle. Assumes a cycle, once formed, stays formed; traces the largest cycles first to bound cost.",
            "See analyses/cycle_origins.rs.",
        ),
        (
            "dependency-cycles",
            "Tarjan 1972 SCC + Fontana et al. 2017 (Arcan) Cyclic Dependency smell",
            "Non-trivial strongly-connected components (size ≥ 2) of the structural import graph — files that import each other transitively. cycle_id groups a tangle; size is its member count. Accuracy follows the import resolver's language coverage.",
            "See analyses/dependency_cycles.rs + analyses/import_graph.rs.",
        ),
        (
            "modularity-violations",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8) + Baldwin/MacCormack 2014 hidden structure",
            "Fisher-significant co-change pairs (from coupling) with NO directed dependency path between them (transitive reachability, either direction) — implicit cross-module dependencies. Ranked by coupling degree. Accuracy follows the import resolver's language coverage.",
            "See analyses/modularity_violations.rs.",
        ),
        (
            "unstable-interface",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8)",
            "revisions × coupled_dependents, gated on fan_in ≥ 3 and revisions ≥ min_revs. A widely-imported file that changes often and co-changes with its dependents, so its instability propagates.",
            "See analyses/unstable_interface.rs.",
        ),
        (
            "crossing",
            "Mo, Cai, Kazman, Xiao 2015 *Hotspot Patterns* (DV8)",
            "A structural 'X' — fan_in ≥ 3 AND fan_out ≥ 3 — that co-changes with ≥1 importer AND ≥1 import, coupling upstream and downstream through itself. crossing_score = coupled_upstream + coupled_downstream.",
            "See analyses/crossing.rs.",
        ),
        (
            "bus-factor",
            "Filatov 2010 (commits mode) / Cury & Avelino SBES'24 (doe mode)",
            "Min number of authors whose departure would leave a module unmaintained. \
             Default mode (--knowledge-model commits): smallest set covering ≥80% of \
             module commits (Filatov 2010). DOE mode (--knowledge-model doe): greedy \
             truck-factor procedure — repeatedly remove the author expert on the most \
             remaining files until >50% of files have no expert (Cury & Avelino, \
             SBES'24 arXiv 2408.08733). DOE mode emits the same per-module row shape \
             with model='doe'.",
            "See analyses/bus_factor.rs.",
        ),
        (
            "stale-code",
            "code-age follow-up + Sonar 'trivial' threshold",
            "Files alive at HEAD AND untouched ≥12 months AND max(cognitive) ≤ 5. Intersection minimises false positives.",
            "See analyses/stale_code.rs.",
        ),
        (
            "pair-programming",
            "Co-Authored-By trailer convention (GitHub 2017)",
            "Counts commits where ≥1 `Co-Authored-By:` trailer present, by unique author pair.",
            "See analyses/pair_programming.rs.",
        ),
        (
            "lead-time",
            "DORA 2018 Accelerate",
            "Seconds between commit author-date and committer-date (proxy for in-flight review time). Schema_v3 carries only committer-date; schema_v4 will add author-date for real values.",
            "See analyses/lead_time.rs.",
        ),
        (
            "knowledge-islands",
            "T8 design + Bird et al. 2011 risk-author",
            "Per-file bus-factor risk: primary author hasn't committed in `--departed-threshold-days` days AND no substantial other owners.",
            "See analyses/knowledge_islands.rs.",
        ),
        (
            "communities",
            "Leiden algorithm (Traag, Waltman, van Eck 2019)",
            "Modularity-optimising community detection on the Fisher-significant coupling graph. Surfaces Conway's-law clusters.",
            "See analyses/communities.rs.",
        ),
        (
            "centrality",
            "Newman 2010 §7",
            "Per-file degree, weighted-degree, and PageRank on the Fisher-significant coupling graph.",
            "See analyses/centrality.rs.",
        ),
        (
            "architecture-violations",
            "Layered architecture rules (Buschmann et al. 1996)",
            "Imports that cross a forbidden layer boundary per `.codelore-arch-rules.toml`. Empty rule set → empty output.",
            "See arch_rules/mod.rs + analyses/arch_violations.rs.",
        ),
        (
            "kamei-risk",
            "Kamei et al. 2013 (Just-In-Time Software Defect Prediction)",
            "Per-commit 14-feature vector (la, ld, nf, nd, ns, entropy, fix, ndev, age, nuc, exp, rexp, sexp, lt). Composite risk dimension explanation in the SPA's Delivery Risk Sparkline.",
            "See output/spa.rs::run_kamei_risk + facts/schema_v1.sql commits table.",
        ),
        (
            "revisions",
            "Nagappan & Ball 2005 — relative churn predicts defect density",
            "COUNT(rev) per file — distinct commits touching the path. Gated by --min-revs, ordered by n-revs descending.",
            "See analyses/revisions.rs.",
        ),
        (
            "authors",
            "Bird, Nagappan, Murphy, Devanbu & Zeller 2011 — \"Don't Touch My Code!\"",
            "Distinct canonical authors per file (n-authors = COUNT(DISTINCT author)), split into human vs bot via .mailmap + bot/AI attribution; n-revs = Σ per-author commit counts.",
            "See analyses/authors.rs.",
        ),
        (
            "ownership",
            "Mockus & Herbsleb 2002 + Hirschman 1980 (Herfindahl–Hirschman index)",
            "Fractal Value = 1 − Σᵢ (aᵢ / nc)², where aᵢ is author i's commit count on the file and nc the file's total commits. 0 = single owner, → 1 = fragmented. main-author = author with the most revisions.",
            "See analyses/ownership.rs.",
        ),
        (
            "code-age",
            "Tornhill 2015 — Your Code as a Crime Scene (software half-life)",
            "Whole calendar months between the file's latest commit (at-or-before the --age-time-now anchor, default now) and the anchor: 12·(yr−yr) + (mo−mo) − 1 if the anchor day-of-month is earlier than the last-commit day. Only files live at the anchor.",
            "See analyses/code_age.rs.",
        ),
        (
            "soc",
            "Tornhill 2018 — Software Design X-Rays (Sum of Coupling)",
            "Σ (commit_size − 1) over every commit the file appears in; a solo commit contributes 0. Per-file centrality across the change-coupling graph. Gated by --min-soc.",
            "See analyses/soc.rs.",
        ),
        (
            "abs-churn",
            "Nagappan & Ball 2005 — relative code churn predicts defect density",
            "Per calendar day across the repo: SUM(lines added), SUM(lines deleted), COUNT(commits). The absolute-churn time series.",
            "See analyses/churn.rs.",
        ),
    ];
    match &args.topic {
        None => {
            println!("Supported topics:");
            for (name, _, _, _) in topics {
                println!("  {name}");
            }
            println!("\nUsage: codelore explain <topic>");
            Ok(())
        }
        Some(topic) => {
            let found = topics.iter().find(|(n, ..)| n.eq_ignore_ascii_case(topic));
            match found {
                Some((name, citation, formula, source)) => {
                    println!("# {name}\n");
                    println!("**Citation**\n  {citation}\n");
                    println!("**Formula**\n  {formula}\n");
                    println!("**Source**\n  {source}\n");
                    println!(
                        "**Foundations**\n  See docs/research-foundations.md for the full citation chain."
                    );
                    Ok(())
                }
                None => match resolve_explain_file(&args.repo, topic) {
                    Some(repo_relative) => run_explain_file(args, &repo_relative),
                    None => Err(CodeLoreError::Analysis(format!(
                        "unknown topic `{topic}` — run `codelore explain` (no arg) to list \
                         supported topics, or pass an existing file path (with --repo) to print \
                         that file's evidence dossier"
                    ))
                    .into()),
                },
            }
        }
    }
}

/// Resolve an `explain` argument that missed the topic table to a repo-relative
/// source-file path, or `None` when it names no existing file.
///
/// The argument is joined onto `--repo`; `Path::join` lets an absolute argument
/// replace the repo, so a repo-relative `src/x.rs`, a `--repo`-prefixed path,
/// and an absolute path to the same file all resolve to the same target. The
/// fact store keys on repo-relative, forward-slash paths, so the resolved path
/// is made relative to `--repo` and its separators are normalized to `/`.
fn resolve_explain_file(repo: &std::path::Path, arg: &str) -> Option<String> {
    let candidate = repo.join(arg);
    if !candidate.is_file() {
        return None;
    }
    let relative = match candidate.strip_prefix(repo) {
        Ok(stripped) => stripped.to_path_buf(),
        Err(_) => std::path::PathBuf::from(arg),
    };
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// Print the deterministic evidence dossier for a repo-relative source file,
/// and — with `--llm` — an advisory grounded narrative plus its citation-check
/// stamp.
///
/// This surface is strictly read-only: it opens (or ingests) the fact store and
/// assembles a fact sheet from the same analyses the CLI already exposes, never
/// touching an analysis row, a gate verdict, or a provenance manifest. Analysis
/// `min_revs` is forced to 1 so any single named file can be explained — the
/// default corpus gate would otherwise hide most files from their own dossier.
/// That 1-revision floor also applies to the dossier's hotspot, coupling, and
/// ownership sections, so their numbers can differ from a default `analyze` run
/// that gates low-revision files out.
///
/// Without `--llm`, when this file's own previously generated narrative exists
/// for a now-changed fact sheet, a one-line staleness note is printed. With
/// `--llm`, a missing LLM configuration is a hard error carrying a setup hint.
fn run_explain_file(args: &args::ExplainArgs, repo_relative: &str) -> Result<()> {
    use codelore_lib::cli_api::cache::default_cache_root;
    use codelore_lib::cli_api::enrichment::client::{LlmEnv, resolve_client};
    use codelore_lib::cli_api::enrichment::fact_sheet::FileFactSheet;
    use codelore_lib::cli_api::enrichment::prompt::Lens;
    use codelore_lib::cli_api::enrichment::{cache, engine};

    let cache_root = args.cache_dir.clone().unwrap_or_else(default_cache_root);
    let defect_calibration = codelore_lib::cli_api::quality_gates::resolve_defect_calibration(
        args.defect_calibration.clone(),
        &args.repo,
    )
    .context("resolve defect calibration")?;
    let opts = Options {
        repo_path: args.repo.clone(),
        min_revs: 1,
        defect_calibration,
        allow_foreign_calibration: args.allow_foreign_calibration,
        ..Options::default()
    };
    let repo = GixRepo::open(&args.repo)
        .with_context(|| format!("open git repo at {}", args.repo.display()))?;
    let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &cache_root)
        .context("open or ingest the fact store")?;
    let sheet = FileFactSheet::build(&db, &repo, &opts, repo_relative)
        .with_context(|| format!("build the evidence dossier for {repo_relative}"))?;

    print!("{}", sheet.to_human_text());

    if args.llm {
        let client = resolve_client(&LlmEnv::from_process_env()).context(
            "configure an LLM endpoint — set CODELORE_LLM_MODEL for a local OpenAI-compatible \
             runner (e.g. a model from `ollama list`), or ANTHROPIC_API_KEY for Anthropic; see \
             the CODELORE_LLM_* variables in the docs",
        )?;
        let canonical = sheet.to_canonical_text();
        let values = sheet.numeric_values();
        let result = engine::narrate(
            client.as_ref(),
            Lens::FileDiagnosis,
            repo_relative,
            engine::SheetFacts {
                text: &canonical,
                values: &values,
            },
            &cache_root,
            &args.repo,
            args.llm_refresh,
        )
        .context("generate the advisory narrative")?;
        println!("\n{}", result.narrative);
        println!("{}", engine::stamp(&result));
    } else if let Some(latest) = cache::latest_for_subject(&cache_root, &args.repo, repo_relative)
        && latest.fact_digest != sheet.digest()
    {
        println!("note: cached narrative is stale — evidence changed; re-run with --llm");
    }

    Ok(())
}
