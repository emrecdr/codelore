//! `codelore calibrate-defects` — self-calibrate the code-health smell weights
//! against a repository's own mined defect history.
//!
//! Opens the repo, builds a dedicated in-memory MINING fact store over full
//! history, classifies fix commits with the defect oracle, and traces their
//! deleted pre-image lines back to the introducing commits via AG-SZZ (the
//! CLI-side `GitBlameOrigin` line-origin source). It then scans historical and
//! HEAD code-health, validates structural risk against the mined defects, tunes
//! the eight smell weights when the evidence clears an honesty floor, and writes
//! a `defects.calib.json` artifact.

use anyhow::{Context, Result};
use codelore_lib::cli_api::facts::FactsDb;
use codelore_lib::cli_api::repo::{GixRepo, Repo as _};
use codelore_lib::cli_api::{CodeLoreError, Options};

use crate::args::CalibrateDefectsArgs;

/// Mine a repository's own fix-commit history (AG-SZZ), validate whether
/// `code-health` predicted where the mined defects landed, and — when the
/// evidence clears an honesty floor — tune the eight smell weights to this
/// repository. Writes a `defects.calib.json` artifact.
///
/// Flow: open the repo → build a dedicated MINING `FactsDb` (in-memory, full
/// history, `include_merges = true` so `commit_parents` covers every commit)
/// → classify fix commits with `DefectOracle` → `link_defects` with the
/// CLI-side `GitBlameOrigin` → `band_history` → the HEAD code-health scan
/// with `capture_intensities` read immediately after (the two share the
/// `code_health_biomarkers_v1` temp table — nothing may run between them) →
/// `validate` → `tune_weights` → assemble and save the artifact.
///
/// Progress is reported to stderr per phase, mirroring `calibrate`'s idiom.
pub(crate) fn run_calibrate_defects_cmd(args: &CalibrateDefectsArgs) -> Result<()> {
    use codelore_lib::cli_api::analyses::code_health::run_code_health;
    use codelore_lib::cli_api::quality_gates::ledger::now_utc_ts;
    use codelore_lib::defect_calibration::validate::{
        band_history, capture_intensities, default_weights, tune_weights, validate,
    };
    use codelore_lib::defect_calibration::{self, DefectArtifact, OracleConfig};

    let repo =
        GixRepo::open(&args.repo).with_context(|| format!("open repo {}", args.repo.display()))?;
    ensure_mining_tree_clean(&repo, args.allow_dirty)?;

    eprintln!(
        "calibrate-defects: mining full history of {}",
        args.repo.display()
    );
    let opts = Options {
        repo_path: args.repo.clone(),
        include_merges: true,
        temp_dir: args.temp_dir.clone(),
        ..Options::default()
    };
    opts.validate().context("validate options")?;
    let db = FactsDb::new_in_memory_with_temp_dir(args.temp_dir.as_deref())
        .context("open in-memory mining fact store")?;
    db.ingest(&repo, &opts).context("ingest full history")?;

    let oracle_cfg = OracleConfig::default();
    let (links, mining_stats, commit_dates) = mine_fix_links(&db, &repo, args, &oracle_cfg)?;

    eprintln!("calibrate-defects: scanning historical code-health bands");
    let bands = band_history(&db, &repo, &opts).context("historical band scan")?;

    eprintln!("calibrate-defects: scanning HEAD code-health");
    let head_health = run_code_health(&db, &opts).context("HEAD code-health scan")?;
    // MUST run immediately after the HEAD scan above, before any other
    // analysis call touches `code_health_biomarkers_v1` — see the module
    // doc comment on `capture_intensities`.
    let intensities = capture_intensities(&db).context("capture biomarker intensities")?;

    eprintln!("calibrate-defects: validating structural risk against mined defects");
    let validation = validate(&links, &commit_dates, &bands, &head_health);

    eprintln!("calibrate-defects: tuning smell weights");
    let (train, validation_split) =
        build_train_validation_split(&links, &commit_dates, &intensities);
    let defaults = default_weights();
    let (weights, tuning) = tune_weights(&intensities, &train, &validation_split, &defaults);

    let generated_at = now_utc_ts();
    let vintage = args
        .vintage
        .clone()
        .unwrap_or_else(|| format!("defects-{}", &generated_at[..10]));
    let artifact = DefectArtifact {
        format_version: defect_calibration::DEFECT_FORMAT_VERSION,
        repo_identity: defect_calibration::repo_identity(&args.repo),
        head_at_mining: repo.head_sha().context("resolve HEAD sha")?,
        vintage,
        generated_at,
        oracle: oracle_cfg,
        mining: mining_stats,
        validation,
        weights,
        tuning,
    };
    defect_calibration::save(&artifact, &args.output)
        .with_context(|| format!("write artifact {}", args.output.display()))?;

    eprintln!(
        "calibrate-defects: wrote {} (vintage {})",
        args.output.display(),
        artifact.vintage,
    );
    for line in format_validation_evidence(&artifact) {
        eprintln!("{line}");
    }
    Ok(())
}

