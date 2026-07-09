//! Release-cadence analysis — inter-release gap statistics from git tags.
//!
//! ## What it measures
//!
//! Tags are a proxy for releases, not deployments. A team that cuts hotfix
//! tags frequently will show short gaps; a team with long-lived release
//! branches will show large gaps. The trend signal (accelerating / stable /
//! slowing) helps detect drift in release velocity over time without
//! requiring any external ticketing or deployment data.
//!
//! ## Algorithm
//!
//! 1. Fetch all tags via [`Repo::tags`] (sorted ascending by date).
//! 2. Filter to names matching `opts.release_tag_glob` (default `v*`).
//! 3. Compute `days_since_prev` for each tag as the float difference
//!    between consecutive tag dates. The first tag has `None`.
//! 4. Compute summary statistics over the gap series:
//!    - **median** (middle gap for odd N; average of two middle for even N).
//!    - **IQR** (Q3 − Q1; P75 − P25 by linear interpolation).
//!    - **trend**: sign of the ordinary-least-squares slope fitted to the
//!      gap sequence (x = 0-based index, y = days). Threshold ±0.1
//!      day/release: positive slope → `"slowing"`, negative → `"accelerating"`,
//!      within ±0.1 → `"stable"`.
//! 5. Emit per-tag rows sorted by date ascending, then a synthetic
//!    `tag = "__summary__"` row carrying median, IQR, and trend in the
//!    `days_since_prev` field (median) and the `date` field (IQR as a
//!    formatted string) and `trend` field. See [`ReleaseCadenceRow`] docs.

use crate::repo::Repo;
use crate::{CodeLoreError, Options, Result};
use globset::Glob;

/// One tag in the release timeline, plus a synthetic summary row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseCadenceRow {
    /// Short tag name (e.g. `"v1.2.0"`), or `"__summary__"` for the
    /// summary row.
    pub tag: String,
    /// ISO-8601 date string (`"YYYY-MM-DD"`) of the tag. For the summary
    /// row this carries the IQR formatted as `"iqr=N.NNd"`.
    pub date: String,
    /// Days since the previous release tag (float). `None` for the first
    /// tag (no predecessor). For the summary row, carries the **median**
    /// gap in days.
    pub days_since_prev: Option<f64>,
    /// `"accelerating"` / `"stable"` / `"slowing"` for the summary row;
    /// empty string for per-tag rows.
    pub trend: String,
}

/// Least-squares slope of y over equally-spaced integer x (0, 1, …, n-1).
/// Returns `None` when fewer than 2 points.
fn ols_slope(ys: &[f64]) -> Option<f64> {
    let n = ys.len();
    if n < 2 {
        return None;
    }
    // `n` and `i` are tag counts; realistically < 10_000, well within f64
    // mantissa (2^53), so the precision-loss cast is exact in practice.
    #[allow(clippy::cast_precision_loss)]
    let n_f = n as f64;
    // x_mean = (n-1)/2
    let x_mean = (n_f - 1.0) / 2.0;
    let y_mean = ys.iter().sum::<f64>() / n_f;
    let mut num = 0.0_f64;
    let mut den = 0.0_f64;
    for (i, &y) in ys.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let xi = i as f64 - x_mean;
        num += xi * (y - y_mean);
        den += xi * xi;
    }
    if den < f64::EPSILON {
        return Some(0.0);
    }
    Some(num / den)
}

/// Median of a non-empty sorted slice.
fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        // Use halving to avoid potential overflow on very large values.
        sorted[n / 2 - 1] / 2.0 + sorted[n / 2] / 2.0
    }
}

