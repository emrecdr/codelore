//! Scan-coverage accounting shared by every pass that reads and parses
//! source: the HEAD-time passes, the working-tree clone scan, and the
//! at-rev passes.
//!
//! Each pass walks its own file set
//! and produces rows for some of it. Without a tally, a pass that failed on
//! most of the repository is arithmetically indistinguishable from a small
//! repository: the tables are simply thinner, and every analysis and gate
//! downstream reads that thinness as a fact about the code.
//!
//! The consequence is not symmetric. A thin `complexity_metrics` drags scores
//! toward whatever the surviving files say, but a thin `clones` reads as
//! *improvement*: `disallow_clone_type_1` is `COUNT(DISTINCT clone_group_id)`
//! and passes on zero, so a scan that saw nothing looks exactly like a
//! repository with no duplication.

/// Proportion of eligible files a scan must cover before the run is
/// considered healthy. Below this, the fact store is thin enough that every
/// analysis reading the pass's table — and every gate reading those analyses
/// — is drawing conclusions from a minority of the codebase.
pub(crate) const MIN_SCAN_COVERAGE: f64 = 0.9;

/// Whether a scan that reached `scored` of `eligible` files falls below
/// [`MIN_SCAN_COVERAGE`].
///
/// The one place the floor is applied. Both consumers route through it — the
/// ingest-time warning and the gate verdict — because they must agree about the
/// same scan, and a second copy of `scored / eligible < FLOOR` in another module
/// would be free to drift from this one. Zero eligible files is *not* below the
/// floor: a tree with nothing to scan is honestly complete.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn below_floor(scored: u64, eligible: u64) -> bool {
    eligible != 0 && (scored as f64) / (eligible as f64) < MIN_SCAN_COVERAGE
}

pub(crate) const REASON_BLOB_READ: &str = "blob read failed";
pub(crate) const REASON_PARSE_ERROR: &str = "parse error";

