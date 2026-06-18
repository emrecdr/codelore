//! Shared helper for routing complexity-metric reads through
//! `complexity_metrics_grouped` when `opts.group_file` is set.
//!
//! `apply_grouping` rewrites `changes` + `hunks` to use group names, but
//! `complexity_metrics` is populated at HEAD-time keyed on raw file
//! paths. Without an analogous rewrite, the `MAX(cognitive) GROUP BY
//! path` reads in `hotspots`, `code_health`, `god_classes`, and
//! `stale_code` silently produced `0` cognitive for grouped entities
//! because `changes.path` (group name) never matched
//! `complexity_metrics.path` (raw path). `apply_grouping` now also
//! materialises `complexity_metrics_grouped` with per-group `MAX`
//! cognitive + `MAX` `kind='unit'` MI rolled up; analyses opt in by
//! reading from the table named here.
//!
//! Centralised so the four affected analyses share one dispatch —
//! mirrors `analyses::lineage` for `--use-canonical-lineage`.

use crate::Options;

/// Returns the complexity-metric source table for `opts`.
///
/// When grouping is active, this names the pre-aggregated
/// `complexity_metrics_grouped` materialised by `apply_grouping`; the
/// table has one row per group with cognitive + MI already collapsed
/// via `MAX`. Wrapping the read in `MAX(cognitive) GROUP BY path` is a
/// no-op on this source, so analyses can use the SAME CTE shape under
/// either source — only the `FROM` table swaps.
#[must_use]
pub fn source_table(opts: &Options) -> &'static str {
    if opts.group_file.is_some() {
        "complexity_metrics_grouped"
    } else {
        "complexity_metrics"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn returns_raw_table_without_grouping() {
        let opts = Options::default();
        assert_eq!(source_table(&opts), "complexity_metrics");
    }

    #[test]
    fn returns_grouped_table_when_group_file_set() {
        let opts = Options {
            group_file: Some(PathBuf::from("groups.txt")),
            ..Options::default()
        };
        assert_eq!(source_table(&opts), "complexity_metrics_grouped");
    }
}
