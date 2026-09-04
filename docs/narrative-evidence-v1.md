# CodeLore — Narrative Groundedness Evidence

**Status:** deterministic characterization of the citation check on a labelled corpus, plus two model-study legs — a pinned 3B local model over this repository, and a contributed frontier-model run over six external repositories. The two legs share no file, so neither is the other's pair; the paired frontier-model leg is still pending an endpoint.
**Date:** 2026-09-02 (corpus numbers), 2026-09-01 (model-study numbers); corpus and assertions current as of the `enrichment_citation_corpus_test` gate.
**Methodology:** the document carries two kinds of numbers, held honest in two different ways. Every **corpus** number is recomputed on every CI run — the corpus is frozen in-tree and replayed through the real `check_citations` by `crates/codelore-lib/tests/enrichment_citation_corpus_test.rs`, which asserts the exact per-entry verdicts and the exact aggregate counts published here, so a checker change that moves any corpus number fails the build until this document is updated with it. **Model-study** numbers are point-in-time evidence, not CI-gated: they are recomputable from the committed per-item record beside this document, and they change only when a study leg is deliberately re-run.

## What is being measured

The `--llm` advisory layer stamps every narrative `grounded ✓` or `⚠ contains uncited claims` (see `advanced-usage` §8.5). That stamp is produced by a **deterministic numeric citation check** — `enrichment/citation.rs::check_citations` — which extracts every numeric token from the narrative and matches it against the fact sheet's values. No model is involved in the check itself.

This document characterizes **the check**, not any model: when a narrative is faithful, does the stamp pass it? When a narrative carries a fabricated or misattributed number, does the stamp catch it? These are the stamp's false-positive and false-negative behaviors — a different question from "how often does a given model hallucinate", which requires live generation and is measured separately in §First model study below.

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

- **This corpus is authored-first, with its first model-generated entries now landed.** Authored entries measure *class-conditional* behavior — "when a narrative quotes a CI bound, what does the checker do?" — which is the question a deterministic checker can answer exactly. What an authored corpus cannot answer is *prevalence* — "how often do real model narratives quote CI bounds?" — because class frequencies here are composition choices. Prevalence requires model-generated narratives; the corpus schema records them via `source: "model:<id>"`. Eight entries now carry one: three `fp-function-span` narratives captured from the model study below, and five contributed by an independent cross-model replication (three further `fp-function-span` cases and two `clean` narratives, contributed by GitHub `@Xxx91n`). All were label-verified against their dossiers before entry. Provenance is not cosmetic here — authored and model-generated narratives differ by a factor of 35 in how often they trip the small-int exemption, so the blind-spot table below reports them separately.
- **Ground truth is enforced structurally, not just asserted.** Each entry's class implies its `(faithful, expected-verdict)` quadrant, and the test derives that implication independently and fails on any mismatch — a mislabelled entry cannot enter the corpus silently.

## Results — checker behavior on the labelled corpus

Per-class agreement between the checker's verdict and the verdict the class predicts. Intervals are Wilson 95% (the same `stats.rs::wilson_ci` convention the corpus percentiles use); with class sizes this small the intervals are wide, which is the honest statement of what n of this size buys.

| Class | n | Ground truth | Checker verdict | Agreement | Wilson 95% |
|---|---:|---|---|---:|---|
| `clean` | 20 | faithful | passes | 20/20 | [0.839, 1.000] |
| `fabricated-value` | 6 | fabricated number | flags | 6/6 | [0.610, 1.000] |
| `sign-inversion` | 3 | inverted sign | flags | 3/3 | [0.438, 1.000] |
| `fn-small-int` | 5 | fabricated count ≤ 12 | **misses** | 5/5 | [0.566, 1.000] |
| `fn-percent-collision` | 5 | invented %, colliding fraction | **misses** | 5/5 | [0.566, 1.000] |
| `fn-wrong-attachment` | 5 | real value, wrong claim | **misses** | 5/5 | [0.566, 1.000] |
| `fp-version-fragment` | 4 | faithful (version string) | **flags** | 4/4 | [0.510, 1.000] |
| `fp-date-fragment` | 3 | faithful (date/vintage) | **flags** | 3/3 | [0.438, 1.000] |
| `fp-ci-bound` | 1 | faithful (CI confidence-level quote) | **flags** | 1/1 | [0.207, 1.000] |
| `fp-ordinal-percentile` | 3 | faithful (ordinal phrasing) | **flags** | 3/3 | [0.438, 1.000] |
| `fp-derived-arithmetic` | 3 | faithful (correct derivation) | **flags** | 3/3 | [0.438, 1.000] |
| `fp-function-span` | 6 | faithful (function-span quote) | **flags** | 6/6 | [0.610, 1.000] |

