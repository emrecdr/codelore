# Task B4 Report — external-findings overlap gate

## Status: COMPLETE (including B4-fix)

**B2 fix commit:** `cb368b8` fix(external): transactional engine replace + multi-file grouping test
**B4 commit:** `49ccb7d` feat(check): external-findings overlap gate
**B4-fix commit:** `75c22ca` fix(check): reuse gate analysis rows in overlap gate + skip-path test
**Branch:** `feat/sarif-fusion`
**Tests:** 6 unit tests (quality_gates) + 1 CLI skip-path test — all green. Clippy clean. fmt clean.

---

## What was built

Optional gate `max_findings_in_hot_files` in `[gates]`:

```toml
[gates]
max_findings_in_hot_files = 0  # zero act-now overlap rows allowed
```

Fails when the number of `"act-now"` rows from `finding-hotspot-overlap`
exceeds the threshold. Acts-now are paths where: findings > 0 AND
revs_percentile ≥ 0.9 AND health_band == "red".

**Skip semantics**: gate is skipped (not failed) when the external findings
sidecar is absent or empty. Prints a distinct warning line; verdict = "skipped"
in ledger; exit code unaffected. Mirrors degraded print style.

---

## Files touched

| File | Change |
|------|--------|
| `quality_gates/mod.rs` | `Gates.max_findings_in_hot_files: Option<u32>` field, `is_empty()` update, `evaluate_finding_overlap_rows()` pure fn, 6 unit tests |
| `quality_gates/ledger.rs` | Extend verdict rustdoc: `"passed" | "failed" | "degraded" | "skipped"` |
| `main.rs` | Wire gate inline in `run_check_cmd` after `evaluate_all_gates`; `violations` made `mut` |
| `docs/advanced-usage.md` | One-sentence §11.8 entry for the behavioral×static overlap gate |

---

## Key design decisions

**Moved into `evaluate_all_gates`** (B4-fix): The overlap gate is now the last gate inside `evaluate_all_gates`, receiving `external_store: Option<&ExternalStore>` as a parameter. This mirrors how `max_red_effort_pct` reuses `code_health` rows. The caller (`run_check_cmd`) opens the store with `ExternalStore::open_existing` — which returns `None` without creating the file when no sidecar exists.

**`eval_hotspot_gates` now returns rows**: Changed return type from `(GateGroupResult, usize)` to `(GateGroupResult, Vec<HotspotRow>)`. The count becomes `rows.len()` at call site. The rows are passed to `run_finding_hotspot_overlap_with` inside `evaluate_all_gates` so hotspots are only run once per check.

**`run_finding_hotspot_overlap_with`**: New pub fn in `finding_hotspot_overlap.rs` that accepts precomputed `hotspot_rows` and `health_rows` slices. The original `run_finding_hotspot_overlap` is now a thin wrapper calling this. Follows the exact shape of `run_effort_exposure_with_health`.

**`ExternalStore::open_existing`**: New method that returns `Ok(None)` when the sidecar file is absent — no directory created, no file written. `open_or_create` is unchanged and still used by `ingest-sarif`.

**No sidecar side-effect**: `run_check_cmd` now calls `open_existing` instead of `open_or_create` for the gate path. A repo that has never run `ingest-sarif` will never see a sidecar created by `codelore check`.

**`cast_precision_loss`**: `act_now_count` (usize) cast to f64 for ledger `value` field. Suppressed with `#[allow]` + comment — repo-scale counts are always < 2^52.

---

## Tests

### Unit tests (6 in quality_gates::tests — unchanged from B4)

1. `finding_overlap_gate_fires_when_act_now_count_exceeds_threshold`
2. `finding_overlap_gate_passes_when_act_now_count_at_threshold`
3. `finding_overlap_gate_ignores_non_act_now_rows`
4. `finding_overlap_gate_measured_value_is_act_now_count`
5. `finding_overlap_gate_toml_key_parses`
6. `finding_overlap_gate_unknown_key_rejected`

### B4-fix: CLI skip-path test (cli_test.rs)

`check_max_findings_gate_skips_gracefully_when_no_sidecar`:
- Sets `max_findings_in_hot_files = 0` in thresholds
- Verifies sidecar does NOT exist before the run
- Runs `codelore check` — asserts exit 0
- Asserts sidecar does NOT exist after the run (no side-effect creation)
- Reads ledger via `read_gate_runs` — asserts `verdict == "skipped"` for `max_findings_in_hot_files`

### B4-fix: `_with` agreement test (finding_hotspot_overlap_test.rs)

`with_variant_and_wrapper_agree_on_identical_inputs`:
- Injects one `semgrep/warning` finding for `src/main.rs`
- Runs `run_finding_hotspot_overlap` (wrapper path: runs hotspots + code_health internally)
- Runs `run_hotspots` + `run_code_health` independently, then `run_finding_hotspot_overlap_with`
- Asserts all fields (`path`, `findings`, `engines`, `worst_level`, `health_band`, `priority`, `hotspot_score`, `revs_percentile`) are byte-identical between both call paths

---

## B4-fix completion

**COMMIT 1 SHA:** `75c22ca` — fix(check): reuse gate analysis rows in overlap gate + skip-path test
**COMMIT 2 SHA:** `9fa9d4b` — chore: untrack process report artifact

**Test summary:** 6/6 finding_hotspot_overlap integration tests pass; 59/59 CLI tests pass (including `check_max_findings_gate_skips_gracefully_when_no_sidecar`); 8/8 MCP tests pass. `cargo fmt --all --check` clean. `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.

**Concerns:** None. The `open_existing` method uses `CREATE TABLE IF NOT EXISTS` after opening an existing file — this is harmless (the table already exists) but technically not pure read-only. The comment in the docstring is accurate: the method never creates the file when absent, so the no-sidecar-side-effect contract holds.
