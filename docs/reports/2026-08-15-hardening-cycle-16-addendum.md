# Cycle 16 addendum — F1 measured, and one of its claims narrowed

**Subject:** validation and extension of `2026-08-15-hardening-cycle-16.md` · **Anchor:** `fbc9c93` (unchanged) · **New evidence:** four measurements run in this session.

Cycle 16 filed F1 with its cost explicitly split into measured and unmeasured, and named `cargo build --timings` as "the ten-minute check that would settle it." This addendum runs that check and three others. The result **strengthens the finding on two axes, narrows it on one, and changes the recommended fix**.

> **Revised after an independent validation pass.** The grammar measurements were
> re-derived from the local crate cache and confirmed; the "only two crates
> contain C++" result — the counterintuitive one — holds exactly. The finding
> itself survives intact. What did not survive is this document's **recommended
> sequencing**, which turned out to be backwards for a structural reason it had
> not traced (§2.1); the C++ source figure (§1.3); the upstream-header quotation
> and the `macros.rs` argument (§2); and both file counts, which were inflated by
> substring matching. One consequence was missed entirely and is added as §1.5,
> and it is the strongest argument in the document.

---

## 1. What was measured

### 1.1 Compile cost — measured, and larger than the working set

Two scratch crates, one depending on the four unreachable grammars and one on the five used grammars, built `--release --offline` on the same 2-core host:

| Set | Grammars | Wall clock |
|---|---|---|
| **Unreachable** | mozcpp, kotlin-ng, ccomment, preproc | **33 s** |
| **Used** | rust, python, java, javascript, typescript | **25 s** |

**The dead grammars cost more to compile than the entire working set — 57% of total grammar build time (33 s of 58 s) is spent on code the product cannot reach.** Cycle 16 asserted the direction; this is the magnitude. On a 2-core runner the effect is a bit over half a minute per clean build, every clean build, in CI and locally.

### 1.2 Binary size — measured at **zero**, which narrows cycle 16's finding

Cycle 16 flagged this as genuinely uncertain: "binary-size saving may be zero if the linker already strips unreferenced sections." It does. Two builds of an identical binary, differing only in whether the grammar is referenced:

| Build | Size |
|---|---|
| `tree-sitter-kotlin-ng` a declared dependency, **never referenced** | **445,880 B** |
| Same crate, grammar **actually referenced** | 3,941,968 B |

A dependency that is present but unreferenced contributes **0 bytes**; referencing it costs 3.33 MB. The linker strips it.

That result only transfers to CodeLore if the product genuinely never reaches the Kotlin/C++ parsers through `codelore-rca`, so I checked the whole chain rather than assuming:

- The product imports exactly six concrete parser types plus `FuncSpace`, `ParserTrait`, `SpaceKind`, `metrics` (`complexity/mod.rs:12-15`).
- `codelore-rca` *does* contain an all-language dispatch — `mk_action!` generates `pub fn action<T: Callback>(lang: &LANG, …)` with a `match` arm constructing every parser, and `get_function_spaces` alongside it.
- **The product calls neither.** `grep -rn "action::<\|get_function_spaces\|LANG::"` across `codelore-lib` and `codelore-cli` returns nothing. `action` is generic, so it is monomorphised only if called; it isn't, so `KotlinParser` and `CppParser` are never instantiated and their static tables are never referenced.

**Conclusion: the binary-size cost is zero, and cycle 16's caution was right to be a caution.** F1 is a *build-time, toolchain and supply-chain* finding, not an artifact-size one. I am recording this as a narrowing of my own finding because the inflated version — "46 MB of dead weight in your binary" — was the tempting one and is false.

### 1.3 The musl blocker — sharper than cycle 16 stated, in the project's favour

Cycle 16 said removing the C++ chain "clears one of two blockers." The measurement makes the claim considerably better than that. Of the nine grammar crates, exactly **two contain any C++ at all**:

| Grammar | C++ files | Reachable? |
|---|---|---|
| `bca-tree-sitter-ccomment` | 1 (`scanner.cc`) | **No** |
| `bca-tree-sitter-preproc` | 1 (`scanner.cc`) | **No** |
| mozcpp, kotlin-ng, rust, python, java, javascript, typescript | 0 — pure C | mixed |