Confusion matrix over the whole corpus (faithful ground truth × checker verdict): **TN 20, FP 20, TP 9, FN 15** over 64 entries. Eight entries are model-generated (`source: "model:<id>"`): three `fp-function-span` rows from the study below (`model:llama3.2-t0-s42`), and five from a contributed cross-model replication (`model:deepseek-v4-pro`) — three more `fp-function-span` cases and two `clean` narratives, the latter written in Chinese, which the checker grounds identically because it reads numeric tokens rather than prose.

Read the matrix carefully: its cell sizes are corpus-composition choices, so ratios like "FP rate = 20/40" are statements about this corpus, not about the world. The informative results are the class rows:

- **Every documented blind spot is total within its class, not occasional.** A fabricated count of ≤ 12, an invented percent that collides with any fraction on the sheet, and a real number attached to the wrong claim pass the check 5/5 each — by construction of the check, and now by demonstration. The stamp provides *no* protection against these three shapes.
- **Every over-flagging route demonstrably fires**, including two routes surfaced by building this corpus that §8.5's original honest-limits list did not name (see below).
- **Everything the check claims to catch, it caught**: 9/9 fabrications and sign inversions flagged, 20/20 faithful narratives passed.

For readers coming from the detector-benchmark literature, the conventional summary metrics on this corpus (positive class = `⚠ contains uncited claims`): precision 0.310, recall 0.375, F1 0.340, balanced accuracy 0.438. Present them with their caveat welded on: this corpus is *deliberately stratified toward the checker's known failure modes* — more than half of the entries are constructed adversarial cases — so these numbers characterize the checker under attack, not its field performance. On a corpus of typical narratives the same checker would score far higher; that corpus does not exist yet (see the extension plan below).

## The over-flagging routes (the safe failure direction)

`check_citations` is designed to fail toward `⚠` on faithful text rather than `✓` on invented numbers. The corpus pins six concrete routes:

1. **Version fragments** — `2.1.0` decomposes into `2.1` (flagged) and `0` (exempt). Documented in the code before this evidence cut.
2. **Date/vintage fragments** — `defects-2026-07-15` flags `2026` and `15` while `07` rides the small-int exemption. Documented in the code before this evidence cut.
3. **CI confidence levels** — the sheet renders `corpus_percentile_ci` as a single en-dash string (`0.62–0.81`) whose two printed endpoints parse into the fact-value set, so quoting either bound — including the sheet's own string verbatim — grounds. The interval's confidence level (`95%`) is not a printed sheet value and flags when quoted. *Surfaced by this corpus.*
4. **Ordinal percentiles** — percentiles live on the sheet as fractions (`percentile = 0.97`); the phrasing "the 97th percentile" carries no `%` sign, so the ×100 fallback never applies and `97` flags. The prompt's "cite the exact number" instruction steers models away from this phrasing but cannot prevent it. *Surfaced by this corpus.*
5. **Derived arithmetic** — "a net 114 lines" from a sheet stating 210 added / 96 removed is correct arithmetic and still flags: the contract demands quotes, not derivations. Contract-level correct, user-level false alarm.
6. **Function-span quotes** — the `functions` section names entries like `write_top_hotspots@72-114`; a narrative quoting a function this way carries line numbers that live on the sheet only inside strings, so they flag. *Discovered in the wild by the model study below and seeded into the corpus from its captured narratives, then independently re-observed by a contributed cross-model replication on different repositories, which supplied three further cases — the dominant over-flagging route real narratives actually hit.*

## Blind-spot incidence instrumentation

