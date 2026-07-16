# Advisory LLM Enrichment — Design

## 1. Purpose and positioning

CodeLore computes more socio-technical signal than any competitor — eight
biomarker intensities, behavioral fusion (churn, ownership, Fisher coupling),
corpus-relative percentiles, and the repository's own mined defect evidence —
but presents it as numbers. This feature adds an opt-in LLM layer that
synthesizes those deterministic facts into staff-engineer-grade prose:
a per-file health **diagnosis**, an evidence-directed **refactoring
direction**, and a reviewer-ready **PR narrative**.

The differentiation is grounding, not generation. CodeScene ACE generates
refactored *code* and validates it with a test harness; CodeLore generates
*claims* and validates them against its own computed facts with a citation
check. Advice with receipts, never generated code.

**Hard constraint (fixed):** enrichment is advisory. It never perturbs
analysis rows, SARIF, gate verdicts, exit codes, fact-store cache keys, or
the provenance manifest. With the flag off, output is byte-identical to a
build without the feature. The corpus-percentile additive lens is the
precedent.

## 2. Product surface

Three outputs, two CLI verbs, one MCP tool — all strictly opt-in per
invocation:

- **`codelore explain <path>`** — when the argument resolves to a tracked
  file path (rather than an analysis name, which keeps its existing static
  documentation behavior), prints the file's **fact sheet** in
  human-readable form: a deterministic per-file evidence dossier (no LLM, no
  network). If a cached narrative exists for the file, a staleness note is
  printed when the current fact sheet's digest no longer matches the one the
  narrative was generated from.
- **`codelore explain <path> --llm`** — the fact sheet plus the grounded
  narrative: a **Diagnosis** section (what is wrong and why, citing the
  computed values) and, only when structural evidence exists (cycle-health
  extract candidate, god-class membership, high-churn functions from
  function-xray), a **Refactoring direction** section citing the cut-points
  and co-change clusters CodeLore already computes.
- **`codelore diff --llm`** — the deterministic diff output prints exactly
  as today; after it, a clearly delimited advisory block containing one
  reviewer-ready paragraph explaining why the verdict is what it is and
  which files drove it.
- **MCP tool `explain_file`** — returns the fact sheet (always,
  deterministic) plus `narrative` and `grounded` fields when the server's
  environment has an LLM configured; otherwise the fact sheet alone, so
  agents without our client still receive structured evidence to narrate
  themselves. LLM failures populate a `narrative_error` field; the tool call
  itself succeeds.

Every narrative is stamped inline with its provenance:
`advisory — model <id>, grounded ✓` or
`advisory — model <id>, ⚠ contains uncited claims`.

## 3. Unit A — fact sheets (`enrichment/fact_sheet.rs`)

`FileFactSheet::build(db, opts, path)` and `DiffFactSheet::build(…)`
assemble a compact, deterministic, sorted-key serialization of values the
existing analyses already compute — they call the same `run_*` functions the
CLI dispatches to, never new computation.

File fact sheet contents: code-health row (score, band, structural risk,
self-relative and corpus percentiles), the eight biomarker intensities,
behavioral terms (churn, author fragmentation, revision count), top coupling
partners with Fisher significance, ownership picture (top authors, decayed
shares, active/departed), function-level churn leaders, cycle-health
membership with cut-point / extract-candidate when present, and — when a
defect-calibration artifact is configured — the file's defect-implication
evidence and the artifact's headline validation numbers.

Diff fact sheet contents: the delta-health verdict and ratio, per-file band
movements, new-hotspot and cycle findings, and gate outcomes.

The serialization carries a `schema_version` constant; adding a fact field
bumps it, which naturally invalidates cached narratives. Fact-sheet builds
are pure over the fact store: building twice yields byte-equal output.

## 4. Unit B — prompts and the citation check

- **`enrichment/prompt.rs`** — one versioned template per lens
  (`PROMPT_VERSION` constant). The system prompt's core instruction: use
  only facts present in the sheet; cite the numbers; where the data does not
  support a claim, say "the data doesn't show" rather than inventing.
- **`enrichment/citation.rs`** — after generation, extract every numeric
  claim from the narrative and match each against the fact sheet's values,
  tolerant of the narrative's rounding precision (a narrative "0.79" matches
  a fact value 0.786; a narrative "80%" matches 0.803). The result is a
  groundedness verdict plus the list of unmatched numbers. The check labels;
  it never blocks, retries, or edits.