/// Render `value` to `decimals` fixed places, or an honest `n/a` when the
/// metric is absent — an artifact mined without both a defect-implicated and a
/// clean file class has no AUC / precision to report, and a silent `0.00`
/// would read as a real, terrible score. Never emits a misleading zero.
fn fmt_metric(value: Option<f64>, decimals: usize) -> String {
    match value {
        Some(x) => format!("{x:.decimals$}"),
        None => "n/a".to_string(),
    }
}

/// One-line tuning verdict from the artifact's [`TuningDecision`]: the honesty
/// signal of whether the mined evidence cleared the tuning floor (smell weights
/// retuned to this repo, with the validation-split AUCs side by side) or the
/// weights were left at their defaults (with the reason the floor named).
fn tuning_verdict(tuning: &codelore_lib::defect_calibration::TuningDecision) -> String {
    use codelore_lib::defect_calibration::TuningDecision;
    match tuning {
        TuningDecision::Applied {
            auc_train,
            auc_validation_default,
            auc_validation_tuned,
        } => format!(
            "weights tuned to this repo (validation AUC {auc_validation_default:.3} \
             -> {auc_validation_tuned:.3}, train {auc_train:.3})"
        ),
        TuningDecision::DefaultsKept { reason, .. } => {
            format!("weights left at defaults ({reason})")
        }
    }
}

/// The compact validation-evidence summary printed after a successful
/// `calibrate-defects` run: the AUC / precision@k / sample sizes / tuning
/// verdict already recorded on the artifact, surfaced at the moment the user
/// looks instead of only inside the JSON. Pure over the artifact (no mining),
/// so the honest-absence rendering is unit-testable. Each returned line is
/// printed verbatim via `eprintln!`, matching the command's progress idiom.
fn format_validation_evidence(
    art: &codelore_lib::defect_calibration::DefectArtifact,
) -> Vec<String> {
    let v = &art.validation;
    let mut lines = Vec::with_capacity(3);
    match v.auc_default {
        Some(auc) => lines.push(format!(
            "calibrate-defects: validation - structural-risk AUC {auc:.3}, \
             precision@10 {}, precision@red {}",
            fmt_metric(v.precision_at_10, 2),
            fmt_metric(v.precision_at_red, 2),
        )),
        None => lines.push(
            "calibrate-defects: validation - not enough defect signal to score \
             structural risk (needs both defect-implicated and clean files)"
                .to_string(),
        ),
    }
    lines.push(format!(
        "calibrate-defects: {} defect-implicated file(s) across {} linked defect(s)",
        v.implicated_files, v.linked_defects,
    ));
    // Mining-guard disclosure: how many fix commits the SZZ guards excluded,
    // so a reader sees the mined-vs-excluded split. These counts live only in
    // the command output (the `MiningStats` guard fields are `#[serde(skip)]`),
    // never in the artifact — so `DEFECT_FORMAT_VERSION` stays put.
    let mining = &art.mining;
    lines.push(format!(
        "calibrate-defects: mining guards - {} fix commit(s) examined, {} excluded as \
         tangled (>{} files or >{} changed lines), {} whole-file deletion(s) skipped as ghost",
        mining.fixes_found,
        mining.fixes_excluded_tangled,
        codelore_lib::defect_calibration::szz::TANGLED_MAX_FILES,
        codelore_lib::defect_calibration::szz::TANGLED_MAX_CHURN,
        mining.ghost_files_skipped,
    ));
    lines.push(format!(
        "calibrate-defects: {}",
        tuning_verdict(&art.tuning)
    ));
    lines
}

