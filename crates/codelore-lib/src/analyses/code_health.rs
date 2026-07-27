//! Code Health composite analysis.
//!
//! ```text
//! codehealth(entity) = 100 × (1
//!     - w_sr · structural_risk(biomarkers)
//!     - w_cn · normalize(churn_rate)
//!     - w_au · normalize(author_fragmentation_FV)
//! )
//!
//! defaults: w_sr = 0.50, w_cn = 0.30, w_au = 0.20
//! ```
//!
//! `structural_risk` is a weighted sum over the per-file biomarker table of
//! eight smells (weights in `SMELL_WEIGHTS`, ordered by defect-correlation
//! strength):
//!
//! - complex-method (0.22) — per-file MAX cyclomatic
//! - god-class (0.18) — cognitive × (fan-in + fan-out)
//! - large-method (0.12) — per-file MAX LOC
//! - dry (0.12) — clone count
//! - shotgun-surgery (0.12) — Fisher-significant coupling-partner count
//! - deep-nesting (0.10) — per-file MAX nesting depth
//! - many-args (0.07) — per-file MAX argument count
//! - complex-conditional (0.07) — per-file MAX boolean-operator count
//!
//! Each biomarker carries an intensity ∈ [0,1]. The complexity-driven ones
//! (complex-method, large-method, god-class, dry, deep-nesting, many-args,
//! complex-conditional) are a per-language `PERCENT_RANK` of the file's worst
//! value over the full file set; shotgun-surgery is a `PERCENT_RANK` over the
//! coupled-file set only (no language partition). The per-smell weights sum to
//! 1.0, so `structural_risk` stays in [0,1] and spreads across the file
//! distribution. Smells absent for a file contribute 0, so co-occurrence is
//! implicit — a file flagged by more smells accumulates more weighted terms.
//! Coupling centrality (Fisher-significant pairs from `coupling::run_coupling`)
//! enters once, as the shotgun-surgery biomarker; it is deliberately not also a
//! separate behavioral term. Score range: [0, 100]; higher = healthier. Band
//! (red/yellow/green) is derived from `structural_risk` thresholds; percentile
//! is the per-language `PERCENT_RANK` of `structural_risk` (Alves, Ypma &
//! Visser 2010).
//!
//! Research basis: see `docs/research-foundations.md` entry
//! "code-health" (composite signal developed in `CodeLore`; underlying
//! inputs cite cyclomatic complexity and LOC biomarker intensities (Campbell 2018),
//! churn (Nagappan & Ball 2005), ownership fragmentation (Mockus & Herbsleb 2002),
//! and coupling centrality (Tornhill 2018)).

use std::collections::HashMap;

use duckdb::params;

use crate::analyses::coupling::{run_coupling, run_coupling_scoped};
use crate::facts::FactsDb;
use crate::{CodeLoreError, Options, Result};

/// Where the DRY biomarker sources its per-file clone-family counts. The
/// standalone scan and the gate PROJECTION fingerprint the live working tree;
/// the gate BASELINE reads HEAD-faithful counts from the ingested `clones`
/// table so a working-tree-introduced duplicate no longer appears in both runs
/// and cancels out of the delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneSource {
    /// Fingerprint the live working tree via
    /// [`crate::analyses::clones::run_clones_memoised`]. On a clean tree this
    /// equals HEAD, so `codelore check` and the gate projection both use it.
    WorkingTree,
    /// Read HEAD clone counts from the `clones` table populated at ingest from
    /// HEAD blobs (`facts::ingest::populate_clones_at_head`). The gate baseline
    /// uses it so `baseline_score` is HEAD-faithful and a newly duplicated
    /// function shows as a real negative delta rather than cancelling.
    Head,
}

/// What revision / sources a code-health scan runs against. `head()` resolves
/// to today's HEAD tables so existing behaviour is byte-identical.
#[derive(Debug, Clone)]
pub struct HealthScanCtx {
    /// Complexity source table (HEAD: `"complexity_metrics"`).
    pub complexity_source: String,
    /// Imports source table for god-class fan-in/out (HEAD: `"imports"`).
    pub imports_source: String,
    /// When `Some(ts)`, history terms (churn, author, coupling) are limited to
    /// `commits.date <= ts`.
    pub history_cutoff: Option<String>,
    /// Include the clone/DRY biomarker (true at HEAD; false at a historical rev
    /// where clone detection is unavailable).
    pub include_clones: bool,
    /// Which surface the DRY biomarker counts clones from (ignored when
    /// [`include_clones`](Self::include_clones) is false).
    pub clone_source: CloneSource,
}

impl HealthScanCtx {
    /// The HEAD scan — every source resolves to today's table, DRY included.
    /// Clones are counted from the working tree, matching `codelore check`;
    /// the gate baseline overrides [`clone_source`](Self::clone_source) to
    /// [`CloneSource::Head`].
    #[must_use]
    pub fn head() -> Self {
        Self {
            complexity_source: "complexity_metrics".to_string(),
            imports_source: "imports".to_string(),
            history_cutoff: None,
            include_clones: true,
            clone_source: CloneSource::WorkingTree,
        }
    }

    /// Convenience alias for `head()`. Named for call-site clarity in analyses
    /// that construct their own context rather than inheriting one from a caller.
    #[must_use]
    pub fn head_default() -> Self {
        Self::head()
    }
}