Every *used* grammar is pure C. Even mozcpp and kotlin-ng are pure C. **The entire C++-toolchain requirement of `codelore-rca` comes from two unreachable crates whose C++ content is two `scanner.cc` files totalling 4,839 bytes.** Removing them doesn't just help with musl — it makes `codelore-rca` a **pure-C dependency**, eliminating the `musl-g++` requirement from that half of the build outright. The remaining blocker is bundled DuckDB alone, which has its own mitigation recorded at `roadmap-v1.x-and-beyond.md:119`.

Stated precisely, because the first draft of this section rounded the wrong quantity: the two crates carry **157,352 B (~154 KB)** of C/C++ source between them (preproc 117,151 B, ccomment 40,201 B), of which the *C++* — the part that actually demands `musl-g++` — is **4,839 B** (`scanner.cc`, 2,417 B and 2,422 B). Everything else in them is generated C.

That reframes the cost/benefit sharply: the entire C++ cross-toolchain requirement, and with it the dropped musl release target, rests on **under 5 KB of hand-written external-scanner code** in two crates the product cannot reach.

### 1.4 Is this a pattern? — checked, and no

I swept every declared dependency in `codelore-lib` and `codelore-cli` for zero source references. Exactly one hit — `headless_chrome` — and it is a **false positive**: `optional = true`, gated behind the `browser-tests` feature, and used in `tests/spa_browser_test.rs`, which a `src/`-scoped grep does not see.

So F1 does not generalise; the first-party dependency lists are clean. And the false positive is instructive, because it shows the project's *own convention* for a heavy optional dependency: mark it `optional`, gate it behind a feature, document why. The grammars are the one place that convention isn't applied — which is explicable, since they arrived wholesale as part of a vendored fork rather than being chosen one at a time.

### 1.5 The `preproc` grammar is also what pins `petgraph` — missed by the analysis above

Neither cycle 16 nor §1.1–§1.4 noticed that step 1 has a second, larger payoff. Tracing the dependency rather than the grammar:

- `petgraph = "0.6"` is declared in exactly one manifest in the workspace — `codelore-rca/Cargo.toml:49`.
- Its only consumer is `src/preproc.rs`, the C/C++ preprocessor include-graph analyzer.
- `preproc.rs` is bound to the grammar: it opens with `use crate::languages::language_preproc::*`. It dies with the grammar.
- It is unreachable from first-party code — zero references from `codelore-lib` or `codelore-cli`.
- `codelore-lib` does **not** declare `petgraph`. Its three source mentions are comments and a doc-link recording that `import_graph.rs` and `centrality.rs` *deliberately avoid* it. `Cargo.lock` carries exactly one `petgraph`, `0.6.5`.

So dropping `preproc` removes `petgraph` from the workspace outright. That retires three separate documented constraints:

1. The `dependabot.yml` ignore rule for `petgraph`, and the paragraph of rationale CLAUDE.md carries for it.
2. `roadmap-v1.x-and-beyond.md:122`'s `petgraph 0.6 → 0.8` backlog item, whose stated risk is that `kosaraju_scc`'s implementation-defined SCC ordering "can silently shift macro-resolution" — a hazard that exists solely inside the unreachable analyzer.
3. **`leiden-rs`'s `petgraph` feature, currently switched off for this reason alone.** `codelore-lib/Cargo.toml:59-63`: *"The `petgraph` feature is intentionally left off so leiden-rs's optional petgraph 0.8 dep doesn't conflict with codelore-rca's pinned 0.6."*

The third is the one that matters, because it is a live constraint on first-party code rather than housekeeping: a feature of a shipped analysis dependency is disabled to accommodate an unreachable grammar's unreachable analyzer. That is a stronger argument for step 1 than anything in §1.3, and it was found by following the dependency graph rather than the language list — the same inward-tracing method cycle 16 used, applied one edge further.

---

## 2. What this changes in the recommendation

Cycle 16 recommended excision, following the documented Mozjs precedent. Having seen §1.4, I considered the obvious alternative — **feature-gate the dead grammars** (`optional = true` + `#[cfg(feature)]`), matching the `headless_chrome`/`spa`/`browser-tests` convention and honouring the crate's stated principle of keeping divergence from upstream minimal (`src/lib.rs:1-4`).

