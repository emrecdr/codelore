//! The closed set of analyses codelore supports. Enum, not string,
//! so the compiler catches typos that code-maat's string dispatch silently misroutes.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisName {
    // v1 Spine — 10 core
    Hotspots,
    // Change-acceleration early warning (recent vs baseline churn rate).
    HotspotVelocity,
    Coupling,
    Ownership,
    CodeAge,
    AbsChurn,
    AuthorChurn,
    EntityChurn,
    Communication,
    CodeHealth,
    Summary,
    // code-maat parity (computed as side-data on hotspots, addressable standalone)
    Revisions,
    Authors,
    // Clone detection (T1+T2 via AST structural hashing)
    Clones,
    // Live-clone × Fisher-significant co-change intersection
    CloneCoupling,
    // Sum of Coupling — per-entity total of (commit-size − 1) across every
    // commit the entity appears in.
    Soc,
    // Commit-message regex matcher.
    Messages,
    // Top-author-per-file analyses (three variants of the same
    // metric-swap pattern). `refactoring-main-dev` is an alias for
    // `MainDevByDeletions` — the analysis is just main-dev with metric
    // = deleted-lines; the "refactoring" name is code-maat's heuristic
    // framing, not a separate commit-filter.
    MainDev,
    MainDevByRevs,
    MainDevByDeletions,
    // Per-(entity, author) row analyses.
    EntityEffort,
    EntityOwnership,
    // codelore's previous per-author commit leaderboard. The `authors`
    // name now resolves to the
    // per-entity Bird et al. risk-indicator query; `top-committers`
    // is the "who commits the most overall" view, enriched with
    // LoC totals and first/last commit dates.
    TopCommitters,
    // T8: knowledge-islands analysis. Per-file bus-factor risk —
    // files whose primary author (by LoC) hasn't committed in
    // `--departed-threshold-days` days AND has no substantial other
    // owners. CodeLore's strategic differentiator vs CodeScene
    // (automatic departure detection from commit-date falloff vs
    // their required manual Ex-Developer marking).
    KnowledgeIslands,
    // Per-file centrality on the Fisher-significant coupling graph.
    // Promotes the SoC-style `coupling_centrality_v1` primitive
    // (currently a bare COUNT(*) materialised inside `code_health`)
    // to a first-class analysis with degree, weighted-degree,
    // PageRank, and eigenvector variants.
    Centrality,
    // Leiden community detection on the coupling graph. Auto-detects
    // Conway's-law clusters; partition modularity is the headline
    // number. CodeLore's strategic differentiator vs CodeScene
    // (paywalled there).
    Communities,
    // God-class detector — files where high cognitive complexity
    // intersects with high coupling (fan_in + fan_out via the
    // imports table). Brown et al. 1998 AntiPatterns. Consumes the
    // architecture import graph; fan_in accuracy follows the
    // resolver's language coverage (Rust + Python + JS/TS today,
    // Java FQN→file mapping skipped).
    GodClasses,
    // Layered-architecture rule validation. Consumes
    // `.codelore-arch-rules.toml` at the repo root + the imports
    // table; flags every import edge that crosses a forbidden layer
    // boundary. No rules file → empty output (opt-in).
    ArchViolations,
    // Dependency cycles — non-trivial strongly-connected components of
    // the structural import graph (Tarjan SCC). Files that import each
    // other transitively (a "tangle"); Arcan "Cyclic Dependency" smell.
    DependencyCycles,
    // Architecture roles — per-file Core/Shared/Control/Periphery
    // classification from the import graph's transitive reachability
    // (Baldwin, MacCormack & Rusnak 2014 "Hidden Structure"). Carries
    // visibility fan-in/out; the mean of vfo/n is the propagation cost.
    ArchitectureRoles,
    // Instability — Robert C. Martin's per-file coupling metrics:
    // afferent Ca (in-degree), efferent Ce (out-degree), instability
    // I = Ce/(Ca+Ce). Martin 1994; the import graph's in/out degree.
    Instability,
    // Architecture metrics — repo-level structural-health numbers:
    // propagation cost, Lakos ACD/NCCD, cycle count, architecture type
    // (Lakos 1996, MacCormack/Baldwin). Emitted as (metric, value) rows.
    ArchitectureMetrics,
    // Architecture trend — structural-health metrics (propagation cost,
    // cycle count, largest tangle) recomputed at sampled historical revs
    // to show architectural decay over time. Reads blobs at past revs.
    ArchitectureTrend,
    // Repo health timeline — arch health, code health, and combined
    // health (each 0–100, higher = healthier) at sampled historical revs.
    // Reuses the architecture-trend sampler + the rev-parameterisable
    // code-health engine. Reads blobs at past revs; on-demand, never cached.
    HealthTrend,
    // Cycle origins — bisects history to find the commit where each HEAD
    // dependency cycle first formed. Reads blobs at past revs.
    CycleOrigins,
    // Modularity violations — file pairs that co-change
    // (Fisher-significant) yet have NO structural import edge between
    // them. The structure×history fusion: implicit cross-module
    // dependencies (Mo et al. 2015 *Hotspot Patterns*, DV8). CodeLore
    // differentiator — needs BOTH the import graph and the co-change
    // graph, which import-only and history-only tools each lack.
    ModularityViolations,
    // Unstable interfaces — heavily-imported files that change often
    // AND co-change with their dependents, so the instability
    // propagates outward (Mo et al. 2015 *Hotspot Patterns*, DV8).
    UnstableInterface,
    // Crossing — a structural "X" (high fan-in AND fan-out) that
    // co-changes with both its importers and its imports, coupling
    // upstream and downstream through itself (Mo et al. 2015 DV8).
    Crossing,
    // Stale-code surfacer — files alive at HEAD untouched ≥N months
    // AND low cognitive (trivial). Intersection minimises false
    // positives.
    StaleCode,
    // Pair-programming detector — counts commits sharing one or
    // more `Co-Authored-By:` trailers, by unique author pair.
    // Surfaces who pair-programs with whom.
    PairProgramming,
    // Lead-time per commit (DORA metric). Computed as `committer_date
    // - date` (committer time minus author time); rebase/squash
    // workflows produce many zeros, merge-via-merge-commit + review
    // workflows surface real values.
    LeadTime,
    // Per-module bus factor (Filatov 2010). Module = top-level
    // directory or --group-file group.
    BusFactor,
    // Per-file delivery friction composite: where technical debt
    // actively slows delivery. Product of three percentile ranks
    // (revisions × median lead-time × cognitive). High requires
    // elevation on ALL THREE axes — one dominant signal alone does
    // not. Counters CodeScene v7.4's Delivery Analysis surface while
    // staying SQL-driven and CLI-only.
    DeliveryFriction,
    // Repo-level delivery flow distributions: batch_size_files,
    // batch_size_loc (merge-unit size), branch_duration_hours
    // (time branch stays open), rework_pct (hunk-overlap within the
    // rework window), lead_proxy_hours (author→committer date gap
    // over non-merge commits). Percentile-first (p50/p75/p90).
    // Requires the commit_parents table (schema v4) and merges ingested
    // with include_merges=true.
    DeliveryMetrics,
    // Effort-aware refactoring ROI ranking: (structural_risk ×
    // hotspot_score) / max(loc, floor). Surfaces files where low
    // code health intersects high activity, normalised by inspection
    // effort so small, dense, churning files outrank large ones with
    // the same raw risk.
    RefactoringTargets,
    // Effort exposure — what fraction of engineering activity (commits,
    // churn, SLOC) falls in each code-health band (red / yellow / green)
    // over the trailing window. Hero KPI: "Are we spending most effort
    // fighting fires or extending healthy code?" Wilson 95% CI on
    // commit-share is included per band.
    EffortExposure,
    // Code familiarity — SLOC-weighted fraction of the codebase actively
    // known by current contributors (authors with ≥1 commit in the
    // trailing window), using exponentially-decayed knowledge shares
    // (Jabrayilzade et al., ICSE-SEIP 2022). Also reports islands
    // percentage (files with dominant single-author knowledge).
    CodeFamiliarity,
    // Team composition — per-author contribution-span buckets (onboarded /
    // experienced / veteran) with a behavioral veteran-breadth gate and
    // an onboarding-velocity metric (weeks to enter the weekly 80%-core
    // set, per arXiv 2601.23142). Founder-period authors (first commit
    // within the project's first 12 weeks) receive NULL onboarding_weeks.
    TeamComposition,
    // Coordination needs — per-file coordination overhead: knowledge
    // fragmentation (HHI complement over decayed shares), author-switch
    // interleave between adjacent commits, and co-change graph entropy
    // contribution (EASE 2025, arXiv 2504.18511). Tier classification
    // (single / low / medium / high) + code-health band join surface the
    // worst cases: high-fragmentation, high-interleave files in the red band.
    CoordinationNeeds,
    // Marginal-owner risk — ownership concentration × code-health fusion.
    // For each file in the yellow/red health band, reports the maximum
    // knowledge share held by any active author (committed within window_days).
    // Risk tiers: high (red band AND share <0.10) and elevated
    // ((red AND share <0.30) OR (yellow AND share <0.10)).
    // Correlational signal; see Palomba et al., EASE 2023, arXiv 2304.11636.
    MarginalOwnerRisk,
    // Release cadence — inter-release gap statistics derived from git tags.
    // Tags matching --release-tag-glob (default "v*") are treated as release
    // markers. Emits per-tag rows (date, days_since_prev) plus a summary row
    // carrying median gap, IQR, and trend (accelerating/stable/slowing from
    // the OLS slope of the gap series; threshold ±0.1 day/release).
    ReleaseCadence,
    // Function-xray — per-function change frequency for a single target file
    // (`--target <path>`). For each function/method alive at HEAD, counts the
    // revisions where at least one hunk overlapped its line span. Reuses the
    // tree-sitter span extractor from the ingest pass. Hunk-overlap attribution
    // is more accurate than blame (it captures the state at change time, not at
    // HEAD). Research: HistoryFinder (Gall et al. ICSM 2003).
    FunctionXray,
    // Function-hotspots — repo-wide function-level hotspot ranking: HEAD-live
    // functions/methods ranked by the same revs × cognitive-style score
    // `hotspots` uses, computed at function instead of file granularity.
    // Reuses function-xray's hunk↔span overlap predicate (transliterated to
    // SQL) against `entities` × `hunks` — no tree-sitter reparse. Research:
    // Gall et al. ICSM 2003 (function-level churn) + Tornhill 2018 (the
    // hotspot score this mirrors).
    FunctionHotspots,
    // Function-coupling — per-function-pair co-change frequency with Fisher
    // significance for a single target file (`--target <path>`). Identifies
    // which functions always change together. Research: Adams et al. ICSM 2006.
    FunctionCoupling,
    // External-findings × behavioral hotspot fusion. Joins the external
    // scanner sidecar (populated by `codelore ingest-sarif`) with hotspot
    // and code-health signal to surface where static findings overlap with
    // the most churned, least healthy files. Requires prior `ingest-sarif`.
    FindingHotspotOverlap,
    // Cycle health — per-SCC behavioral heat, live/fossil verdict, and the
    // cheapest cut point for each import tangle. Ranks non-trivial SCCs by
    // their share of repo LOC churn (structure×history fusion of Baldwin /
    // MacCormack & Mo et al.). Reports the extraction candidate whose
    // removal minimises the largest surviving SCC (trial-removal Tarjan)
    // and the predicted propagation-cost drop, for cycles of ≤ 64 members.
    CycleHealth,
    // Defect-validation — reads an own-repo defect-calibration artifact
    // (built by `codelore calibrate-defects`) and reports its evidence as
    // flat (metric, value) rows: the band table (where mined defects landed
    // by code-health band at the time), AUC / precision@k of HEAD structural
    // risk against the defect labels, mining tallies, the weight-tuning
    // decision with both validation AUCs, and the artifact vintage. Reads the
    // artifact only — never mines; without one it emits zero rows plus a
    // stderr hint. Association, not causation.
    DefectValidation,
}

