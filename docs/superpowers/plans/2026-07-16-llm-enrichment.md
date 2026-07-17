# Advisory LLM Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Opt-in LLM narratives (per-file diagnosis + refactoring direction, PR-diff narrative) grounded in CodeLore's deterministic facts, with a citation check, two-dialect client (Anthropic-native + OpenAI-compat, local-first), and a content-hash sidecar cache — never touching scored output.

**Architecture:** New advisory module `crates/codelore-lib/src/enrichment/` (fact sheets → versioned prompts → sync HTTP client → citation check → sidecar cache), surfaced via `explain <path>` / `explain <path> --llm` / `diff --llm` and an MCP `explain_file` tool. Nothing in the scoring path imports it.

**Tech Stack:** Rust workspace; `ureq` 3 (rustls) promoted from existing build-dep to runtime dep of codelore-lib; existing analyses as fact feeds; rmcp MCP server.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-16-llm-enrichment-design.md` — binding.
- **Contract 1**: without `--llm`, every command's output is byte-identical to today (no default-path filesystem/network reads; no output changes).
- **Contract 2**: scoring isolation — no module under `analyses/`, `quality_gates/`, `facts/`, `calibration`, `defect_calibration`, `provenance`, `output/` imports `enrichment` (guard test).
- **Contract 3**: with `--llm`, analysis rows / SARIF / gate verdicts / exit codes / fact-store cache keys / provenance manifest are unchanged; narratives are additive text or fields only.
- **Contract 4**: every narrative carries `advisory — model <id>, grounded ✓` or `⚠ contains uncited claims` inline.
- Env surface (exact names): `CODELORE_LLM_PROVIDER` (`anthropic`|`openai-compat`), `ANTHROPIC_API_KEY`, `CODELORE_LLM_BASE_URL` (default `http://localhost:11434/v1`), `CODELORE_LLM_API_KEY` (optional), `CODELORE_LLM_MODEL` (required on openai-compat; Anthropic default = the `DEFAULT_ANTHROPIC_MODEL` const `"claude-sonnet-5"`). Keys never persisted.
- Sidecar cache: `cache::repo_cache_dir(cache_root, repo_path)/enrichment/<key>.json`, `key = lowercase hex sha256("{fact_sheet_text}|{SCHEMA_VERSION}|{PROMPT_VERSION}|{model}")` (plain hex — Windows-safe filename; do NOT use `hashing::sha256_prefixed`, its `sha256:` prefix contains `:`).
- No `#[allow]` (precedented idioms with inline justification only); no `unwrap()`/`expect()` outside tests; lib errors `CodeLoreError`, CLI `anyhow::Context`; no ticket IDs / plan refs / version numbers / test counts in code or docs (CHANGELOG only); conventional commits; NEVER `Co-Authored-By`.
- Gates per task: `/Users/emrec/.cargo/bin/cargo fmt --all --check`, `/Users/emrec/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, tests via the same pinned cargo (shell-default cargo is Homebrew 1.97 and ignores the 1.96 pin; keep the tree clean under BOTH). CI performs no live network calls.
- Shared CARGO_TARGET_DIR (`/Users/emrec/.cache/cargo-target`); on ENOSPC: verify no other cargo/rustc running, `rm -rf .../debug/incremental`, retry.

## Validated interfaces (verified at branch point — cite, don't re-derive)

- `ExplainArgs { topic: Option<String> }` (args.rs:247); `run_explain_cmd(&ExplainArgs)` (main.rs:1880) matches a hardcoded topics slice case-insensitively; unknown topic → `Err(Analysis("unknown topic …"))`. NO path handling exists — a path argument is a clean new branch.
- `DiffArgs` (args.rs:661, no `cache_dir` field); `run_diff_cmd` (main.rs:2293) → `diff::run_diff(args) -> Result<(DiffOutput, FactsDb, Options)>` (diff.rs:613) → `diff_output::emit(out, &output, format, repo, Some((&head_db, &head_opts)))`; exit 4 via `diff::should_fail`. Text emitter `emit_text` ends after the `pr_touched_existing` block (diff_output.rs:199-208); markdown ends with an all-sections-empty `✅` fallback (diff_output.rs:402-409) — insert the advisory section before it and extend its condition.
- MCP: `CodeLoreServer { repo: PathBuf }`; tools are `async fn(&self, params: Parameters<X>) -> Result<String, ErrorData>` with `X: Debug+Deserialize+JsonSchema(+Default)`, DuckDB work inside `tokio::task::spawn_blocking`, `FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())`, JSON out via `serde_json::to_string`, shared `internal(e)` error mapper; registration is automatic via the single `#[tool_router] impl` block (mcp.rs:206-565).
- Cache: `cache::default_cache_root() -> PathBuf`; `cache::repo_cache_dir(cache_root: &Path, repo_path: &Path) -> PathBuf` (pub) → `<root>/codelore/<hash8>/`; ledger precedent `ledger.rs::now_utc_ts() -> String`.
- Fact feeds: `run_code_health(db, opts) -> Result<Vec<CodeHealthRow>>` (row: path, cognitive, score, structural_risk, percentile, band, corpus_percentile, beyond_corpus); `defect_calibration::validate::capture_intensities(db) -> Result<HashMap<String,[f64;8]>>` — MUST be called immediately after a HEAD code-health run on the same db; `run_coupling(db, opts) -> Vec<CouplingRow{entity_a,entity_b,shared,revs_a,revs_b,degree,fisher_p}>`; `run_ownership(db, opts) -> Vec<OwnershipRow{path,main_author,total_revs,fractal_value}>`; `run_hotspots(db, opts) -> Vec<HotspotRow{path,hotspot_score,revisions,…}>`; `run_function_xray<R: Repo>(db, repo, opts, target: &str) -> Vec<FunctionXrayRow{function,change_freq,loc,cyclomatic,cognitive,last_changed}>`; `run_cycle_health(db, opts) -> Vec<CycleHealthRow{cycle_id,size,members_preview,heat_pct,verdict,extract_candidate,predicted_pc_drop}>`; `defect_calibration::{load(path), check_repo_identity(art, repo_path, allow_foreign)}`, `ValidationMetrics{band_table,auc_default,precision_at_10,precision_at_red,implicated_files,linked_defects,…}`.
- `DiffOutput` fields incl. `gate_violations`, delta-health section (`DeltaHealthSection{ratio: Option<f64>, verdict, counts, functions}`), rank entrants, score-increased, coupling absences, new clones, pr_touched.
- `ureq = "3"` already a `[build-dependencies]` of codelore-lib; ureq 3.3.0 + rustls family already in Cargo.lock and deny-clean. `codelore-lib` has NO async runtime; keep it that way.
- Env reads are ad-hoc `std::env::var` (no helper exists); `output/banner.rs::should_color()` is the NO_COLOR precedent.
- `SMELL_WEIGHTS` order (biomarker slot names, index 0..7): complex-method, god-class, large-method, dry, shotgun-surgery, deep-nesting, many-args, complex-conditional.