/// Refuse to mine from a dirty working tree unless `allow_dirty` opts in.
/// Mining reads only committed state (git history + object-database blobs at
/// HEAD), so uncommitted edits are invisible to it — the artifact describes
/// the commit stamped as `head_at_mining`, not the tree the user is looking
/// at. Surfacing that mismatch loudly (instead of silently mining something
/// other than what is on disk) is the point of this guard.
fn ensure_mining_tree_clean(repo: &GixRepo, allow_dirty: bool) -> Result<()> {
    if !repo.is_worktree_dirty() {
        return Ok(());
    }
    if allow_dirty {
        eprintln!(
            "calibrate-defects: warning: working tree has uncommitted changes; \
             mining reads only committed state, so the artifact describes HEAD, \
             not your uncommitted edits (--allow-dirty set, continuing)"
        );
        return Ok(());
    }
    Err(CodeLoreError::InvalidOptions(
        "working tree has uncommitted changes; mining reads only committed state, \
         so the artifact would describe HEAD rather than your current edits — \
         commit them or pass --allow-dirty to proceed"
            .to_string(),
    )
    .into())
}

/// `(links, mining_stats, commit_dates)` — see [`mine_fix_links`].
type MinedLinks = (
    Vec<codelore_lib::defect_calibration::szz::SzzLink>,
    codelore_lib::defect_calibration::MiningStats,
    std::collections::HashMap<String, String>,
);

/// The oracle + AG-SZZ mining phase: classify fix commits, then trace their
/// deleted pre-image lines back to the commits that introduced them via
/// [`GitBlameOrigin`]. Returns the surviving links, the mining tallies, and
/// the full rev→date map (`link_defects`' clock-skew guard input, reused by
/// `validate`'s band lookups and the temporal train/validation split).
fn mine_fix_links(
    db: &FactsDb,
    repo: &GixRepo,
    args: &CalibrateDefectsArgs,
    oracle_cfg: &codelore_lib::defect_calibration::OracleConfig,
) -> Result<MinedLinks> {
    use codelore_lib::defect_calibration::DefectOracle;
    use codelore_lib::defect_calibration::szz::link_defects;

    eprintln!("calibrate-defects: classifying fix commits");
    let oracle = DefectOracle::new(oracle_cfg).context("build defect oracle")?;
    let window_cutoff = match args.window_days {
        Some(0) => {
            return Err(
                CodeLoreError::InvalidOptions("--window-days must be > 0".to_string()).into(),
            );
        }
        Some(days) => window_cutoff_date(db, days).context("compute window cutoff")?,
        None => None,
    };
    let (fixes, commit_dates, root_fixes_skipped, window_excluded) =
        collect_fixes(db, &oracle, window_cutoff.as_deref()).context("collect fix commits")?;
    eprintln!(
        "calibrate-defects: {} fix commit(s) found ({root_fixes_skipped} root-commit fix(es) \
         skipped — no parent to blame; {window_excluded} outside --window-days excluded)",
        fixes.len(),
    );

    eprintln!("calibrate-defects: linking defects (AG-SZZ)");
    let origin = GitBlameOrigin {
        repo_path: args.repo.clone(),
    };
    let (links, mining_stats) = link_defects(db, repo, &origin, &fixes, &commit_dates)
        .context("link defects to their introducing commits")?;
    eprintln!(
        "calibrate-defects: {} link(s) found ({} file(s) blamed, {} cosmetic line(s) \
         dropped, {} blame failure(s))",
        mining_stats.links_found,
        mining_stats.files_blamed,
        mining_stats.lines_dropped_cosmetic,
        mining_stats.blame_failures,
    );
    Ok((links, mining_stats, commit_dates))
}

/// The `commits.date - INTERVAL '<days> days'` cutoff for `--window-days`,
/// as `CAST(... AS TEXT)` — zero-padded UTC, lexicographically comparable
/// against the same-format dates [`collect_fixes`] reads. `None` when the
/// mining store has no commits at all (empty repo — `MAX(date)` is `NULL`).
fn window_cutoff_date(db: &FactsDb, days: u32) -> Result<Option<String>> {
    let now_anchor = codelore_lib::cli_api::analyses::query::clamped_now_anchor("date");
    let sql =
        format!("SELECT CAST((SELECT {now_anchor} FROM commits) - INTERVAL '{days} days' AS TEXT)");
    db.query_row(&sql, [], |r| r.get::<_, Option<String>>(0))
        .context("query window cutoff")
}