## 5. Unit C — LLM client layer (`enrichment/client.rs`)

- **`ChatClient` trait** — `complete(system, user) -> Result<String>`,
  synchronous, implemented over a blocking rustls HTTP client (crate choice
  validated against cargo-deny at plan time; no tokio in the library path).
- **`AnthropicClient`** — native Anthropic messages dialect. Key from
  `ANTHROPIC_API_KEY`; model from `CODELORE_LLM_MODEL`, defaulting to a
  current Sonnet-class model constant.
- **`OpenAiCompatClient`** — chat-completions dialect covering
  ollama, llama.cpp, LM Studio, vLLM, OpenAI, and OpenRouter. Base URL from
  `CODELORE_LLM_BASE_URL` (default `http://localhost:11434/v1` — a local
  ollama); key from `CODELORE_LLM_API_KEY`, optional because local servers
  need none; model from `CODELORE_LLM_MODEL`, required on this path — the
  error message names the variable and suggests `ollama list`.
- **Resolution order** — `CODELORE_LLM_PROVIDER=anthropic|openai-compat`
  when set; otherwise `ANTHROPIC_API_KEY` present selects Anthropic-native,
  else the OpenAI-compat local default.
- **Posture** — local-first by default: out of the box nothing leaves the
  machine; a hosted provider requires an explicit environment change. Keys
  live in the environment only and are never persisted by codelore. The
  fact sheet (repository evidence) is the only content ever sent. No
  accounts, no telemetry, unchanged.

## 6. Unit D — sidecar cache (`enrichment/cache.rs`)

`<cache_root>/codelore/<repo_hash>/enrichment/<key>.json` where
`key = sha256(fact_sheet ++ schema_version ++ prompt_version ++ model)`.
Stored: narrative, groundedness verdict and unmatched numbers, model id,
prompt and schema versions, fact-sheet digest, created-at timestamp.

Same-evidence re-runs are free and byte-stable. Any change to the file's
evidence, the prompt, the schema, or the model misses naturally.
`--llm-refresh` bypasses the cache read (still writes). The `.json`
extension keeps entries invisible to the fact-store LRU prune, per the
gate-ledger precedent. Cache write failures degrade to a warning; the
narrative still prints.

## 7. Error handling

- `diff --llm` — LLM or configuration failure degrades to a one-line
  warning; the deterministic output and exit code are untouched.
- `explain <path> --llm` — fails hard with a configuration-hint error (the
  narrative is the requested product); `explain <path>` without the flag
  never touches the network and cannot fail for LLM reasons.
- MCP `explain_file` — never fails the tool call for LLM reasons; the fact
  sheet is returned with a `narrative_error` field.
- Citation-check anomalies (unparseable narrative) label the output
  ungrounded rather than erroring.
- Timeouts: a bounded per-request timeout with no retries — enrichment is
  interactive, not batch.

## 8. Contracts and guarantees

1. **Byte-identical without the flag** — contract-tested: no `--llm` means
   output identical to a build without the feature.
2. **Scoring isolation** — no module in the scoring path imports
   `enrichment/`; guarded by a dependency test.
3. **Additive with the flag** — analysis rows, SARIF, gate verdicts, exit
   codes, fact-store cache keys, and the provenance manifest are unchanged
   with `--llm` on; narratives are additive text/fields only.
4. **Grounding is visible** — every narrative carries its model id and
   groundedness verdict inline.

## 9. Testing

- Unit: fact-sheet determinism (build twice → byte-equal), citation-check
  table tests (grounded, ungrounded, rounding tolerance, percent forms),
  provider-resolution matrix, cache-key invalidation on each component
  (schema, prompt, model, evidence).
- Contract: byte-identical-without-flag; the scoring-isolation import
  guard.
- Integration: a `MockChatClient` with canned responses drives
  `explain --llm`, `diff --llm`, and the MCP tool end-to-end without
  network; one `#[ignore]`d live test against a local ollama for manual
  runs. CI performs no live network calls.

## 10. Out of scope

Generated code / auto-refactoring; SPA-drawer embedding and SARIF message
enrichment (both later reduce to reading the sidecar); repo-level executive
summaries; streaming output; per-token cost accounting; prompt customization
files; retry/fallback provider chains.
