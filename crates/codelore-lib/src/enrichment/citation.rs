//! Numeric citation check for advisory narratives.
//!
//! The advisory layer asks a model to speak only from a deterministic fact
//! sheet. This module verifies the promise after the fact: it extracts every
//! number the narrative quotes and confirms each one traces back to a value the
//! fact sheet actually carries. A narrative with an uncited number is flagged
//! (via [`Groundedness`]) so the caller can stamp it `⚠ contains uncited claims`
//! rather than silently presenting an invented statistic as grounded.

use std::sync::OnceLock;

use regex::Regex;

/// Whether a narrative's numbers are all grounded in the fact sheet, and which
/// numeric tokens were not.
#[derive(Debug, Clone)]
pub struct Groundedness {
    /// True iff every non-exempt numeric token in the narrative matched a fact
    /// value — equivalently, `unmatched.is_empty()`.
    pub grounded: bool,
    /// The numeric tokens (with thousands separators already stripped, sign
    /// and `%` retained as quoted) that matched no fact value and are not
    /// exempt small integers.
    pub unmatched: Vec<String>,
}

/// Largest magnitude of a whole-number token treated as prose scaffolding
/// rather than a cited statistic. See [`check_citations`] for why these are
/// exempt.
const SMALL_INT_EXEMPTION: f64 = 12.0;

/// Check that every number a narrative quotes is grounded in the fact sheet's
/// values.
///
/// Numeric tokens are extracted with `(?:\d+\.\d+|\d+)(?:%)?` after thousands
/// separators are stripped (so `1,234` is read as `1234`), then sign-checked:
/// a leading `-` binds to the token unless it is an infix hyphen in a date or
/// range (`2026-07-15`, `defects-2026`), in which case the token stays
/// positive. A token matches iff some fact value, rounded to the token's own
/// number of decimal places, equals the token's signed value; a percent token
/// additionally matches when `fact * 100` rounds to it, so `80%` is grounded
/// by a fact of `0.803` (→ `80.3` → `80`).
///
/// Whole-number tokens with a magnitude `≤ 12` and no percent sign are
/// **exempt**: they are almost always list positions, section numbering, or
/// small counts in prose (`the 3 files`, `1.`), not cited statistics, and
/// demanding a matching fact value for them would flag ordinary writing.
/// A fact value below `fmt_num`'s resolution renders as `0`, and a narrative
/// quoting `0` is grounded both by
/// this exemption and by the standard rounding rule (a `1e-9` fact rounds to
/// `0` at zero decimal places).
///
/// `grounded` is true exactly when `unmatched` is empty.
///
/// # Known limitations — this check labels, it does not prove
///
/// Sign inversions are caught by the sign-aware extraction above. Two classes
/// of invented claims still pass as grounded, though: a **small-integer
/// statistic** covered by the `≤ 12` exemption (`a risk score of 9` passes
/// with no fact of `9`); and a **percent collision**,
/// where any fraction fact grounds an unrelated percent token via the `× 100`
/// fallback (`50%` passes whenever some fact is `0.5`). A third limitation is
/// structural rather than numeric: the check is per-number, not per-claim, so
/// a real, correctly-grounded value can still be attached to the wrong
/// statement (the right count next to the wrong file). The failure direction
/// everywhere else is the safe one — over-flagging (version-like strings
/// decompose into fragments that read as uncited) — but a `grounded ✓` stamp
/// means "every quoted magnitude appears in the evidence", not "every claim
/// is true".
#[must_use]
pub fn check_citations(narrative: &str, fact_values: &[f64]) -> Groundedness {
    let stripped = strip_thousands_separators(narrative);
    let mut unmatched = Vec::new();
    for token in token_regex().find_iter(&stripped) {
        let raw = token.as_str();
        let is_percent = raw.ends_with('%');
        let digits = raw.trim_end_matches('%');
        let decimals = digits.split_once('.').map_or(0, |(_, frac)| frac.len());
        let Ok(unsigned_value) = digits.parse::<f64>() else {
            continue;
        };
        let negated = is_unary_minus(&stripped, token.start());
        let value = if negated {
            -unsigned_value
        } else {
            unsigned_value
        };
        // Small whole numbers in prose (list positions, section numbers, "the 3
        // files") are scaffolding, not cited statistics — never flag them.
        if !is_percent && decimals == 0 && value.abs() <= SMALL_INT_EXEMPTION {
            continue;
        }
        let matched = fact_values
            .iter()
            .any(|&fact| matches_at(fact, value, decimals, is_percent));
        if !matched {
            let display = if negated {
                format!("-{raw}")
            } else {
                raw.to_string()
            };
            unmatched.push(display);
        }
    }
    Groundedness {
        grounded: unmatched.is_empty(),
        unmatched,
    }
}

