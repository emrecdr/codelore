# SPA Layout Overhaul — Design

Reorganizes the dashboard from a flat 23-card wall into six titled sections
with a sticky scrollspy navigation, and fixes the responsive layout so laptop
screens get one chart per row. Grouping and ordering follow dashboard-design
research (overview-first, four-factor section spine, trend → ranked → drill-down
within sections) and the layout idioms competitive analysis surfaced.

## Problem

- Thirteen wide widgets use a span class that only takes effect at ≥1280px, so
  on laptop widths (768–1279px) they render half-width in ~500px columns —
  the most cramped presentation exactly where most users sit.
- The 23 widgets are flat siblings with no grouping tier; related widgets
  (architecture suite, health suite) are scattered, and the only navigation is
  scrolling.

## Sections and ordering

Six titled sections, in this order, each ordered internally
overview → ranked → diagnostics:

| Section | Widgets in order |
|---|---|
| **Overview** | Quality dimensions (factor tiles) · Codebase at a glance (KPI tiles) · Guided tour · Hotspots hero (circle-pack with its lens tabs) |
| **Hotspots & Risk** | Hotspot table · Hotspots treemap · Function X-Ray |
| **Code Health** | Repo health timeline · Trends · Effort distribution (share bars) · Health improvements & regressions · Cognitive distribution · Multi-metric comparison |
| **Architecture** | Architecture graph · Dependency structure matrix · Architecture trend · Module coupling · Change coupling |
| **Knowledge** | Knowledge surfaces · Knowledge islands |
| **Delivery** | Delivery · Delivery risk (Kamei) · Commit activity |

The guided tour stays adjacent to the hotspots hero (the tour drives that
chart). Temporal delivery widgets (Kamei, calendar) group together. The
detail drawer is a modal and stays outside the section flow.

Each section is a `<section>` with a heading and its own grid container.
Group headings become the page's `<h2>` tier; widget titles demote from
`<h2>` to `<h3>` with styling preserved (heading hierarchy stays valid:
h1 page → h2 section → h3 widget). Any CSS selectors or tests that key on
widget `<h2>`s are updated in the same change.

## Responsive rules

- **Below 1280px** (laptops and smaller): every widget spans the full row —
  a single-column dashboard. The only multi-up presentation is *inside*
  cards whose content is a tile grid (factor tiles, KPI tiles), which keep
  their existing internal auto-fit packing.
- **At ≥1280px**: each section's grid becomes two columns; half-width cards
  pair *within their section* (Code Health: improvements feed + cognitive
  boxplot; Knowledge: surfaces + islands; Delivery: the delivery card may
  sit alone in its row). Wide chart widgets span both columns.
- The section grid is a hand-written CSS class in the template's inline
  `<style>` block (`display:grid` with a 1280px media query) — deliberately
  independent of the frozen pre-built Tailwind bundle, so no CSS-toolchain
  rebuild is required. Per-widget spans reuse the `xl:col-span-2` utility
  already present in the bundle; the inconsistent `md:col-span-2` uses are
  normalized to it.
- Wide inner content (DSM, tables) scrolls horizontally inside its card;
  the page body never scrolls sideways.
- Fixed-height chart hosts get a visual pass at full laptop width; heights
  may be adjusted where stretching distorts, but the height mechanism
  itself is unchanged.

## Navigation

- A sticky bar (hand-written CSS: `position: sticky; top: 0`, above-card
  z-index) under the existing header carries one chip per section plus the
  theme toggle relocation is NOT in scope (header stays as is; the sticky
  bar is net-new chrome below it).
- Scrollspy: an `IntersectionObserver` highlights the chip of the section
  in view; chip clicks use `scrollIntoView({behavior:'smooth'})`.
  **`location.hash` is never written or read for navigation** — the SPA
  already owns the hash as its state serializer, and anchor-based nav would
  corrupt it. A reduced-motion preference disables smooth scrolling.
- The four factor tiles double as jump links to their sections (Code →
  Code Health, Architecture, Knowledge, Delivery), using the same
  scrollIntoView path.
- Sections are collapsible (chevron on the heading) but always render
  expanded on load — collapse state is not persisted, so ECharts instances
  never initialize inside a hidden container. Expanding a section triggers
  the existing per-container resize sweep. A back-to-top button appears
  after scrolling.

## Implementation constraints (validated against the codebase)

- Renderers target widgets by element id, so reordering DOM sections is
  safe; the boot **paint order** lives separately in the `WIDGETS` registry
  and is re-ordered to match the new section order (factor header still
  paints first).
- Fullscreen/reset-zoom button injection iterates `section.widget` — the
  new grouping tier must not add `widget` to group containers.
- The selection/brush buses and the guided tour are DOM-order independent.
- The stale layout comment in the template claiming a different grid class
  is corrected in passing.

## Testing

- SPA integration test: the six section containers exist with their
  headings in order; every widget id is inside its assigned section; the
  nav bar lists six chips; widget titles are `<h3>`.
- Browser tests (real headless Chrome, existing conventions): (1) boot at
  ~1100px viewport → two representative chart cards each occupy the full
  content width (single column proven by geometry, not class names); (2) at
  ≥1440px the designated pairs share a row; (3) clicking a nav chip scrolls
  its section into view and the chip highlights, with zero console errors
  and no `location.hash` mutation; (4) collapsing and re-expanding a
  section leaves its charts sized (non-zero canvas) — the resize-on-expand
  path; (5) the full existing suite stays green.
- Real-CLI: generate the dashboard for this repository and visually verify
  at laptop width via a headless measurement (widget widths ≥ 90% of main
  content width below 1280px).

## Out of scope

- New widgets, chart content changes, or lens changes.
- URL-addressable sections (blocked by the hash serializer).
- Per-section lazy rendering (boot already yields between widgets).
- Moving the theme toggle or restyling the header.
- Mobile-specific (<768px) work beyond what single-column already gives.