/// Percentile by linear interpolation (0-indexed rank method).
/// `p` in [0, 1]. Requires `sorted` non-empty.
fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    // `n` is a small tag count (< 10_000); idx is in [0, n-1] so the
    // truncation and sign casts are safe.
    #[allow(clippy::cast_precision_loss)]
    let idx = p * (n as f64 - 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    #[allow(clippy::cast_precision_loss)]
    let frac = idx - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// Run the release-cadence analysis.
///
/// Returns per-tag rows sorted ascending by date, followed by a
/// `"__summary__"` row. Returns an empty `Vec` when no tags match the
/// glob (no summary row is emitted in that case).
pub fn run_release_cadence<R: Repo>(repo: &R, opts: &Options) -> Result<Vec<ReleaseCadenceRow>> {
    // Compile the glob once; error at the analysis boundary.
    let matcher = Glob::new(&opts.release_tag_glob)
        .map_err(|e| {
            CodeLoreError::InvalidOptions(format!(
                "--release-tag-glob {:?} is not a valid glob: {e}",
                opts.release_tag_glob
            ))
        })?
        .compile_matcher();

    let all_tags = repo.tags()?;
    let filtered: Vec<_> = all_tags
        .into_iter()
        .filter(|t| matcher.is_match(&t.name))
        .collect();

    if filtered.is_empty() {
        return Ok(Vec::new());
    }

    // Compute per-tag rows and collect the gap series.
    let mut rows: Vec<ReleaseCadenceRow> = Vec::with_capacity(filtered.len() + 1);
    let mut gaps: Vec<f64> = Vec::with_capacity(filtered.len().saturating_sub(1));

    for (i, tag) in filtered.iter().enumerate() {
        let days_since_prev = if i == 0 {
            None
        } else {
            let prev = &filtered[i - 1];
            // Duration in whole seconds → fractional days. Durations between
            // release tags are days-to-years; i64 seconds fits comfortably in
            // f64 for any realistic project lifetime (< 2^40 s).
            #[allow(clippy::cast_precision_loss)]
            let secs = (tag.date - prev.date).whole_seconds() as f64;
            let d = secs / 86_400.0;
            gaps.push(d);
            Some(d)
        };
        rows.push(ReleaseCadenceRow {
            tag: tag.name.clone(),
            date: format!(
                "{:04}-{:02}-{:02}",
                tag.date.year(),
                tag.date.month() as u8,
                tag.date.day()
            ),
            days_since_prev,
            trend: String::new(),
        });
    }

    // Summary row: only when there is at least one gap.
    if gaps.is_empty() {
        return Ok(rows);
    }

    let mut sorted_gaps = gaps.clone();
    sorted_gaps.sort_by(f64::total_cmp);

    let median = median_sorted(&sorted_gaps);
    let q1 = percentile_sorted(&sorted_gaps, 0.25);
    let q3 = percentile_sorted(&sorted_gaps, 0.75);
    let iqr = q3 - q1;

    let trend = match ols_slope(&gaps) {
        Some(slope) if slope > 0.1 => "slowing",
        Some(slope) if slope < -0.1 => "accelerating",
        _ => "stable",
    };

    rows.push(ReleaseCadenceRow {
        tag: "__summary__".to_string(),
        // Encode IQR in the date field so callers can retrieve it without
        // a schema change. Format: "iqr=N.NNd" where N is days (2 d.p.).
        date: format!("iqr={iqr:.2}d"),
        days_since_prev: Some(median),
        trend: trend.to_string(),
    });

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ols_slope_rising() {
        // gaps [10, 20, 30] → slope = 10
        let s = ols_slope(&[10.0, 20.0, 30.0]).unwrap();
        assert!((s - 10.0).abs() < 1e-9, "slope={s}");
    }

    #[test]
    fn ols_slope_flat() {
        let s = ols_slope(&[5.0, 5.0, 5.0]).unwrap();
        assert!(s.abs() < 1e-9, "slope={s}");
    }

    #[test]
    fn ols_slope_none_for_single_point() {
        assert!(ols_slope(&[42.0]).is_none());
    }

    #[test]
    fn median_odd() {
        // sorted odd-length slice: middle element
        let got = median_sorted(&[1.0, 3.0, 5.0]);
        assert!((got - 3.0).abs() < 1e-12, "got {got}");
    }

    #[test]
    fn median_even() {
        // sorted even-length slice: average of two middle elements
        let got = median_sorted(&[10.0, 20.0]);
        assert!((got - 15.0).abs() < 1e-12, "got {got}");
    }

    #[test]
    fn percentile_single() {
        let got = percentile_sorted(&[7.0], 0.25);
        assert!((got - 7.0).abs() < 1e-12, "got {got}");
    }

    #[test]
    fn trend_slowing() {
        // slope = 10 > 0.1 → slowing
        let gaps = vec![10.0, 20.0, 30.0];
        let slope = ols_slope(&gaps).unwrap();
        let trend = if slope > 0.1 {
            "slowing"
        } else if slope < -0.1 {
            "accelerating"
        } else {
            "stable"
        };
        assert_eq!(trend, "slowing");
    }

    #[test]
    fn trend_accelerating() {
        let gaps = vec![30.0, 20.0, 10.0];
        let slope = ols_slope(&gaps).unwrap();
        let trend = if slope > 0.1 {
            "slowing"
        } else if slope < -0.1 {
            "accelerating"
        } else {
            "stable"
        };
        assert_eq!(trend, "accelerating");
    }

    #[test]
    fn trend_stable_near_zero() {
        // gaps [10, 10] → slope = 0.0 → stable
        let gaps = vec![10.0, 10.0];
        let slope = ols_slope(&gaps).unwrap();
        let trend = if slope > 0.1 {
            "slowing"
        } else if slope < -0.1 {
            "accelerating"
        } else {
            "stable"
        };
        assert_eq!(trend, "stable");
    }
}