/// Whether the `-` immediately preceding a token at byte offset `token_start`
/// in `text` (if any) is a unary minus rather than an infix hyphen.
///
/// A `-` is infix (a date `2026-07-15`, a vintage `defects-2026`, a range
/// `5-3`) when the character before it is alphanumeric; then the token is
/// positive, exactly as if no `-` were there. Otherwise the `-` binds to the
/// token as a sign. If there is no `-` immediately before the token at all,
/// this returns `false`.
fn is_unary_minus(text: &str, token_start: usize) -> bool {
    let prefix = &text[..token_start];
    let mut chars = prefix.chars().rev();
    let Some(prev) = chars.next() else {
        return false;
    };
    if prev != '-' {
        return false;
    }
    match chars.next() {
        Some(prev2) => !prev2.is_alphanumeric(),
        None => true,
    }
}

/// Whether `fact` grounds `token_value` at `decimals` decimal places, trying the
/// `fact * 100` reading as well for percent tokens.
fn matches_at(fact: f64, token_value: f64, decimals: usize, is_percent: bool) -> bool {
    rounds_to(fact, token_value, decimals)
        || (is_percent && rounds_to(fact * 100.0, token_value, decimals))
}

/// Whether `value` rounded to `decimals` decimal places equals `target`.
///
/// Both sides are rounded to whole units of the same `10^decimals` scale and
/// compared as integers-in-`f64`, sidestepping a direct float equality.
fn rounds_to(value: f64, target: f64, decimals: usize) -> bool {
    let factor: f64 = (0..decimals).fold(1.0, |acc, _| acc * 10.0);
    let scaled = (value * factor).round();
    let expected = (target * factor).round();
    (scaled - expected).abs() < 0.5
}

/// Compiled `(?:\d+\.\d+|\d+)(?:%)?` numeric-token matcher.
fn token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // The pattern is a compile-time literal, so compilation cannot fail; this
    // mirrors the static-regex idiom in `analyses::lineage`.
    RE.get_or_init(|| Regex::new(r"(?:\d+\.\d+|\d+)(?:%)?").unwrap())
}

/// Compiled matcher for a comma between two digits — a thousands separator.
fn thousands_sep_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Compile-time literal pattern; compilation cannot fail (see `token_regex`).
    RE.get_or_init(|| Regex::new(r"(\d),(\d)").unwrap())
}

/// Remove thousands separators (`1,234` → `1234`), leaving list commas (`3, 5`)
/// untouched. Iterates because one pass leaves the separators of adjacent
/// groups in tightly packed numbers unresolved.
fn strip_thousands_separators(text: &str) -> String {
    let mut out = text.to_string();
    loop {
        let replaced = thousands_sep_regex().replace_all(&out, "$1$2").into_owned();
        if replaced == out {
            return out;
        }
        out = replaced;
    }
}

#[cfg(test)]
mod tests {
    use super::check_citations;

    #[test]
    fn fully_grounded_narrative_has_no_unmatched() {
        let facts = [87.5, 0.803];
        let g = check_citations("Health score 87.5 with coupling 0.803.", &facts);
        assert!(g.grounded);
        assert!(g.unmatched.is_empty());
    }