The checker now reports, on every result, which tokens its two numeric blind spots absorbed — `Groundedness::exempt_small_ints` (tokens the ≤ 12 whole-number exemption skipped) and `Groundedness::percent_fallback_only` (percent tokens grounded *only* by the ×100 fallback). Neither influences the verdict; they exist so the blind spots' real-world incidence can be measured rather than guessed at.

On this corpus the two routes are exercised by **disjoint halves of it**, so the counts are reported per provenance rather than pooled:

| Provenance | Entries | With exempt tokens | Exempt tokens | Per entry | Fallback-only entries | Fallback tokens |
|---|---:|---:|---:|---:|---:|---:|
| Hand-authored | 56 | 17 | 19 | 0.34 | 8 | 8 |
| Model-generated | 8 | 8 | 95 | 11.88 | 0 | 0 |
| **Combined** | **64** | **25** | **114** | **1.78** | **8** | **8** |

Every model-generated narrative trips the small-int exemption and none reaches the percent fallback; the authored entries are the mirror image. The gap is a factor of 35, and it is not simply human-versus-model — the two contributed models sit at 3.00 (`llama3.2-t0-s42`) and 17.20 (`deepseek-v4-pro`) exempt tokens per entry, because a model that enumerates every zero-valued biomarker field by field emits far more exempt-range integers than one that summarises in prose. A single pooled total would therefore track *which model contributed most recently* rather than any property of the checker: five of the sixty-four entries carry 86 of the 114 exempted tokens. Reporting the strata is what keeps the combined row honest, and the test pins the headline as the sum of the rows that explain it.

The benign/collision split inside the fallback column — 3 benign unit conversions of a genuine sheet quantity, 5 collisions with an unrelated fraction — is exactly why the fallback exists and exactly what it risks; the same mechanism serves both. These remain corpus numbers; the diagnostics' own counts are still not surfaced through any output channel, so their in-the-wild incidence remains open.

## First model study — a 3B local model, one repository

**Per-item records and provenance:** [`narrative-evidence/leg1-llama3.2-3b-q4km-t0-s42.json`](narrative-evidence/leg1-llama3.2-3b-q4km-t0-s42.json). Its `run` block holds every pin — model card (`llama3.2-t0-s42`: `temperature 0` and `seed 42` baked into the model id, deriving from `llama3.2:latest`, 3.2B, Q4_K_M), runtime (ollama 0.33.0, macOS/Apple M4 Pro, default local OpenAI-compatible endpoint), `PROMPT_VERSION`, and the subject commit — and its `items` hold one record per file: three verdicts, the run-1 uncited tokens, and a narrative hash per run. Every aggregate below is recomputed from `items` and reproduced here for reading; the record, not this prose, is the source of truth. The sweep: every file of this repository with a code-health row (n = 110), each narrated 3 times via `--llm-refresh`, 330 generations. Two host-side interruptions (disk exhaustion unrelated to the model) killed the sweep mid-file; interrupted files were regenerated in full under identical pins, and the final record carries zero failed invocations and no excluded results.

| Measure | k / n | Rate | Wilson 95% |
|---|---:|---:|---|
| `grounded ✓` — first generation | 72/110 | 65.5% | [0.562, 0.737] |
| `grounded ✓` — majority of 3 | 71/110 | 64.5% | [0.553, 0.729] |
| Verdict flipped across the 3 runs | 13/110 | 11.8% | [0.070, 0.192] |

**What the grounded rate means — and does not.** The study's sharpest new fact is a **sixth over-flagging route, first observed in the wild** (now the corpus's `fp-function-span` class, seeded from this study's captured narratives): models quote function spans from the `functions` section's `name@start-end` identifiers, and those line numbers live on the sheet only inside strings, so they flag. Span and CI-composite quotes together account for 57 of the 82 uncited tokens the first-generation stamps name, and in **18 of the 38 flagged items every uncited token traces to sheet text this way** (one more item mixes both kinds) — so `⚠` is not a hallucination rate, and 65.5% is a **lower bound** on faithful generations. The token shapes: 49 integers (the span route above), 26 decimals (dominated by verbatim `corpus_percentile_ci` quotes — the `fp-ci-bound` route the corpus predicted, confirmed in real output), 7 percents (unit conversions and genuine fabrications mixed). One stamp carries both directions at once: verbatim CI bounds `0.978549` and `0.979442` (faithful, flagged) beside an invented "95.9% confidence interval" whose level appears nowhere on the sheet (fabrication, correctly flagged).

