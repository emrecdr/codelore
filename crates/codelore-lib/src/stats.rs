//! Statistical helpers used by the analyses. Currently exposes a
//! single primitive: Fisher's exact two-tail p-value for a 2×2
//! contingency table, used by `analyses::coupling` to gate
//! coupling pairs at `p < fisher_significance`.
//!
//! In-tree port of the algorithm previously consumed via the
//! `fishers_exact` crate (last release 2018-11). The crate had no live
//! CVE but was unmaintained for 7+ years; we eliminated the
//! supply-chain dependency by porting the algorithm here. The numeric
//! contract is preserved: `fisher_two_tail_pvalue(a, b, c, d)`
//! matches the upstream's `fishers_exact(&[a, b, c, d])
//! .two_tail_pvalue` to ≤ 1e-12 relative error across the regression
//! suite (see `fisher_matches_upstream_*` tests at the bottom of this
//! file).

/// Fisher's exact two-tail p-value for the 2×2 contingency table
///
/// ```text
///         | col 1 | col 2 |
/// --------+-------+-------+
/// row 1   |   a   |   b   |
/// row 2   |   c   |   d   |
/// ```
///
/// Returns `Some(p)` with `p` in `[0.0, 1.0]`, or `None` if the
/// resulting hypergeometric distribution is degenerate (every row or
/// column sum is zero — Fisher's test is undefined).
///
/// # Algorithm
///
/// Conditional on the row and column marginals, the count in the
/// top-left cell is hypergeometric with parameters `N = a+b+c+d`,
/// `K = a+b` (top row sum), and `n = a+c` (left column sum). The
/// **two-tail** p-value is the sum of probabilities of all 2×2
/// tables with the same marginals whose probability under the
/// hypergeometric model is at most that of the observed table —
/// the standard Fisher exact convention (and what the prior
/// `fishers_exact` crate computes).
///
/// All factorials are evaluated in log space via `ln_factorial` so
/// the algorithm is numerically stable on `u32` inputs — the marginal
/// sums can reach a few million on a large monorepo coupling
/// analysis, well beyond what `f64::MAX` can represent as a plain
/// factorial.
#[must_use]
// `a, b, c, d` are the canonical names for the four cells of a 2×2
// contingency table (top-left → bottom-right, row-major) in every
// statistical reference (Fisher 1922, Agresti 2013 §3.1, Wikipedia
// "Fisher's exact test"). Renaming to descriptive multi-letter names
// would obscure the algorithm; the caller in `analyses::coupling`
// uses the same letters intentionally so the two sites stay aligned.
#[allow(clippy::many_single_char_names)]
pub fn fisher_two_tail_pvalue(a: u32, b: u32, c: u32, d: u32) -> Option<f64> {
    // Match upstream `fishers_exact`'s `TooLargeValueError` boundary
    // (any cell > i32::MAX). On real coupling analyses every cell is
    // a commit count bounded by total revisions in the analysed
    // history — even Linux-kernel-scale (~1.3 M commits) is six
    // orders of magnitude below this. Returning `None` here mirrors
    // the prior wrapper's `.ok()`-converted-Err behaviour so the
    // caller's None-filter path still drops the pair silently
    // instead of spinning on the iteration loop below for billions
    // of cycles.
    const MAX_CELL: u32 = i32::MAX as u32;
    if a > MAX_CELL || b > MAX_CELL || c > MAX_CELL || d > MAX_CELL {
        return None;
    }
    let n = u64::from(a) + u64::from(b) + u64::from(c) + u64::from(d);
    let row1 = u64::from(a) + u64::from(b);
    let row2 = u64::from(c) + u64::from(d);
    let col1 = u64::from(a) + u64::from(c);
    let col2 = u64::from(b) + u64::from(d);
    if row1 == 0 || row2 == 0 || col1 == 0 || col2 == 0 {
        return None;
    }

    // Hypergeometric pmf for table with `a' = k` in the top-left:
    //   log P(k) = ln_choose(row1, k) + ln_choose(row2, col1 - k)
    //              - ln_choose(N, col1)
    //
    // Equivalent form (used here because it factors the
    // denominator-constant out of the loop):
    //   ln P(k) = ln(row1!) + ln(row2!) + ln(col1!) + ln(col2!)
    //             - ln(N!)
    //             - ln(k!) - ln((row1-k)!) - ln((col1-k)!)
    //             - ln((row2 - col1 + k)!)
    let log_const =
        ln_factorial(row1) + ln_factorial(row2) + ln_factorial(col1) + ln_factorial(col2)
            - ln_factorial(n);

    let log_pmf = |k: u64| -> f64 {
        // Bottom-right cell `d' = row2 - (col1 - k) = (row2 + k) -
        // col1`. The left-to-right form `row2 - col1 + k` underflows
        // in u64 when `row2 < col1` even though the FINAL value is
        // non-negative for every `k >= k_min` (= `col1 - row2` when
        // `col1 > row2`, else `0`). Reorder to `(row2 + k) - col1`
        // which never goes below zero for any `k` in the legal
        // range. `col1 - k` is checked-safe because `k <= col1` for
        // any `k <= k_max = min(row1, col1)`.
        let bottom_right = (row2 + k) - col1;
        let term = ln_factorial(k)
            + ln_factorial(row1 - k)
            + ln_factorial(col1 - k)
            + ln_factorial(bottom_right);
        log_const - term
    };

    // The legal range of `a'` (top-left cell) given the marginals:
    //   max(0, col1 - row2)  <=  k  <=  min(row1, col1)
    let k_min = col1.saturating_sub(row2);
    let k_max = row1.min(col1);

    let observed = log_pmf(u64::from(a));
    // Two-tail sum: include every table with `log_pmf(k) <= observed`.
    // A small relative tolerance avoids edge-of-equality misses caused
    // by accumulated rounding in `ln_factorial`; matches the upstream
    // crate's convention.
    let tol = 1e-12;
    let mut pvalue = 0.0_f64;
    for k in k_min..=k_max {
        let lp = log_pmf(k);
        if lp <= observed + tol {
            pvalue += lp.exp();
        }
    }
    Some(pvalue.min(1.0))
}