/// The structural-risk smell weights, ordered by empirical defect-correlation
/// strength. The single source of truth for the `file_structural` CASE (which
/// is generated from this table by [`smell_weights_case`]) and the no-DRY
/// renormalization divisor. Weights sum to exactly 1.0 so `structural_risk`
/// stays in [0,1].
///
/// `pub(crate)` (rather than private) so `defect_calibration::validate` can
/// key its captured biomarker intensities and its Rust-side risk formula to
/// this exact order — see `defect_calibration::validate::default_weights`.
pub(crate) const SMELL_WEIGHTS: &[(&str, f64)] = &[
    ("complex-method", 0.22),
    ("god-class", 0.18),
    ("large-method", 0.12),
    ("dry", 0.12),
    ("shotgun-surgery", 0.12),
    ("deep-nesting", 0.10),
    ("many-args", 0.07),
    ("complex-conditional", 0.07),
];

/// Divisor appended to the `structural_risk` SUM when the DRY biomarker is
/// excluded: the seven remaining weights sum to 0.88, so dividing by 0.88
/// renormalizes the risk scale back to 1.0. Empty at HEAD. Documents the
/// default-weights value; runs with a defect-calibration artifact recompute
/// the divisor from the active DRY weight instead.
const STRUCTURAL_SCALE_NO_DRY: &str = " / 0.88";

/// [`SMELL_WEIGHTS`] as owned `(name, weight)` tuples — the default argument
/// for [`smell_weights_case`], and (via
/// `defect_calibration::validate::default_weights`) the baseline the weight
/// tuner steps from.
pub(crate) fn default_smell_weights() -> Vec<(String, f64)> {
    SMELL_WEIGHTS
        .iter()
        .map(|&(name, w)| (name.to_string(), w))
        .collect()
}

