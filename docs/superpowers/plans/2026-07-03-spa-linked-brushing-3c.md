# SPA Linked-Brushing 3c — Set-Brush + A11y + Emphasis

> **Status (2026-07-03): the three core items below SHIPPED this session.** Bivariate set-brush (`7b9d3b6`, DECISION resolved: a separate `brush` store), screen-reader announce + parallel-coords visible highlight / F238 (`dd9cfad`, DECISION resolved: restyle per-item `lineStyle`, not re-enable the load-bearing-disabled emphasis), and the module-depth browser test (`5f7f5d0`). What remains is the 3d/3e work at the bottom. The task sections are kept for the record.
>
> **For agentic workers:** Builds on Plan 3b (four single-`.path` subscribers, shipped) and the 3b improvement pass (publish symmetry — map/sankey/treemap/X-Ray now broadcast; trends A→B fix; sankey module-depth mapping; aria-current on the table row).

**Goal:** Advance spec §5 beyond single-focus highlighting: add the **bivariate legend set-brush** (click a 3×3 health×activity cell → brush ALL files in that quadrant), a **spoken announcement** for cross-widget selection (a11y), and resolve the **parallel-coords visually-inert highlight** (F238). Each is a distinct interaction the single-`.path` store cannot express as-is.

**Architecture context (verified in the 3b validation):** the selection bus is a single nullable `Alpine.store('selection').path` fanned out to `window._codeloreSelectionListeners`. It models ONE focused file. A SET brush (many files at once) is a different data shape — it must NOT be conflated with the single-focus `.path` (that was the explicit reason 3b deferred it). The only `.set()` publish is `_codeloreShowDetail` (`widgets.js:638`); the drawer is non-modal; clearing is on drawer close.

**Tech Stack:** Vanilla JS in `widgets.js` + `template.html`, Alpine.js 3 store, ECharts 6. Gated behind the `spa` Cargo feature. Tested via `spa_integration_test` (HTML-string) + `spa_browser_test` (headless Chrome — the behavior gate).

## Global Constraints (carry from 3b)

- Offline single-file SPA sacrosanct — no new CDN/npm/build step/vendored lib.
- Mirror existing conventions (subscriber registration idiom, `_codeloreShowDetail` publish route). Highlight, don't hide.
- British "colour" in prose comments; American identifiers. NO task/version/PR/step markers in `widgets.js` comments (present-state only). The browser test keeps its own `Step N:` phase-label convention.
- Build/test on the macOS dev box: prefix cargo with `MACOSX_DEPLOYMENT_TARGET=15.0`; do NOT run `just ci` (spa link fails locally; GitHub Actions macOS-15 is the gate).
- Conventional Commits; never `Co-Authored-By: Claude`.

## Task 1 — Bivariate legend set-brush (the core 3c item)

**DECISION (design, resolve first):** how to model a multi-file brush without conflating it with the single-`.path` store. Options:
- (a) A SECOND store `Alpine.store('brush')` holding a `Set<path>` (or a quadrant key + a predicate), with its own fan-out `window._codeloreBrushListeners`, parallel to selection. Subscribers dim/emphasise the whole set. Keeps single-focus and set-brush orthogonal (recommended starting point).
- (b) Extend the selection store to carry an optional `paths: string[]` alongside `path`. Riskier — every existing subscriber must learn the set shape.
- Interaction: clicking a 3×3 cell brushes all files whose (health-band, activity-band) fall in that quadrant; clicking again clears. Brush + single-focus can coexist (brush = context, focus = one file).

**Scope:** clicking a `#bivariate-legend` cell computes the quadrant's file set (from the fusion overlay data already embedded) and brushes it across map + table + coupling + DSM. Highlight (emphasise the set / dim the rest), never hide.

## Task 2 — A11y: spoken announcement for cross-widget selection

The 3b improvement pass added `aria-current` on the selected table row (pure JS). The remaining a11y gap (validated, needs a template change so deferred here): a polite `aria-live` announcement of the selected file for screen-reader users. The existing `#hotspot-table-summary` live region is owned by the filter/row-count text (`refreshActions()` clobbers it) — so add a DEDICATED `aria-live="polite"` sr-only element in `template.html` and write the selected path into it from the fan-out (or a small selection subscriber). Assert via `spa_browser_test`.

## Task 3 — Resolve parallel-coords visually-inert highlight (F238)

The pre-existing `parallel-coords` subscriber calls ECharts `highlight`/`downplay`, but the series sets `emphasis: { disabled: true }`, so the cross-widget highlight has NO visible effect there. **DECISION:** either (a) re-enable emphasis on the parallel series with a tuned `lineStyle`/`opacity` so a selected file's polyline stands out (investigate WHY emphasis was disabled first — likely visual clutter), or (b) switch the subscriber to a non-emphasis mechanism (e.g. redraw the selected line with a distinct series/style). If neither is desirable, downgrade F238 to a documented known-limitation. When re-enabling, add the `downplay`-first guard (the trends A→B fix pattern) since emphasis then becomes additive.

## Test hardening carried from the 3b improvement pass

- **Module-depth coverage for the coupling subscriber — SHIPPED + now LIVE (F242 resolved).** The module-depth mapping fix maps the bus path through `modulePathSeg(selectedPath, userSankeyDepth)`. The original Step 13 (`5f7f5d0`) always skipped on `differential_repo` (near-zero co-changes → no depth-2 sankey nodes). Resolved (`373747e`, `9030159`): a dedicated `coupling_repo` fixture (3 modules; `alpha/svc`↔`beta/svc` co-changed 6× → guaranteed `src/alpha`↔`src/beta` depth-2 edge) + a standalone `sankey_module_depth_highlights_mapped_node` test that FAILS-not-skips on a missing node and asserts the highlight name equals the module prefix; the inert step was removed from the smoke test. `spa_browser_test` now 9/9, the new test exercises its assert.

## Deferred to 3d / 3e (unchanged)

- 3d: tabbed drawer + edge-bundled coupling.
- 3e: Observable Plot migration + IA restructure.

## Self-review checklist (fill at execution time)

- Set-brush store is orthogonal to single-`.path` (no conflation); brush + focus coexist.
- Every set-brush subscriber highlights/dims, never hides; clearing returns to neutral.
- A11y announcement uses a dedicated live region (not the clobbered summary one).
- parallel-coords decision recorded with rationale; downplay-first added if emphasis re-enabled.
