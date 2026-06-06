//! Run-time configuration for the bca pipeline. Defaults match
//! code-maat for parity; see spec §1.1.

use std::path::PathBuf;
use time::Date;

#[derive(Debug, Clone)]
pub struct Options {
    // Input
    pub repo_path: PathBuf,
    pub after: Option<Date>,
    pub before: Option<Date>,
    pub commit_range: Option<String>,

    // Aggregation (Plan 4 — left here so Options shape is stable from v1)
    pub group_file: Option<PathBuf>,
    pub team_map_file: Option<PathBuf>,
    pub temporal_period_days: Option<u32>,

    // Analysis thresholds — code-maat parity
    pub min_revs: u32,
    pub min_shared_revs: u32,
    pub min_coupling_pct: u8,
    pub max_coupling_pct: u8,
    pub max_changeset_size: u32,
    pub fisher_significance: f64,

    // Specific analyses
    pub message_regex: Option<String>,
    pub age_time_now: Option<Date>,

    // Output
    pub rows_limit: Option<u32>,
    pub verbose_results: bool,
    pub include_merges: bool,
    pub strict_grouping: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            repo_path: PathBuf::from("."),
            after: None,
            before: None,
            commit_range: None,
            group_file: None,
            team_map_file: None,
            temporal_period_days: None,
            min_revs: 5,
            min_shared_revs: 5,
            min_coupling_pct: 30,
            max_coupling_pct: 100,
            max_changeset_size: 30,
            fisher_significance: 0.05,
            message_regex: None,
            age_time_now: None,
            rows_limit: None,
            verbose_results: false,
            include_merges: false,
            strict_grouping: false,
        }
    }
}
