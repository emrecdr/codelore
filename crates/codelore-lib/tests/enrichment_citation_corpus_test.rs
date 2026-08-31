//! Citation-check evaluation over the labelled narrative corpus.
//!
//! The corpus (`fixtures/narratives/labelled_corpus.json`) pairs narratives
//! with the fact values they were checked against, a ground-truth `faithful`
//! label, and the pinned checker verdict. These tests replay every entry
//! through the real `check_citations` — never a re-implementation of its
//! matching — and assert three layers: each entry's verdict, the confusion
//! matrix the verdicts form against the ground-truth labels, and the incidence
//! of the checker's two instrumented blind spots. The aggregate numbers are
//! published in `docs/narrative-evidence-v1.md`; a corpus edit, a checker
//! change, and the doc must therefore land together, and a checker change that
//! silently moves any entry's verdict fails here naming the entry.

use std::collections::{BTreeMap, BTreeSet};

use codelore_lib::enrichment::citation::check_citations;
use serde::Deserialize;

/// The corpus fixture, frozen in-tree so the evaluation is deterministic and
/// offline — no model is involved in replaying it.
const CORPUS_JSON: &str = include_str!("fixtures/narratives/labelled_corpus.json");

#[derive(Deserialize)]
struct Corpus {
    readme: String,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    lens: String,
    source: String,
    class: String,
    narrative: String,
    fact_values: Vec<f64>,
    faithful: bool,
    expect_grounded: bool,
    notes: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(CORPUS_JSON).expect("labelled corpus parses")
}

/// Per-class corpus composition. Published in `docs/narrative-evidence-v1.md`;
/// update both together.
const EXPECTED_CLASS_COUNTS: [(&str, usize); 11] = [
    ("clean", 16),
    ("fabricated-value", 6),
    ("sign-inversion", 3),
    ("fn-small-int", 5),
    ("fn-percent-collision", 5),
    ("fn-wrong-attachment", 5),
    ("fp-version-fragment", 4),
    ("fp-date-fragment", 3),
    ("fp-ci-bound", 3),
    ("fp-ordinal-percentile", 3),
    ("fp-derived-arithmetic", 3),
];

#[test]
fn corpus_entries_are_well_formed_and_match_their_pinned_verdicts() {
    let corpus = corpus();
    assert!(
        corpus.readme.contains("docs/narrative-evidence-v1.md"),
        "the corpus readme names the evidence doc its numbers feed"
    );

    let mut ids = BTreeSet::new();
    for entry in &corpus.entries {
        assert!(
            ids.insert(entry.id.clone()),
            "duplicate corpus id {}",
            entry.id
        );
        assert!(
            matches!(entry.lens.as_str(), "file" | "diff"),
            "{}: unknown lens {}",
            entry.id,
            entry.lens
        );
        assert_eq!(
            entry.source, "authored",
            "{}: v1 corpus entries are authored; model-generated entries use \
             source \"model:<id>\" and join these assertions when collected",
            entry.id
        );
        assert!(!entry.notes.is_empty(), "{}: notes are mandatory", entry.id);

        // The class encodes the (faithful, verdict) quadrant by construction:
        // clean entries pass, fp-* classes are faithful yet flagged, fn-*
        // classes are unfaithful yet pass, and outright fabrications are
        // caught. A mislabelled entry fails here before the verdict is run.
        let (want_faithful, want_grounded) = match entry.class.as_str() {
            "clean" => (true, true),
            "fabricated-value" | "sign-inversion" => (false, false),
            c if c.starts_with("fn-") => (false, true),
            c if c.starts_with("fp-") => (true, false),
            other => panic!("{}: unknown class {other}", entry.id),
        };
        assert_eq!(
            entry.faithful, want_faithful,
            "{}: class {} implies faithful = {want_faithful}",
            entry.id, entry.class
        );
        assert_eq!(
            entry.expect_grounded, want_grounded,
            "{}: class {} implies expect_grounded = {want_grounded}",
            entry.id, entry.class
        );

        let verdict = check_citations(&entry.narrative, &entry.fact_values);
        assert_eq!(
            verdict.grounded, entry.expect_grounded,
            "{} ({}): checker verdict moved — unmatched {:?}; notes: {}",
            entry.id, entry.class, verdict.unmatched, entry.notes
        );
    }
}

#[test]
fn confusion_matrix_matches_the_published_numbers() {
    let corpus = corpus();

    let mut class_counts: BTreeMap<&str, usize> = BTreeMap::new();
    // Quadrants of (faithful ground truth × checker verdict): the stamp's
    // false positives are faithful-but-flagged, its false negatives are
    // unfaithful-but-passed.
    let (mut tn, mut fp, mut tp, mut fn_) = (0usize, 0usize, 0usize, 0usize);
    for entry in &corpus.entries {
        *class_counts.entry(entry.class.as_str()).or_default() += 1;
        let verdict = check_citations(&entry.narrative, &entry.fact_values);
        match (entry.faithful, verdict.grounded) {
            (true, true) => tn += 1,
            (true, false) => fp += 1,
            (false, false) => tp += 1,
            (false, true) => fn_ += 1,
        }
    }

    let expected: BTreeMap<&str, usize> = EXPECTED_CLASS_COUNTS.into_iter().collect();
    assert_eq!(class_counts, expected, "corpus composition moved");
    assert_eq!(corpus.entries.len(), 56, "corpus size moved");

    // Published in docs/narrative-evidence-v1.md §"Checker behavior on the
    // labelled corpus" — update the doc with any change here.
    assert_eq!(
        (tn, fp, tp, fn_),
        (16, 16, 9, 15),
        "confusion matrix (tn, fp, tp, fn) moved"
    );
}

#[test]
fn blind_spot_incidence_matches_the_published_numbers() {
    let corpus = corpus();

    let (mut exempt_entries, mut exempt_tokens) = (0usize, 0usize);
    let (mut fallback_entries, mut fallback_tokens) = (0usize, 0usize);
    for entry in &corpus.entries {
        let verdict = check_citations(&entry.narrative, &entry.fact_values);
        if !verdict.exempt_small_ints.is_empty() {
            exempt_entries += 1;
            exempt_tokens += verdict.exempt_small_ints.len();
        }
        if !verdict.percent_fallback_only.is_empty() {
            fallback_entries += 1;
            fallback_tokens += verdict.percent_fallback_only.len();
        }
    }

    // Published in docs/narrative-evidence-v1.md §"Blind-spot incidence" —
    // update the doc with any change here.
    assert_eq!(
        (exempt_entries, exempt_tokens),
        (17, 19),
        "small-int exemption incidence moved"
    );
    assert_eq!(
        (fallback_entries, fallback_tokens),
        (8, 8),
        "percent-fallback incidence moved"
    );
}
