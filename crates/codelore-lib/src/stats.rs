//! Statistical helpers used by the analyses.
//!
//! Fisher's exact two-tail p-value for a 2×2 contingency table, used
//! by `analyses::coupling` to gate coupling pairs at
//! `p < fisher_significance`.
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
//!
//! `auc` and `precision_at_k` are ranking-quality metrics for the
//! own-repo defect-calibration validation pass: given a per-file score
//! (e.g. a code-health structural-risk value) and a binary defect
//! label, they answer whether the score actually separates the
//! defective files from the rest. Both are sort-based with no
//! external dependencies.

use std::cell::RefCell;
use std::cmp::Ordering;

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

/// Natural log of `n!`.
///
/// Backed by a lazily-grown cumulative-sum table: entry `k` holds
/// `ln(k!)`, so a lookup is O(1) and the table extends only to the
/// largest `n` seen. The largest `n` here is bounded by the analysed
/// repo's commit count, well within `f64` precision for the
/// difference-of-factorials use in `log_pmf`.
///
/// The direct cumulative sum is used over a Stirling / Lanczos
/// approximation deliberately: the hot path is `analyses::coupling`,
/// which calls this in tight loops over modest marginals where
/// precision matters and the sum stays within `f64` precision for the
/// input range.
///
/// The table lives in a `thread_local!` `RefCell<Vec<f64>>`; `ln(k!)`
/// is a pure constant, so a per-thread memo is race-free and fits the
/// single-connection, `!Send` analysis model.
#[must_use]
fn ln_factorial(n: u64) -> f64 {
    thread_local! {
        // Entry `k` == ln(k!). Seeded with ln(0!) = ln(1!) = 0.
        static TABLE: RefCell<Vec<f64>> = RefCell::new(vec![0.0, 0.0]);
    }
    TABLE.with(|cell| {
        let mut table = cell.borrow_mut();
        // `n` is a marginal count bounded by the analysed repo's commit
        // count, which never approaches `usize::MAX` even on 32-bit targets.
        #[allow(clippy::cast_possible_truncation)]
        let idx = n as usize;
        if idx >= table.len() {
            // Extend by continuing the running sum from the current high-water
            // mark: each new entry is `previous + ln(k)` in strictly increasing
            // `k`, so the accumulation order — and every rounding — is identical
            // to a fresh `sum_{i=2}^{n} ln(i)`, independent of call order.
            let mut acc = *table.last().expect("seeded with two entries");
            for k in table.len() as u64..=n {
                #[allow(clippy::cast_precision_loss)]
                {
                    acc += (k as f64).ln();
                }
                table.push(acc);
            }
        }
        table[idx]
    })
}