**I do not recommend it, and the reason is structural** — though the first draft of this section argued it badly, on two grounds that do not hold.

The language set is declared by a single `mk_langs!` invocation at `src/langs.rs:11` (the macro itself is defined at `src/macros.rs:297`), which expands into the `LANG` enum, the `action` match, the extension table, the emacs-mode table and the code enums simultaneously. **That is the real argument, and it survives:** deleting a language means deleting its line from the invocation list, whereas feature-gating one means `cfg`-ing an entry *inside* a macro argument list — which works only fragilely, and has to keep working across all five expansions at once.

Two supports for that argument have to be withdrawn:

- **The upstream-header quotation was trimmed in a way that changed its scope.** The text at `src/lib.rs:1-4` reads "Don't refactor upstream code **to satisfy newer clippy lints** — keep the divergence from upstream minimal." The excised clause is the scoping one: the header forbids cosmetic, lint-driven churn across the vendored tree. It is a crate-wide note, not a prohibition on `macros.rs`. "Keep the divergence minimal" remains a fair general principle to cite; "the crate header says not to touch this file" was not supportable.
- **"It pushes edits into `macros.rs`" does not discriminate between the options.** `UPSTREAM.md:49` records that the Mozjs excision *itself* edited `src/macros.rs` — it restored the mozcpp special-case for `get_language!(tree_sitter_cpp)` and removed two `implement_metric_trait!` arms. Step 2 below would have to undo that same special-case. Excision touches `macros.rs` too, so the file's upstream-coupling is a cost of *both* paths, not a reason to prefer one.

Excision remains the recommendation, on the narrower and better ground that the precedent exists: `UPSTREAM.md` documents "Option B chosen: fully excise Mozjs" — 5 files removed, 18 modified, plus `Cargo.toml` — which established both that the project will accept this divergence and exactly how to execute it. So: **excision, in two independently shippable steps.**

**The order below is the reverse of this document's first draft.** That draft had it backwards, and the reason is structural rather than a matter of taste — see §2.1.

- **Step A — drop `mozcpp` + `kotlin-ng` (46 MB, 33 s).** The build-time win. Removes language entries, `*Code` trait impls and two generated enum files. **No signature changes to any trait.** Touches ~23 files, all of them mechanically.
- **Step B — drop `ccomment` + `preproc`.** Buys the pure-C dependency (§1.3) and removes `petgraph` from the workspace (§1.5). Touches **19 files in `codelore-rca` plus one in `codelore-lib`**, and — unavoidably — changes the signature of `ParserTrait::new`.

### 2.1 Why the order has to flip

`preproc.rs` is not a leaf. Three facts, each verified against source, chain together:

1. **`preproc.rs::preprocess` takes a `&PreprocParser`** (`src/preproc.rs:189`). `PreprocParser` is generated by the `mk_langs!` entry that names `tree_sitter_preproc`, so removing the grammar removes the parser and breaks the module. `preproc.rs` cannot survive its grammar.
2. **`preproc.rs` exports `PreprocResults`, which is a parameter of the crate's central constructor** — `ParserTrait::new(code, path, pr: Option<Arc<PreprocResults>>)` (`src/traits.rs:54`), mirrored in `parser.rs:74,116` and in the `action` / `get_function_spaces` / `get_ops` signatures that `mk_langs!` generates (`src/macros.rs:142,172,204`). Deleting `preproc.rs` therefore forces that parameter out of the public API of every parser, and out of the product's one call site (`codelore-lib/src/complexity/mod.rs:176`, which already passes `None`).
3. **The parameter exists solely to serve C++** (`src/parser.rs:76-84`):

   ```rust
   if let Some(pr) = pr {
       match T::get_lang() {
           LANG::Cpp => { let macros = get_macros(path, &pr.files); c_macro::replace(code, &macros) }
           _ => None,
       }
   ```

   The only arm that consumes `pr` is `LANG::Cpp` — **step A's target.**

So doing step B first means deleting `get_macros` while `LANG::Cpp` still exists and still wants it: you would have to gut the C++ macro-expansion path of a language you are still shipping. Doing step A first makes that branch dead by construction, after which `preproc.rs` is genuinely vestigial and step B removes it — and the now-unreachable `pr` parameter — with nothing left to degrade.