---

### Task 1: Fact sheets + scoring-isolation guard

**Files:** Create `crates/codelore-lib/src/enrichment/mod.rs` (`pub mod fact_sheet;` + module doc: advisory layer, never imported by scoring), `crates/codelore-lib/src/enrichment/fact_sheet.rs`; Modify `crates/codelore-lib/src/lib.rs` (`pub mod enrichment;`). Test: `crates/codelore-lib/tests/enrichment_fact_sheet_test.rs`, `crates/codelore-lib/tests/enrichment_isolation_test.rs`.

**Interfaces (produces):**
```rust
pub const SCHEMA_VERSION: u32 = 1;
pub struct FileFactSheet { pub path: String, pub sections: Vec<(String, Vec<(String, String)>)> } // ordered (section, [(key, value)]) — already sorted, values pre-formatted
impl FileFactSheet {
    pub fn build<R: Repo>(db: &FactsDb, repo: &R, opts: &Options, path: &str) -> Result<Self>;
    pub fn to_canonical_text(&self) -> String;   // deterministic; the cache-key + prompt input
    pub fn to_human_text(&self) -> String;       // the `explain <path>` dossier rendering
    pub fn digest(&self) -> String;              // lowercase hex sha256 of canonical text
    pub fn numeric_values(&self) -> Vec<f64>;    // every parseable numeric value, for the citation check
}
pub struct DiffFactSheet { pub sections: Vec<(String, Vec<(String, String)>)> }
impl DiffFactSheet { pub fn from_output(output: &DiffOutput_like) -> Self; /* same to_canonical_text/digest/numeric_values via a shared trait or duplicated small impls — implementer: extract a private helper, one impl */ }
```
`DiffOutput` lives in codelore-cli (diff.rs). To keep the lib crate independent, `DiffFactSheet::from_sections(sections: Vec<(String, Vec<(String,String)>)>) -> Self` is the lib-side constructor; the CLI (Task 6) flattens `DiffOutput` into sections itself. Both sheets share one canonical-text renderer: `section\n  key = value\n` lines, sections and keys in insertion order (builders insert deterministically), floats formatted `{:.6}` then trailing-zero-trimmed via one shared `fmt_num(f64) -> String`.

