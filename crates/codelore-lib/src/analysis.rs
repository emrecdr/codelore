//! The closed set of analyses codelore supports. Enum, not string,
//! so the compiler catches typos that code-maat's string dispatch silently misroutes.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisName {
    // v1 Spine — 10 core
    Hotspots,
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
    // Plan 7: clone detection (T1+T2 via AST structural hashing)
    Clones,
    // Plan 8 §6: live-clone × Fisher-significant co-change intersection
    CloneCoupling,
    // code-maat parity sprint (PAR-1+): Sum of Coupling — per-entity total
    // of (commit-size − 1) across every commit the entity appears in.
    Soc,
    // PAR-2: commit-message regex matcher.
    Messages,
    // PAR-3: top-author-per-file analyses (three variants of the same
    // metric-swap pattern). `refactoring-main-dev` is an alias for
    // `MainDevByDeletions` — the analysis is just main-dev with metric
    // = deleted-lines; the "refactoring" name is code-maat's heuristic
    // framing, not a separate commit-filter.
    MainDev,
    MainDevByRevs,
    MainDevByDeletions,
    // PAR-4 + PAR-5: per-(entity, author) row analyses.
    EntityEffort,
    EntityOwnership,
    // PAR-1 (modernise-don't-migrate): codelore's previous per-author
    // commit leaderboard. The `authors` name now resolves to the
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
    // Stale-code surfacer — files alive at HEAD untouched ≥N months
    // AND low cognitive (trivial). Intersection minimises false
    // positives.
    StaleCode,
    // Pair-programming detector — counts commits sharing one or
    // more `Co-Authored-By:` trailers, by unique author pair.
    // Surfaces who pair-programs with whom.
    PairProgramming,
    // Lead-time per commit (DORA metric). Today's schema carries
    // only committer date so rows ship zero lead-time; a future
    // schema bump adds author_date and surfaces real review-time
    // values.
    LeadTime,
    // Per-module bus factor (Filatov 2010). Module = top-level
    // directory or --group-file group.
    BusFactor,
}

impl AnalysisName {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hotspots => "hotspots",
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
            Self::StaleCode => "stale-code",
            Self::PairProgramming => "pair-programming",
            Self::LeadTime => "lead-time",
            Self::BusFactor => "bus-factor",
        }
    }

    #[must_use]
    pub fn all() -> &'static [Self] {
        // The match below is the compile-time exhaustiveness guard for
        // the `all()` registry. Adding a variant to the enum without
        // also adding it to this match fails to compile with
        // "non-exhaustive patterns" — preventing the silent
        // registry-drift class of bug where a new analysis is
        // dispatchable via `from_str` but invisible to `--help`'s
        // supported-names list, the `Supported: ...` error message,
        // and the round-trip test (which only iterates `all()`).
        //
        // The const block forces evaluation at compile time. The
        // sentinel `let _: () = ...` body is type-only; we don't use
        // the value, we use the fact that the match must cover every
        // variant. The actual array below stays the source of truth
        // for runtime order (which is the documented CLI ordering).
        const fn _exhaustive_check(name: AnalysisName) {
            match name {
                AnalysisName::Hotspots
                | AnalysisName::Coupling
                | AnalysisName::Ownership
                | AnalysisName::CodeAge
                | AnalysisName::AbsChurn
                | AnalysisName::AuthorChurn
                | AnalysisName::EntityChurn
                | AnalysisName::Communication
                | AnalysisName::CodeHealth
                | AnalysisName::Summary
                | AnalysisName::Revisions
                | AnalysisName::Authors
                | AnalysisName::Clones
                | AnalysisName::CloneCoupling
                | AnalysisName::Soc
                | AnalysisName::Messages
                | AnalysisName::MainDev
                | AnalysisName::MainDevByRevs
                | AnalysisName::MainDevByDeletions
                | AnalysisName::EntityEffort
                | AnalysisName::EntityOwnership
                | AnalysisName::TopCommitters
                | AnalysisName::KnowledgeIslands
                | AnalysisName::Centrality
                | AnalysisName::Communities
                | AnalysisName::GodClasses
                | AnalysisName::ArchViolations
                | AnalysisName::StaleCode
                | AnalysisName::PairProgramming
                | AnalysisName::LeadTime
                | AnalysisName::BusFactor => {}
            }
        }
        &[
            Self::Hotspots,
            Self::Coupling,
            Self::Ownership,
            Self::CodeAge,
            Self::AbsChurn,
            Self::AuthorChurn,
            Self::EntityChurn,
            Self::Communication,
            Self::CodeHealth,
            Self::Summary,
            Self::Revisions,
            Self::Authors,
            Self::Clones,
            Self::CloneCoupling,
            Self::Soc,
            Self::Messages,
            Self::MainDev,
            Self::MainDevByRevs,
            Self::MainDevByDeletions,
            Self::EntityEffort,
            Self::EntityOwnership,
            Self::TopCommitters,
            Self::KnowledgeIslands,
            Self::Centrality,
            Self::Communities,
            Self::GodClasses,
            Self::ArchViolations,
            Self::StaleCode,
            Self::PairProgramming,
            Self::LeadTime,
            Self::BusFactor,
        ]
    }

    /// F14 + F15 fix: classify which analyses can run under
    /// `--time-bucket`. The bucketed source table (`changes_bucketed`)
    /// is materialised by `lineage::materialize_source` and synthesises
    /// `rev` as a date-truncated string. Any analysis that JOINs
    /// `c.rev = commits.rev` against the non-bucketed `commits` table
    /// silently returns zero rows (F15). Any analysis that uses
    /// `materialize_if_needed` (no-op on the bucketed branch) crashes
    /// with `Catalog Error: Table changes_bucketed does not exist`
    /// (F14). Both failure modes are unacceptable user experiences.
    ///
    /// Currently only `coupling`, `soc`, `hotspots`, and `code-health`
    /// invoke `materialize_source` AND have SQL that doesn't depend on
    /// the `commits` JOIN for `rev` equality. These four are the only
    /// analyses that semantically MAKE SENSE under bucketing (they're
    /// all about co-change, which `--time-bucket` is designed to
    /// smooth). The other 18 are either per-file or per-author
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
            // (8 tables: commits, changes, complexity, clones, mailmap, …),
            // strictly richer than code-maat's raw-log seq. Rather than
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
                 dumps the full DuckDB fact store (commits, changes, complexity, clones, \
                 mailmap, provenance, hunks, author_aliases — 8 tables, strictly richer \
                 than code-maat's parsed-log seq). The `identity` analysis name is not \
                 registered in CodeLore."
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
                // can pick from. Plan 8 §2 Task 6 wired `Authors`, so it's no
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
                 dumps the full DuckDB fact store (8 tables: commits, changes, \
                 complexity, clones, mailmap, provenance, hunks, author_aliases) \
                 — strictly richer than code-maat's parsed-log seq.",
            ),
        }
    }
}