/// `(fixes, commit_dates, root_fixes_skipped, window_excluded_fixes)` —
/// see [`collect_fixes`].
type FixCollection = (
    Vec<(String, String, String)>,
    std::collections::HashMap<String, String>,
    u32,
    u32,
);

/// Collect every commit's date (for the clock-skew guard + band lookups)
/// plus the `(rev, first_parent, date)` triples for fix commits the oracle
/// classifies, restricted to `window_cutoff` when set (only which FIXES are
/// mined is narrowed — a candidate defect-introducing commit may predate the
/// window freely). A classified fix with no first parent (a root commit) has
/// nothing to blame against and is skipped, tallied in the returned count.
fn collect_fixes(
    db: &FactsDb,
    oracle: &codelore_lib::defect_calibration::DefectOracle,
    window_cutoff: Option<&str>,
) -> Result<FixCollection> {
    use std::collections::HashMap;

    let commit_rows: Vec<(String, String, String, bool)> = db
        .prepare("SELECT rev, CAST(date AS TEXT), message, is_merge FROM commits")
        .context("prepare commits query")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })
        .context("run commits query")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collect commits rows")?;

    let first_parents: HashMap<String, String> = db
        .prepare("SELECT rev, parent_rev FROM commit_parents WHERE position = 0")
        .context("prepare first-parent query")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .context("run first-parent query")?
        .collect::<std::result::Result<HashMap<_, _>, _>>()
        .context("collect first-parent rows")?;

    let mut commit_dates = HashMap::with_capacity(commit_rows.len());
    let mut fixes = Vec::new();
    let mut root_fixes_skipped = 0u32;
    let mut window_excluded = 0u32;
    for (rev, date, message, is_merge) in commit_rows {
        if !oracle.is_fix(&message, is_merge) {
            commit_dates.insert(rev, date);
            continue;
        }
        if let Some(cutoff) = window_cutoff
            && date.as_str() < cutoff
        {
            window_excluded += 1;
            commit_dates.insert(rev, date);
            continue;
        }
        match first_parents.get(&rev) {
            Some(parent) => fixes.push((rev.clone(), parent.clone(), date.clone())),
            None => root_fixes_skipped += 1,
        }
        commit_dates.insert(rev, date);
    }
    Ok((fixes, commit_dates, root_fixes_skipped, window_excluded))
}

/// `(train, validation)` — see [`build_train_validation_split`].
type TrainValidationSplit = (Vec<(String, bool)>, Vec<(String, bool)>);

/// Build the `(path, label)` train/validation split `tune_weights` expects.
///
/// Positive rows are one per [`SzzLink`](codelore_lib::defect_calibration::szz::SzzLink)
/// — deliberately NOT deduplicated by path, matching `tune_weights`'s own
/// honesty-floor semantics (it counts `(defect, file)` incidences, not
/// distinct files: a file hit by three separate defects contributes three
/// rows). They are ordered by the FIX commit's date — the spec's "temporal
/// split ... by fix date" — so the older 60% trains and the newer 40%
/// validates: a leakage guard asking "would tuning generalize to the NEXT
/// defect", not "does it fit today's".
///
/// This is deliberately independent of `ValidationMetrics::linked_defects`
/// (the distinct-defect-rev count `validate()` reports) — that field and
/// `tune_weights`'s internal floor count different things over different
/// inputs; feeding one where the other belongs would silently corrupt the
/// honesty floor.
///
/// Negative rows are every file the HEAD scan captured biomarker intensities
/// for that no link ever touched — deduplicated (a file's "never
/// implicated" status is one static fact, not a repeated incidence).
/// Negatives carry no fix date, so they are split in the same 60/40 ratio by
/// a deterministic path-sorted order instead, disjoint between the two
/// splits so no single file's intensity vector is memorized across both.
///
/// Returns `(train, validation)`.
fn build_train_validation_split(
    links: &[codelore_lib::defect_calibration::szz::SzzLink],
    commit_dates: &std::collections::HashMap<String, String>,
    intensities: &std::collections::HashMap<String, [f64; 8]>,
) -> TrainValidationSplit {
    let mut positives: Vec<(&str, &str)> = links
        .iter()
        .filter_map(|link| {
            commit_dates
                .get(&link.fix_rev)
                .map(|date| (date.as_str(), link.path.as_str()))
        })
        .collect();
    positives.sort_unstable();

    let implicated: std::collections::HashSet<&str> =
        links.iter().map(|l| l.path.as_str()).collect();
    let mut negatives: Vec<&str> = intensities
        .keys()
        .map(String::as_str)
        .filter(|path| !implicated.contains(path))
        .collect();
    negatives.sort_unstable();

    let train_share = |n: usize| n * 60 / 100;
    let (pos_train, pos_val) = positives.split_at(train_share(positives.len()));
    let (neg_train, neg_val) = negatives.split_at(train_share(negatives.len()));

    let mut train: Vec<(String, bool)> = pos_train
        .iter()
        .map(|&(_, path)| (path.to_string(), true))
        .collect();
    train.extend(neg_train.iter().map(|&path| (path.to_string(), false)));

    let mut validation: Vec<(String, bool)> = pos_val
        .iter()
        .map(|&(_, path)| (path.to_string(), true))
        .collect();
    validation.extend(neg_val.iter().map(|&path| (path.to_string(), false)));

    (train, validation)
}

