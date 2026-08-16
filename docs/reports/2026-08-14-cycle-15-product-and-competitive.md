# Cycle 15 — competitive position and validated improvement options

**Anchor:** `fbc9c93` (v0.27.4+) · **Type:** product/competitive analysis, not a defect audit.

> **Revised after an independent validation pass.** Codebase claims verified
> against source and held. Competitor figures did not: four repowise numbers
> had drifted and are corrected below against the vendor's own page, and one
> CodeLore tool enumeration was short by two. The **P1–P6 ranking is unchanged
> and remains the maintainer's call** — but two of its premises weakened under
> checking, noted inline at P2 and P3.

---

## 0. Why this report is different

The last five cycles narrowed to defect-hunting on a codebase that had become very hard to find defects in, and the findings shrank accordingly — a one-ULP artifact, a guard-matcher gap, an accessibility count. That was a failure of framing on my part, not a property of the project. The standing brief has always asked for *improvement options*, *feature sets*, *latest industry standards*, and *competitors*; I let those become a footnote under "residuals and currency" while the defect list got the whole budget.

So this cycle spent its budget on two things: a deep competitive scan, and **validating every idea against the actual codebase before proposing it** — which is the one discipline the last five cycles did teach. Everything below is marked with what exists today, at a file anchor, so nothing here is a suggestion to build something that is already built. That check changed three of the six proposals materially, and killed the version of the headline one I would otherwise have written.

---

## 1. The competitive picture, and one reframe that matters

**CodeLore's stated positioning — "code-maat successor / CodeScene alternative" — aims at a dormant project and at the wrong part of a competitor.**

- **code-maat** is dormant: last release v1.0.4, February 2023, and its own README says the analyses "evolved into CodeScene." Claiming succession to it is accurate but wins nothing.
- **CodeScene** is real but the overlap isn't where the positioning implies. Every *behavioural* capability — hotspots, technical-debt goals, code ownership — requires `CS_ACCESS_TOKEN` plus a hosted instance, and the product is dual-licensed with proprietary closed components, not open source. *(Two corrections from validation: the draft said their free/standalone MCP tier is "single-file static Code Health only" — it also performs **delta reviews** and business-case calculations locally, so the free tier is meaningfully wider than stated. And their published seat pricing is **€18 Standard / €27 Pro**; the draft's "€18–30" was fair at the bottom and loose at the top. The separately-quoted "€8–9/month" MCP add-on could not be confirmed on their pricing page and should be treated as unverified.)*
- **The actual competitor is `repowise`** (AGPL-3.0, **~5.9k stars**, `pip install repowise`): fully self-hosted, local web UI, **10 MCP tools** — a count they describe as "a deliberate ceiling rather than a limit we ran into" — code health from deterministic markers, hotspots, bus factor, ownership, **hidden coupling via co-change**, dead code, tree-sitter dependency graph over **19 languages parsed to AST, 13 of them at a "Full" framework-aware tier**, with PageRank centrality and Leiden communities. That is a near-total functional overlap with CodeLore's core, shipped monthly, in the same self-hosted OSS niche. *(Figures re-read from the vendor's repository during validation; the first draft's "~3.6k stars, 9 MCP tools, 15 languages" had all drifted. The release version is not stated on that page, so the draft's "v0.31.0 July 2026" is dropped rather than replaced with another number that will rot.)*

Two consequences. First, **the "self-hosted OSS behavioural analyzer with an MCP server" slot is contested, not empty** — the positioning needs to be against repowise, on depth rather than on existing at all. That depth claim has to be stated carefully: the draft's "57 analyses vs ~9 tools" compared CodeLore's *analysis catalogue* against repowise's *agent surface*, which are different things. Like for like, the agent surfaces are close — **11 MCP tools against 10**, and repowise caps theirs deliberately. The real asymmetry is behind the surface: 57 analyses against a much smaller set, architecture metrics, statistical rigour (Fisher significance, BH-FDR, Wilson intervals), Rust performance, and 11 output formats. Second, repowise **publishes validation numbers** — ROC AUC 0.74 (95% CI 0.68–0.79) across 21 repos, external validation on PROMISE/jEdit, and a claimed 2.3× defect density vs CodeScene under fixed review budget. Those are vendor self-benchmarks with no independent replication, and should be treated as unverified — but *the act of publishing them* has moved the price of entry.

**Where nobody is:** the engineering-intelligence vendors (DX, LinearB, Swarmia, Jellyfish, Faros) are converging on *AI-impact measurement* at team level and explicitly not on file-level behavioural analysis — Swarmia says so as a design stance, not a roadmap gap. And **no MCP server in this landscape exposes architecture metrics** (propagation cost, cycles, modularity violations) to an agent.

