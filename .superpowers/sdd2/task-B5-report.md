# Task B5 Report — MCP tool + docs

## Status: COMPLETE

## MCP tool — `finding_hotspot_overlap`

**File:** `crates/codelore-cli/src/mcp.rs`

Added `FindingHotspotOverlapParams` struct (empty, `Default`) and the `finding_hotspot_overlap` tool method following the established `spawn_blocking` pattern. Key behaviour:

- Checks `repo_cache_dir(&cache_root, &repo_path).join("external-findings.duckdb-ext")` for existence **before** calling `open_or_create` — the MCP tool never creates the sidecar.
- Absent sidecar → `{"findings": [], "note": "run codelore ingest-sarif first"}` (structured JSON, not an error).
- Empty store (file exists, 0 rows) → same note response.
- Present + populated → `run_finding_hotspot_overlap` + serialise rows.
- Two independent connections: `FactsDb` via `open_or_ingest_with_cache_root` and `ExternalStore` via `open_or_create`, never attached.

Imports added: `finding_hotspot_overlap` from `analyses`, `repo_cache_dir` from `cache`, `ExternalStore` from `external`.

## Tests — `mcp_test.rs`

- Count assert updated 7 → 8.
- `"finding_hotspot_overlap"` added to the expected names list in `mcp_tools_list_and_repo_overview`.
- New test `mcp_finding_hotspot_overlap_returns_note_when_sidecar_absent`: uses `tiny_repo` (sidecar never created), asserts response is not a tool error, `findings` array is empty, `note` string contains `"ingest-sarif"`.

All 8 MCP tests pass.

## Docs — `advanced-usage.md`

Replaced the brief one-line "Behavioral×static overlap gate" section with a full three-step section covering:
- `ingest-sarif` command with multi-file example and sidecar location
- Three dialect bullets: Semgrep (matchBasedId fingerprint, %SRCROOT% URI, rule-default level), clippy-sarif (no fingerprints, relative URIs), CodeQL (guaranteed primaryLocationLineHash, ruleIndex indirection, absolute file:// URIs)
- `finding-hotspot-overlap` column reference table and priority rules (act-now / plan / note)
- Gate configuration example

Added `finding_hotspot_overlap` tool entry to the MCP tool reference section (after `check_gates`), documenting the structured note response for absent sidecar.

## CHANGELOG

Updated the `codelore mcp` entry from "seven tools" to "eight tools", adding `finding_hotspot_overlap` to the list with its absent-sidecar note behaviour.

## README

Inserted "Behavioral×static fusion" paragraph after the `codelore check` paragraph describing the full ingest-sarif → finding-hotspot-overlap → gate → MCP tool pipeline.

## Gate results

- `cargo clippy --workspace --all-targets --all-features -- -D warnings` → clean
- `cargo fmt --all --check` → clean
- `cargo test -p codelore-cli --test mcp_test` → 8 passed, 0 failed

## B3-fix status

The required B3 fixes (SQL-equivalent tied ranks via `compute_percent_ranks`, `findings: usize → u32`, tied-ranks unit tests, biomarker_repo integration test) are committed in `958c262` ("fix(check): strongly typed check format flag") which also absorbed the A-lane's lint fixes for the B-lane files. The commit message names these fixes explicitly.