/// Area under the ROC curve for binary labels ranked by score, computed via
/// the Mann-Whitney U statistic with midpoint tie handling. None when either
/// class is empty.
///
/// NaN scores are accepted, never panic, and rank deterministically at an
/// extreme of the ordering (IEEE-754 total order via `f64::total_cmp`) —
/// callers wanting NaN rejected must filter beforehand.
///
/// # Algorithm
///
/// Sort all `(score, label)` pairs ascending by score and assign each a
/// 1-based rank; a run of tied scores shares the **average** of the ranks
/// it would otherwise occupy (the standard mid-rank tie-breaking rule for
/// the Mann-Whitney U statistic). Summing the ranks of the positive-labeled
/// rows and subtracting `n_pos * (n_pos + 1) / 2` (the minimum possible sum,
/// reached when every positive ranks below every negative) yields U, which
/// counts — across every positive/negative pair — how many pairs rank the
/// positive above the negative (a tie contributing one half). Normalizing
/// by `n_pos * n_neg` gives the AUC: the probability a randomly chosen
/// positive scores higher than a randomly chosen negative.
///
/// Comparisons use `f64::total_cmp` rather than `==`/`<` so scores sort into
/// a well-defined total order (and ties are detected via `Ordering::Equal`)
/// without tripping over partial-ordering edge cases.
#[must_use]
pub fn auc(scored: &[(f64, bool)]) -> Option<f64> {
    let n_pos = scored.iter().filter(|(_, label)| *label).count();
    let n_neg = scored.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return None;
    }

    let mut order: Vec<usize> = (0..scored.len()).collect();
    order.sort_by(|&i, &j| scored[i].0.total_cmp(&scored[j].0));

    // Assign 1-based average ranks to each run of tied scores.
    let mut ranks = vec![0.0_f64; scored.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i + 1;
        while j < order.len()
            && scored[order[j]].0.total_cmp(&scored[order[i]].0) == Ordering::Equal
        {
            j += 1;
        }
        // The tied run occupies 1-based ranks `i+1..=j`; its shared rank is
        // their average. `i` and `j` are bounded by `scored.len()`,
        // realistically at most a few hundred thousand rows in an analysis
        // pass — well within f64's exact-integer range, so this cast is
        // lossless.
        #[allow(clippy::cast_precision_loss)]
        let avg_rank = (i + 1 + j) as f64 / 2.0;
        for &idx in &order[i..j] {
            ranks[idx] = avg_rank;
        }
        i = j;
    }

    let rank_sum_pos: f64 = scored
        .iter()
        .zip(&ranks)
        .filter_map(|((_, label), &rank)| label.then_some(rank))
        .sum();

    // Same lossless-cast rationale as above: counts bounded by `scored.len()`.
    #[allow(clippy::cast_precision_loss)]
    let (n_pos_f, n_neg_f) = (n_pos as f64, n_neg as f64);
    let u = rank_sum_pos - n_pos_f * (n_pos_f + 1.0) / 2.0;
    Some(u / (n_pos_f * n_neg_f))
}

/// Of the k highest-scored items (ties broken by stable input order), the
/// fraction labeled positive. None when k == 0 or k > len.
///
/// Items are sorted descending by score with a stable sort, so items with
/// equal scores keep their relative order from `scored` — the top k is
/// therefore deterministic for any input, not dependent on sort
/// implementation details. NaN scores are accepted and rank
/// deterministically per `f64::total_cmp`, as in [`auc`].
#[must_use]
pub fn precision_at_k(scored: &[(f64, bool)], k: usize) -> Option<f64> {
    if k == 0 || k > scored.len() {
        return None;
    }

    let mut order: Vec<usize> = (0..scored.len()).collect();
    order.sort_by(|&i, &j| scored[j].0.total_cmp(&scored[i].0));

    let positives = order[..k].iter().filter(|&&idx| scored[idx].1).count();
    // Lossless: `positives <= k <= scored.len()`, realistically bounded by
    // the number of files in an analysis pass.
    #[allow(clippy::cast_precision_loss)]
    let result = positives as f64 / k as f64;
    Some(result)
}

/// Benjamini-Hochberg FDR step-up p-value cutoff for a family of p-values
/// at false-discovery-rate level `q`. Returns the largest p(k) with
/// p(k) <= (k/m)*q (sorted ascending); every p <= the returned cutoff is a
/// discovery. Returns `f64::NEG_INFINITY` (reject nothing) for an empty
/// family or when no rank satisfies the criterion.
#[must_use]
pub fn bh_fdr_threshold(pvalues: &[f64], q: f64) -> f64 {
    let m = pvalues.len();
    if m == 0 {
        return f64::NEG_INFINITY;
    }
    let mut sorted = pvalues.to_vec();
    sorted.sort_by(f64::total_cmp);
    #[allow(clippy::cast_precision_loss)]
    let m_f = m as f64;
    for k in (1..=m).rev() {
        #[allow(clippy::cast_precision_loss)]
        let crit = (k as f64 / m_f) * q;
        if sorted[k - 1] <= crit {
            return sorted[k - 1];
        }
    }
    f64::NEG_INFINITY
}