**Determinism: temperature 0 + a fixed seed is not one number, it is two.** Repeat generations of an identical prompt against the warm server were byte-identical in **110/110** items — but the *first* generation of each prompt differed from that stable pair in 35/110, and all 13 verdict flips follow exactly this cold-vs-warm pattern. This is consistent with the documented facts that temperature-0 decoding does not guarantee determinism ([arXiv 2606.26185](https://arxiv.org/abs/2606.26185)) and that local-model reproducibility holds under pinned seed/runtime for *repeated identical* conditions ([arXiv 2601.22025](https://arxiv.org/abs/2601.22025)) — the divergence here appears exactly where server state differs (the first evaluation of a prompt) and vanishes where it repeats. It is why the protocol pre-registered flip-rate measurement instead of assuming temp-0 determinism: the realistic user path is one generation per dossier, and that path carries ~12% verdict instability on this setup. Numbers above report the first generation as primary — the honest analogue of what a user sees.

**Limits of this leg.** One model, one repository, and a grounded rate that conflates model faithfulness with checker over-flagging (the 18-of-38 figure above traces tokens to sheet text; a full per-stamp separation requires the labelling pass the growth plan describes). The paired frontier-model leg is pending an endpoint; to preserve the paired design it must reuse the same subject commit and file list, both recorded in the data file's `run` block and `items`. This paragraph is the canonical statement of that requirement. The second study below is a frontier-model leg but **not** that pair — it shares no file with this one, so it answers a different question and leaves this requirement open.

**The prompt has changed since this leg ran.** The `run` block pins
`prompt_version`, and the value it records is no longer the one the tool ships:
the prompt has since been hardened to fence the fact sheet in explicit markers
and to escape control characters in rendered values. The rates above therefore
characterise the prompt as it stood at the subject commit, not the current one.
The distinction is not hypothetical — prompt wording is exactly the kind of
change that can move how a model emits numbers, which is why `PROMPT_VERSION`
participates in the narrative cache key at all. Reproducing *this* leg is
unaffected, because checking out the subject commit restores its prompt along
with everything else; what this document does not have is a current-prompt
rate, and no figure above should be read as one.

**A current-prompt rate needs a new leg, not a re-run.** The record stores
verdicts, not fact sheets, so replaying against today's build would regenerate
the sheets from whatever the analyses now produce — and those have moved
independently of the prompt, the hotspots section's ranks and scores among
them. Prompt version and sheet content would advance together, confounding the
one comparison such a leg exists to make. The design that isolates it is to
apply the prompt change alone to the pinned subject tree, leaving the fact
sheets byte-identical so `PROMPT_VERSION` is the only variable that moves. The
local model makes that leg free of API cost, and every pin it needs is already
recorded here.

## Second model study — a frontier model, six external repositories

**Per-item records and provenance:** [`narrative-evidence/leg2-deepseek-v4-pro-six-repos.json`](narrative-evidence/leg2-deepseek-v4-pro-six-repos.json), contributed by GitHub `@Xxx91n` and reconciled against the contributor's raw run log before it was accepted — every rate, denominator and per-repository count in this section was recomputed from that log rather than taken from the submission. Its `run` block holds the pins: model `deepseek-v4-pro` behind a local OpenAI-compatible gateway, `PROMPT_VERSION` 1, and a per-repo subject commit for each of the six subjects. The sweep: 34 code-health-scored files from each of ripgrep, typer, bat, hono, gson and express (n = 204), each narrated 3 times, 612 generations. Two pins leg 1 carries are **absent by construction**: the client sends only `model` and `messages`, so temperature and seed were never transmitted and the gateway's defaults applied. This leg is therefore not reproducible from its record alone, and the record says so rather than implying a determinism it cannot claim.

| Measure | k / n | Rate | Wilson 95% |
|---|---:|---:|---|
| `grounded ✓` — first generation | 14/204 | 6.9% | [0.041, 0.112] |
| `grounded ✓` — majority of 3 | 11/204 | 5.4% | [0.030, 0.094] |
| Verdict flipped across the 3 runs | 17/204 | 8.3% | [0.053, 0.129] |

**This is not a model-quality comparison, and reading it as one inverts the finding.** Leg 1's 65.5% and this leg's 6.9% differ by a factor of nine, but model, corpus, host and prompt-time all move together between them, and the largest identifiable driver is **numeric verbosity, not faithfulness**. Counting the uncited tokens each run-1 stamp names, `deepseek-v4-pro` emits **11.69× more per narrative** than `llama3.2-t0-s42` (9.03 vs 0.77). A checker that flags a narrative when *any* single quoted number fails to match is close to a per-token race: a model that writes many more numbers loses it far more often at equal per-token faithfulness. An independent observation points the same way: §Blind-spot incidence instrumentation measures these same two models at 17.20 and 3.00 exempt tokens per corpus entry — a different corpus, a different metric, and a ratio of the same order arrived at without reference to either leg. None of this is proof: the clean test is the paired leg, where corpus is held fixed and model is the only variable. It is enough to say that the nine-fold gap should not be attributed to the frontier model being nine times less faithful.

**Per-repository, the rate varies more than the pooled figure suggests** — bat 5/34, express 4/34, ripgrep 3/34, hono 1/34, typer 1/34, gson 0/34. The subject repositories span four languages, and the balanced 34-file design makes the strata directly comparable; the zero for gson is a real cell, not a missing one.

**The stamp-named token basis does not transfer across models.** Leg 1's record states that its aggregates use the tokens the stamp *names*, and the stamp previews at most five. That basis is sound where truncation is rare — leg 1 loses it on 3 items of 110, moving its mean from 0.77 to 0.75. It is unsound here: this leg truncates on **150 items of 204**, and the stamp-named count understates its token total by **2.09×** (4.33 against 9.03). Both records preserve the residue as a trailing `…+N` marker, so the true count is recoverable, and every token figure in this section uses the recovered total. The generalisable point is that a convention validated on a terse model can silently bias a verbose one, so the basis belongs with the aggregate rather than with the study that first used it.

**What this leg cannot answer.** It is not the paired frontier leg — it shares no file with leg 1, so no paired per-item difference exists and the requirement in that leg's limits paragraph stands unmet. It cannot reproduce leg 1's byte-identity result either: `--llm-refresh` rewrites a single cache key once per generation, so only the third narrative survives on disk and `narrative_sha16` is null for the first two of every item. Verdicts are complete for all three generations, so the flip rate above is unaffected — but the cold-versus-warm mechanism leg 1 identified cannot be checked here, and the two flip rates (11.8% and 8.3%) are not measuring the same thing under the same pins.

## Relation to established practice

The design choices above are the current (2026) consensus for evaluating groundedness checkers, not inventions:

- **Taxonomy.** The corpus classes refine the standard hallucination axes — *absent-from-source* vs *conflicts-with-source* ([RAGTruth, ACL 2024](https://arxiv.org/abs/2401.00396)) — into checker-specific routes, and the binary `faithful` label is the worst-pooled rollup the field uses because fine-grained labels have poor inter-annotator agreement ([FaithBench, NAACL 2025](https://arxiv.org/abs/2410.13210): α 0.748 binary vs 0.58 fine-grained).
- **The wrong-attachment blind spot is externally documented as common, not exotic.** A 2026 re-annotation of RAGTruth found *numeric/logic inconsistency* — every number present in the source, attached to the wrong claim ("100 people (95 passengers and 5 crew)" rewritten as "100 passengers and 5 crew") — to be roughly a quarter of newly identified hallucination spans (900 spans, 25.42%; [arXiv 2603.27752](https://arxiv.org/abs/2603.27752)). A purely numeric check is structurally blind to exactly this class, which is why `fn-wrong-attachment` gets its own row above instead of being averaged away.
- **Measuring the checker itself is the recommended posture.** Factuality metrics disagree with each other and misestimate system-level performance often enough that validating the metric in-domain before relying on it is explicit guidance ([arXiv 2501.14883](https://arxiv.org/abs/2501.14883)); this document is that validation for CodeLore's check.
- **Deterministic replay as the PR gate is the industry pattern.** Deterministic assertions gate; model-graded evaluation runs out-of-band — the split promptfoo documents as best practice and inspect-ai builds around via response caching. Because CodeLore's checker is deterministic, the corpus sections' gate is *stricter* than the industry's pass-rate compromise: it pins exact confusion-matrix counts, so a single moved verdict fails the build. (The model-study numbers are the out-of-band half of that split.)
- **Intervals.** Wilson is the recommended interval for a binary proportion at small-to-mid n (and the convention `stats.rs` already uses); error bars on evals at all is the direction the field is being pushed ([arXiv 2411.00640](https://arxiv.org/abs/2411.00640)). For a deterministic checker on a frozen corpus the interval expresses corpus-sampling uncertainty only — there is no run-to-run noise to average over.
- **Scale.** Peer-reviewed checker evaluations have published at n=143 ([HalluJudge](https://arxiv.org/abs/2601.19072)); ~150–200 entries is where corpus-level rate claims stop being anecdotal (Wilson ±3 points at n=200). This corpus's 64 entries sit above the practitioner smoke-test floor (~50) and below the rate-claim threshold — consistent with the class-conditional framing above, and the growth target below closes the gap.

## Honest limits of this evidence cut

- **The corpus makes no prevalence claims.** It is authored-first; class frequencies are chosen. In-the-wild frequency is what the two model studies measure — one model over this repository, one over six external ones — and both now carry a committed per-item record, so both contribute rates to this cut. What neither supplies is a rate for the same files under two models, which is the only form in which the two can be subtracted.
- **Numeric claims only.** The stamp — and therefore this evaluation — sees numbers. A narrative inventing a file name, an author, or a trend with no number attached is invisible to the check and out of scope here; §8.5 already says the dossier, not the narrative, is the authority.
- **Single-source ground truth.** Entries were authored to embody their class, and the class-consistency assertions plus per-entry notes make labels auditable — but no independent relabeling pass has happened. The planned pass follows the two-independent-annotators-plus-adjudication norm, reporting Cohen's κ on the binary rollup with κ ≥ 0.75 as the credibility bar (comparable evaluations report κ 0.78–0.84).
- **Model-sensitivity data covers two models, neither of them paired** (leg status and the pairing requirement: see §First model study, "Limits of this leg"). Both legs now meet the same bar — a per-item record committed here, with the prose recomputed from it — but they were run over disjoint corpora, so the protocol's central instrument, the paired per-item difference, has still never been computed. Two of the protocol's pins are also unmet by the second leg: its temperature and seed were never sent, and its model ID carries no date. Its rates are published as a separate leg for exactly that reason, rather than being compared against leg 1's as though one number followed the other. The study protocol, fixed in advance so the numbers can't be shaped after the fact: the same ≥ 100 fact sheets to every model (paired design, per the error-bars guidance above); one pinned prompt template with its hash recorded (`PROMPT_VERSION` already exists for this); temperature 0 everywhere, with the local model pinned by model tag, quantization, runtime version, and seed, and the hosted model by dated model ID; 3 runs per item to measure and report the flip rate (temperature-0 nondeterminism is documented for hosted APIs); grounded rate per model with Wilson 95% CIs plus the paired per-item difference; and the checker's own FP/FN from this document stated alongside, since an imperfect judge bounds what the study can claim. Live models stay out of per-PR CI — a scheduled or offline refresh only.

## Reproducing and extending

```bash
cargo test -p codelore-lib --features test-support --test enrichment_citation_corpus_test
```

The corpus is `crates/codelore-lib/tests/fixtures/narratives/labelled_corpus.json`. To extend it: append entries (model-generated narratives use `source: "model:<id>"` and record the generating model), update the pinned totals in the test, and update the tables here — the test enforces that all three move together.

The growth path, in priority order: convert every real-world stamp failure into a fixture as it is found (the cheapest source of honest entries); add model-generated narratives with the fact sheets they were generated from, which is what turns class-conditional characterization into prevalence measurement; and grow toward 150–200 labelled entries, the scale at which corpus-level rate claims carry ±3-point intervals instead of anecdote.
