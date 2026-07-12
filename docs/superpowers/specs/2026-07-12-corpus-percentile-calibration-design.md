# Corpus-Relative Percentile Scoring + Full Biomarkers — Design

Phase 2 of the code-health initiative. Phase 1 (shipped) made the scoring
architecture "speak percentile" via self-relative, per-language
`PERCENT_RANK`; this phase adds the cross-repo reference corpus that makes
"is 78/100 good?" answerable, and completes the biomarker set the corpus is
calibrated over.

## Decisions (locked with the user)

1. **Scope:** corpus percentile + the four deferred biomarkers
   (deep-nesting, many-arguments, complex-conditional, LCOM4-style
   cohesion). Coverage ingestion and code-cartography stay deferred.
2. **Distribution:** the world calibration artifact is embedded in the
   binary (`include_bytes!`); `--calibration <path>` overrides it with a
   custom (e.g. organization-internal) artifact.
3. **Builder:** a public `codelore calibrate` subcommand builds artifacts —
   the same measurement pipeline that scores users builds the corpus, by
   construction.
4. **Integration:** additive second lens. The shipped self-relative
   `percentile` and bands are unchanged; `corpus_percentile: Option<f64>`
   appears alongside. The corpus signal is never blended into the 0–100
   composite (formula transparency is a differentiator). The new
   biomarkers DO fold into `structural_risk` like the existing five — a
   deliberate scoring improvement carried by a schema-version bump.

## 1. User-visible behavior

- Every health-scored file gains `corpus_percentile` — "structural risk
  worse than N% of comparable files in the reference corpus", computed
  per language. Rendered as a paired reading: `P92 in-repo · P74 vs
  corpus`.
- Function-grained biomarkers gain the same lens where the artifact
  carries function-level distributions.
- Five → nine biomarkers in the composite. Scores and bands shift;
  `CURRENT_SCHEMA_VERSION` bumps (new persisted metric columns), which
  invalidates caches naturally. Documentation describes only the new
  contract.

## 2. Calibration artifact

Per `(language × metric [× size-stratum])`: a **quantile-breakpoint
vector** — ~1,000 evenly spaced quantiles of the pooled corpus
distribution. Percentile lookup = binary search + linear interpolation;
smooth, compact (kilobytes across all languages/metrics), and risk
thresholds are derivable from it rather than stored separately.

Container: a versioned file (CBOR or compact JSON — implementer chooses
against existing serde conventions) with a header:

- `format_version` (integer; unknown → the whole artifact is ignored with
  one warning),
- `corpus_vintage` (opaque id, e.g. `world-2026-07`),
- `generated_at`,
- per-language function/file sample counts.

The **size-stratum dimension exists in the format from v1** (Alves-style
size stratification to avoid size confounding) even though the v1 world
artifact ships a single stratum — the format never needs a breaking
change to add strata later.

Degradation rules:
- language absent from the artifact → `corpus_percentile: None`, one
  deduped notice per run;
- per-language sample below a floor (500 functions) → treated as absent;
- observed value beyond the corpus maximum → `1.0` plus a
  `beyond_corpus: true` flag on the row (never silently clamped).

## 3. `codelore calibrate`

```
codelore calibrate --repos calibration/corpus.toml --output world.calib
codelore calibrate --repos org-repos.toml --merge existing.calib --output org.calib
```

- Reads a manifest (TOML: repo URL/path + pinned SHA + language tags),
  clones or opens each repo at its pinned SHA, runs the standard ingest +
  health pipeline, pools per-language metric observations, emits the
  artifact.
- Per-repo progress lines; an individual repo failure is reported and
  skipped; the artifact header records actual coverage (repos attempted /
  included).
- `--merge` extends an existing artifact incrementally (pooled
  re-quantiling), so organizations can grow a private corpus over time.

## 4. World corpus

- Checked-in manifest `calibration/corpus.toml`: permissive-license,
  active OSS repos, size-stratified, ~25–50 per Tier-1 language, each
  pinned to an exact SHA.
- Regeneration = run `calibrate` over the manifest; the committed
  artifact + manifest are the reproducible vintage.
- Every scoring run stamps `corpus_vintage` into the provenance sidecar.
- Licensing posture: the artifact contains only aggregate quantiles — no
  code — so there is no license exposure; the manifest still prefers
  permissive licenses as courtesy.
- Scheduled CI rebuilds of the world artifact: deferred (manual vintages
  for v1).

## 5. New biomarkers

Deep-nesting (Bumpy-Road-style), many-arguments, complex-conditional,
LCOM4-style cohesion. Each integrates exactly like the existing five:
per-language percentile intensity, co-occurrence multiplier, contributes
to `structural_risk`.

**Hard validation gate for the plan:** metric availability per language in
the vendored rust-code-analysis layer must be verified per marker before
tasks are written (nesting and argument counts are likely present;
LCOM4-class cohesion may not be derivable for every Tier-1 language). A
marker unavailable for a language contributes nothing for that language —
same honest-absence rule the composite already uses. Any newly persisted
per-function columns ride the same schema bump.

## 6. Surfaces

- **CLI:** `corpus_percentile` column in code-health CSV/markdown/JSON
  (Option-valued; empty cell when absent).
- **SPA:** corpus percentile in the file-drawer header and the bivariate
  map tooltip; a compact "vs corpus" annotation on the Code factor tile.
- **MCP:** field on `code_health` tool rows.
- **Gates:** new optional `corpus_percentile_max` key in
  `[gates]`, following the existing rows-based evaluator pattern
  (skip-with-ledger-record when no calibration data, mirroring the
  sidecar gate's honest-skip contract).

## 7. Testing

- Golden-artifact determinism: a tiny test calibration built from the
  bundled fixture repos, committed; building it twice yields identical
  bytes (fixture-bundle precedent).
- Interpolation unit tests: exact breakpoints, midpoints, below-min,
  beyond-max (`beyond_corpus`), unknown language, unknown format version.
- `calibrate` round-trip over two fixture repos, including `--merge`.
- Additivity contract test: with the corpus lens active, previously
  shipped fields (`percentile`, `band`, `score`) are byte-identical to a
  run without it.
- Per-biomarker tests with hand-verifiable fixtures (extend
  `biomarker_repo` content via bundle regeneration if needed).

## Out of scope

Coverage ingestion, code-cartography, LLM enrichment, own-repo defect
calibration (Phase 3 candidates, unchanged), scheduled world-artifact CI
rebuilds.

## Risks

- **Biomarker availability drift across languages** — mitigated by the
  per-language honest-absence rule and the plan-time validation gate.
- **Corpus representativeness** — mitigated by size stratification in the
  format, pinned-SHA reproducibility, and the override mechanism (an org
  corpus is always more representative for that org than any world
  corpus).
- **Artifact staleness** — vintages are explicit, stamped in provenance,
  and cheap to regenerate.