impl AnalysisName {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotspots => "hotspots",
            Self::HotspotVelocity => "hotspot-velocity",
            Self::Coupling => "coupling",
            Self::Ownership => "ownership",
            Self::CodeAge => "code-age",
            Self::AbsChurn => "abs-churn",
            Self::AuthorChurn => "author-churn",
            Self::EntityChurn => "entity-churn",
            Self::Communication => "communication",
            Self::CodeHealth => "code-health",
            Self::Summary => "summary",
            Self::Revisions => "revisions",
            Self::Authors => "authors",
            Self::Clones => "clones",
            Self::CloneCoupling => "clone-coupling",
            Self::Soc => "soc",
            Self::Messages => "messages",
            Self::MainDev => "main-dev",
            Self::MainDevByRevs => "main-dev-by-revs",
            Self::MainDevByDeletions => "main-dev-by-deletions",
            Self::EntityEffort => "entity-effort",
            Self::EntityOwnership => "entity-ownership",
            Self::TopCommitters => "top-committers",
            Self::KnowledgeIslands => "knowledge-islands",
            Self::Centrality => "centrality",
            Self::Communities => "communities",
            Self::GodClasses => "god-classes",
            Self::ArchViolations => "architecture-violations",
            Self::DependencyCycles => "dependency-cycles",
            Self::ArchitectureRoles => "architecture-roles",
            Self::Instability => "instability",
            Self::ArchitectureMetrics => "architecture-metrics",
            Self::ArchitectureTrend => "architecture-trend",
            Self::HealthTrend => "health-trend",
            Self::CycleOrigins => "cycle-origins",
            Self::ModularityViolations => "modularity-violations",
            Self::UnstableInterface => "unstable-interface",
            Self::Crossing => "crossing",
            Self::StaleCode => "stale-code",
            Self::PairProgramming => "pair-programming",
            Self::LeadTime => "lead-time",
            Self::BusFactor => "bus-factor",
            Self::DeliveryFriction => "delivery-friction",
            Self::DeliveryMetrics => "delivery-metrics",
            Self::RefactoringTargets => "refactoring-targets",
            Self::EffortExposure => "effort-exposure",
            Self::CodeFamiliarity => "code-familiarity",
            Self::TeamComposition => "team-composition",
            Self::CoordinationNeeds => "coordination-needs",
            Self::MarginalOwnerRisk => "marginal-owner-risk",
            Self::ReleaseCadence => "release-cadence",
            Self::FunctionXray => "function-xray",
            Self::FunctionHotspots => "function-hotspots",
            Self::FunctionCoupling => "function-coupling",
            Self::FindingHotspotOverlap => "finding-hotspot-overlap",
            Self::CycleHealth => "cycle-health",
            Self::DefectValidation => "defect-validation",
        }
    }

    #[must_use]
    pub fn all() -> &'static [Self] {
        // Single source of truth for the registry. The macro expands
        // ONCE into both:
        //   (a) the `&[Self::X, ...]` array `all()` returns (drives
        //       `--help`'s supported-names list, the `Supported: ...`
        //       error message, and the round-trip test).
        //   (b) a `const fn _guard` match arm `Self::X => ()` for
        //       every variant in the same token list.
        //
        // Adding a new variant to the enum forces a non-exhaustive-
        // match compile error inside `_guard`. The author MUST add
        // the variant to the macro call to fix it — and that single
        // addition also populates the array. The two surfaces cannot
        // drift, which the prior "array literal next to a separate
        // match" shape allowed (a new variant could be added to the
        // separate match while the array silently lost coverage,
        // making the new analysis invisible to `--help` and the
        // round-trip test even though it was dispatchable).
        macro_rules! registry {
            ($($variant:ident),* $(,)?) => {{
                const ALL: &[AnalysisName] = &[$(AnalysisName::$variant),*];
                const fn _guard(name: AnalysisName) {
                    match name {
                        $(AnalysisName::$variant => {}),*
                    }
                }
                ALL
            }};
        }
        registry!(
            Hotspots,
            HotspotVelocity,
            Coupling,
            Ownership,
            CodeAge,
            AbsChurn,
            AuthorChurn,
            EntityChurn,
            Communication,
            CodeHealth,
            Summary,
            Revisions,
            Authors,
            Clones,
            CloneCoupling,
            Soc,
            Messages,
            MainDev,
            MainDevByRevs,
            MainDevByDeletions,
            EntityEffort,
            EntityOwnership,
            TopCommitters,
            KnowledgeIslands,
            Centrality,
            Communities,
            GodClasses,
            ArchViolations,
            DependencyCycles,
            ArchitectureRoles,
            Instability,
            ArchitectureMetrics,
            ArchitectureTrend,
            HealthTrend,
            CycleOrigins,
            ModularityViolations,
            UnstableInterface,
            Crossing,
            StaleCode,
            PairProgramming,
            LeadTime,
            BusFactor,
            DeliveryFriction,
            DeliveryMetrics,
            RefactoringTargets,
            EffortExposure,
            CodeFamiliarity,
            TeamComposition,
            CoordinationNeeds,
            MarginalOwnerRisk,
            ReleaseCadence,
            FunctionXray,
            FunctionHotspots,
            FunctionCoupling,
            FindingHotspotOverlap,
            CycleHealth,
            DefectValidation,
        )
    }

    /// Classify which analyses can run under
    /// `--time-bucket`. The bucketed source table (`changes_bucketed`)
    /// is materialised by `lineage::materialize_source` and synthesises
    /// `rev` as a date-truncated string. Any analysis that JOINs
    /// `c.rev = commits.rev` against the non-bucketed `commits` table
    /// silently returns zero rows. Any analysis that uses
    /// `materialize_if_needed` (no-op on the bucketed branch) crashes
    /// with `Catalog Error: Table changes_bucketed does not exist`.
    /// Both failure modes are unacceptable user experiences.
    ///
    /// Currently only `coupling`, `soc`, `hotspots`, and `code-health`
    /// invoke `materialize_source` AND have SQL that doesn't depend on
    /// the `commits` JOIN for `rev` equality. These four are the only
    /// analyses that semantically MAKE SENSE under bucketing (they're
    /// all about co-change, which `--time-bucket` is designed to
    /// smooth). The other 50 are either per-file or per-author
    /// aggregations where bucketing is semantically a no-op or
    /// outright invalid.
    ///
    /// Adding a new analysis: opt INTO bucketing by adding the variant
    /// to this match arm AND wiring `materialize_source` (not
    /// `materialize_if_needed`) in the analysis's `run_*` function.
    #[must_use]
    pub fn supports_time_bucket(&self) -> bool {
        matches!(
            self,
            Self::Coupling | Self::Soc | Self::Hotspots | Self::CodeHealth
        )
    }
}