/// Build the `SUM(intensity * CASE smell … END)` weight expression from
/// `weights` — [`SMELL_WEIGHTS`] converted on the default path, or a
/// defect-calibration artifact's tuned set. Smells absent for a file
/// contribute 0, so co-occurrence is implicit — a file flagged by more smells
/// accumulates more weighted terms.
fn smell_weights_case(weights: &[(String, f64)]) -> String {
    use std::fmt::Write as _;
    let mut case = String::from("CASE smell");
    for (smell, weight) in weights {
        // Names and values come from the compile-time SMELL_WEIGHTS table or
        // from an artifact whose weights `active_weights` has already pinned
        // to exactly those names, so direct interpolation carries no
        // injection surface. Writing to a String is infallible, so the fmt
        // Result is discarded.
        let _ = write!(case, "\n                WHEN '{smell}' THEN {weight}");
    }
    case.push_str("\n                ELSE 0.0\n            END");
    case
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeHealthRow {
    pub path: String,
    pub cognitive: f64,
    pub score: f64,           // 0..=100; higher = healthier
    pub structural_risk: f64, // 0..=1; higher = worse
    pub percentile: f64, // 0..=1; per-language self-relative rank of structural_risk (1 = riskiest)
    pub band: String,    // "red" | "yellow" | "green"
    /// Corpus-relative percentile: the file's WORST raw dimension
    /// (`cyclomatic`, `cognitive`, `sloc`, `nargs`, `max_nesting`) versus a
    /// reference corpus, in `0..=1`. `None` when no calibration artifact is
    /// active or the file's language / metrics aren't covered — an additive
    /// lens that never perturbs the shipped self-relative fields above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_percentile: Option<f64>,
    /// At least one of the file's raw metrics exceeded the corpus maximum
    /// breakpoint (its `corpus_percentile` saturated at `1.0`). Serialized only
    /// when true.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub beyond_corpus: bool,
    /// Wilson 95% lower / upper bound for `corpus_percentile`, reflecting the
    /// sampling uncertainty of the reference pool (a finite corpus sample, not
    /// the population) — not measurement error in the file's own metrics. `Some`
    /// exactly when `corpus_percentile` is `Some`; the pool size is the file
    /// language's pooled per-function sample count. Serialized only when present,
    /// so a run without an active corpus stays byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_percentile_ci_low: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_percentile_ci_high: Option<f64>,
}

const SQL: &str = "
    WITH file_cognitive AS (
        SELECT path, MAX(cognitive) AS cognitive
        FROM {cm_src}
        GROUP BY path
    ),
    file_churn AS (
        SELECT path, COALESCE(SUM(loc_added), 0) + COALESCE(SUM(loc_deleted), 0) AS churn
        FROM {src}
        GROUP BY path
    ),
    file_revs AS (
        -- (rev, path) is the changes PK; COUNT(rev) == COUNT(DISTINCT rev)
        -- per path. Plain COUNT skips DuckDB's distinct-tracking overhead.
        SELECT path, COUNT(rev) AS revs
        FROM {src}
        GROUP BY path
        HAVING revs >= ?
    ),
    author_revs AS (
        SELECT
            c.path,
            commits.canonical_author AS author,
            -- (rev, path) is the changes PK so rev is unique within each
            -- (path, author) group. Plain COUNT skips DuckDB's
            -- distinct-tracking overhead.
            COUNT(c.rev) AS revs
        FROM {src} c
        INNER JOIN commits ON c.rev = commits.rev
        GROUP BY c.path, commits.canonical_author
    ),
    file_fv AS (
        SELECT
            ar.path,
            1.0 - SUM(POWER(ar.revs::DOUBLE / NULLIF(t.total, 0), 2)) AS fv
        FROM author_revs ar
        INNER JOIN (SELECT path, SUM(revs) AS total FROM author_revs GROUP BY path) t
            ON ar.path = t.path
        GROUP BY ar.path
    ),
    -- Weighted sum of per-file biomarker intensities (each a per-language
    -- PERCENT_RANK in [0,1]; smells absent for a file contribute 0). The
    -- weights sum to 1.0 and are ordered by empirical defect-correlation
    -- strength, so structural risk stays in [0,1] and spreads across the file
    -- distribution instead of saturating at the ceiling. Co-occurrence is
    -- implicit: a file flagged by more smells accumulates more weighted terms.
    file_structural AS (
        SELECT
            path,
            LEAST(1.0, SUM(intensity * {smell_weights_case}){structural_scale}) AS structural_risk
        FROM code_health_biomarkers_v1
        GROUP BY path
    ),
    -- Coupling centrality enters the composite once, as the shotgun-surgery
    -- biomarker inside structural_risk. It is deliberately NOT also added as a
    -- separate behavioral term here — that would double-count the same signal.
    joined AS (
        SELECT
            fc.path,
            fc.cognitive,
            COALESCE(fch.churn, 0) AS churn,
            COALESCE(ffv.fv, 0.0) AS fv,
            COALESCE(fs.structural_risk, 0.0) AS structural_risk
        FROM file_cognitive fc
        INNER JOIN file_revs fr ON fc.path = fr.path
        LEFT JOIN file_churn fch ON fc.path = fch.path
        LEFT JOIN file_fv ffv ON fc.path = ffv.path
        LEFT JOIN file_structural fs ON fc.path = fs.path
    ),
    normalized AS (
        SELECT
            path,
            cognitive,
            churn,
            fv,
            structural_risk,
            CASE WHEN MAX(churn) OVER () > 0 THEN churn::DOUBLE / MAX(churn) OVER () ELSE 0 END AS n_cn,
            fv AS n_au
        FROM joined
    ),
    scored AS (
        SELECT
            path,
            cognitive,
            structural_risk,
            GREATEST(0.0, LEAST(100.0,
                100.0 * (1.0 - 0.50 * structural_risk - 0.30 * n_cn - 0.20 * n_au)
            )) AS score,
            CASE lower(split_part(path, '.', -1))
                WHEN 'rs' THEN 'rust'
                WHEN 'py' THEN 'python' WHEN 'pyi' THEN 'python'
                WHEN 'java' THEN 'java'
                WHEN 'js' THEN 'javascript' WHEN 'jsx' THEN 'javascript'
                WHEN 'mjs' THEN 'javascript' WHEN 'cjs' THEN 'javascript'
                WHEN 'ts' THEN 'typescript' WHEN 'tsx' THEN 'typescript'
                ELSE 'other'
            END AS lang
        FROM normalized
    )
    SELECT
        path,
        cognitive,
        score,
        structural_risk,
        PERCENT_RANK() OVER (PARTITION BY lang ORDER BY structural_risk) AS percentile,
        -- Absolute structural_risk thresholds. Deliberately self-relative:
        -- the corpus-relative lens is an ADDITIVE second reading and never
        -- moves these bands. red = high on a majority of the weighted
        -- smell mass.
        CASE
            WHEN structural_risk >= {risk_red_min} THEN 'red'
            WHEN structural_risk >= {risk_yellow_min} THEN 'yellow'
            ELSE 'green'
        END AS band
    FROM scored
    ORDER BY score ASC, path ASC
    LIMIT ?
";

/// A changes view limited to commits at/-before a cutoff timestamp, so
/// history-derived terms (churn, author fragmentation, coupling) are rev-scoped.
/// The `{ts}` cutoff is inlined as a quoted literal (single quotes doubled)
/// rather than bound as a `?` parameter: `DuckDB` rejects prepared parameters
/// inside a `CREATE VIEW` statement ("this type of statement can't be prepared").
///
/// This view reads raw `changes` (not the lineage-rewritten source), so churn /
/// author terms built on it lose rename-awareness when a cutoff is combined with
/// `--use-canonical-lineage`. Out of scope: the timeline consumer uses a cutoff
/// without lineage (the primary path), matching `run_coupling_scoped`'s own
/// cutoff limitation.
const CHANGES_AT_TS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY VIEW changes_at_ts AS
    SELECT c.* FROM changes c
    INNER JOIN commits ON commits.rev = c.rev
    WHERE commits.date <= CAST('{ts}' AS TIMESTAMP)
";

/// DDL for the temporary centrality table that backs the `shotgun-surgery`
/// biomarker (its intensity is the `PERCENT_RANK` of a file's Fisher-significant
/// coupling-partner count). Computed in Rust from the Fisher-filtered output of
/// `coupling::run_coupling`, then materialized in a session-local temp table so
/// `SHOTGUN_INSERT` can read it.
const CENTRALITY_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE coupling_centrality_v1 (
        path TEXT PRIMARY KEY,
        centrality INTEGER NOT NULL
    )
";

/// DDL for the per-file biomarker table. NOTE: this session-local table is
/// materialized as a side effect of `run_code_health` and is also read by an
/// EXTERNAL consumer — `analyses::refactoring_targets` queries it (after
/// calling `run_code_health`) to find each file's dominant smell. It is
/// therefore a cross-analysis contract: it must stay session-scoped and
/// readable after `run_code_health` returns — do not make it private, drop it
/// on exit, or rename its columns without updating that consumer.
const BIOMARKERS_DDL: &str = "
    CREATE OR REPLACE TEMPORARY TABLE code_health_biomarkers_v1 (
        path    TEXT   NOT NULL,
        smell   TEXT   NOT NULL,
        intensity DOUBLE NOT NULL
    )
";

/// Populate function-level structural biomarkers from the raw
/// `complexity_metrics` snapshot: complex-method (cyclomatic), large-method
/// (LOC), deep-nesting (nesting depth), many-args (argument count), and
/// complex-conditional (boolean-operator count). Intensity is the per-language
/// `PERCENT_RANK` of the function metric, rolled up to the file by `MAX`.
/// Language is derived from the file extension; no stored language column
/// exists in `complexity_metrics` so the CASE expression mirrors the one
/// in the scored CTE of the main code-health SQL.
const BIOMARKERS_INSERT: &str = "
    INSERT INTO code_health_biomarkers_v1 (path, smell, intensity)
    WITH lang_fn AS (
        SELECT
            path,
            name,
            cyclomatic,
            loc,
            max_nesting,
            nargs,
            bool_ops,
            CASE lower(split_part(path, '.', -1))
                WHEN 'rs'  THEN 'rust'
                WHEN 'py'  THEN 'python'
                WHEN 'pyi' THEN 'python'
                WHEN 'java' THEN 'java'
                WHEN 'js'  THEN 'javascript'
                WHEN 'jsx' THEN 'javascript'
                WHEN 'mjs' THEN 'javascript'
                WHEN 'cjs' THEN 'javascript'
                WHEN 'ts'  THEN 'typescript'
                WHEN 'tsx' THEN 'typescript'
                ELSE 'other'
            END AS lang
        FROM {cm_src}
        WHERE cyclomatic IS NOT NULL
            AND loc IS NOT NULL
    ),
    -- Aggregate to the file FIRST (worst function per file), THEN rank files.
    -- Ranking functions and taking the per-file MAX saturates: any file with
    -- enough functions has one in the top percentile, so nearly every file
    -- scored ~1.0. Ranking files against files spreads the intensity uniformly.
    file_metric AS (
        SELECT
            path,
            lang,
            MAX(cyclomatic) AS file_cx,
            MAX(loc) AS file_loc,
            MAX(max_nesting) AS file_nesting,
            MAX(nargs) AS file_nargs,
            MAX(bool_ops) AS file_bool_ops
        FROM lang_fn
        GROUP BY path, lang
    ),
    ranked AS (
        SELECT
            path,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_cx) AS cx_i,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_loc) AS loc_i,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_nesting) AS nesting_i,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_nargs) AS nargs_i,
            PERCENT_RANK() OVER (PARTITION BY lang ORDER BY file_bool_ops) AS bool_ops_i
        FROM file_metric
    )
    SELECT path, 'complex-method' AS smell, cx_i AS intensity FROM ranked
    UNION ALL
    SELECT path, 'large-method' AS smell, loc_i AS intensity FROM ranked
    UNION ALL
    SELECT path, 'deep-nesting' AS smell, nesting_i AS intensity FROM ranked
    UNION ALL
    SELECT path, 'many-args' AS smell, nargs_i AS intensity FROM ranked
    UNION ALL
    SELECT path, 'complex-conditional' AS smell, bool_ops_i AS intensity FROM ranked
