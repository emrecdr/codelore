# Delta Health — per-change health verdict for `codelore diff` / `codelore check`

**Status:** approved design, ready for implementation planning.

## Problem

Snapshot health scores are provably insensitive to individual changes: the Delta
Maintainability Model paper (SIG/TU Delft, TechDebt 2019) shows a ~200-line
bug-fix that introduced worse code moved a 244-KLOC system score by −0.007 on a
−5..5 scale. CodeLore's `diff` mode today gates on base→head *median* code
health — the same snapshot-aggregate weakness. Reviewers and CI need a score
that judges **the change itself**.

DMM's own weak spot is its unsupported 0.5 Good/Bad binarization (39%
misclassification in its middle band). We improve on it with an explicit
uncertainty verdict, and we harden it against AI-era churn by making
copy/paste-shaped additions unable to score well.

## Decisions (settled in brainstorming)

| Fork | Decision |
|---|---|
| What is judged | **Change + context**: the changed code's risk profile, weight-modulated by the health band of the file it lands in |
| Granularity | **Function-level**, by diffing per-function `complexity_metrics` at base vs head |
| Risk thresholds | **Fixed absolute constants** (deterministic, PR-stable); snapshot code-health keeps its percentile philosophy — the two coexist deliberately |
| Headline output | **Ratio 0–100 + verdict** `improving` / `indeterminate` / `degrading`, middle band explicitly low-signal |

## 1. Placement & data flow

New analysis module `crates/codelore-lib/src/analyses/delta_health.rs`, invoked
from the existing `codelore diff` flow in `codelore-cli/src/diff.rs`, which
already materializes full facts (including per-function `complexity_metrics`)
for base and head in temporary worktrees.

**Changed-function detection is table-diffing, not git-diff parsing.** Join the
base and head `complexity_metrics` tables on `(path, name)`:

- present only at head → **added**
- present only at base → **removed**
- present in both, any metric differs → **modified**
- identical rows → untouched (excluded entirely)

No `Repo` trait change and no diff-hunk parsing — the differential-test surface
is untouched.

**Documented v1 limitations:** file renames and within-file function renames
read as remove+add; functions whose bodies changed without moving any persisted
metric read as untouched (acceptable: a change that moves no risk metric cannot
change the risk class either).

## 2. Risk model

Each changed function is classified **low / medium / high** risk from absolute
thresholds over the properties with validated signal (LOC, cyclomatic, nesting
— cognitive complexity deliberately excluded: two independent studies show it
adds no predictive power), plus clone membership as an operation-typed penalty:

| Property | Low | Medium | High | Anchor |
|---|---|---|---|---|
| Function LOC | ≤ 30 | 31–70 | > 70 | SIG unit-size bands / CodeScene Large Method > 70 |
| Cyclomatic | ≤ 5 | 6–10 | > 10 | SIG unit-complexity bands / CodeScene CC > 9 |
| Nesting depth | ≤ 2 | 3 | ≥ 4 | CodeScene Deep Nested Complexity ≥ 4 |
| Clone membership (head Type-1/2 group) | — | — | forced **high** | copy/paste penalty; makes AI-pasted additions unable to score low-risk |

Function class = the worst class any property triggers. Constants live in one
documented table in `delta_health.rs`; **not** TOML-configurable in v1, so the
gate cannot be quietly loosened.

Per function we record **direction**: `before_class → after_class` (added =
∅ → class; removed = class → ∅).

## 3. Scoring

- **Weight** `w` = function LOC at head (at base for removed functions).
- **Outcome** per changed function:
  - *good* — after-class is low, OR direction strictly improved
    (after < before), OR a high-risk function was removed;
  - *bad* — after-class is high, OR direction strictly degraded
    (after > before) ending at ≥ medium;
  - *neutral* — everything else (e.g. stayed medium with no class change).
- **Context modulation:** for functions in files whose **base** file-level
  code-health band is red, good and bad weights are multiplied by **1.5** —
  work inside alert-band files counts more in both directions. Neutral weight
  is never modulated: touching legacy code without worsening it is not
  punished.
- **`delta_health_ratio`** = `100 × Σ good_w / (Σ good_w + Σ neutral_w + Σ bad_w)` — naturally 0–100.
- **Verdict:** `degrading` if ratio < 40, `improving` if ratio > 70, else
  `indeterminate` (explicitly labeled low-signal — the honest replacement for
  DMM's 0.5 cut, whose 0.33–0.66 band misclassified 39% of expert judgments).
  Cut-points are documented constants, calibratable in a future phase against
  own-repo fix-commit data.
- **No changed functions** (docs/config-only diff): verdict `no-code-change`,
  ratio omitted (`None`), gates vacuously pass.

## 4. Gates & output

New `[diff]` keys in `.codelore-thresholds.toml` (parsed alongside the existing
`DiffGates`, `deny_unknown_fields` preserved):

- `delta_health_min = <f64>` — fail if ratio < floor (skipped on `no-code-change`);
- `deny_degrading_verdict = true` — fail on a `degrading` verdict.

Violations flow through the existing `GateViolation` → `GITHUB_OUTPUT` /
`::error` plumbing in `run_check_cmd`.

`DiffOutput` gains a `delta_health` section:

```json
{
  "ratio": 62.5,
  "verdict": "indeterminate",
  "counts": { "added": 3, "modified": 5, "removed": 1, "skipped": 2 },
  "functions": [
    { "path": "...", "function": "...", "before": "medium", "after": "high",
      "outcome": "bad", "weight": 84, "in_red_file": true,
      "reasons": ["cyclomatic 14 > 10", "clone group member"] }
  ]
}
```

Rendered by the existing JSON/markdown diff emitters. `skipped` counts
functions in files the complexity scan does not cover — no silent omission.
This JSON shape is also the future MCP `analyze_change_set` payload and the
data source for the dashboard improvements feed.

## 5. Testing & invariants

- Unit tests: classification table (every property boundary), outcome/direction
  matrix, clone-penalty forcing, context multiplier, ratio math, verdict
  cut-points.
- Integration: `test-support` fixture builder constructs base/head repos with
  known function edits; assert exact ratio, verdict, per-function rows.
- Regression: whitespace-only change in a red-band file ⇒ `no-code-change`
  (not `degrading`); removing a high-risk function ⇒ good outcome.
- No facts-schema change, no cache-semantics change ⇒ **no `CACHE_EPOCH`
  bump**. No `Repo` trait change ⇒ differential gate untouched.
- Exit codes: gate failures surface through the existing check-mode paths
  (spec §6.6 codes unchanged).

## Out of scope (v1)

Line-level weighting; moved-code detection; rename tracking; percentile or
own-repo calibration of thresholds/cut-points; any LLM involvement.