impl fmt::Display for AnalysisName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AnalysisName {
    type Err = UnknownAnalysisError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // code-maat compatibility aliases. Canonical names describe what
        // the analysis actually computes; aliases preserve code-maat's
        // surface for migration. `refactoring-main-dev` → main-dev with
        // deletions as the ranking metric; `fragmentation` → ownership,
        // which already emits the same Herfindahl-Hirschman fractal-value
        // alongside a superset of code-maat's columns.
        match s {
            "refactoring-main-dev" => return Ok(Self::MainDevByDeletions),
            // `fragmentation` is code-maat's name; `code-ownership` is the
            // name CodeLore's own user-facing docs use to disambiguate
            // from `entity-ownership`. Both resolve to the canonical
            // `ownership` enum variant.
            "fragmentation" | "code-ownership" => return Ok(Self::Ownership),
            // code-maat's `-a identity` is a debugging-only passthrough that
            // dumps the parsed dataset. CodeLore's modern equivalent is the
            // SQLite output emitter — it dumps the full DuckDB fact store
            // (commits, changes, hunks, entities, complexity_metrics,
            // clones, imports, author_aliases, provenance), strictly richer
            // than code-maat's raw-log seq. Rather than
            // pollute the canonical AnalysisName enum with a debug-only
            // alias (and risk it appearing in --help as a "supported"
            // analysis), we intercept here with a dedicated error variant
            // that points migrating users at the right tool.
            "identity" => return Err(UnknownAnalysisError::identity_redirect()),
            _ => {}
        }
        Self::all()
            .iter()
            .find(|a| a.as_str() == s)
            .copied()
            .ok_or_else(|| UnknownAnalysisError::unknown(s.to_string()))
    }
}