";

/// Shotgun Surgery / Divergent Change: a file that co-changes with many
/// Fisher-significant partners is definitionally a temporal smell. Reuses the
/// already-materialized centrality table; intensity = self-relative rank.
const SHOTGUN_INSERT: &str = "
    INSERT INTO code_health_biomarkers_v1 (path, smell, intensity)
    SELECT path, 'shotgun-surgery' AS smell,
           PERCENT_RANK() OVER (ORDER BY centrality) AS intensity
    FROM coupling_centrality_v1
    WHERE centrality > 0
";

fn materialize_biomarkers(db: &FactsDb, opts: &Options, cx: &HealthScanCtx) -> Result<()> {
    db.conn()
        .execute(BIOMARKERS_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create biomarker temp table: {e}")))?;
    let biomarkers_insert = BIOMARKERS_INSERT.replace("{cm_src}", &cx.complexity_source);
    db.conn()
        .execute(&biomarkers_insert, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert complexity biomarkers: {e}")))?;
    db.conn()
        .execute(SHOTGUN_INSERT, [])
        .map_err(|e| CodeLoreError::Analysis(format!("insert shotgun-surgery biomarkers: {e}")))?;

    // God-class and DRY are SPARSE smells (present for few files). Rank each
    // file's raw metric over the FULL per-language file universe — the same set
    // complex-method / large-method rank over — with absent files contributing
    // 0. This keeps a lone or tied occurrence high (it still ranks above the
    // zero-majority) instead of collapsing to a 0 rank, and aligns god-class /
    // dry with the complex/large per-file percentile scheme (min-max before).
    // (shotgun-surgery ranks over its own coupled-file set — see SHOTGUN_INSERT.)
    //
    // `--rows N` MUST NOT propagate into `run_god_classes`: the biomarker set
    // needs ALL god classes, not the user's output truncation.
    let gods = crate::analyses::god_classes::run_god_classes_scoped(
        db,
        &opts.with_no_row_limit(),
        &cx.complexity_source,
        &cx.imports_source,
    )?;
    let god_by_path: HashMap<String, f64> =
        gods.iter().map(|g| (g.path.clone(), g.god_score)).collect();

    let dry_counts: HashMap<String, u32> = match (cx.include_clones, cx.clone_source) {
        (false, _) => HashMap::new(),
        (true, CloneSource::WorkingTree) => {
            // Memoised so the agent-loop gate's projection walks the working
            // tree once, not twice. The first scan populates the per-`FactsDb`
            // memo; every other caller (which scores a repo once) sees
            // identical rows and identical cost.
            let clones = crate::analyses::clones::run_clones_memoised(db, opts)?;
            let mut m: HashMap<String, u32> = HashMap::new();
            for c in clones.iter() {
                *m.entry(c.entity.clone()).or_insert(0) += 1;
            }
            m
        }
        // Gate baseline: HEAD-faithful counts from the ingested `clones` table
        // so a working-tree-introduced duplicate is present only in the
        // projection's working-tree walk, not this run — the delta stops
        // cancelling. On a clean tree these equal the working-tree walk, so the
        // unchanged-tree delta stays exactly 0.0.
        (true, CloneSource::Head) => crate::analyses::clones::head_clone_counts(db)?,
    };

    // Full file universe (files with complexity data), grouped by language —
    // the same universe the SQL-side complex/large biomarkers rank over.
    let universe_sql = "SELECT DISTINCT path FROM {cm_src} \
         WHERE cyclomatic IS NOT NULL AND loc IS NOT NULL"
        .replace("{cm_src}", &cx.complexity_source);
    let universe = crate::analyses::query::query_map_collect(
        db,
        &universe_sql,
        [],
        "biomarker-universe",
        |r| r.get::<_, String>(0),
    )?;
    let mut by_lang: HashMap<&'static str, Vec<String>> = HashMap::new();
    for path in universe {
        let lang =
            crate::complexity::Tier1Language::from_path(&path).map_or("other", |l| l.as_str());
        by_lang.entry(lang).or_default().push(path);
    }

    let mut stmt = db
        .conn()
        .prepare("INSERT INTO code_health_biomarkers_v1 (path, smell, intensity) VALUES (?, ?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare biomarker insert: {e}")))?;
    for files in by_lang.values() {
        if files.len() <= 1 {
            continue; // PERCENT_RANK is degenerate for a single-file language
        }
        let denom = f64::from(u32::try_from(files.len() - 1).unwrap_or(u32::MAX));
        let smells: &[&str] = if cx.include_clones {
            &["god-class", "dry"]
        } else {
            &["god-class"]
        };
        for &smell in smells {
            let vals: Vec<f64> = files
                .iter()
                .map(|p| {
                    if smell == "god-class" {
                        god_by_path.get(p).copied().unwrap_or(0.0)
                    } else {
                        dry_counts.get(p).map_or(0.0, |n| f64::from(*n))
                    }
                })
                .collect();
            for (path, &v) in files.iter().zip(vals.iter()) {
                if v <= 0.0 {
                    continue; // only files that HAVE the smell get a row
                }
                let less = vals.iter().filter(|&&x| x < v).count();
                let intensity = f64::from(u32::try_from(less).unwrap_or(u32::MAX)) / denom;
                stmt.execute(params![path, smell, intensity])
                    .map_err(|e| CodeLoreError::Analysis(format!("{smell} biomarker: {e}")))?;
            }
        }
    }
    Ok(())
}