/// Production [`LineOriginSource`](codelore_lib::defect_calibration::szz::LineOriginSource):
/// shells `git blame -w -M --porcelain` once per `(rev, path)` pair requested
/// by the AG-SZZ engine, batching every requested line into that single
/// invocation via one `-L` range per contiguous run (multiple `-L` flags in
/// one `git blame` call, never one call per range). `-w` neutralises pure
/// reindentation; `-M` follows within-file line moves so a deleted line that
/// an intermediate commit merely relocated is attributed to its true
/// introducer rather than to the relocating commit (the AG-SZZ genealogy the
/// engine relies on). Detached stdio (`stdin`
/// nulled; `stdout`/`stderr` captured via `Command::output`'s own default
/// piping) — the same child-process invariant `calibrate`'s git children use.
struct GitBlameOrigin {
    repo_path: std::path::PathBuf,
}

impl codelore_lib::defect_calibration::szz::LineOriginSource for GitBlameOrigin {
    fn origins(
        &self,
        rev: &str,
        path: &str,
        lines: &[u32],
    ) -> codelore_lib::cli_api::Result<Vec<(u32, String)>> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let repo_str = self
            .repo_path
            .to_str()
            .ok_or_else(|| CodeLoreError::Analysis("non-UTF-8 repo path".to_string()))?;

        let mut cmd_args: Vec<String> = vec![
            "-C".to_string(),
            repo_str.to_string(),
            "blame".to_string(),
            "-w".to_string(),
            "-M".to_string(),
            "--porcelain".to_string(),
        ];
        for (start, end) in merge_line_ranges(lines) {
            cmd_args.push("-L".to_string());
            cmd_args.push(format!("{start},{end}"));
        }
        cmd_args.push(rev.to_string());
        cmd_args.push("--".to_string());
        cmd_args.push(path.to_string());

