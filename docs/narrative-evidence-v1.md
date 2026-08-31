# CodeLore — Narrative Groundedness Evidence

**Status:** first evidence cut — deterministic characterization of the citation check on a labelled corpus. The model-sensitivity study (grounded rates per model) requires live LLM endpoints and is tracked below as future work.
**Date:** 2026-08-31 (numbers); corpus and assertions current as of the `enrichment_citation_corpus_test` gate.
**Methodology:** every number in this document is recomputed on every CI run — the corpus is frozen in-tree and replayed through the real `check_citations` by `crates/codelore-lib/tests/enrichment_citation_corpus_test.rs`, which asserts the exact per-entry verdicts and the exact aggregate counts published here. A checker change that moves any number fails the build until this document is updated with it.

## What is being measured

The `--llm` advisory layer stamps every narrative `grounded ✓` or `⚠ contains uncited claims` (see `advanced-usage` §8.5). That stamp is produced by a **deterministic numeric citation check** — `enrichment/citation.rs::check_citations` — which extracts every numeric token from the narrative and matches it against the fact sheet's values. No model is involved in the check itself.

This document characterizes **the check**, not any model: when a narrative is faithful, does the stamp pass it? When a narrative carries a fabricated or misattributed number, does the stamp catch it? These are the stamp's false-positive and false-negative behaviors — a different question from "how often does a given model hallucinate", which requires live generation and is out of scope for this evidence cut.

Two readings of "false positive" exist and the tables below keep them separate:

- **Contract-level:** §8.5 promises "every number the narrative quotes appears in the evidence". Flagging a correctly *derived* number (a sum the sheet supports but does not state) honors that contract.
- **User-level:** a reviewer seeing `⚠` on a narrative that is not lying experiences a false alarm regardless of the contract.

The `fp-*` classes below are user-level false positives; several of them are contract-level correct behavior. The ground-truth label `faithful` means "the narrative contains no fabricated or misattributed numeric claim".

## Corpus design

The corpus is `crates/codelore-lib/tests/fixtures/narratives/labelled_corpus.json`: labelled entries, each pairing a narrative with the fact values it is checked against, a ground-truth `faithful` label, and the pinned checker verdict. Narratives follow the shapes the prompt contract produces (`## Diagnosis` sections for the file lens, tight prose for the diff lens, "the data doesn't show" phrasing); fact arrays follow the fact-sheet schema in `enrichment/fact_sheet.rs` (score/risk/percentile/cognitive, the eight biomarkers, hotspot rank+score, coupling partner rows, ownership, diff line counts).

Classes cover, by construction:

- the checker's **documented blind spots** (`fn-*` classes — the §8.5 honest-limits list),
- its **over-flagging routes** (`fp-*` classes — the safe failure direction),
- outright **fabrications it must catch**, and
- **faithful narratives it must pass**, including rounding tolerance, thousands separators, signed values, benign percent-unit conversions, and a numberless narrative.

Two design decisions worth stating plainly:

- **This corpus is authored, not model-generated.** Authored entries measure *class-conditional* behavior — "when a narrative quotes a CI bound, what does the checker do?" — which is the question a deterministic checker can answer exactly. What an authored corpus cannot answer is *prevalence* — "how often do real model narratives quote CI bounds?" — because class frequencies here are composition choices. Prevalence requires model-generated narratives; the corpus schema reserves `source: "model:<id>"` for appending them without changing the harness.
- **Ground truth is enforced structurally, not just asserted.** Each entry's class implies its `(faithful, expected-verdict)` quadrant, and the test derives that implication independently and fails on any mismatch — a mislabelled entry cannot enter the corpus silently.

## Results — checker behavior on the labelled corpus

Per-class agreement between the checker's verdict and the verdict the class predicts. Intervals are Wilson 95% (the same `stats.rs::wilson_ci` convention the corpus percentiles use); with class sizes this small the intervals are wide, which is the honest statement of what n of this size buys.

| Class | n | Ground truth | Checker verdict | Agreement | Wilson 95% |
|---|---:|---|---|---:|---|
| `clean` | 16 | faithful | passes | 16/16 | [0.806, 1.000] |
| `fabricated-value` | 6 | fabricated number | flags | 6/6 | [0.610, 1.000] |
| `sign-inversion` | 3 | inverted sign | flags | 3/3 | [0.438, 1.000] |
| `fn-small-int` | 5 | fabricated count ≤ 12 | **misses** | 5/5 | [0.566, 1.000] |
| `fn-percent-collision` | 5 | invented %, colliding fraction | **misses** | 5/5 | [0.566, 1.000] |
| `fn-wrong-attachment` | 5 | real value, wrong claim | **misses** | 5/5 | [0.566, 1.000] |
| `fp-version-fragment` | 4 | faithful (version string) | **flags** | 4/4 | [0.510, 1.000] |
| `fp-date-fragment` | 3 | faithful (date/vintage) | **flags** | 3/3 | [0.438, 1.000] |
| `fp-ci-bound` | 3 | faithful (CI bound quote) | **flags** | 3/3 | [0.438, 1.000] |
| `fp-ordinal-percentile` | 3 | faithful (ordinal phrasing) | **flags** | 3/3 | [0.438, 1.000] |
| `fp-derived-arithmetic` | 3 | faithful (correct derivation) | **flags** | 3/3 | [0.438, 1.000] |