/// Build the centrality temp table from Fisher-significant coupling pairs.
/// Each path appears once with `centrality = count of pairs that include it`.
/// When `cx.history_cutoff` is set, coupling is restricted to the already-
/// materialized `changes_at_ts` view; at HEAD (cutoff None) the standard
/// memoised [`run_coupling`] path is taken — output is byte-identical.
fn materialize_centrality(db: &FactsDb, opts: &Options, cx: &HealthScanCtx) -> Result<()> {
    // `--rows N` MUST NOT propagate into the inner coupling query — the
    // centrality term needs the FULL coupled-pair graph, not the user's
    // output truncation. See `Options::with_no_row_limit` for the full
    // bug narrative.
    let pairs = if cx.history_cutoff.is_some() {
        run_coupling_scoped(db, &opts.with_no_row_limit(), "changes_at_ts")?
    } else {
        run_coupling(db, &opts.with_no_row_limit())?
    };

    // Count Fisher-significant partners per path. Each pair contributes to
    // both endpoints' centrality.
    let mut counts: HashMap<String, u32> = HashMap::new();
    for p in &pairs {
        *counts.entry(p.entity_a.clone()).or_insert(0) += 1;
        *counts.entry(p.entity_b.clone()).or_insert(0) += 1;
    }

    db.conn()
        .execute(CENTRALITY_DDL, [])
        .map_err(|e| CodeLoreError::Analysis(format!("create centrality temp table: {e}")))?;

    if counts.is_empty() {
        return Ok(()); // Nothing to insert; LEFT JOIN handles absence.
    }

    // Bulk INSERT via prepared statement — small N (typically <= 100s of
    // distinct paths), so per-row insert is fine without the Appender.
    let mut stmt = db
        .conn()
        .prepare("INSERT INTO coupling_centrality_v1 (path, centrality) VALUES (?, ?)")
        .map_err(|e| CodeLoreError::Analysis(format!("prepare centrality insert: {e}")))?;
    for (path, count) in &counts {
        stmt.execute(params![path, *count])
            .map_err(|e| CodeLoreError::Analysis(format!("centrality row insert: {e}")))?;
    }
    Ok(())
}

/// The raw per-metric corpus names, paired with the `complexity_metrics` column
/// each rolls up from via `MAX`. Order is the `SELECT` order in
/// [`FILE_AGGREGATES_SQL`]; index 0 is `path`.
const CORPUS_METRICS: &[&str] = &["cyclomatic", "cognitive", "sloc", "nargs", "max_nesting"];