---

## 2. Validated improvement options, ranked

Each is marked **[exists]**, **[partial]** or **[absent]** against the tree, with anchors. Effort is my judgement; alignment is against the project's stated goals (self-hosted, deterministic, offline, no backward-compatibility debt).

### P1 — Ship the AI-code-quality analysis. You are one analysis away from the only tool that can answer 2026's most-asked question. **[partial — ~80% already built]**

This is the headline, and validating it changed it completely. I was going to propose building AI-authorship attribution. **It already exists.**

- `commits.ai_attribution TEXT` is a real column (`facts/schema_v1.sql:27`), populated at ingest.
- `identity::ai_attribution(email, name, message)` classifies into **`ai-authored` / `ai-assisted` / `human`** from bot identity plus `Co-authored-by:` trailers for Claude, Copilot, Cursor, Cody, Continue, Codeium, Windsurf, Devin, Tabnine, Amazon Q, and `(aider)` message tags.
- It is **user-extensible** via `.codelorebots` (`ai_attribution_with`, routed through `BotPatterns`).
- It is already consumed: `hotspots.rs:244` computes a per-file `ai_pct`, `knowledge/shares.rs:115,213` weights knowledge by it, `authors.rs:89` counts it.

That detection method — bot logins, author emails, `Co-authored-by` trailers — is **exactly the method arXiv 2603.28592 (SMU/HUST, April 2026) validated** across 302,579 AI-authored commits in 6,299 repos, where it found 89.3% of introduced issues were smells, 22.7% still alive at HEAD, and AI introducing 1.5× more security issues than it fixed. The research community converged on the same signal CodeLore already mines.

**What is missing is the last mile:**

1. **No analysis answers the question directly.** There is no `ai-attribution` analysis in the 57. `ai_pct` is a *column on hotspots*, not a lens. The question every engineering leader is now asking — *"is AI-generated code degrading our codebase, and where?"* — is answerable from data already in the store and is not asked.
2. **It is entirely absent from MCP.** `grep ai_attribution crates/codelore-cli/src/mcp.rs` → **0**. The agent surface, which is where this question is most naturally asked, cannot see it.
3. It is thinly surfaced elsewhere (1 reference each in CSV/markdown/SARIF, 3 in the SPA) and reads as a column, not a capability.

**Proposal:** an `ai-attribution` analysis crossing the existing attribution against the existing analyses — AI-authored share of hotspots, of red-band code-health files, of change-coupling edges, of knowledge islands; AI-vs-human defect density using the *existing* `calibrate-defects` fix-link mining; trend over time using the existing sampled-revision machinery. Plus an MCP tool and a `--format spa` section. Optionally a gate (`max_ai_hotspot_share`) once the numbers are understood — not before.

**Why this is the right bet:** DORA 2025 (still the current report — there is no 2026 edition) found ~30% of developers have little or no trust in AI-generated code and that AI correlates positively with throughput but *still negatively with delivery stability*. GitClear's 2026 data reports refactoring collapsing from 21% to 3.8% of changes and duplicated blocks up 81%. Every vendor in §1 is measuring AI adoption at the *team* level; **none can say which of your hotspots are AI-authored, or whether AI-authored code couples faster than human-authored code.** CodeLore can, offline, today, from data it already has.

**Effort:** medium (one analysis module + one MCP tool + SPA section; no new ingest, no schema change). **Alignment:** exact — deterministic, offline, no new dependencies.

### P2 — Package the agent guardrail loop as a first-class product. **[partial — primitives exist, packaging absent]**