/// Error returned by `AnalysisName::from_str` when the requested name
/// doesn't resolve to a registered analysis.
///
/// Two variants:
/// - `Unknown(name)`: garden-variety unknown-name case (typos, etc.)
/// - `IdentityRedirect`: special-case for code-maat's `-a identity` debug
///   dump — surfaces a redirect message pointing migrating users at
///   `--format sqlite` instead of the generic supported-names enum.
#[derive(Debug)]
pub enum UnknownAnalysisError {
    Unknown(String),
    IdentityRedirect,
}

impl UnknownAnalysisError {
    #[must_use]
    pub fn unknown(name: String) -> Self {
        Self::Unknown(name)
    }

    #[must_use]
    pub fn identity_redirect() -> Self {
        Self::IdentityRedirect
    }
}

impl std::error::Error for UnknownAnalysisError {}

impl From<UnknownAnalysisError> for crate::CodeLoreError {
    fn from(e: UnknownAnalysisError) -> Self {
        match e {
            UnknownAnalysisError::Unknown(name) => crate::CodeLoreError::UnknownAnalysisName {
                name,
                supported: AnalysisName::all().iter().map(|a| a.as_str()).collect(),
            },
            UnknownAnalysisError::IdentityRedirect => crate::CodeLoreError::Analysis(
                "code-maat's `-a identity` (raw data dump) maps to CodeLore's \
                 SQLite output: `codelore analyze --format sqlite --output facts.db` \
                 dumps the full DuckDB fact store (commits, changes, hunks, entities, \
                 complexity_metrics, clones, imports, author_aliases, provenance — \
                 strictly richer than code-maat's parsed-log seq). The `identity` \
                 analysis name is not registered in CodeLore."
                    .into(),
            ),
        }
    }
}

impl fmt::Display for UnknownAnalysisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => {
                // Enumerate every public analysis name so the user sees what they
                // can pick from. `Authors` is wired, so it's no
                // longer filtered out.
                let names: Vec<&str> = AnalysisName::all().iter().map(|a| a.as_str()).collect();
                write!(
                    f,
                    "unknown analysis {name:?}. Supported: {}",
                    names.join(", ")
                )
            }
            Self::IdentityRedirect => write!(
                f,
                "code-maat's `-a identity` (raw data dump) is provided in \
                 CodeLore via the SQLite output emitter — try: \
                 `codelore analyze --format sqlite --output facts.db`. This \
                 dumps the full DuckDB fact store (commits, changes, hunks, \
                 entities, complexity_metrics, clones, imports, author_aliases, \
                 provenance) — strictly richer than code-maat's parsed-log seq.",
            ),
        }
    }
}