/// Widen a raw metric aggregate to `f64` for corpus lookup. These are
/// `MAX(INTEGER)` complexity counts (cyclomatic, sloc, nargs, …) — thousands at
/// most, orders of magnitude below `2^53` — so the conversion is exact and the
/// `cast_precision_loss` lint is a false positive.
#[allow(clippy::cast_precision_loss)]
fn metric_to_f64(n: i64) -> f64 {
    n as f64
}

/// Per-file MAX of each raw driver the corpus lens compares. Grouped by path so
/// each output row joins to exactly one aggregate. Columns follow
/// [`CORPUS_METRICS`] order after `path`. Read as nullable — a metric absent for
/// every function of a file (e.g. a NULL column on a legacy row) yields NULL and
/// is skipped in the lookup rather than treated as zero.
const FILE_AGGREGATES_SQL: &str = "
    SELECT
        path,
        MAX(cyclomatic)  AS cyclomatic,
        MAX(cognitive)   AS cognitive,
        MAX(sloc)        AS sloc,
        MAX(nargs)       AS nargs,
        MAX(max_nesting) AS max_nesting
    FROM {cm_src}
    GROUP BY path
";

/// One file's worst-dimension corpus percentile against `art`.
///
/// For each raw metric the file has a value for, look up its corpus percentile
/// and keep the MAX — the file's worst standing versus the corpus. `beyond` is
/// the OR of the per-metric beyond-corpus flags. Returns `None` (no lens) when
/// the language is unknown to the artifact, is pooled below the sample floor, or
/// none of the file's metrics resolve — the additive-absence contract.
fn file_corpus_lens(
    art: &crate::calibration::CalibrationArtifact,
    language: &str,
    metrics: &[(&str, Option<f64>)],
) -> Option<(f64, bool)> {
    let mut worst: Option<f64> = None;
    let mut beyond = false;
    for &(metric, value) in metrics {
        let Some(value) = value else { continue };
        if let Some(cp) = crate::calibration::percentile(art, language, metric, value) {
            worst = Some(worst.map_or(cp.p, |w: f64| w.max(cp.p)));
            beyond |= cp.beyond_corpus;
        }
    }
    worst.map(|p| (p, beyond))
}