/// Wilson score 95% confidence interval for a proportion `k / n`.
///
/// Returns `(low, high)` in `[0.0, 1.0]`. Returns `(0.0, 0.0)` when `n = 0`
/// (undefined proportion). Edge cases `k = 0` and `k = n` are handled correctly
/// by the formula without special-casing.
///
/// A thin wrapper over [`wilson_ci_from_proportion`] that forms the point
/// estimate `p_hat = k / n` from an integer success count — the interval a
/// `k`-of-`n` binomial proportion carries. Standard Wilson score interval
/// (Wilson 1927); z = 1.96 for 95% coverage.
///
/// Parameters are `u32` (not `u64`) to avoid precision-loss on the f64
/// conversion — all realistic counts fit comfortably in 32 bits.
#[must_use]
pub fn wilson_ci(k: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let p_hat = f64::from(k) / f64::from(n);
    wilson_ci_from_proportion(p_hat, n)
}

/// Wilson score 95% confidence interval around an already-computed proportion
/// `p_hat` observed over a pool of `n`.
///
/// Unlike [`wilson_ci`], the point estimate is supplied directly rather than
/// formed from an integer `k / n`. This is the entry point the corpus-percentile
/// lens uses: the reported percentile is a midpoint-rank (or breakpoint-
/// interpolated) empirical-CDF estimate, not an integer success count, and the
/// interval must wrap that SAME estimate so `low <= p_hat <= high` holds. `n` is
/// the honest pool size behind the estimate — the number of corpus repos in a
/// repo-level pool, or a language's pooled per-function sample count.
///
/// Returns `(low, high)` clamped to `[0.0, 1.0]`; `(0.0, 0.0)` when `n = 0`.
/// `p_hat` is assumed in `[0.0, 1.0]` (every caller's estimator is a percentile),
/// which keeps the radicand non-negative so the result is never `NaN`. Standard
/// Wilson score interval (Wilson 1927); z = 1.96 for 95% coverage.
#[must_use]
pub fn wilson_ci_from_proportion(p_hat: f64, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let n = f64::from(n);
    let z = 1.96_f64;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let centre = (p_hat + z2 / (2.0 * n)) / denom;
    let radius = (z / denom) * (p_hat * (1.0 - p_hat) / n + z2 / (4.0 * n * n)).sqrt();
    (
        f64::max(0.0, centre - radius),
        f64::min(1.0, centre + radius),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ln_factorial_direct(n: u64) -> f64 {
        if n <= 1 {
            return 0.0;
        }
        let mut acc = 0.0_f64;
        for i in 2..=n {
            #[allow(clippy::cast_precision_loss)]
            {
                acc += (i as f64).ln();
            }
        }
        acc
    }

    #[test]
    fn ln_factorial_is_bit_identical_to_direct_sum() {
        for n in 0..=50u64 {
            assert_eq!(
                ln_factorial(n).to_bits(),
                ln_factorial_direct(n).to_bits(),
                "cached ln_factorial({n}) must be bit-identical to the direct sum"
            );
        }
        for &n in &[100u64, 1_000, 100_000] {
            assert_eq!(ln_factorial(n).to_bits(), ln_factorial_direct(n).to_bits());
        }
        // Call order must not perturb the result.
        assert_eq!(ln_factorial(7).to_bits(), ln_factorial_direct(7).to_bits());
    }

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

    #[test]
    fn auc_perfect_separation_is_one() {
        // Every positive scores strictly above every negative.
        let scored = [(0.9, true), (0.8, true), (0.3, false), (0.2, false)];
        assert_eq!(auc(&scored), Some(1.0));
    }

    #[test]
    fn auc_reversed_scores_is_zero() {
        // Every positive scores strictly below every negative.
        let scored = [(0.2, true), (0.3, true), (0.8, false), (0.9, false)];
        assert_eq!(auc(&scored), Some(0.0));
    }

    #[test]
    fn auc_tie_case_matches_hand_derivation() {
        // scores [0.9,0.7,0.7,0.1], labels [pos,pos,neg,neg]. Pairwise:
        // 0.9 beats both negatives (2), the tied 0.7 vs 0.7 counts as a
        // half-win (0.5), and 0.7 beats 0.1 (1). Total 3.5 / 4 pairs = 0.875.
        let scored = [(0.9, true), (0.7, true), (0.7, false), (0.1, false)];
        assert_eq!(auc(&scored), Some(0.875));
    }

    #[test]
    fn auc_returns_none_when_positive_class_empty() {
        let scored = [(0.9, false), (0.1, false)];
        assert_eq!(auc(&scored), None);
    }

    #[test]
    fn auc_returns_none_when_negative_class_empty() {
        let scored = [(0.9, true), (0.1, true)];
        assert_eq!(auc(&scored), None);
    }

    #[test]
    fn precision_at_k_matches_known_top_two() {
        // Sorted descending: 0.9(pos), 0.7(neg), 0.5(pos), 0.3(neg), 0.1(pos).
        // Top 2 are 0.9 (pos) and 0.7 (neg) -> 1 of 2 positive.
        let scored = [
            (0.9, true),
            (0.7, false),
            (0.5, true),
            (0.3, false),
            (0.1, true),
        ];
        assert_eq!(precision_at_k(&scored, 2), Some(0.5));
    }

    #[test]
    fn precision_at_k_breaks_ties_by_input_order() {
        // All three scores tie at 0.5; stable descending sort keeps input
        // order, so the top 2 are index 0 (pos) and index 1 (neg) -> 0.5.
        let scored = [(0.5, true), (0.5, false), (0.5, true)];
        assert_eq!(precision_at_k(&scored, 2), Some(0.5));
    }

    #[test]
    fn precision_at_k_returns_none_for_k_zero() {
        let scored = [(0.9, true), (0.1, false)];
        assert_eq!(precision_at_k(&scored, 0), None);
    }

    #[test]
    fn precision_at_k_returns_none_when_k_exceeds_len() {
        let scored = [(0.9, true), (0.1, false)];
        assert_eq!(precision_at_k(&scored, 3), None);
    }

    #[test]
    fn bh_fdr_threshold_matches_hand_computed() {
        // The cutoff is always one of the input p-values (or NEG_INFINITY),
        // returned verbatim with no arithmetic, so bit-equality is the exact
        // and correct comparison here — as in `ln_factorial_is_bit_identical`.
        fn bits_eq(a: f64, b: f64) {
            assert_eq!(a.to_bits(), b.to_bits(), "expected {b}, got {a}");
        }

        // Family of 5 p-values at q = 0.05. The per-test gate (p < 0.05) would
        // keep 4 (0.001, 0.008, 0.039, 0.041); Benjamini-Hochberg keeps only
        // the first 2. Walking k from 5 down: (5/5)·0.05=0.05 < 0.9,
        // (4/5)·0.05=0.04 < 0.041, (3/5)·0.05=0.03 < 0.039, then
        // (2/5)·0.05=0.02 ≥ 0.008 → cutoff is 0.008.
        let p = [0.001, 0.008, 0.039, 0.041, 0.9];
        bits_eq(bh_fdr_threshold(&p, 0.05), 0.008);

        // Empty family rejects nothing.
        bits_eq(bh_fdr_threshold(&[], 0.05), f64::NEG_INFINITY);
        // No p-value clears its rank's criterion → reject nothing.
        bits_eq(bh_fdr_threshold(&[0.9, 0.95], 0.05), f64::NEG_INFINITY);
        // Every p-value clears → cutoff is the largest.
        bits_eq(bh_fdr_threshold(&[0.001, 0.002], 0.05), 0.002);

        // Order-independence: the internal sort makes the cutoff invariant to
        // input order, bit-for-bit.
        let shuffled = [0.9, 0.041, 0.001, 0.039, 0.008];
        bits_eq(
            bh_fdr_threshold(&shuffled, 0.05),
            bh_fdr_threshold(&p, 0.05),
        );
    }

    #[test]
    fn wilson_ci_k_zero() {
        let (lo, hi) = wilson_ci(0, 100);
        assert!(lo >= 0.0, "lo must be ≥ 0: {lo}");
        assert!(
            hi > 0.0 && hi < 0.05,
            "hi for k=0/n=100 should be small: {hi}"
        );
    }

    #[test]
    fn wilson_ci_k_equals_n() {
        let (lo, hi) = wilson_ci(100, 100);
        assert!(lo > 0.95 && lo <= 1.0, "lo for k=n should be near 1: {lo}");
        assert!(
            (hi - 1.0).abs() < 1e-9,
            "hi for k=n should be exactly 1: {hi}"
        );
    }

    #[test]
    fn wilson_ci_half() {
        let (lo, hi) = wilson_ci(50, 100);
        // p_hat = 0.5; Wilson CI for 0.5 with n=100, z=1.96 ≈ [0.401, 0.599]
        assert!(lo > 0.39 && lo < 0.50, "lo for k=50/n=100: {lo}");
        assert!(hi > 0.50 && hi < 0.61, "hi for k=50/n=100: {hi}");
        assert!(lo < hi, "interval must be non-empty");
    }

    #[test]
    fn wilson_ci_n_zero_returns_zeros() {
        let (lo, hi) = wilson_ci(0, 0);
        assert_eq!((lo, hi), (0.0, 0.0));
    }

    #[test]
    fn wilson_ci_interval_contains_p_hat() {
        let k = 30_u32;
        let n = 100_u32;
        let (lo, hi) = wilson_ci(k, n);
        let p_hat = f64::from(k) / f64::from(n);
        assert!(
            lo <= p_hat && p_hat <= hi,
            "interval must contain p_hat={p_hat}: [{lo}, {hi}]"
        );
    }

    #[test]
    fn wilson_ci_from_proportion_matches_integer_form() {
        // Supplying p_hat = k/n directly must reproduce the integer-`k` form
        // exactly — `wilson_ci` is defined as this call with `p_hat = k/n`.
        let (a_lo, a_hi) = wilson_ci(30, 100);
        let (b_lo, b_hi) = wilson_ci_from_proportion(0.30, 100);
        assert!((a_lo - b_lo).abs() < 1e-12, "{a_lo} vs {b_lo}");
        assert!((a_hi - b_hi).abs() < 1e-12, "{a_hi} vs {b_hi}");
    }

    #[test]
    fn wilson_ci_from_proportion_wraps_midpoint_rank_small_n() {
        // The corpus-lens case: p_hat is a midpoint-rank percentile (2/3),
        // n is tiny (3 corpus repos), so the interval is wide and must still
        // straddle the point estimate. Hand-computed: centre ≈ 0.5731,
        // radius ≈ 0.3654 → [0.2077, 0.9385].
        let p_hat = 2.0_f64 / 3.0;
        let (lo, hi) = wilson_ci_from_proportion(p_hat, 3);
        assert!(
            lo <= p_hat && p_hat <= hi,
            "must contain p_hat: [{lo}, {hi}]"
        );
        assert!((0.20..0.22).contains(&lo), "lo ≈ 0.208: {lo}");
        assert!((0.93..0.95).contains(&hi), "hi ≈ 0.939: {hi}");
        assert!(hi - lo > 0.6, "n=3 interval must be wide: {}", hi - lo);
    }

    #[test]
    fn wilson_ci_from_proportion_saturates_at_one() {
        // p_hat = 1.0 (value above every pool member), n = 2. Hand-computed:
        // centre ≈ 0.6712, radius ≈ 0.3288 → lo ≈ 0.3424, hi clamps to 1.0.
        let (lo, hi) = wilson_ci_from_proportion(1.0, 2);
        assert!((0.33..0.35).contains(&lo), "lo ≈ 0.342: {lo}");
        assert!((hi - 1.0).abs() < 1e-9, "hi must clamp to 1.0: {hi}");
    }

    #[test]
    fn wilson_ci_from_proportion_n_zero_returns_zeros() {
        assert_eq!(wilson_ci_from_proportion(0.5, 0), (0.0, 0.0));
    }

    #[test]
    fn wilson_ci_from_proportion_wider_for_smaller_n() {
        // Same point estimate, fewer observations → strictly wider interval.
        let (lo_small, hi_small) = wilson_ci_from_proportion(0.5, 10);
        let (lo_big, hi_big) = wilson_ci_from_proportion(0.5, 1000);
        assert!(
            (hi_small - lo_small) > (hi_big - lo_big),
            "n=10 width {} must exceed n=1000 width {}",
            hi_small - lo_small,
            hi_big - lo_big
        );
    }
}
