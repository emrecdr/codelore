# Hardening cycle 17 — validating my own finding as shipped, and a misquote I have to own

**Anchor:** `9811bd8` (v0.28.0) · **Baseline:** `fbc9c93` (cycle-16 anchor, v0.27.4) · **Delta:** 7 commits (#277–#282) plus the v0.28.0 cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `9811bd8`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled from this workspace.

This cycle audits an implementation of my own finding, so the standard has to be higher than usual, not lower. Cycle 16's F1 shipped as **#278, a breaking change**, and three of my reports landed with independent validation corrections attached. Both directions are adjudicated below, and the corrections land harder than the finding did.

> **Revised after an independent validation pass.** Every structural claim was
> re-derived from source, including at the pre-change commit: the six `LANG`
> variants, the zero-residual identifier sweep, the trait signature, the
> `21`/`7` counts in §2 (both correct — the first collapses numbered node
> variants, the second counts module dependents), and the verbatim CHANGELOG
> quotation. Two things did not survive and are corrected in place: the
> `aho-corasick` lockfile claim (§1.2) and the C++-dependency enumeration
> behind the new finding (§3, conclusion unaffected). A third correction was
> made to the `gh-pages` figure in §6 and has since been **withdrawn** — the
> original figure was right and the correction was not; see the retraction at
> §6 and the evidence in cycle 18 §2.3.

---

## 1. The excision, validated — and it went past my finding in four places

**Complete and clean.** `codelore-rca` now declares five grammar crates plus the tree-sitter core; `LANG` has exactly six variants (Java, Javascript, Python, Rust, Tsx, Typescript), matching the six parsers the product dispatches. A word-boundary sweep for every removed identifier — `KotlinParser`, `CppParser`, `CcommentParser`, `PreprocParser`, `PreprocResults`, and the four `tree_sitter_*` crate names — returns **zero** hits across `.rs`, `.toml` and `.yml`. `preproc.rs` and all four `language_*.rs` files are gone.

*(My first sweep here reported eleven residual hits. All were `DocCommentMarker` matching the substring `ccomment` — the exact error #277 had just corrected me for. I caught it before it reached this page; recording it because the lesson evidently needed a second application.)*

**Four things the implementation found that my finding did not:**

1. **The four could not be separated** — which makes my cycle-16 addendum's central recommendation wrong. I proposed two independently shippable steps, `ccomment`+`preproc` first as "smallest change, largest structural payoff." Verified against the pre-change source: `ParserTrait::new(code: Vec<u8>, path: &Path, pr: Option<Arc<PreprocResults>>)` — `PreprocResults` was **a parameter of the crate's central trait**, consumed by exactly one arm (`LANG::Cpp`) in `get_fake_code`. Dropping preproc first would have gutted macro expansion for a language still shipped. Preproc was the *most* invasive of the four, not the least. My sequencing was exactly backwards.
2. **Two more dead dependencies**: `petgraph` (whose only workspace consumer was `preproc.rs`) and `aho-corasick` (whose only use was a Mozilla bindgen marker in the C++ checker). My cycle-16 §1.4 sweep missed them because — as I wrote at the time — I swept "`codelore-lib` and `codelore-cli`." I did not sweep `codelore-rca`, the one crate the entire finding was about.

   *(Corrected after validation: only **`petgraph`** actually left the dependency graph — zero entries in `Cargo.lock`. **`aho-corasick` is still there**, one entry, pulled transitively by `globset`, `regex` and `regex-automata`. Dropping it from `codelore-rca`'s manifest removed a direct dependency that was no longer used; it did not remove the crate from the build, and its compile cost is unchanged. I wrote "both now absent from `Cargo.lock`" without opening the lockfile — the same defect class this report spends §2 adjudicating, committed in the paragraph claiming credit for finding dead dependencies.)*
3. **Downstream consumers I never traced**: the Dependabot ignore rule for petgraph is retired, the deferred `0.6 → 0.8` bump is closed, and `leiden-rs`'s `petgraph` feature is no longer *constrained* — handled correctly by leaving the feature off but rewriting the comment to say it is now "a plain 'we don't use it' rather than a constraint." Enabling an unused feature would have added a dependency for nothing; reclassifying the reason is the right call.
4. **`ParserTrait::new` simplified further** to `fn new(code: Vec<u8>)`, dropping the `path` argument that only `get_fake_code` needed. `metrics_with_guard` still takes `path` — for the diagnostic log line, where it is genuinely used. Dropped exactly where unused, kept exactly where used.

**Consumer coverage is complete.** The CHANGELOG documents the breaking API change explicitly ("Removing `LANG` variants, the parser types, and the `ParserTrait::new` signature is a breaking change to `codelore-rca`'s public API"), the workspace version moved `0.27.4 → 0.28.0` which is the correct semver-breaking bump for a published `0.x` crate, and the musl roadmap row was **rewritten** rather than left stale: it now records that one of two blockers is gone and DuckDB is the only one left.

One line in that CHANGELOG entry is worth quoting because it is a better observation than anything in my finding: *"Unused-dependency tooling would not have found any of this: the grammars were referenced, from inside the `mk_langs!` invocation, and merely unreachable."* `cargo-udeps` looks for *unreferenced* crates; these were referenced and unreachable, which only a reachability analysis from the product's entry points finds.

**Payoff realised by construction:** the surviving grammar set is byte-identical to the set I benchmarked at 25 s in the cycle-16 addendum, so the measured 33 s of unreachable compile work is removed exactly as predicted.

---

## 2. Adjudicating the corrections to my reports

Each checkable claim in #277 and #282, verified against source before acceptance.

| Correction | Verdict |
|---|---|
| Sequencing backwards; `PreprocResults` is a `ParserTrait::new` parameter | ✅ **Confirmed** — `traits.rs` pre-change: `fn new(code, path, pr: Option<Arc<PreprocResults>>)` |
| My file counts inflated by substring matching | ✅ **Confirmed** — `language_cpp.rs` alone carries 21 distinct `Preproc*` node-type names; my method matched 19 files, only 7 reference the module |
| The C++ figure named the wrong quantity | ✅ **Confirmed** — my "~120 KB" was the C+C++ total from `du`; the C++ actually forcing `musl-g++` is two `scanner.cc` files at 4,839 B |
| **The upstream-header quote was trimmed past its scoping clause** | ✅ **Confirmed — and this is the serious one** (below) |
| P3 asserted a property of eleven MCP tools while listing nine | ✅ Confirmed — `repo_overview` and `gate_changes` were missing from my list |
| Depth claim compared 57 analyses to a tool count | ✅ Confirmed — like for like the agent surfaces are 11 vs 10 |
| repowise/CodeScene figures stale or wrong | ✅ Accepted — vendor pages are the better source than my research pass |

**On the misquote.** The header reads: *"Don't refactor upstream code **to satisfy newer clippy lints** — keep the divergence from upstream minimal."* I quoted it as *"Don't refactor upstream code… keep the divergence from upstream minimal"*, and the ellipsis removed the scoping clause. That converted a narrow instruction about clippy-driven churn into a general prohibition on editing the file — **and I then used that manufactured prohibition as the reason to reject the feature-gating alternative.** The objection was also self-defeating on its own terms: the Mozjs excision I cited as precedent edited `macros.rs` itself, so the argument applied equally to the option I recommended.

That is the most serious error in seventeen cycles of these reports. Every other correction has been a wrong number or a mis-attributed mechanism; this one used a truncated quotation to close off a design option. The standing rule I am adding: **when a quotation is load-bearing for a recommendation, quote it whole, and check whether the objection it supports also applies to the option being recommended.**

I note that #282 left the P1–P6 ranking deliberately unchanged, on the grounds that it is "a product judgement for the maintainer, and this pass corrected facts rather than making that call." That separation of fact-correction from judgement is the right discipline, and P1 — the AI-attribution proposal — went unchallenged.

---

## 3. New finding

### F — LOW (new) — Two roadmap rows are now one problem, and only one of the three listed options closes both

`docs/roadmap-v1.x-and-beyond.md` carries these as separate rows:

- `:119` — **Bundled-DuckDB compile dominator**: `libduckdb-sys` `bundled` compiles ~6000 `.cpp` files every run (~5–7 min). Options offered: better sccache hit rate, switch to `dynamic` + ship pre-built DuckDB, or a build-once-and-cache job.
- `:121` — **Re-add the musl release target**: blocked, now solely, by DuckDB's `.cpp` needing a C++ cross-toolchain.

Before #278 these were genuinely separate — the grammars contributed C++ to the musl problem but nothing to the compile dominator. **The excision merged them.** `libduckdb-sys` is now the only C++-compiling dependency that builds on any target this project ships (`ring` compiles C; the remaining `*-sys` crates are bindings). One dependency, one `bundled` feature, both rows.

*(Precision added after validation: one other lockfile crate does ship a C++ source — `iana-time-zone-haiku`'s `implementation.cc` — but it sits behind `[target.'cfg(target_os = "haiku")'.dependencies]` and therefore compiles on no target this project builds for, musl included. The enumeration above was incomplete; the conclusion it supports is unaffected.)*

That matters because the three options at `:119` are not equivalent once the rows are joined:

- **sccache tuning** — helps the compile dominator, does nothing for musl.
- **`dynamic` + pre-built DuckDB** — helps the compile dominator, and **does not deliver musl in the form that motivates it**: the point of `x86_64-unknown-linux-musl` here is a *static* binary for Alpine, distroless-static and air-gapped installs, which is precisely what dynamic linking gives up.
- **Build DuckDB once and cache the artifact** — the only option that closes both, *provided* the cached artifact is built with a musl C++ toolchain, at which point it is simultaneously the compile-dominator fix and the musl unblock.

**Recommendation:** merge the two rows, and record that the build-once-and-cache option is now load-bearing for two outcomes rather than one of three interchangeable ways to speed up a build. This is an alignment finding, not a defect — nothing is broken, but two roadmap entries now describe one decision, and the option ranking implied by `:119` is misleading once they are read together.

Severity **Low**: it changes planning, not behaviour.

---

## 4. Residuals

**Unchanged and open:** the gitlink differential fixture (still the only item with no decision recorded against it, carried since cycle 6); `outputSchema` at 1 of 11 MCP tools; M8 cancellation (a design question per E9); zizmor not yet a required context in `protect-main`. From cycle 13, still undecided: the tested `cargo publish --no-verify` split that would make Trusted Publishing compatible with Build L3. From cycle 15: P1–P6, with P1 (AI attribution) carrying the design sketched in cycle 16 §2 and surviving #282's fact-correction pass.

**Currency:** not re-verified this cycle. The two dependabot bumps in this delta (`thiserror` 2.0.19→2.0.20, `taiki-e/install-action`) indicate the automation is working; last live check of rmcp/zizmor was cycle 14 and both were current.

---

## 5. Honesty ledger

- **My cycle-16 addendum's central recommendation was wrong.** "Two independently shippable steps, smallest first" was not merely suboptimal — the steps could not be separated, and the one I called least invasive reached the crate's central trait. I had the pre-change `traits.rs` available and did not read the signature before recommending the sequencing.
- **My dependency sweep covered two of the three workspace crates**, omitting the one the finding was about. I stated the scope accurately in cycle 16 ("`codelore-lib` and `codelore-cli`"), which is how the gap is visible now — but stating a scope accurately is not the same as choosing the right one, and `petgraph` and `aho-corasick` were sitting in the crate I skipped.
- **The misquote (§2) is the worst error in this engagement's history**, and it is worse than a wrong number because it was load-bearing for a design recommendation.
- **I nearly repeated the substring error inside this report** (§1), one cycle after being corrected for it. Caught, and recorded.
- **And I did repeat the underlying error, in the paragraph claiming credit for catching dead dependencies.** §1.2 asserted `petgraph` and `aho-corasick` were "both now absent from `Cargo.lock`" without opening the lockfile. Only `petgraph` left; `aho-corasick` is still pulled by `globset`, `regex` and `regex-automata`, so its compile cost never went away. The validation pass caught it. Two sections later I was adjudicating exactly this defect class in my own prior reports — a claim inherited from a plausible mental model rather than re-derived from the artifact it names. Proximity to the lesson is evidently no protection against it.
- **Limits.** Nothing compiled from this workspace. The excision's correctness is established structurally — by identifier sweeps, the `LANG` variant list, the trait signature, and the call-site dispatch — not by running the test suite. The claim that no *supported* language's behaviour changed rests on the same structural argument the commit message makes (no impl for a supported `*Code` type touched, `get_fake_code` returned `None` for every language except C++), which I verified is consistent with the surviving source but did not execute.

---

## 6. Housekeeping

- **All prior report branches have landed.** `docs/hardening-cycle-12`, `-14`, `-16` and `cycle-15-product` are merged and deleted; the branch list is `main` + `gh-pages` (13 behind locally, which is the publishing job).

  > **The correction that previously sat here was wrong, and is withdrawn.** It
  > claimed "13 behind" was meaningless because `gh-pages` is an orphan branch
  > with no common ancestor with `main`. The orphan fact is true and irrelevant:
  > `[behind N]` never compares to `main`, it compares a branch to its own
  > upstream. `gh-pages` tracks `origin/gh-pages`, `git rev-list --count
  > gh-pages..origin/gh-pages` is 13, and `git branch -vv` prints
  > `[origin/gh-pages: behind 13]` verbatim. The original figure was correct and
  > standard; the correction measured `gh-pages..main` (807) — a comparison this
  > report never made — and generalised from it. Cycle 18 §2.3 rejected it with
  > this evidence and the rejection is upheld.
- `_to_delete/` carries this cycle's artifacts. `HANDOFF.md` remains yours.
- **This report** is committed to branch `docs/hardening-cycle-17`, based on `main` (`9811bd8`).

---

## 7. Method

The subject was an implementation of my own finding, so the audit ran in both directions: the shipped change was verified against the finding it closes (identifier sweeps with word boundaries, `LANG` variant enumeration, trait signature, call-site dispatch, lockfile absence, version bump, CHANGELOG breaking-change disclosure, roadmap consumer update), and the corrections attached to my reports were each re-derived from the pre-change source before being accepted rather than taken on the commit message's word. The new finding came from noticing that a change I had recommended altered the relationship between two roadmap items neither of which the change touched.