**This also corrects the first draft's central sequencing claim.** Step B is not "the least invasive part of the whole change"; it is the *most* invasive, because it is the only one that touches `ParserTrait`. Step A is the mechanical one. The payoff ordering and the invasiveness ordering point in opposite directions, which the first draft did not notice because it never traced `PreprocResults` past `preproc.rs`.

**A correction to how the two were compared.** The first draft called the preproc step "the least invasive part of the whole change" on the strength of its source volume (~120 KB against 46 MB) while costing the other step in files touched — two different metrics, and the switch flattered the wrong step. Both counts were also inflated by substring matching: `language_cpp.rs` contains ~40 C++ *node names* beginning `Preproc` (`PreprocInclude`, `PreprocArg`, …) that have nothing to do with the `Preproc` language. Counting the language's actual symbols — `PreprocCode`, `PreprocParser`, `tree_sitter_preproc`, `language_preproc`, `PreprocResults` — the true spread is **18 files for preproc, 14 for ccomment, 19 for their union.**

**Semver.** Both steps remove public API — `LANG` variants, parser types, and in step B a trait-method signature — from a crate published at `0.27.4`. Under Cargo's 0.x rules that is breaking, so the next cut must be a **minor** bump (`0.28.0`), not a patch. The workspace shares one version, so `scripts/cut-release.sh 0.28.0` handles all three crates in one move; the constraint is simply that this work must not land in a release cut as a patch.

### 2.2 Preventing recurrence — the standard tooling does not apply here

The obvious modern answer is an unused-dependency checker in CI: `cargo-machete` (the widely-adopted one) or `cargo-shear`. Checked against this case, **neither would have caught F1, and one would actively mislead.**

- Both tools answer "is this dependency *referenced* by this crate's source?" `bca-tree-sitter-preproc` **is** referenced — from the `mk_langs!` invocation in `langs.rs`. It is not an unused dependency; it is a *referenced dependency reached only by code the product cannot call*. That is a cross-crate reachability question, and neither tool models it.
- `cargo-shear` parses source without expanding macros unless run on nightly with `--expand`. Every grammar in this crate is named **inside** a macro invocation, so the likely result is that it flags all nine — including the five that are load-bearing. A checker that reports the working set as dead is worse than no checker.

The mechanism that actually fits is the project's own convention: a guard test asserting the declared grammar set matches the set reachable from `Tier1Language`'s dispatch, in the style of `workflow_action_pin_test.rs` and `rust_version_pins_test.rs`, and of the live-vs-hardcoded drift detector in `cut-release.sh`. It is worth adding *after* the excision — written now it would simply fail, and a guard whose first act is to be suppressed teaches the wrong reflex. Adding a dependency checker in addition is defensible for the first-party crates (§1.4 shows they are already clean, so it would start green), but it should not be sold as protection against this finding, because it is not.

---

## 3. Corrections and confidence

**Corrected in this addendum:**

- Cycle 16 §1 implied the C++ chain was a large removal. It is two crates and ~154 KB of C/C++ source, of which the C++ proper is 4,839 B; the *bulk* (46 MB) is mozcpp and kotlin-ng, which are pure C and irrelevant to the musl question. The size story and the toolchain story are **different findings that cycle 16 ran together**, and separating them changes the recommended sequencing.
- Cycle 16's "one of two blockers" understated the result: removing the two C++ scanners makes `codelore-rca` pure C, so DuckDB becomes the *only* remaining blocker rather than one of two co-equal ones.
- The recommendation moved from "excise, per the Mozjs precedent" to "excise in two steps, smallest-and-most-valuable first," and the feature-gating alternative is now considered-and-rejected on record rather than unexamined.