**Build recipe (FileFactSheet::build — exact order, all reuse):** (1) `run_code_health(db, &opts.with_no_row_limit())`, retain the target path's row → section "code-health" (score, band, structural_risk, percentile, corpus_percentile?, cognitive); error `CodeLoreError::Analysis("no code-health data for {path} — is it a tracked source file?")` if absent; (2) IMMEDIATELY `capture_intensities(db)` → section "biomarkers" with the 8 named intensities in SMELL_WEIGHTS order (skip section if path absent); (3) `run_hotspots` → rank + score if present; (4) `run_coupling` filtered to partners of path, top 5 by degree → "coupling" (partner, shared, degree, fisher_p); (5) `run_ownership` row → "ownership" (main_author, total_revs, fractal_value); (6) `run_function_xray(db, repo, opts, path)` top 5 by change_freq → "functions" (skip section on error — not all paths are Tier-1); (7) `run_cycle_health` rows where `extract_candidate == path` or `members_preview.contains(path)` → "cycle" (cycle_id, size, heat_pct, verdict, extract_candidate, predicted_pc_drop); (8) iff `opts.defect_calibration` set: `load` + `check_repo_identity` + section "defect-evidence" (artifact vintage, auc_default, band_table rows, whether path ∈ … not derivable per-file from artifact — include headline metrics only). Sections 3/6/7/8 are conditional; 1 is mandatory.