        let out = std::process::Command::new("git")
            .args(&cmd_args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| {
                CodeLoreError::Analysis(format!("spawn git blame for {path}@{rev}: {e}"))
            })?;
        if !out.status.success() {
            return Err(CodeLoreError::Analysis(format!(
                "git blame failed for {path}@{rev}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        codelore_lib::defect_calibration::szz::parse_blame_porcelain(&stdout)
    }
}

/// Merge a set of (possibly duplicate, unsorted) 1-based line numbers into
/// the minimal set of contiguous inclusive ranges, sorted ascending —
/// [`GitBlameOrigin`]'s batching so every requested line is covered by ONE
/// `git blame` invocation regardless of how many separate hunks they came
/// from.
fn merge_line_ranges(lines: &[u32]) -> Vec<(u32, u32)> {
    let mut sorted: Vec<u32> = lines.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for line in sorted {
        match ranges.last_mut() {
            Some((_, end)) if line == *end + 1 => *end = line,
            _ => ranges.push((line, line)),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use codelore_lib::defect_calibration::DefectArtifact;
    use codelore_lib::defect_calibration::OracleConfig;
    use codelore_lib::defect_calibration::szz::SzzLink;

    /// Run `git -C <repo> <args>` with a deterministic identity, asserting the
    /// invocation succeeds. When `date` is set it fixes both the author and
    /// committer date so the mined history is reproducible (a fresh fixture
    /// repo inherits no ambient git identity or clock).
    fn git_in(repo: &std::path::Path, args: &[&str], date: Option<&str>) -> std::process::Output {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t");
        if let Some(d) = date {
            cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
        }
        let out = cmd.output().expect("run git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        out
    }

    fn head_sha(repo: &std::path::Path) -> String {
        let out = git_in(repo, &["rev-parse", "HEAD"], None);
        String::from_utf8(out.stdout)
            .expect("HEAD sha is utf-8")
            .trim()
            .to_string()
    }

    /// A relocated line's defect must be blamed on the commit that *introduced*
    /// it, not the one that merely moved it within the same file. The fixture:
    ///   A — introduces a distinctive line near the top of a handler;
    ///   B — moves that exact line (byte-identical) past three stable lines to
    ///       the end of the handler — a genuine relocation, not a reindent
    ///       (which `-w` already neutralises);
    ///   C — a `fix:`-typed commit that deletes the relocated line.
    /// The AG-SZZ engine blames C's deleted pre-image line at C's first parent
    /// (B's tree). Without move-following, blame attributes the line to B (the
    /// relocator); `git blame -M` follows the move back to A (the introducer),
    /// which is the `defect_rev` this asserts. The distinctive line is chosen
    /// to clear `-M`'s default 20-alphanumeric-character move-detection
    /// threshold so a bare `-M` recognises the relocation.
    #[test]
    fn szz_blame_attributes_a_relocated_line_to_its_true_introducer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
        git_in(repo, &["init", "-q", "-b", "main"], None);

        let line = "    let checksum = computePayloadChecksum(inputPayloadBytes);\n";

        // A — introduce the distinctive line near the top of the handler.
        std::fs::write(
            repo.join("src/lib.rs"),
            format!("fn handler() {{\n{line}    validate();\n    persist();\n    respond();\n}}\n"),
        )
        .expect("write A");
        git_in(repo, &["add", "."], None);
        git_in(
            repo,
            &["commit", "-q", "-m", "feat: add request handler"],
            Some("2026-01-01T00:00:00Z"),
        );
        let a = head_sha(repo);

        // B — relocate the exact same line to the end of the handler.
        std::fs::write(
            repo.join("src/lib.rs"),
            format!("fn handler() {{\n    validate();\n    persist();\n    respond();\n{line}}}\n"),
        )
        .expect("write B");
        git_in(repo, &["add", "."], None);
        git_in(
            repo,
            &[
                "commit",
                "-q",
                "-m",
                "refactor: relocate checksum to end of handler",
            ],
            Some("2026-01-02T00:00:00Z"),
        );

        // C — fix that deletes the relocated line.
        std::fs::write(
            repo.join("src/lib.rs"),
            "fn handler() {\n    validate();\n    persist();\n    respond();\n}\n",
        )
        .expect("write C");
        git_in(repo, &["add", "."], None);
        git_in(
            repo,
            &["commit", "-q", "-m", "fix: drop stale checksum line"],
            Some("2026-01-03T00:00:00Z"),
        );
        let c = head_sha(repo);

        let git_repo = GixRepo::open(repo).expect("open fixture repo");
        let opts = Options {
            repo_path: repo.to_path_buf(),
            include_merges: true,
            ..Options::default()
        };
        let db = FactsDb::new_in_memory().expect("in-memory fact store");
        db.ingest(&git_repo, &opts).expect("ingest fixture history");

        let args = CalibrateDefectsArgs {
            repo: repo.to_path_buf(),
            output: repo.join("defects.calib.json"),
            vintage: None,
            window_days: None,
            temp_dir: None,
            allow_dirty: false,
        };
        let (links, _stats, _dates) =
            mine_fix_links(&db, &git_repo, &args, &OracleConfig::default())
                .expect("mine fix links");

        // Without `-M` this vec would carry `defect_rev: <B>` (the relocator)
        // and the assertion would fail; with `-M` the move is followed to A.
        assert_eq!(
            links,
            vec![SzzLink {
                defect_rev: a,
                fix_rev: c,
                path: "src/lib.rs".to_string(),
            }],
        );
    }

    // ─── completion-summary evidence formatting ──────────────────────────────

    /// Build a synthetic artifact carrying `validation`/`tuning` — the only two
    /// fields [`format_validation_evidence`] reads — so the summary formatter
    /// can be exercised without mining a repo.
    fn artifact_with(
        validation: codelore_lib::defect_calibration::ValidationMetrics,
        tuning: codelore_lib::defect_calibration::TuningDecision,
    ) -> DefectArtifact {
        DefectArtifact {
            format_version: codelore_lib::defect_calibration::DEFECT_FORMAT_VERSION,
            repo_identity: "0".repeat(64),
            head_at_mining: "0".repeat(40),
            vintage: "defects-2026-07-20".to_string(),
            generated_at: "2026-07-20T00:00:00Z".to_string(),
            oracle: OracleConfig::default(),
            mining: codelore_lib::defect_calibration::MiningStats::default(),
            validation,
            weights: codelore_lib::defect_calibration::validate::default_weights(),
            tuning,
        }
    }

    #[test]
    fn validation_evidence_summary_reports_auc_precision_and_tuning() {
        use codelore_lib::defect_calibration::{TuningDecision, ValidationMetrics};
        let art = artifact_with(
            ValidationMetrics {
                band_table: vec![],
                auc_default: Some(0.803),
                precision_at_10: Some(0.6),
                precision_at_red: Some(0.75),
                implicated_files: 12,
                linked_defects: 20,
                sample_dates: vec![],
                excluded_no_data: 0,
            },
            TuningDecision::Applied {
                auc_train: 0.812,
                auc_validation_default: 0.780,
                auc_validation_tuned: 0.834,
            },
        );
        let out = format_validation_evidence(&art).join("\n");
        assert!(out.contains("AUC 0.803"), "{out}");
        assert!(out.contains("precision@10 0.60"), "{out}");
        assert!(out.contains("precision@red 0.75"), "{out}");
        assert!(out.contains("12 defect-implicated file(s)"), "{out}");
        assert!(out.contains("20 linked defect(s)"), "{out}");
        assert!(
            out.contains("weights tuned to this repo") && out.contains("0.780 -> 0.834"),
            "{out}"
        );
    }

    #[test]
    fn validation_evidence_summary_honest_absence_never_prints_zero() {
        use codelore_lib::defect_calibration::{TuningDecision, ValidationMetrics};
        let art = artifact_with(
            ValidationMetrics {
                band_table: vec![],
                auc_default: None,
                precision_at_10: None,
                precision_at_red: None,
                implicated_files: 0,
                linked_defects: 0,
                sample_dates: vec![],
                excluded_no_data: 0,
            },
            TuningDecision::DefaultsKept {
                reason: "too few linked defects".to_string(),
                auc_validation_default: None,
                auc_validation_tuned: None,
            },
        );
        let out = format_validation_evidence(&art).join("\n");
        assert!(out.contains("not enough defect signal"), "{out}");
        assert!(out.contains("weights left at defaults"), "{out}");
        assert!(out.contains("too few linked defects"), "{out}");
        // An absent metric must never render as a real-looking zero score.
        assert!(!out.contains("0.00"), "absent metric read as 0.00: {out}");
        assert!(!out.contains("0.000"), "absent metric read as 0.000: {out}");
    }

    #[test]
    fn validation_evidence_summary_discloses_mining_guards() {
        use codelore_lib::defect_calibration::szz::{TANGLED_MAX_CHURN, TANGLED_MAX_FILES};
        use codelore_lib::defect_calibration::{MiningStats, TuningDecision, ValidationMetrics};
        let mut art = artifact_with(
            ValidationMetrics::default(),
            TuningDecision::DefaultsKept {
                reason: "too few linked defects".to_string(),
                auc_validation_default: None,
                auc_validation_tuned: None,
            },
        );
        art.mining = MiningStats {
            fixes_found: 50,
            fixes_excluded_tangled: 4,
            ghost_files_skipped: 3,
            ..MiningStats::default()
        };
        let out = format_validation_evidence(&art).join("\n");
        assert!(out.contains("50 fix commit(s) examined"), "{out}");
        assert!(out.contains("4 excluded as tangled"), "{out}");
        assert!(
            out.contains("3 whole-file deletion(s) skipped as ghost"),
            "{out}"
        );
        // The thresholds are surfaced from the shipped consts, not hard-coded.
        let thresholds =
            format!(">{TANGLED_MAX_FILES} files or >{TANGLED_MAX_CHURN} changed lines");
        assert!(out.contains(&thresholds), "{out}");
    }
}