Three independent vendors (CodeScene, Sigrid/SIG, and repowise's framing) have converged on the same agent-guardrail shape, and it is now a recognisable pattern:

1. a **pre-commit safeguard** on staged files;
2. a **branch/PR delta review** against a base ref;
3. **a hard stop rule in the agent's instruction file** — CodeScene's `AGENTS.md` says "Code Health is authoritative… if it regresses, refactor — don't declare done";
4. **distribution as skills/instruction files**, written into `.cursor/rules/`, `.github/copilot-instructions.md`, etc.

CodeLore has (1) and (2) as primitives — `delta_health`, `gate_changes`, `check_gates` over MCP, `codelore diff` with exit codes — and **none of (3) or (4)**. There is no consumer-facing `AGENTS.md`, no `.pre-commit-hooks.yaml` (only a copy-paste `pre-push` script at `docs/advanced-usage.md:1297`), and no skill-distribution command. Both absences were re-confirmed during validation, so the gap is real.

**Premise weakened under validation.** The headroom is narrower than the draft implied. CodeScene ships `codescene-mcp-server/AGENTS.md` — confirmed to exist — *and* performs delta reviews in its free local tier, so the competitor is further along this path than "single-file static Code Health" suggested. P2 is still a genuine gap in CodeLore's packaging; it is no longer a gap in the category.

CodeScene reports MCP-guided agents produced 2–5× more Code Health improvement, with Extract Method refactorings rising 7,550→21,702 — vendor-published and unreplicated, but directionally corroborated by MSR 2026 (arXiv 2601.20160), which found across 86 Java projects that **the five most common agent refactorings are annotation-related** — agents refactor superficially unless told what to optimise. A guardrail is what converts an agent from a debt generator into a debt reducer, and that is a claim with independent academic support (arXiv 2601.02200, with Tornhill as co-author, ties code health to semantic preservation after AI refactoring).

**Proposal:** ship `.pre-commit-hooks.yaml`; add a `codelore agent-rules --write` that emits an `AGENTS.md`/`CLAUDE.md`/Cursor-rules fragment stating the stop condition in the project's own gate terms; document the loop as a named workflow. This is packaging and docs, not engine work.

**Effort:** low. **Alignment:** exact — it is the existing gates, addressed to a new consumer.

### P3 — Expose architecture over MCP. **[absent — 0 of 11 tools]**

The 11 MCP tools are `repo_overview`, `hotspots`, `code_health`, `delta_health`, `refactoring_targets`, `function_xray`, `check_gates`, `finding_hotspot_overlap`, `explain_file`, `change_context`, `gate_changes`. *(The draft named nine of them, silently omitting `repo_overview` and `gate_changes`; the full list is now enumerated from source so the "none expose architecture" claim can be checked rather than trusted.)* **None expose architecture** — propagation cost, dependency cycles, modularity violations, instability, architecture roles — despite all five existing as analyses.

**Premise weakened under validation.** The draft called this "the cleanest unoccupied surface in the whole landscape". It is a real gap but a contested one: repowise ships cycle detection and architecture summaries over MCP, framing its dependency graph as *retrieval* (`get_context`, `search_codebase`) rather than architectural debt. CodeScene's architectural analysis is still absent from its MCP tool list, and Sonar and Codacy have no behavioural or architectural analysis at all. So the accurate claim is narrower: **nobody exposes *quantified* architecture metrics to an agent** — an agent asking *"will this change create an import cycle or raise propagation cost?"* can get a qualitative answer elsewhere but not a measured one. Whether that narrower gap still justifies P3's rank is a judgement left to the maintainer.

**Effort:** low (wrap existing analyses; the `Json<T>` pattern from `check_gates` is the template). **Alignment:** exact.

### P4 — Publish the defect-calibration benchmark. **[partial — computed, not published]**

`defect_calibration/mod.rs` already computes `auc_default`, `auc_train`, `auc_validation_default`, `precision_at_10`, `precision_at_red`. The numbers exist; they are not published anywhere a prospective user can see them.

repowise publishes ROC AUC with confidence intervals and external validation. CodeScene has peer-reviewed work with Tornhill's name on it. **A reproducible benchmark is now the price of entry in this category**, and CodeLore is the only one of the three that could publish a *fully reproducible* one — open corpus, open code, open artifact, no hosted service in the loop. That is a stronger claim than either competitor can make, and it is a reporting exercise rather than a research project.

**Effort:** low-medium (run the existing pipeline over a public corpus; publish method, data and artifact). **Alignment:** exact — it is what the calibration feature was built for.

### P5 — MCP structured output and protocol currency. **[partial — 1 of 11 structured]**

Only `check_gates` returns `Json<GateSummary>`; the other ten return `String`. Agents parse JSON out of text blocks, which is the pre-2025 shape. Separately, **MCP spec 2026-07-28** made the core stateless, added multi-round-trip requests, and deprecated Roots/Sampling/Logging and HTTP+SSE on a 12-month window — and the **Rust SDK is still in beta for that revision**. CodeLore is stdio-only, which insulates it from the transport half, but the deprecations and the SDK's beta status are a scheduled maintenance cost that should be planned rather than discovered.

The blocker is real and already documented in the ledger (E8): the ten remaining returns are heterogeneous — bare arrays, objects, arrays carrying a trailing `{omitted, total, note}` summary, and a plain-text briefing — and the summary shape needs redesign to fit a schema. That is per-tool design work, which is why this is P5 and not P2.

**Effort:** medium. **Alignment:** exact.

### P6 — Let an agent query the fact store. **[absent]**

Every competitor exposes fixed tool outputs. CodeLore has a **DuckDB fact store** with a documented schema. A read-only, statement-timeout-bounded, `SELECT`-only query tool would be a *category* difference rather than a feature difference: an agent could ask questions nobody shipped a tool for.

I am flagging this as the highest-variance item, not recommending it unreserved. It is in tension with the project's own discipline — every current tool is bounded, capped and disclosed, and an arbitrary-SQL surface is none of those. If it ships it needs a hard statement timeout, a row cap, a parser-level `SELECT`-only restriction (not a regex), and no access to DuckDB's filesystem or extension-install functions. Worth a design note before any code.

**Effort:** medium. **Alignment:** partial — genuinely novel, genuinely at odds with the bounded-surface principle.

---

## 3. Positioning and risk

**Positioning.** "Successor to a dormant project" undersells this. The defensible claim, from the evidence in §1, is narrower and stronger: **the only behavioural code analyzer that gives an AI agent hotspots, coupling, ownership and architecture with no account, no token, and no code leaving the machine.** CodeScene paywalls exactly those behind a hosted instance; Sonar and Codacy MCP require cloud; repowise is self-hosted but shallower and AGPL. For regulated, air-gapped or IP-sensitive buyers that combination is not a preference, it is a hard requirement.

**Licence risk (informational, flagged not recommended).** GPL-3.0 has no network copyleft: a SaaS competitor can host CodeLore without contributing back — which is plausibly why repowise chose AGPL. GPL also blocks proprietary embedding, where the Semgrep, Codacy and Sonar MCP servers are permissive. This is a strategic choice with real trade-offs in both directions and it is the maintainer's alone; I raise it only because the competitive set has diverged on it and the reasoning should be deliberate.

---

## 4. Honesty ledger

- **The narrowing was mine.** The brief asked for competitors and improvement options every cycle; I answered with a residuals list. Five cycles of shrinking findings should have prompted a change of surface far earlier than a user having to ask for it.
- **Validation changed three proposals and killed one.** P1 was drafted as "build AI-authorship attribution" and is now "you built it, ship the last mile" — a materially different, much cheaper proposal, and the check that produced it took ten minutes. P2 and P4 shrank from "build" to "package/publish" for the same reason. Had I not checked, this report would have recommended building three things that already exist, which is precisely the failure mode E7 caught in cycle 12.
- **Competitor numbers are not independently verified.** repowise's ROC AUC and 2.3×-vs-CodeScene claims, and CodeScene's 2–5× agentic-improvement figure and the 7,550→21,702 Extract Method counts, are vendor self-benchmarks with no replication I could find. They are reported as such and should not be repeated as fact. The Swarmia design-stance statement in §1 and the code-maat v1.0.4 release (the README surfaces v1.0.2) are likewise unconfirmed.
- **Every competitor figure in the draft had drifted, and the validation pass corrected its own notes too.** repowise's stars, tool count and language count were all wrong in the first draft; re-reading the vendor page also contradicted the intermediate correction I had written down (which claimed "10 core + 3 opt-in" tools and 18 languages — the page says ten tools with no opt-in split, and 19 languages). Volatile third-party numbers rot between the check and the write-up, which is why the release version is now omitted rather than restated.
- **The tool enumeration in P3 was short by two.** The draft listed nine of eleven MCP tools while asserting a property of all eleven. The list is now taken from source. This is the same defect class the cycle-14 and cycle-16 validations found: a claim inherited from an earlier document rather than re-derived.
- **DORA has no 2026 report.** The 2025 edition is current; anyone citing "DORA 2026" is wrong.
- **Limits.** Nothing compiled. Codebase claims are anchored to files in the v0.27.4+ tree and were checked by reading, not running. Competitor claims come from vendor docs, repos and licences fetched this cycle, with confidence marked; pricing and star counts move. Effort estimates are judgement, not measurement.

---

## 5. Recommended sequence

If only two things ship: **P1 and P2.** They compound — the AI-quality analysis gives an agent guardrail something to be authoritative *about*, and the guardrail packaging gives the analysis a consumer. Together they are a coherent product claim ("your agent cannot silently degrade this codebase, and you can prove what it did") that no competitor in §1 can currently make. P3 and P4 are cheap and independent. P5 is maintenance with a deadline attached. P6 needs a design decision first.