/// What a HEAD scan did with one file.
///
/// The distinction that matters is [`NotCounted`](ScanOutcome::NotCounted) vs
/// [`Lost`](ScanOutcome::Lost). Both produce no row, but only the second is a
/// coverage loss: a `README.md` is *supposed* to produce nothing, whereas a
/// `.rs` file whose blob would not read is a file the scan owed the user and
/// could not deliver. Collapsing the two — which a bare `Option` does — is what
/// lets a scan that reached 200 of 5,200 files look like a small repository.
///
/// The split follows the per-file log level each pass already used, which is
/// the authority on which outcomes are routine: `debug!` cases (a path in
/// `changes` that HEAD no longer tracks, a file over the AST size cap) are
/// expected and land in `NotCounted`; `warn!` cases (an object-database
/// failure, a parse error) are the ones a healthy run does not produce.
/// Counting the routine cases as losses put CodeLore's own repository at 86%
/// and fired the aggregate warning on a scan that had not lost anything.
///
/// `Scored` means *the scan reached this file and succeeded* — not that it
/// produced a row. A source file with no imports, or no clone members, is
/// fully covered and contributes an empty payload the drain skips. Classifying
/// those as `NotCounted` would shrink the denominator and make coverage read
/// better than it is, which is the exact blindness this type exists to remove.
pub(crate) enum ScanOutcome<T> {
    /// No row, and none was owed: not an eligible source file; or a path
    /// carried by `changes` that HEAD no longer tracks (`live_paths` is derived
    /// from history, so a file deleted before HEAD is legitimately absent); or
    /// a file past the AST size cap, which is the generated/minified case the
    /// cap exists to skip. Excluded from the denominator — including these
    /// would mark a healthy repository degraded.
    NotCounted,
    /// Eligible, and the scan failed on it. Carries the reason so the aggregate
    /// can say *why* coverage was lost rather than only that it was.
    Lost(&'static str),
    /// Eligible, and the scan succeeded. The payload may be empty.
    Scored(T),
    /// No row, none owed by the loss ratio — but unlike `NotCounted`, worth
    /// counting: a file that *looks* like eligible source, skipped only for
    /// its size (past the AST byte cap). Deliberately not a loss — the cap
    /// exists to skip generated/minified bundles, and bundle-carrying
    /// repositories skip some routinely — but tracked, because a scan where
    /// these outnumber the scanned files reads as 100% covered over a
    /// near-empty table, which is the reads-as-improvement failure mode one
    /// classification to the left of the one the loss ratio removes.
    SkippedOversize,
}

/// How much of the eligible file set a pass actually covered.
pub(crate) struct ScanCoverage {
    eligible: usize,
    scored: usize,
    /// Files skipped past the AST byte cap — outside the loss ratio, inside
    /// the majority-oversize disclosure.
    skipped_oversize: usize,
    /// Skip counts by reason, most frequent first when rendered.
    by_reason: Vec<(&'static str, usize)>,
}

impl ScanCoverage {
    pub(crate) fn tally<T>(outcomes: &[ScanOutcome<T>]) -> Self {
        let mut scored = 0usize;
        let mut skipped_oversize = 0usize;
        let mut counts: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for o in outcomes {
            match o {
                ScanOutcome::Scored(..) => scored += 1,
                ScanOutcome::Lost(reason) => *counts.entry(reason).or_default() += 1,
                ScanOutcome::SkippedOversize => skipped_oversize += 1,
                ScanOutcome::NotCounted => {}
            }
        }
        let lost: usize = counts.values().sum();
        let mut by_reason: Vec<(&'static str, usize)> = counts.into_iter().collect();
        by_reason.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        Self {
            eligible: scored + lost,
            scored,
            skipped_oversize,
            by_reason,
        }
    }

    /// Denominator behind [`Self::ratio`] — scored plus lost, with routine
    /// skips (non-Tier-1, history-only paths, oversize) already excluded.
    ///
    /// Exposed so the value can be persisted and the same predicate recomputed
    /// later by a consumer that never saw the scan, rather than having that
    /// consumer invent its own denominator from the HEAD tree.
    pub(crate) fn eligible(&self) -> usize {
        self.eligible
    }

    /// Numerator behind [`Self::ratio`]. See [`Self::eligible`].
    pub(crate) fn scored(&self) -> usize {
        self.scored
    }

    /// Fraction of eligible files the scan covered. Vacuously 1.0 when the
    /// repository carries no eligible source at all — a docs-only tree is
    /// honestly complete, not degraded.
    #[allow(clippy::cast_precision_loss)]
    pub(crate) fn ratio(&self) -> f64 {
        if self.eligible == 0 {
            1.0
        } else {
            self.scored as f64 / self.eligible as f64
        }
    }

    /// Emit one aggregate warning when coverage falls below the floor.
    ///
    /// `scan` names the pass ("complexity") and `table` the fact it fills
    /// ("`complexity_metrics`"), so the message says both what went thin and
    /// what downstream reads it.
    ///
    /// Deliberately `warn!` and not `debug!`: the default `EnvFilter` is
    /// `warn`, so this is the one level at which the message reaches a user who
    /// did not opt into logging. The per-file messages stay where they are —
    /// they say *which* file, this says *how much of the repository is
    /// missing*.
    pub(crate) fn warn_if_degraded(&self, scan: &str, table: &str) {
        if !below_floor(self.scored as u64, self.eligible as u64) {
            return;
        }
        let detail = self
            .by_reason
            .iter()
            .map(|(reason, n)| format!("{n} {reason}"))
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            "{scan} scan covered {scored}/{eligible} eligible source files \
             ({pct:.0}%); {detail}. Analyses and quality gates that read \
             `{table}` are drawing on a minority of this repository. A blobless \
             partial clone (`git clone --filter=blob:none`, or \
             `actions/checkout` with a filter) is the usual cause and is not \
             detected by the shallow-clone check, because such a clone has \
             complete commit history.",
            scored = self.scored,
            eligible = self.eligible,
            pct = self.ratio() * 100.0,
        );
    }

    /// True when more of what looked like source was size-skipped than
    /// scanned. The loss ratio cannot see this case — oversize skips are
    /// deliberately not losses — so it is the predicate the oversize
    /// disclosure fires on. Its own tests call it directly, so the warning
    /// and the tests cannot drift apart.
    pub(crate) fn oversize_majority(&self) -> bool {
        self.skipped_oversize > self.scored
    }