Confusion matrix over the whole corpus (faithful ground truth × checker verdict): **TN 16, FP 16, TP 9, FN 15** over 56 entries.

Read the matrix carefully: its cell sizes are corpus-composition choices, so ratios like "FP rate = 16/32" are statements about this corpus, not about the world. The informative results are the class rows:

- **Every documented blind spot is total within its class, not occasional.** A fabricated count of ≤ 12, an invented percent that collides with any fraction on the sheet, and a real number attached to the wrong claim pass the check 5/5 each — by construction of the check, and now by demonstration. The stamp provides *no* protection against these three shapes.
- **Every over-flagging route demonstrably fires**, including two routes surfaced by building this corpus that §8.5's original honest-limits list did not name (see below).
- **Everything the check claims to catch, it caught**: 9/9 fabrications and sign inversions flagged, 16/16 faithful narratives passed.

For readers coming from the detector-benchmark literature, the conventional summary metrics on this corpus (positive class = `⚠ contains uncited claims`): precision 0.360, recall 0.375, F1 0.367, balanced accuracy 0.438. Present them with their caveat welded on: this corpus is *deliberately stratified toward the checker's known failure modes* — nearly 60% of entries are constructed adversarial cases — so these numbers characterize the checker under attack, not its field performance. On a corpus of typical narratives the same checker would score far higher; that corpus does not exist yet (see the extension plan below).

## The over-flagging routes (the safe failure direction)

`check_citations` is designed to fail toward `⚠` on faithful text rather than `✓` on invented numbers. The corpus pins five concrete routes:

1. **Version fragments** — `2.1.0` decomposes into `2.1` (flagged) and `0` (exempt). Documented in the code before this evidence cut.
2. **Date/vintage fragments** — `defects-2026-07-15` flags `2026` and `15` while `07` rides the small-int exemption. Documented in the code before this evidence cut.
3. **CI-bound quotes** — the sheet renders `corpus_percentile_ci` as a single en-dash string (`0.62–0.81`), which never parses into the fact-value set, so a narrative quoting either bound — including quoting the sheet's own string verbatim — is flagged. The interval's confidence level (`95%`) is not a sheet value either and flags with it. *Surfaced by this corpus.*
4. **Ordinal percentiles** — percentiles live on the sheet as fractions (`percentile = 0.97`); the phrasing "the 97th percentile" carries no `%` sign, so the ×100 fallback never applies and `97` flags. The prompt's "cite the exact number" instruction steers models away from this phrasing but cannot prevent it. *Surfaced by this corpus.*
5. **Derived arithmetic** — "a net 114 lines" from a sheet stating 210 added / 96 removed is correct arithmetic and still flags: the contract demands quotes, not derivations. Contract-level correct, user-level false alarm.

## Blind-spot incidence instrumentation

The checker now reports, on every result, which tokens its two numeric blind spots absorbed — `Groundedness::exempt_small_ints` (tokens the ≤ 12 whole-number exemption skipped) and `Groundedness::percent_fallback_only` (percent tokens grounded *only* by the ×100 fallback). Neither influences the verdict; they exist so the blind spots' real-world incidence can be measured rather than guessed at.

On this corpus: **17 of 56 entries carry exempt tokens (19 tokens total)**, and **8 entries carry fallback-only groundings (8 tokens: 3 benign unit conversions of a genuine sheet quantity, 5 collisions with an unrelated fraction)**. The benign/collision split is exactly why the fallback exists and exactly what it risks — the same mechanism serves both. These are corpus numbers; incidence on real model narratives is the first question the future model study answers.

## Relation to established practice

The design choices above are the current (2026) consensus for evaluating groundedness checkers, not inventions:

- **Taxonomy.** The corpus classes refine the standard hallucination axes — *absent-from-source* vs *conflicts-with-source* ([RAGTruth, ACL 2024](https://arxiv.org/abs/2401.00396)) — into checker-specific routes, and the binary `faithful` label is the worst-pooled rollup the field uses because fine-grained labels have poor inter-annotator agreement ([FaithBench, NAACL 2025](https://arxiv.org/abs/2410.13210): α 0.748 binary vs 0.58 fine-grained).
- **The wrong-attachment blind spot is externally documented as common, not exotic.** A 2026 re-annotation of RAGTruth found *numeric/logic inconsistency* — every number present in the source, attached to the wrong claim ("100 people (95 passengers and 5 crew)" rewritten as "100 passengers and 5 crew") — to be roughly a quarter of newly identified hallucination spans (900 spans, 25.42%; [arXiv 2603.27752](https://arxiv.org/abs/2603.27752)). A purely numeric check is structurally blind to exactly this class, which is why `fn-wrong-attachment` gets its own row above instead of being averaged away.
- **Measuring the checker itself is the recommended posture.** Factuality metrics disagree with each other and misestimate system-level performance often enough that validating the metric in-domain before relying on it is explicit guidance ([arXiv 2501.14883](https://arxiv.org/abs/2501.14883)); this document is that validation for CodeLore's check.
- **Deterministic replay as the PR gate is the industry pattern.** Deterministic assertions gate; model-graded evaluation runs out-of-band — the split promptfoo documents as best practice and inspect-ai builds around via response caching. Because CodeLore's checker is deterministic, this document's gate is *stricter* than the industry's pass-rate compromise: it pins exact confusion-matrix counts, so a single moved verdict fails the build.
- **Intervals.** Wilson is the recommended interval for a binary proportion at small-to-mid n (and the convention `stats.rs` already uses); error bars on evals at all is the direction the field is being pushed ([arXiv 2411.00640](https://arxiv.org/abs/2411.00640)). For a deterministic checker on a frozen corpus the interval expresses corpus-sampling uncertainty only — there is no run-to-run noise to average over.
- **Scale.** Peer-reviewed checker evaluations have published at n=143 ([HalluJudge](https://arxiv.org/abs/2601.19072)); ~150–200 entries is where corpus-level rate claims stop being anecdotal (Wilson ±3 points at n=200). This corpus's 56 entries sit above the practitioner smoke-test floor (~50) and below the rate-claim threshold — consistent with the class-conditional framing above, and the growth target below closes the gap.

## Honest limits of this evidence cut

- **No prevalence claims.** The corpus is authored; class frequencies are chosen. Nothing here says how often a real narrative hits a blind spot or an over-flag route.
- **Numeric claims only.** The stamp — and therefore this evaluation — sees numbers. A narrative inventing a file name, an author, or a trend with no number attached is invisible to the check and out of scope here; §8.5 already says the dossier, not the narrative, is the authority.
- **Single-source ground truth.** Entries were authored to embody their class, and the class-consistency assertions plus per-entry notes make labels auditable — but no independent relabeling pass has happened. The planned pass follows the two-independent-annotators-plus-adjudication norm, reporting Cohen's κ on the binary rollup with κ ≥ 0.75 as the credibility bar (comparable evaluations report κ 0.78–0.84).
- **No model-sensitivity data yet.** Whether a 3B local model's narratives ground at a different rate than a frontier model's is unmeasured. It requires live endpoints and belongs in a scheduled or offline study appended to this document — deliberately not in per-PR CI, where a live model would import non-determinism and secrets exposure into a gate that is otherwise fully deterministic. The protocol, fixed in advance so the numbers can't be shaped after the fact: the same ≥ 100 fact sheets to every model (paired design, per the error-bars guidance above); one pinned prompt template with its hash recorded (`PROMPT_VERSION` already exists for this); temperature 0 everywhere, with the local model pinned by model tag, quantization, runtime version, and seed, and the hosted model by dated model ID; 3 runs per item to measure and report the flip rate (temperature-0 nondeterminism is documented for hosted APIs); grounded rate per model with Wilson 95% CIs plus the paired per-item difference; and the checker's own FP/FN from this document stated alongside, since an imperfect judge bounds what the study can claim.

## Reproducing and extending

```bash
cargo test -p codelore-lib --features test-support --test enrichment_citation_corpus_test
```

The corpus is `crates/codelore-lib/tests/fixtures/narratives/labelled_corpus.json`. To extend it: append entries (model-generated narratives use `source: "model:<id>"` and record the generating model), update the pinned totals in the test, and update the tables here — the test enforces that all three move together.

The growth path, in priority order: convert every real-world stamp failure into a fixture as it is found (the cheapest source of honest entries); add model-generated narratives with the fact sheets they were generated from, which is what turns class-conditional characterization into prevalence measurement; and grow toward 150–200 labelled entries, the scale at which corpus-level rate claims carry ±3-point intervals instead of anecdote.