**Corrected in the validation pass (this document's own errors):**

- **The C++ figure was the wrong quantity, rounded down.** "Two unreachable crates totalling ~120 KB" matched preproc alone. The pair is ~154 KB of C/C++, and the C++ that actually forces `musl-g++` is 4,839 B. The corrected number is *better* for the finding, which is a sign the original was reached by estimate rather than measurement.
- **The upstream-header quotation was trimmed past the point of fidelity**, dropping the clause that scopes it to clippy-driven refactors, and was then used as a file-specific prohibition it never stated. §2 now quotes it in full and drops the claim.
- **The `macros.rs` objection applied to both options, not one.** The Mozjs excision edited `macros.rs` itself (`UPSTREAM.md:49`); step 2 must edit it again. Withdrawn as a discriminator; the `mk_langs!` argument stands on its own.
- **The two steps were compared on different metrics** — source volume for one, files touched for the other — and both file counts were inflated by substring matching on `Preproc` against C++ node names in `language_cpp.rs`. Corrected spread: 18 / 14 / 19.
- **The recommended sequencing was backwards, and not marginally.** `PreprocResults` is a parameter of `ParserTrait::new`, and the only code that consumes it is the `LANG::Cpp` arm in `parser.rs`. Dropping preproc before dropping C++ means gutting the macro-expansion path of a language still being shipped. The preproc step is the *most* invasive of the two, not the least — the one place the change reaches the crate's central trait. §2.1 records the corrected order and the evidence for it.
- **§1.5 was missed entirely.** The `preproc` → `petgraph` → `leiden-rs` chain is the strongest single argument for the preproc step, and four measurement passes over the *grammars* never found it because none of them followed the *dependency*.
- **Semver was never considered.** Both steps remove public API from a published crate; the next cut must be a minor bump.

**Confidence:**

- **Measured here:** compile times, binary sizes, C++ file counts, source volumes, dependency sweep. All reproducible from the commands in §1.
- **Inferred:** that the 33 s scales linearly into the workspace build. It probably does not — the workspace builds these grammars in parallel with much else on a >2-core runner, so the *marginal* wall-clock cost in CI is likely smaller than 33 s. The honest claim is "33 s of CPU work that is never used," not "33 s off every CI run."
- **Now measured, having previously been an estimate:** the excision surface. The Mozjs precedent (5 files removed, 18 modified, plus `Cargo.toml`) was the original stand-in; the validation pass counted the actual spread for *this* job — 20 files for step 1, 23 for step 2, 28 for both. What is still unmeasured is the *difficulty* per file, which the precedent suggests is mechanical (trait-impl and match-arm removal) but does not prove.
- **Unchanged from cycle 16:** nothing was compiled *from this workspace* — the grammar builds were standalone scratch crates on `cargo 1.95.0` against the workspace's pinned grammar versions, which establishes the grammars' cost, not this workspace's total build time.

---

## 4. Net effect on F1

The finding survives, better evidenced and more precisely bounded:

| Claim | Cycle 16 | After measurement |
|---|---|---|
| Unreachable generated source | 46.2 MB | 46.2 MB ✅ confirmed |
| Compile cost | "certain in direction, unquantified" | **33 s vs 25 s for the entire working set** ✅ quantified |
| Binary size | "may be zero" | **zero** ✅ resolved — claim narrowed |
| musl | "clears one of two blockers" | **makes `codelore-rca` pure-C; DuckDB becomes the only blocker** ✅ strengthened |
| C++ actually involved | "~120 KB" | **4,839 B across two `scanner.cc` files** ✅ corrected — the real figure is smaller and better |
| Generalises to other deps? | not asked | **No** — first-party dep lists are clean |
| Knock-on dependencies | not asked | **`petgraph` leaves the workspace; `leiden-rs`'s `petgraph` feature is unblocked** ✅ new (§1.5) |
| Excision surface | "24-file precedent", estimated | **18 preproc / 14 ccomment / 19 union** ✅ measured, substring inflation removed |
| Sequencing | preproc first ("least invasive") | **C++/Kotlin first** ✅ inverted — preproc is the only step touching `ParserTrait` |
| Semver | not considered | **breaking; next cut must be a minor bump** ✅ new |
| Fix | excise per Mozjs precedent | **two steps, order flipped; feature-gating rejected on `mk_langs!` grounds** |

Severity stays **Medium**. Nothing computes a wrong number and no exit code moves; what changed is that the cost is now a measurement rather than an argument, and the cheapest step has the clearest payoff.

The finding's centre of gravity has moved twice under measurement — from artifact size to build time (§1.2), and now from build time to *dependency constraints* (§1.5). Each move was found by asking what the previous version had assumed rather than what it had claimed. If there is a third, it is most likely in the same direction: what else in the workspace is shaped around a grammar the product cannot reach?