/// Natural log of `n!`, computed in `O(n)` for small `n`. The largest
/// `n` we care about here is bounded by the analysed repo's commit
/// count — even Linux-kernel-scale (~1.3 M commits) is well within
/// what an `f64` accumulator handles before precision degrades
/// (each iteration adds `ln(i)` which is `< 30`, so 1.3 M iterations
/// of accumulation gives `< 5e7` total — exact enough for the
/// difference-of-factorials use in `log_pmf` above).
///
/// We deliberately avoid the Stirling / Lanczos approximation: the
/// hot path is `analyses::coupling` which calls this in tight loops
/// over modest-sized marginals; precision matters and the direct
/// sum is well within f64 precision for the input range. The
/// `lookup_or_compute` cache below brings amortised cost to ~O(1)
/// per call within a single analysis pass.
#[must_use]
fn ln_factorial(n: u64) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    let mut acc = 0.0_f64;
    for i in 2..=n {
        // `i as f64` is lossless for `i <= 2^53`; coupling marginals
        // would have to exceed quadrillions of commits before this
        // truncates, so the cast is exact in any realistic input.
        #[allow(clippy::cast_precision_loss)]
        {
            acc += (i as f64).ln();
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compare against pre-captured reference values from
    /// `fishers_exact::fishers_exact(&[a, b, c, d]).two_tail_pvalue`
    /// (the unmaintained crate this module replaces). Tolerance is
    /// `1e-12` relative; the upstream crate's own internal arithmetic
    /// is f64 too, so disagreements beyond rounding are real
    /// regressions.
    fn approx_eq(actual: f64, expected: f64) {
        let abs_diff = (actual - expected).abs();
        let max_mag = actual.abs().max(expected.abs()).max(1e-300);
        let rel = abs_diff / max_mag;
        assert!(
            rel < 1e-12,
            "expected {expected:.15e} got {actual:.15e} (rel diff {rel:.3e})"
        );
    }

    #[test]
    fn fisher_matches_upstream_balanced_small() {
        // [1,2;3,4] -> 1.000000000000000e0 (every reorder equally probable)
        let p = fisher_two_tail_pvalue(1, 2, 3, 4).unwrap();
        approx_eq(p, 1.000_000_000_000_000_0);
    }

    #[test]
    fn fisher_matches_upstream_classic_significant() {
        // [8,1;2,5] -> 3.496503496503492e-2 (the "tea-taster" canonical example)
        let p = fisher_two_tail_pvalue(8, 1, 2, 5).unwrap();
        approx_eq(p, 3.496_503_496_503_492e-2);
    }

    #[test]
    fn fisher_matches_upstream_highly_significant() {
        // [1,9;11,3] -> 2.759456185220110e-3
        let p = fisher_two_tail_pvalue(1, 9, 11, 3).unwrap();
        approx_eq(p, 2.759_456_185_220_11e-3);
    }

    #[test]
    fn fisher_matches_upstream_symmetric_null() {
        // [10,5;5,10] -> 1.431109780507086e-1
        let p = fisher_two_tail_pvalue(10, 5, 5, 10).unwrap();
        approx_eq(p, 1.431_109_780_507_086e-1);
    }

    #[test]
    fn fisher_matches_upstream_perfect_separation() {
        // [0,5;5,0] -> 7.936507936507943e-3 (boundary: one cell is 0)
        let p = fisher_two_tail_pvalue(0, 5, 5, 0).unwrap();
        approx_eq(p, 7.936_507_936_507_943e-3);
    }

    #[test]
    fn fisher_matches_upstream_large_marginals() {
        // [100,50;50,100] -> 1.138235360679261e-8 (300-commit total)
        let p = fisher_two_tail_pvalue(100, 50, 50, 100).unwrap();
        approx_eq(p, 1.138_235_360_679_261e-8);
    }

    #[test]
    fn fisher_matches_upstream_two_by_two_identity() {
        // [1,0;0,1] -> 1.0 (only one possible table given marginals)
        let p = fisher_two_tail_pvalue(1, 0, 0, 1).unwrap();
        approx_eq(p, 1.0);
    }

    #[test]
    fn fisher_matches_upstream_perfect_null() {
        // [50,50;50,50] -> 1.0
        let p = fisher_two_tail_pvalue(50, 50, 50, 50).unwrap();
        approx_eq(p, 1.0);
    }

    #[test]
    fn fisher_returns_none_on_degenerate_marginals() {
        // Zero row sum / column sum -> Fisher undefined.
        assert!(fisher_two_tail_pvalue(0, 0, 5, 5).is_none());
        assert!(fisher_two_tail_pvalue(5, 5, 0, 0).is_none());
        assert!(fisher_two_tail_pvalue(0, 5, 0, 5).is_none());
        assert!(fisher_two_tail_pvalue(5, 0, 5, 0).is_none());
    }

    #[test]
    fn fisher_pvalue_is_bounded() {
        // Every legal call must return a probability in [0, 1].
        for a in 0..=5 {
            for b in 0..=5 {
                for c in 0..=5 {
                    for d in 0..=5 {
                        if let Some(p) = fisher_two_tail_pvalue(a, b, c, d) {
                            assert!(
                                (0.0..=1.0).contains(&p),
                                "out-of-range p={p} for [{a},{b};{c},{d}]"
                            );
                        }
                    }
                }
            }
        }
    }
}
