# Hardening cycle 16 — the largest unused artifact in the tree, and a design for the cycle-15 headline

**Anchor:** `fbc9c93` (v0.27.4+) · **Baseline:** `fbc9c93` (cycle-14 anchor) · **Delta: zero commits.**

> **Revised after an independent validation pass.** Every measurement in §1 was
> re-derived from the local crate cache and confirmed to within rounding; every
> file:line anchor in §1–§2 verified exact. One count was wrong (grammar
> dependencies) and is corrected below. See the addendum for the measured
> follow-up and one further correction to §1's framing.

---

## 0. The codebase has not changed

`git rev-list --count fbc9c93..main` → **0**. `main`, `origin/main` and HEAD are all `fbc9c93`, dated 2026-08-14 10:55, and the tree hash is byte-identical to the subject of cycle 14. Nothing has been committed since.

So the brief's first instruction — *make sure you are analyzing the current updated codebase with latest changes* — is satisfied trivially, and re-running the delta audit would produce nothing. I am saying that plainly rather than manufacturing a cycle from an unchanged tree, because reporting "no new findings" after a fresh look at a moved codebase and reporting it after a look at a stationary one are different claims, and conflating them is how audit reports become theatre.

The budget therefore went to two things that do not require the code to have moved: **a surface no prior cycle examined** (the vendored parser crate, §1), and **making cycle 15's top proposal implementable** (§2). Three report branches remain unmerged — `docs/hardening-cycle-12`, `docs/hardening-cycle-14`, `docs/cycle-15-product` — which is the only housekeeping item.

---

## 1. Finding

### F1 — MEDIUM (new) — 46 MB of generated C/C++ is compiled into every build for two languages the product cannot analyse, and one of them is blocking a release target

`codelore-rca` declares **nine** tree-sitter grammar dependencies plus the `tree-sitter` core crate, **all mandatory** — the only non-default `[features]` entry is `metrics-experimental`, so nothing is feature-gated (`crates/codelore-rca/Cargo.toml:23-40`).

The product dispatches exactly six parsers (`complexity/mod.rs:202-207`): `RustParser`, `PythonParser`, `JavaParser`, `JavascriptParser`, `TypescriptParser`, `TsxParser`. **`KotlinParser` and `CppParser` are never referenced** from `codelore-lib` or `codelore-cli` — the import list at `complexity/mod.rs:12-15` names six and the file-extension map (`complexity/language.rs:28-34`) resolves only `rs, py, pyi, java, js, jsx, mjs, cjs, ts, tsx`. Same five-language set in `imports/` and `clones/`.

I measured the generated parser source in each grammar crate (downloaded from crates.io, uncompressed):

| Grammar | Generated C/C++ | Reachable from the product? |
|---|---|---|
| `bca-tree-sitter-mozcpp` | **24.7 MB** | **No** |
| `tree-sitter-kotlin-ng` | **21.4 MB** | **No** |
| `bca-tree-sitter-preproc` | 0.1 MB | **No** (C++ chain) |
| `bca-tree-sitter-ccomment` | 0.02 MB | **No** (C++ chain) |
| `tree-sitter-typescript` | 16.7 MB | Yes |
| `tree-sitter-rust` | 5.9 MB | Yes |
| `tree-sitter-python` | 3.3 MB | Yes |
| `tree-sitter-java` | 2.4 MB | Yes |
| `tree-sitter-javascript` | 2.4 MB | Yes |

**46.2 MB unreachable, against 30.7 MB actually used.** The dead grammars are *one and a half times larger than the entire working set*, and tree-sitter's generated `parser.c` files — giant switch tables — are close to a worst case for a C compiler.

Three things make this worth acting on rather than noting:

1. **It is already costing a shipped capability.** `docs/roadmap-v1.x-and-beyond.md:121` records that the `x86_64-unknown-linux-musl` release target was *dropped* because "`bca-tree-sitter-preproc`'s `scanner.cc` and bundled DuckDB's .cpp files need a C++ cross-toolchain." `preproc` exists solely to serve the C++ grammar the product cannot use. **Honest qualification:** removing it clears *one of two* blockers — bundled DuckDB is the other, and it has its own documented mitigation two lines earlier (`:119`: dynamic linking or a cached pre-built artifact). So this does not restore musl on its own; it halves the problem. That matters because a static musl binary is what makes Alpine and distroless-static deployment easy, which is the exact air-gapped, self-hosted buyer cycle 15 identified as the differentiator.
2. **It is supply-chain surface the project otherwise refuses.** Three of the four dead grammars are third-party forks — `bca-tree-sitter-mozcpp`, `bca-tree-sitter-ccomment`, `bca-tree-sitter-preproc` — outside the official tree-sitter namespace, plus the community `tree-sitter-kotlin-ng`. This is a repository that SHA-pins every third-party GitHub Action, enforces it with `workflow_action_pin_test`, and configured zizmor to agree with that policy. Compiling ~46 MB of unreachable C++ from four forks sits oddly beside that standard.
3. **The excision method is already written down and already executed once.** `UPSTREAM.md` documents "Option B chosen: fully excise Mozjs" — 5 files removed, 18 modified, plus `Cargo.toml`; the modified set is `langs.rs`, `macros.rs`, `checker.rs`, `getter.rs`, `alterator.rs`, `metrics/*.rs`. (The original "24 enumerated files" here totalled those three groups without saying so; the components are given explicitly now, and the addendum measures the spread for *this* job directly.) Kotlin appears in 13 files and Cpp in 22 (heavily overlapping — the same core traits), plus four dedicated generated files (`language_cpp.rs` 52 KB, `language_kotlin.rs` 22 KB, `language_preproc.rs`, `language_ccomment.rs`). The shape is the same as a job this project has done before, with its own runbook. (The Ccomment/Preproc figures originally given here — 14 and 19 — were counted by substring and inflated: `language_cpp.rs` alone contains ~40 C++ *node names* beginning `Preproc`. Counted by the languages' actual symbols the spread is 14 and 18, union 19; see the addendum §2.1, which also finds the recommended sequencing of the two removals to be backwards.)

**What I did not measure, stated as such:** I cannot compile here, so the build-time and binary-size savings are unquantified. Compile cost is certain — `cc` compiles every non-optional dependency regardless of reachability. Binary-size saving is *uncertain*: with `--gc-sections` a linker may already strip the unreferenced parse tables, so the win may be entirely in build time and supply-chain surface rather than artifact size. Measuring `cargo build --timings` before and after is a ten-minute check and should decide the priority.

**Counter-argument, considered and rejected:** that the grammars are deliberate investment in future C++/Kotlin support. The evidence runs the other way. `docs/maximum-feature-plan.md:403` asked whether to do resolvers for "all six" Tier-1 languages including C++, and what shipped is five languages with no C++. No roadmap item commits to Kotlin or C++. And the Mozjs precedent shows the project's answer to an unused vendored language is excision, not retention.

**Severity Medium**, by this engagement's own rule: nothing produces a wrong number and no exit code moves, so it is not High. But it is the largest instance in the tree of the brief's standing "clean implementation without unused or legacy code" clause, it carries real supply-chain and build cost, and it has a documented downstream consequence.

---

## 2. Making the cycle-15 headline implementable: `ai-attribution`

Cycle 15's P1 established that AI-authorship attribution is ~80% built and absent from MCP. That was a strategic claim; here is the design work to make it actionable, validated against the existing code so it fits the architecture rather than fighting it.

**What exists** (re-verified): `commits.ai_attribution TEXT` (`facts/schema_v1.sql:27`), populated at ingest via `identity::ai_attribution_with` through user-extensible `.codelorebots` patterns (`facts/ingest/consumer.rs:111-117`), classifying `ai-authored` / `ai-assisted` / `human`. Already consumed by `hotspots.rs:244` (`ai_pct`), `knowledge/shares.rs:115,213`, `authors.rs:89`.

**The gap:** no analysis asks the question; `grep ai_attribution crates/codelore-cli/src/mcp.rs` → **0**.

**Design, fitting existing patterns:**

- **New analysis `ai-attribution`** as a module under `analyses/`, registered in the `AnalysisName` enum. Because the dispatch is an exhaustive match with no `_` arm — enforced by `registration_surfaces`, `dispatch_surface_reaches_every_analysis` and `spa_surface_accounts_for_every_analysis` — the compiler and the guards will force every surface to be updated, which is the property that makes this cheap to do correctly. `supported_formats` should start at `csv|json|markdown` (the `STREAM` default); HTML only if a widget is built.
- **Rows:** per-file `ai_pct` already exists; the analysis's value is the *crossing*. Per-file: attribution share × code-health band × hotspot rank × coupling degree × knowledge-island status. Repo-level: AI share of red-band files vs overall AI share (the headline ratio), AI share of hotspot top-N, and — using the existing `calibrate-defects` AG-SZZ fix-link mining — AI-vs-human defect density, which is the number nobody else can produce.
- **Trend:** reuse the sampled-revision machinery `health-trend` and `architecture-trend` already use; no new storage, consistent with the "no historical metric store" architecture.
- **MCP tool** `ai_attribution(limit)` returning `Json<T>` — following `check_gates`, the one tool that already does structured output, so this also advances cycle-15 P5 by one tool rather than fighting it.
- **Gate:** deliberately *not* in the first cut. A `max_ai_hotspot_share` gate before the distribution is understood would be a threshold with no calibration behind it, which is precisely what `calibration.rs`'s `MIN_LANG_SAMPLE` and the corpus-percentile work exist to avoid. Ship the measurement, look at real repositories, then decide.

**Two correctness cautions**, from reading the existing consumers:

1. `hotspots.rs:134-156` documents that `ai_pct` joins raw `changes`, never `changes_bucketed`, because bucketed revs "never match, leaving `ai_pct` NULL under `--time-bucket`." Any new AI analysis inherits that constraint and must either follow the same rule or reject `--time-bucket` explicitly — silently emitting NULLs would be the exact "confident empty answer" failure the zero-row notice work (#223) was built to prevent.
2. Attribution is a **lower bound and must be labelled as one**. It detects trailers and bot identities; a developer who accepts an AI completion without a trailer is invisible, and `Co-authored-by` conventions vary by tool and by team policy. The honest framing is "AI-attributed" not "AI-written", and the docs should say what the detector can and cannot see — the same discipline `calibrate-defects` already applies when it records why weights stayed at defaults.

---

## 3. Residuals

Unchanged from cycle 14, since the code is unchanged: the **gitlink differential fixture** (still the only open item with no decision recorded against it, carried since cycle 6); `outputSchema` at 1 of 11 MCP tools; **M8** cancellation (a design question per E9, not wiring); zizmor not yet a required context in `protect-main`. From cycle 13, still awaiting a decision: whether the tested `cargo publish --no-verify` split is worth adopting to get Trusted Publishing without trading away Build L3. From cycle 15: P1–P6, of which P1 now has a design above.

**Currency:** rmcp `3.1.2`, zizmor `1.29.0` — both current as of the last live check two days ago; the Rust pin `1.96.0` remains deferred to the next cut by documented convention. No re-verification this cycle would be meaningful on a stationary tree, and I have not re-run it, which is why no currency claim here is stated as fresh.

---

## 4. Honesty ledger

- **A zero-delta cycle is reported as one.** The alternative — re-deriving the same conclusions on the same bytes and presenting them as a fresh pass — would be the most straightforward way to make these reports worthless, and it is a real temptation when the instruction says "the codebase is updated" and it is not.
- **F1 comes with its cost split into measured and unmeasured.** 46.2 MB of unreachable generated source is measured. The compile-time saving is certain in direction and unquantified in size; the binary-size saving may be zero if the linker already strips it. I have said so rather than implying a bigger win, and named the ten-minute measurement that would settle it.
- **The musl claim is deliberately halved.** Removing the C++ chain clears one of two documented blockers, not both. Stating it as "this restores musl" would have been a better headline and false.
- **§2 is design, not a finding.** It is included because cycle 15's proposal was strategic and the natural objection is "how would that actually fit" — but it is my proposal, not a defect, and the two cautions in it are the parts most worth arguing with.
- **Limits.** Nothing compiled. Grammar sizes were measured from crates.io tarballs, not from a build of this workspace. Reachability is established by reading the dispatch and extension maps; a `cargo tree`/`cargo bloat` run would confirm it independently and is the natural next check.

---

## 5. Method

Confirmed the zero delta by tree hash before doing anything else. Then chose a surface no prior cycle had touched — the vendored parser crate — on the reasoning that fifteen cycles of auditing the first-party code make the vendored dependency the least-examined significant code in the repository. The finding was built by tracing reachability from the product's dispatch inward rather than from the dependency list outward, then quantified by downloading and measuring the actual grammar tarballs, then tested against the fair counter-argument (deliberate future support) using the project's own roadmap and vendoring precedent. §2 was written against the existing registration and format-dispatch machinery so the proposal fits the architecture's exhaustive-match discipline rather than requiring an exception to it.