    #[test]
    fn one_invented_number_is_listed_unmatched() {
        let facts = [0.786, 0.803];
        let g = check_citations(
            "Grounded 0.786 and 0.803, but invented 42.5 appears too.",
            &facts,
        );
        assert!(!g.grounded);
        assert_eq!(g.unmatched, vec!["42.5".to_string()]);
    }

    #[test]
    fn rounded_citation_is_grounded() {
        let facts = [0.786];
        let g = check_citations("about 0.79 coupling", &facts);
        assert!(g.grounded, "unmatched: {:?}", g.unmatched);
    }

    #[test]
    fn percent_citation_matches_fraction() {
        let facts = [0.803];
        let g = check_citations("roughly 80% of changes", &facts);
        assert!(g.grounded, "unmatched: {:?}", g.unmatched);
    }

    #[test]
    fn small_whole_numbers_are_exempt() {
        let facts: [f64; 0] = [];
        let g = check_citations("the 3 files in this module", &facts);
        assert!(g.grounded);
        assert!(g.unmatched.is_empty());
    }

    #[test]
    fn empty_narrative_is_grounded() {
        let facts = [1.0, 2.0];
        let g = check_citations("", &facts);
        assert!(g.grounded);
    }

    #[test]
    fn zero_matches_subepsilon_fact_value() {
        // `fmt_num` renders sub-1e-6 fact values as "0"; a narrative quoting "0"
        // must be grounded by such a fact value.
        let facts = [1e-9];
        let g = check_citations("effectively 0 signal here", &facts);
        assert!(g.grounded);
    }

    #[test]
    fn thousands_separator_is_stripped_before_matching() {
        let facts = [1234.0];
        let g = check_citations("touched 1,234 lines", &facts);
        assert!(g.grounded, "unmatched: {:?}", g.unmatched);
    }

    #[test]
    fn large_uncited_whole_number_is_flagged() {
        let facts = [3.0];
        let g = check_citations("spanning 4200 revisions", &facts);
        assert!(!g.grounded);
        assert_eq!(g.unmatched, vec!["4200".to_string()]);
    }

    #[test]
    fn signed_token_mismatching_positive_fact_is_flagged() {
        let facts = [0.5];
        let g = check_citations("a delta of -0.5", &facts);
        assert!(!g.grounded);
        assert_eq!(g.unmatched, vec!["-0.5".to_string()]);
    }

    #[test]
    fn signed_token_matching_negative_fact_is_grounded() {
        let facts = [-420.7];
        let g = check_citations("MI of -420.7", &facts);
        assert!(g.grounded, "unmatched: {:?}", g.unmatched);
    }

    #[test]
    fn positive_token_no_longer_matches_negative_fact() {
        let facts = [-0.5];
        let g = check_citations("a value of 0.5", &facts);
        assert!(!g.grounded);
        assert_eq!(g.unmatched, vec!["0.5".to_string()]);
    }

    #[test]
    fn hyphenated_date_fragments_stay_unsigned() {
        let facts: [f64; 0] = [];
        let g = check_citations("vintage defects-2026-07-15", &facts);
        assert!(!g.grounded);
        assert_eq!(g.unmatched, vec!["2026".to_string(), "15".to_string()]);
        assert!(
            g.unmatched.iter().all(|tok| !tok.starts_with('-')),
            "hyphenated date/vintage fragments must never read as negative: {:?}",
            g.unmatched
        );
    }

    #[test]
    fn negative_small_int_is_exempt() {
        let facts: [f64; 0] = [];
        let g = check_citations("a delta of -3", &facts);
        assert!(g.grounded, "unmatched: {:?}", g.unmatched);
    }

    #[test]
    fn negative_large_int_is_not_exempt() {
        let facts: [f64; 0] = [];
        let g = check_citations("a delta of -15", &facts);
        assert!(!g.grounded);
        assert_eq!(g.unmatched, vec!["-15".to_string()]);
    }

    #[test]
    fn unmatched_percent_token_reports_the_percent_sign() {
        let facts: [f64; 0] = [];
        let g = check_citations("about 99.5%", &facts);
        assert_eq!(g.unmatched, vec!["99.5%".to_string()]);
    }
}