/// Additive corpus-percentile pass: after the shipped SQL builds `rows`, join a
/// per-language, per-file corpus percentile onto each. A pure post-pass — it
/// reads only the raw complexity aggregates and never touches the score /
/// band / percentile the SQL already computed, so a run without an active
/// artifact leaves every row byte-identical to today.
fn apply_corpus_lens(
    db: &FactsDb,
    opts: &Options,
    cx: &HealthScanCtx,
    rows: &mut [CodeHealthRow],
) -> Result<()> {
    let Some(art) = crate::calibration::load_active_artifact(opts)? else {
        return Ok(()); // No artifact active → every row keeps its absent lens.
    };

    // Per-file MAX of each raw driver, keyed by path. NULL metrics stay `None`
    // and are skipped in the lookup.
    let aggregates_sql = FILE_AGGREGATES_SQL.replace("{cm_src}", &cx.complexity_source);
    let mut stmt = db
        .conn()
        .prepare(&aggregates_sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare corpus aggregates: {e}")))?;
    let mut by_path: HashMap<String, [Option<f64>; 5]> = HashMap::new();
    let aggregate_rows = stmt
        .query_map([], |r| {
            let path = r.get::<_, String>(0)?;
            let mut vals = [None; 5];
            for (i, slot) in vals.iter_mut().enumerate() {
                // Columns are INTEGER (or NULL); read as optional i64 then widen.
                *slot = r.get::<_, Option<i64>>(i + 1)?.map(metric_to_f64);
            }
            Ok((path, vals))
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query corpus aggregates: {e}")))?;
    for row in aggregate_rows {
        let (path, vals) =
            row.map_err(|e| CodeLoreError::Analysis(format!("collect corpus aggregates: {e}")))?;
        by_path.insert(path, vals);
    }

    for row in rows.iter_mut() {
        let Some(language) = crate::complexity::Tier1Language::from_path(&row.path) else {
            continue; // Not a Tier-1 language → no corpus comparison.
        };
        let Some(vals) = by_path.get(&row.path) else {
            continue; // No complexity aggregate for this path.
        };
        let metrics: Vec<(&str, Option<f64>)> = CORPUS_METRICS
            .iter()
            .zip(vals.iter())
            .map(|(&m, &v)| (m, v))
            .collect();
        if let Some((p, beyond)) = file_corpus_lens(&art, language.as_str(), &metrics) {
            row.corpus_percentile = Some(p);
            row.beyond_corpus = beyond;
            // Wilson interval on the SAME percentile estimate, at the honest
            // pool size = the language's pooled per-function sample count. The
            // language cleared the trust floor inside `file_corpus_lens`, so the
            // same-floored `language_sample_functions` resolves to `Some` here.
            if let Some(n) = crate::calibration::language_sample_functions(&art, language.as_str())
            {
                let (lo, hi) = crate::stats::wilson_ci_from_proportion(
                    p,
                    u32::try_from(n).unwrap_or(u32::MAX),
                );
                row.corpus_percentile_ci_low = Some(lo);
                row.corpus_percentile_ci_high = Some(hi);
            }
        }
    }
    Ok(())
}

#[tracing::instrument(name = "code-health", skip_all, fields(min_revs = opts.min_revs))]
pub fn run_code_health(db: &FactsDb, opts: &Options) -> Result<Vec<CodeHealthRow>> {
    // Route complexity reads through the grouped table when `--group-file` is
    // active (the `grouped_complexity` contract), so the cognitive + biomarker
    // CTEs key on the same group names the `changes` history is rewritten to.
    // Without grouping this resolves to `complexity_metrics`, keeping HEAD
    // output byte-identical.
    let cx = HealthScanCtx {
        complexity_source: crate::analyses::grouped_complexity::source_table(opts).to_string(),
        ..HealthScanCtx::head()
    };
    run_code_health_scoped(db, opts, &cx)
}

/// Code health against the sources named by `cx`. `cx = HealthScanCtx::head()`
/// reproduces the HEAD analysis byte-for-byte.
pub fn run_code_health_scoped(
    db: &FactsDb,
    opts: &Options,
    cx: &HealthScanCtx,
) -> Result<Vec<CodeHealthRow>> {
    crate::analyses::lineage::materialize_source(db, opts)?;
    let src_owned;
    let src: &str = if let Some(ts) = &cx.history_cutoff {
        // Escape single quotes (SQL-standard doubling) before inlining — the
        // literal cannot be a bound `?` parameter inside a CREATE VIEW.
        let cutoff_sql = CHANGES_AT_TS_DDL.replace("{ts}", &ts.replace('\'', "''"));
        db.conn()
            .execute(&cutoff_sql, [])
            .map_err(|e| CodeLoreError::Analysis(format!("create changes_at_ts view: {e}")))?;
        src_owned = "changes_at_ts".to_string();
        &src_owned
    } else {
        crate::analyses::lineage::source_table(opts)
    };
    materialize_centrality(db, opts, cx)?;
    materialize_biomarkers(db, opts, cx)?;

    let cm_src = &cx.complexity_source;
    // Tuned smell weights from an opt-in defect-calibration artifact replace
    // the built-in defaults for the whole scoring pipeline; without the flag
    // this resolves to `None` (no filesystem read) and the run is untouched.
    let tuned = crate::defect_calibration::active_weights(opts)?;
    let weights = tuned
        .as_ref()
        .map_or_else(default_smell_weights, |(w, _)| w.clone());
    let structural_scale_owned;
    let structural_scale: &str = if cx.include_clones {
        ""
    } else if let Some((w, _)) = &tuned {
        // Recompute the no-DRY renormalization divisor from the active DRY
        // weight (the const below documents the default-weights value).
        let dry = w.iter().find(|(n, _)| n == "dry").map_or(0.0, |(_, v)| *v);
        let divisor = 1.0 - dry;
        if divisor > f64::EPSILON {
            structural_scale_owned = format!(" / {divisor}");
            &structural_scale_owned
        } else {
            // Degenerate hand-crafted artifact putting all weight on DRY:
            // skip the rescale rather than divide by zero — LEAST(1.0, …)
            // still caps the risk.
            ""
        }
    } else {
        STRUCTURAL_SCALE_NO_DRY
    };
    let sql = SQL
        .replace("{smell_weights_case}", &smell_weights_case(&weights))
        .replace("{src}", src)
        .replace("{cm_src}", cm_src)
        .replace("{structural_scale}", structural_scale)
        .replace("{risk_red_min}", &crate::bands::RISK_RED_MIN.to_string())
        .replace(
            "{risk_yellow_min}",
            &crate::bands::RISK_YELLOW_MIN.to_string(),
        );
    let row_limit: i64 = opts.rows_limit.map_or(i64::MAX, i64::from);
    crate::analyses::query::explain_if_requested(
        db,
        &sql,
        params![opts.min_revs, row_limit],
        "code-health",
        opts,
    )?;
    let mut stmt = db
        .conn()
        .prepare(&sql)
        .map_err(|e| CodeLoreError::Analysis(format!("prepare code-health: {e}")))?;
    let rows = stmt
        .query_map(params![opts.min_revs, row_limit], |r| {
            Ok(CodeHealthRow {
                path: r.get::<_, String>(0)?,
                cognitive: r.get::<_, f64>(1)?,
                score: r.get::<_, f64>(2)?,
                structural_risk: r.get::<_, f64>(3)?,
                percentile: r.get::<_, f64>(4)?,
                band: r.get::<_, String>(5)?,
                // Populated by the additive corpus pass below (or left absent).
                corpus_percentile: None,
                beyond_corpus: false,
                corpus_percentile_ci_low: None,
                corpus_percentile_ci_high: None,
            })
        })
        .map_err(|e| CodeLoreError::Analysis(format!("query code-health: {e}")))?;
    let mut rows = rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| CodeLoreError::Analysis(format!("collect code-health: {e}")))?;

    apply_corpus_lens(db, opts, cx, &mut rows)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    #[test]
    fn head_ctx_defaults_to_head_tables() {
        let c = super::HealthScanCtx::head();
        assert_eq!(c.complexity_source, "complexity_metrics");
        assert_eq!(c.imports_source, "imports");
        assert!(c.history_cutoff.is_none());
        assert!(c.include_clones);
    }

    #[test]
    fn smell_weights_sum_to_one() {
        // The composite normalizes on the invariant that the smell weights sum
        // to exactly 1.0, so structural_risk stays in [0,1]. Guards against a
        // future weight edit that silently breaks the scale.
        let sum: f64 = super::SMELL_WEIGHTS.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "SMELL_WEIGHTS must sum to 1.0, got {sum}"
        );
    }

    use crate::calibration::{
        CALIBRATION_FORMAT_VERSION, CalibrationArtifact, LanguageTable, MetricQuantiles,
        QUANTILE_POINTS, Stratum,
    };

    /// A `QUANTILE_POINTS`-long ramp where `q[i] == i`, so a metric value `v`
    /// resolves to corpus percentile `v / (QUANTILE_POINTS - 1)`. With
    /// `QUANTILE_POINTS == 1001` the last index is 1000, so value 750 → 0.750.
    #[allow(clippy::cast_precision_loss)]
    fn index_ramp() -> Vec<f64> {
        (0..QUANTILE_POINTS).map(|i| i as f64).collect()
    }

    /// One-language artifact whose `cyclomatic` metric is the index ramp and is
    /// pooled above the sample floor.
    fn ramp_artifact(language: &str) -> CalibrationArtifact {
        CalibrationArtifact {
            format_version: CALIBRATION_FORMAT_VERSION,
            corpus_vintage: "test-ramp".to_string(),
            generated_at: "2026-07-12T00:00:00Z".to_string(),
            repos_included: 1,
            repos_attempted: 1,
            languages: vec![LanguageTable {
                language: language.to_string(),
                sample_functions: 4_000,
                strata: vec![Stratum {
                    sloc_min: 0,
                    sloc_max: u64::MAX,
                    metrics: vec![MetricQuantiles {
                        metric: "cyclomatic".to_string(),
                        quantiles: index_ramp(),
                    }],
                }],
            }],
            repo_metrics: None,
        }
    }

    #[test]
    fn corpus_lens_resolves_q750_breakpoint() {
        // A file whose only covered metric (cyclomatic) sits exactly at the
        // corpus q750 breakpoint (value 750 on the index ramp) → 0.75, not
        // beyond-corpus.
        let art = ramp_artifact("rust");
        let metrics = [("cyclomatic", Some(750.0)), ("cognitive", None)];
        let (p, beyond) = super::file_corpus_lens(&art, "rust", &metrics)
            .expect("covered metric must yield a percentile");
        assert!((p - 0.75).abs() < 1e-9, "expected 0.75 at q750, got {p}");
        assert!(!beyond, "a value inside the corpus is not beyond-corpus");
    }

    #[test]
    fn corpus_lens_is_none_for_unknown_language() {
        // The artifact covers rust only; a python file falls outside → None.
        let art = ramp_artifact("rust");
        let metrics = [("cyclomatic", Some(750.0))];
        assert!(
            super::file_corpus_lens(&art, "python", &metrics).is_none(),
            "an uncovered language must yield no corpus lens"
        );
    }

    #[test]
    fn corpus_lens_saturates_and_flags_beyond_max() {
        // A value past the corpus maximum breakpoint (ramp tops out at 1000)
        // saturates to 1.0 AND sets beyond_corpus — never silently clamped.
        let art = ramp_artifact("rust");
        let metrics = [("cyclomatic", Some(5_000.0))];
        let (p, beyond) = super::file_corpus_lens(&art, "rust", &metrics)
            .expect("a covered metric beyond the max still resolves");
        assert!(
            (p - 1.0).abs() < 1e-9,
            "beyond-max must saturate to 1.0, got {p}"
        );
        assert!(
            beyond,
            "a value past the corpus maximum must flag beyond_corpus"
        );
    }

    #[test]
    fn corpus_lens_keeps_the_worst_dimension() {
        // corpus_percentile is the MAX over the file's per-metric percentiles:
        // a low cyclomatic (250 → 0.25) and a high one via a second covered
        // metric must surface the worst. Here only cyclomatic is covered, so add
        // a second file value to prove the MAX picks the larger.
        let art = ramp_artifact("rust");
        // Two cyclomatic readings can't co-exist for one file, so exercise MAX
        // by passing the same metric twice with different values — the helper
        // folds them with max().
        let metrics = [("cyclomatic", Some(250.0)), ("cyclomatic", Some(900.0))];
        let (p, _) = super::file_corpus_lens(&art, "rust", &metrics).expect("covered");
        assert!(
            (p - 0.90).abs() < 1e-9,
            "MAX must keep the worst (0.90), got {p}"
        );
    }

    #[test]
    fn corpus_lens_is_none_below_sample_floor() {
        // A language pooled below MIN_LANG_SAMPLE is treated as absent.
        let mut art = ramp_artifact("rust");
        art.languages[0].sample_functions = 10; // below the 500 floor
        let metrics = [("cyclomatic", Some(750.0))];
        assert!(
            super::file_corpus_lens(&art, "rust", &metrics).is_none(),
            "an under-sampled language must yield no corpus lens"
        );
    }

    #[test]
    fn no_dry_scale_renormalizes_the_remaining_weights() {
        // Dropping the DRY weight leaves the other seven smells; the no-DRY
        // divisor renormalizes their sum back to a 1.0 ceiling. It must equal
        // (1.0 − dry_weight) so the scale stays exact.
        let dry_weight = super::SMELL_WEIGHTS
            .iter()
            .find(|(s, _)| *s == "dry")
            .map(|(_, w)| *w)
            .expect("dry weight present");
        let no_dry_sum: f64 = 1.0 - dry_weight;
        assert_eq!(
            super::STRUCTURAL_SCALE_NO_DRY,
            format!(" / {no_dry_sum}"),
            "no-DRY divisor must renormalize the remaining weights"
        );
    }
}