- [ ] Step 1: failing tests — `fact_sheet_is_deterministic` (biomarker fixture, ingest, build twice for the same path → `to_canonical_text()` byte-equal; digest equal; canonical text contains "code-health" and the file's band), `fact_sheet_unknown_path_errors` (message names the path), `numeric_values_extracts_floats` (unit-style on a hand-built sheet). Isolation test: `enrichment_isolation_test.rs` reads every `.rs` file under `crates/codelore-lib/src/{analyses,quality_gates,facts,calibration.rs,defect_calibration,provenance,output}` at test time (std::fs, relative to `CARGO_MANIFEST_DIR`) and asserts none contains `use crate::enrichment` / `crate::enrichment::`.
- [ ] Step 2: FAIL (module absent). Step 3: implement. Step 4: pass + fmt + clippy (pinned cargo). Step 5: commit `feat(enrichment): deterministic per-file and diff fact sheets`.

### Task 2: Prompts + citation check

**Files:** Create `crates/codelore-lib/src/enrichment/prompt.rs`, `crates/codelore-lib/src/enrichment/citation.rs`; Modify `enrichment/mod.rs` (pub mods). Tests in-module.

**Interfaces (produces):**
```rust
pub const PROMPT_VERSION: u32 = 1;
pub enum Lens { FileDiagnosis, DiffNarrative }
pub fn system_prompt(lens: Lens) -> &'static str;   // grounding rules: use only sheet facts, cite numbers, say "the data doesn't show" when unsupported; FileDiagnosis additionally: emit "## Diagnosis" always and "## Refactoring direction" ONLY when the sheet has a cycle/functions section with evidence
pub fn user_prompt(lens: Lens, fact_sheet_text: &str) -> String; // wraps the sheet verbatim
pub struct Groundedness { pub grounded: bool, pub unmatched: Vec<String> }
pub fn check_citations(narrative: &str, fact_values: &[f64]) -> Groundedness;
```
**Citation matching (exact):** extract numeric tokens with regex `(?:\d+\.\d+|\d+)(?:%)?` (also strip thousands separators); a token matches iff some fact value, rounded to the token's decimal places, equals the token's value — percent tokens additionally try `value*100` (so "80%" matches 0.803 → 80.3 → rounds to 80 at 0 dp). Whole-number tokens ≤ 12 with no `%` are exempt (list positions, section numbering) — document this in the fn doc. `grounded = unmatched.is_empty()`.

- [ ] Step 1: failing table tests — grounded narrative (every number present), ungrounded (one invented number → listed in `unmatched`), rounding ("0.79" vs 0.786 → grounded), percent ("80%" vs 0.803 → grounded), small-int exemption ("the 3 files" with no fact value 3 → still grounded), empty narrative → grounded. Prompt tests: `system_prompt(FileDiagnosis)` contains the grounding instruction and both section headers; `user_prompt` embeds the sheet verbatim.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass + gates. Step 5: commit `feat(enrichment): versioned prompts and the numeric citation check`.

### Task 3: Two-dialect ChatClient

**Files:** Create `crates/codelore-lib/src/enrichment/client.rs`; Modify `enrichment/mod.rs`, `crates/codelore-lib/Cargo.toml` (move `ureq = "3"` — verify exact existing build-dep version spec and reuse it — into `[dependencies]` with `features = ["json"]`; keep the build-dep entry). Tests in-module + `crates/codelore-lib/tests/enrichment_client_test.rs`.

**Interfaces (produces):**
```rust
pub trait ChatClient { fn complete(&self, system: &str, user: &str) -> Result<String>; fn model_id(&self) -> &str; }
pub struct AnthropicClient { /* api_key, model, base_url (default https://api.anthropic.com) */ }
pub struct OpenAiCompatClient { /* base_url, api_key: Option<String>, model */ }
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5";
pub const DEFAULT_OPENAI_COMPAT_BASE_URL: &str = "http://localhost:11434/v1";
pub const REQUEST_TIMEOUT_SECS: u64 = 120;
pub struct LlmEnv { pub provider: Option<String>, pub anthropic_key: Option<String>, pub base_url: Option<String>, pub api_key: Option<String>, pub model: Option<String> }
impl LlmEnv { pub fn from_process_env() -> Self; }        // the ONLY env-reading site
pub fn resolve_client(env: &LlmEnv) -> Result<Box<dyn ChatClient>>; // pure over LlmEnv → unit-testable without env races
```
**Dialects (exact):** Anthropic — `POST {base}/v1/messages`, headers `x-api-key`, `anthropic-version: 2023-06-01`, body `{"model", "max_tokens": 1024, "system", "messages":[{"role":"user","content": user}]}`, response text at `content[0].text`. OpenAI-compat — `POST {base}/chat/completions`, optional `Authorization: Bearer`, body `{"model", "messages":[{"role":"system"...},{"role":"user"...}]}`, response at `choices[0].message.content`. Both: ureq agent with `REQUEST_TIMEOUT_SECS` total timeout, no retries; non-2xx → `CodeLoreError::Analysis` including status + first 200 chars of body. **Resolution:** provider explicit → that dialect (anthropic requires key, openai-compat requires model); else anthropic_key present → Anthropic (model = `model` or default); else OpenAiCompat (base_url or default; model required, error text: `set CODELORE_LLM_MODEL (e.g. from \`ollama list\`)`).

- [ ] Step 1: failing tests — resolution matrix (7 cases: explicit anthropic ok/missing-key, explicit compat ok/missing-model, key-implies-anthropic, default-local, provider unknown → error naming valid values) as pure `LlmEnv` unit tests; dialect round-trip: spawn a `std::net::TcpListener` on 127.0.0.1:0 in the test serving one canned HTTP response per dialect, point the client at it, assert request path/headers/body shape and parsed response (no external network).
- [ ] Step 2: FAIL. Step 3: implement (incl. Cargo.toml move; run `/Users/emrec/.cargo/bin/cargo deny check` — expected clean since the crate family is already resolved). Step 4: pass + gates + deny. Step 5: commit `feat(enrichment): two-dialect chat client — Anthropic-native and OpenAI-compatible, local-first`.

### Task 4: Sidecar cache + engine orchestrator

**Files:** Create `crates/codelore-lib/src/enrichment/cache.rs`, `crates/codelore-lib/src/enrichment/engine.rs`; Modify `enrichment/mod.rs`. Tests in-module + extend `tests/enrichment_fact_sheet_test.rs`.

**Interfaces (produces):**
```rust
// cache.rs
pub struct CachedNarrative { pub narrative: String, pub grounded: bool, pub unmatched: Vec<String>, pub model: String, pub prompt_version: u32, pub schema_version: u32, pub fact_digest: String, pub created_at: String }
pub fn cache_key(fact_sheet_text: &str, model: &str) -> String; // hex sha256("{text}|{SCHEMA_VERSION}|{PROMPT_VERSION}|{model}")
pub fn cache_path(cache_root: &Path, repo_path: &Path, key: &str) -> PathBuf; // repo_cache_dir()/enrichment/<key>.json
pub fn read(cache_root: &Path, repo_path: &Path, key: &str) -> Option<CachedNarrative>; // None on missing/corrupt (corrupt → tracing::warn)
pub fn write(cache_root: &Path, repo_path: &Path, key: &str, entry: &CachedNarrative); // create dirs; warn-not-fail on io error
pub fn latest(cache_root: &Path, repo_path: &Path) -> Option<CachedNarrative>; // newest entry by created_at — Task 5 compares its fact_digest for the staleness note
// engine.rs
pub struct NarrativeResult { pub narrative: String, pub grounded: bool, pub unmatched: Vec<String>, pub model: String, pub from_cache: bool }
pub fn narrate(client: &dyn ChatClient, lens: Lens, fact_sheet_text: &str, fact_values: &[f64], cache_root: &Path, repo_path: &Path, refresh: bool) -> Result<NarrativeResult>;
pub fn stamp(result: &NarrativeResult) -> String; // "advisory — model {m}, grounded ✓" | "advisory — model {m}, ⚠ contains uncited claims"
```
`narrate` flow: key → (unless refresh) cache read → hit returns with `from_cache=true`; miss → `client.complete(system_prompt(lens), &user_prompt(lens, text))` → `check_citations` → write cache → return.

- [ ] Step 1: failing tests — cache round-trip in a tempdir; key changes when any of {text, model} changes and when SCHEMA_VERSION/PROMPT_VERSION consts change (test by asserting the key embeds current consts via recomputation, not by mutating consts); corrupt file → None + no panic; engine with a local `MockChatClient` (a test struct implementing ChatClient with a canned reply — define it in the test file): first call `from_cache=false`, second `from_cache=true`, refresh forces regeneration; `stamp` renders both verdicts.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass + gates. Step 5: commit `feat(enrichment): sidecar narrative cache and the narrate orchestrator`.

### Task 5: `explain <path>` and `explain <path> --llm`

**Files:** Modify `crates/codelore-cli/src/args.rs` (ExplainArgs += `repo: PathBuf` default ".", `llm: bool`, `llm_refresh: bool`, `cache_dir: Option<PathBuf>` — doc comments per existing style), `crates/codelore-cli/src/main.rs` (`run_explain_cmd` new branch). Test: `crates/codelore-cli/tests/cli_test.rs`.

**Dispatch rule (exact):** keep the existing topic lookup FIRST (byte-identical behavior for every current invocation). On lookup miss, if `args.repo.join(topic)` exists as a file → the fact-sheet branch; else the existing unknown-topic error, extended to mention that an existing file path prints the file's evidence dossier. Fact-sheet branch: `GixRepo::open(&args.repo)` → `FactsDb::open_or_ingest_with_cache_root` (cache_root = `cache_dir.unwrap_or_else(default_cache_root)`) → `FileFactSheet::build` → print `to_human_text()`; then if a cached narrative exists whose `fact_digest != sheet.digest()` print `note: cached narrative is stale — evidence changed; re-run with --llm` (via `cache::latest`); with `--llm`: `resolve_client(&LlmEnv::from_process_env())?` (hard error, config hint) → `narrate(...)` → print narrative + `stamp()` line.

- [ ] Step 1: failing e2e in cli_test.rs — (a) `explain hotspots` still prints the topic text (golden unchanged); (b) `explain <fixture file> --repo <fixture>` exits 0, output contains "code-health" and the band, no network env set; (c) `explain <nonexistent> --repo <fixture>` errors mentioning both topics and file paths; (d) `explain <file> --llm --repo <fixture>` with `CODELORE_LLM_BASE_URL` pointed at a test-local TcpListener serving a canned chat-completions reply → output contains the canned narrative and `advisory — model`, and a grounded/ungrounded stamp; (e) same but no LLM env and no local server → non-zero exit, error names `CODELORE_LLM_MODEL`/provider setup.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass + full cli_test + gates. Step 5: commit `feat(cli): explain <path> — deterministic evidence dossier with opt-in grounded narrative`.

### Task 6: `diff --llm` + MCP `explain_file`

**Files:** Modify `crates/codelore-cli/src/args.rs` (DiffArgs += `llm: bool`, `llm_refresh: bool`), `crates/codelore-cli/src/main.rs` (`run_diff_cmd`), `crates/codelore-cli/src/diff_output.rs` (advisory block in `emit_text` after the pr_touched block and in `emit_markdown` before the `✅` fallback — extend the fallback's emptiness condition; json/sarif untouched), `crates/codelore-cli/src/mcp.rs` (9th tool). Tests: cli_test.rs + `crates/codelore-cli/tests/mcp_test.rs`.

**diff flow:** after `run_diff` returns, iff `args.llm` and format is `text`/`markdown`: flatten `DiffOutput` into `DiffFactSheet::from_sections` (sections: "verdict" — delta ratio + verdict + counts; "gates" — violations; "entrants"/"score-increased"/"absences"/"clones" — per-row key facts), resolve client + narrate; ANY failure → `eprintln!("warning: llm narrative unavailable: {e}")` and continue — deterministic output + `should_fail` exit code untouched (Contract 3). Success → pass the narrative + stamp into the emitter (thread as `Option<(String, String)>` argument to `emit`, defaulting None), rendered as a delimited block titled `LLM narrative (advisory)`. `--llm` with json/sarif → one-line stderr note that the flag applies to text/markdown only.

**MCP tool (exact pattern from the validated `hotspots` sample):**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExplainFileParams { pub path: String }
#[tool(name = "explain_file", description = "Deterministic per-file evidence dossier (fact sheet) plus, when the server environment has an LLM configured, a grounded narrative with a citation-check verdict. The fact sheet is always returned; narrative_error is set instead of failing when the LLM is unavailable.")]
async fn explain_file(&self, params: Parameters<ExplainFileParams>) -> Result<String, ErrorData> { /* spawn_blocking: open db, FileFactSheet::build, then LlmEnv::from_process_env → resolve_client ok? narrate → {fact_sheet, narrative, grounded, model} : {fact_sheet, narrative_error} — serde_json::json! object as String */ }
```

- [ ] Step 1: failing tests — cli: (a) `diff` WITHOUT `--llm` on the existing diff fixture → output byte-identical to the current golden (Contract 1 for diff); (b) `diff --llm` with the test-local server → text output ends with the advisory block + stamp, exit code equals the no-flag run's; (c) `diff --llm` with no LLM env → warning on stderr, stdout identical to no-flag run, same exit code. mcp_test: `explain_file` on the fixture returns fact_sheet always; without LLM env the JSON has `narrative_error` and no `narrative`.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: pass + gates. Step 5: commit `feat(cli): diff --llm advisory narrative and the explain_file MCP tool`.

### Task 7: Contracts, docs, CHANGELOG

**Files:** Test additions in `crates/codelore-cli/tests/cli_test.rs`; Modify `docs/advanced-usage.md` (new "LLM enrichment" section: the three outputs, grounding + citation check, env configuration table with the local-first default, cache behavior + `--llm-refresh`, the advisory guarantees), `README.md` (one feature bullet + MCP tool count wording stays count-free), `CHANGELOG.md` `[Unreleased]` (entries: explain path dossier; explain --llm; diff --llm; explain_file MCP tool; env vars + local-first posture). Optional live check: one `#[ignore]`d test hitting `http://localhost:11434` when ollama is present.

- [ ] Step 1: contract tests — `analyze`/`check` invocations unchanged (no new flags there; assert `--llm` is rejected by clap on `analyze` — it was never added); re-run the Task 1 isolation guard; grep guard `git grep -nE "F[0-9]{3}|Task-[0-9]|v0\.[0-9]" crates/ docs/advanced-usage.md README.md` → no new hits vs branch point.
- [ ] Step 2: docs written per hard rules (current contract only). Step 3: full workspace suite + both-toolchain clippy + fmt. Step 4: real-CLI smoke on THIS repo: `explain crates/codelore-lib/src/output/spa.rs --repo .` prints the dossier; with ollama running, the `--llm` variant produces a stamped narrative (paste into report; skip gracefully if no ollama, note it). Step 5: commit `docs(enrichment): LLM enrichment guide + changelog`.

---

## Self-review notes (spec coverage)

Spec §2 surfaces → Tasks 5/6; §3 fact sheets → Task 1; §4 prompts/citation → Task 2; §5 client → Task 3; §6 cache → Task 4; §7 error handling → embedded in Tasks 5/6 (hard-error explain, degrade diff, MCP narrative_error); §8 contracts → Task 1 (isolation), Tasks 5/6/7 (byte-identical + additive), Task 4 (stamp); §9 testing → distributed per task + Task 7; §10 out of scope — no task builds SPA/SARIF/summary/streaming/cost/custom prompts. Staleness note (spec §2) → Task 5 via `cache::latest`.

## Out of scope (spec)

Generated code / auto-refactoring; SPA-drawer + SARIF enrichment; repo-level executive summary; streaming; cost accounting; prompt customization files; retry/fallback chains.