    /// Emit one aggregate warning when the size cap, not scan failure, is
    /// what left the table thin. Same `warn!` rationale as
    /// [`Self::warn_if_degraded`]; fires only on a majority so the routine
    /// bundle-carrying repository stays quiet.
    pub(crate) fn warn_if_mostly_oversize(&self, scan: &str, table: &str) {
        if !self.oversize_majority() {
            return;
        }
        tracing::warn!(
            "{scan} scan skipped {skipped} file(s) past the {cap}-byte AST \
             cap — more than the {scored} it scanned. `{table}` describes a \
             minority of what looks like source in this repository: the cap \
             exists to skip generated/minified bundles, but at this share the \
             skipped set IS the repository. Exclude bundle directories via \
             `.codeloreignore` so the census reflects maintained code.",
            skipped = self.skipped_oversize,
            cap = crate::constants::DEFAULT_MAX_AST_FILE_BYTES,
            scored = self.scored,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_SCAN_COVERAGE, REASON_BLOB_READ, REASON_PARSE_ERROR, ScanCoverage, ScanOutcome,
    };

    /// The tally is payload-agnostic, so the tests pick the simplest one.
    type Outcome = ScanOutcome<String>;

    fn scored(path: &str) -> Outcome {
        ScanOutcome::Scored(path.to_string())
    }

    #[test]
    fn oversize_skips_stay_out_of_the_loss_ratio() {
        let outcomes: Vec<Outcome> = vec![
            scored("a.rs"),
            ScanOutcome::SkippedOversize,
            ScanOutcome::SkippedOversize,
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert!(
            (cov.ratio() - 1.0).abs() < 1e-12,
            "an oversize skip is deliberately not a loss — the cap exists to \
             skip bundles, and bundle-carrying repositories skip routinely"
        );
    }

    #[test]
    fn the_oversize_disclosure_fires_on_a_strict_majority() {
        let majority = ScanCoverage::tally::<String>(&[
            scored("a.rs"),
            ScanOutcome::SkippedOversize,
            ScanOutcome::SkippedOversize,
        ]);
        assert!(
            majority.oversize_majority(),
            "two skipped vs one scanned is a majority-blind table"
        );
        let tie = ScanCoverage::tally::<String>(&[scored("a.rs"), ScanOutcome::SkippedOversize]);
        assert!(!tie.oversize_majority(), "a tie is not a majority");
        let none = ScanCoverage::tally::<String>(&[scored("a.rs")]);
        assert!(!none.oversize_majority());
    }

    #[test]
    fn ineligible_files_are_not_a_coverage_loss() {
        // A docs-only tree is honestly complete, not degraded. If `NotCounted`
        // counted toward the denominator, every repository with a README would
        // report a thin scan — which is the false positive that makes a
        // coverage sentinel unusable.
        let outcomes = vec![
            ScanOutcome::NotCounted,
            ScanOutcome::NotCounted,
            scored("src/lib.rs"),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(cov.eligible, 1, "only the Tier-1 file is eligible");
        assert_eq!(cov.scored, 1);
        assert!((cov.ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn routine_skips_do_not_lower_coverage() {
        // Regression: the first version of this counted every non-scored file
        // as a loss, including paths that `changes` carries but HEAD no longer
        // tracks. CodeLore's own repository reported 349/404 (86%) and tripped
        // the warning on a scan that had failed at nothing. These are the exact
        // numbers from that run.
        let mut outcomes: Vec<Outcome> = (0..349).map(|i| scored(&format!("f{i}.rs"))).collect();
        for _ in 0..55 {
            outcomes.push(ScanOutcome::NotCounted);
        }
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(
            cov.eligible, 349,
            "paths history carries but HEAD does not track are not files the scan owed"
        );
        assert!(
            (cov.ratio() - 1.0).abs() < f64::EPSILON,
            "a scan that lost nothing must read as complete, got {}",
            cov.ratio()
        );
        assert!(
            cov.by_reason.is_empty(),
            "nothing was lost, so nothing to attribute"
        );
    }

    #[test]
    fn a_source_less_tree_is_vacuously_complete() {
        let cov = ScanCoverage::tally(&[Outcome::NotCounted]);
        assert_eq!(cov.eligible, 0);
        assert!(
            (cov.ratio() - 1.0).abs() < f64::EPSILON,
            "no eligible files must not read as 0% coverage"
        );
    }

    #[test]
    fn skips_lower_the_ratio_and_are_attributed_by_reason() {
        // The defect this guards: a scan that reached a minority of its files
        // must not be arithmetically indistinguishable from a small repository.
        let mut outcomes = vec![scored("a.rs")];
        for _ in 0..9 {
            outcomes.push(ScanOutcome::Lost(REASON_BLOB_READ));
        }
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(cov.eligible, 10);
        assert_eq!(cov.scored, 1);
        assert!(
            (cov.ratio() - 0.1).abs() < 1e-9,
            "1 of 10 eligible files is 10% coverage, got {}",
            cov.ratio()
        );
        assert!(
            cov.ratio() < MIN_SCAN_COVERAGE,
            "10% coverage must fall below the floor that triggers disclosure"
        );
        assert_eq!(cov.by_reason, vec![(REASON_BLOB_READ, 9)]);
    }

    #[test]
    fn reasons_are_ranked_most_frequent_first() {
        let outcomes: Vec<Outcome> = vec![
            ScanOutcome::Lost(REASON_PARSE_ERROR),
            ScanOutcome::Lost(REASON_BLOB_READ),
            ScanOutcome::Lost(REASON_BLOB_READ),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(
            cov.by_reason,
            vec![(REASON_BLOB_READ, 2), (REASON_PARSE_ERROR, 1)],
            "the dominant failure mode must be named first so the message leads with it"
        );
    }

    #[test]
    fn a_healthy_scan_stays_above_the_floor() {
        let outcomes = vec![
            scored("a.rs"),
            scored("b.rs"),
            scored("c.rs"),
            scored("d.rs"),
            scored("e.rs"),
            scored("f.rs"),
            scored("g.rs"),
            scored("h.rs"),
            scored("i.rs"),
            scored("j.rs"),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert!(cov.ratio() >= MIN_SCAN_COVERAGE);
    }

    /// A file the scan read and walked successfully, which simply had nothing
    /// to report, is **covered** — not ineligible.
    ///
    /// This is the case the clones and imports passes hit constantly: most
    /// source files declare no clone family and many declare no imports. Their
    /// old code returned the same `None` for that as for a failed blob read,
    /// and routing the empty case to `NotCounted` here would reproduce the bug
    /// one level up — shrinking the denominator so coverage reads *better* the
    /// more import-free files a repository has.
    #[test]
    fn a_successful_scan_with_no_output_still_counts_as_covered() {
        let outcomes: Vec<ScanOutcome<Vec<u32>>> = vec![
            ScanOutcome::Scored(Vec::new()),
            ScanOutcome::Scored(vec![1]),
            ScanOutcome::Lost(REASON_BLOB_READ),
        ];
        let cov = ScanCoverage::tally(&outcomes);
        assert_eq!(cov.eligible, 3, "both scored files are eligible");
        assert_eq!(cov.scored, 2, "an empty payload is still a covered file");
        assert!(
            (cov.ratio() - 2.0 / 3.0).abs() < 1e-12,
            "ratio should be 2/3, got {}",
            cov.ratio()
        );
    }

    /// The tally is payload-agnostic by construction, and this pins it: the
    /// three passes carry three different payload types through the same
    /// accounting, so a change that accidentally specialised it would break
    /// two callers at once.
    #[test]
    fn the_tally_is_independent_of_the_payload_type() {
        let strings: Vec<ScanOutcome<String>> = vec![
            ScanOutcome::Scored("a".into()),
            ScanOutcome::Lost(REASON_PARSE_ERROR),
        ];
        let pairs: Vec<ScanOutcome<(String, Vec<u8>)>> = vec![
            ScanOutcome::Scored(("a".into(), Vec::new())),
            ScanOutcome::Lost(REASON_PARSE_ERROR),
        ];
        let a = ScanCoverage::tally(&strings);
        let b = ScanCoverage::tally(&pairs);
        assert_eq!((a.eligible, a.scored), (b.eligible, b.scored));
        assert!((a.ratio() - b.ratio()).abs() < 1e-12);
    }
}
