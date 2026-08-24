# Hardening cycle 18 — completing the sweep I ran twice badly, and one correction I don't accept

**Anchor:** `19a00ec` · **Baseline:** `9811bd8` (cycle-17 anchor, v0.28.0) · **Delta:** 2 commits (#283, #284), no release cut.

Audited from `git archive main` read-only; `main` = `origin/main` = `19a00ec`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled.

Both commits in this delta are consequences of cycle 17: my roadmap finding implemented (#284), and three more corrections to that report (#283). This cycle validates the first, adjudicates the second — **accepting two corrections and rejecting one, with evidence** — and then finally runs the dependency sweep properly, which is where the new finding comes from.

> **Revised after an independent validation pass.** The rejection in §2.3 was
> upheld: `git branch -vv` prints `[origin/gh-pages: behind 13]`, the local ref
> tracks `origin/gh-pages`, and the count was 13 when written — so the retracted
> correction has itself been retracted, in this PR. §3 did **not** survive
> intact: `num-traits` is required via derive expansion, proven by a build that
> fails without it, so the finding is two removable dependencies rather than
> three and its "no build change" bound was wrong. Corrections are inline.

---

## 1. #284 — the roadmap merge, validated

My cycle-17 finding F said the DuckDB compile-dominator row and the musl row had become one problem, and that the three options listed under the first were no longer interchangeable. #284 implements exactly that: one row, titled "Bundled DuckDB: the compile dominator **and** the musl blocker (one decision)", with the three options ranked rather than listed — sccache tuning helps build time and does nothing for musl; `dynamic` + pre-built DuckDB helps build time but "forfeits the reason musl is wanted"; build-once-and-cache is the only option closing both, conditional on a musl C++ toolchain. It adds the toolchain routes.

One detail worth crediting: the row says `libduckdb-sys` is "the only C++-compiling dependency **that builds on any target we ship**." That phrasing incorporates the correction #283 made to my §3 — I had said "the only C++-compiling dependency" flat, and `iana-time-zone-haiku` also ships a scanner, behind `cfg(target_os = "haiku")`. The implementation absorbed the fix into the artifact rather than leaving it in a report nobody reads later.

---

## 2. Adjudicating #283's three corrections

### 2.1 aho-corasick — **accepted**

Cycle 17 §1.2 said petgraph and aho-corasick were "both now absent from `Cargo.lock`." Verified: `grep -c 'name = "aho-corasick"' Cargo.lock` → **1**. It is still there, pulled by `globset`, `regex` and `regex-automata`. Removing it from `codelore-rca`'s manifest deleted an unused *direct* dependency; it did not remove the crate from the build, and its compile cost is unchanged.

#283's framing of where I made this error is fair and worth quoting: *"Written without opening the lockfile — and written in the paragraph claiming credit for finding dead dependencies, two sections before adjudicating this exact defect class in earlier reports."* Correct on all three counts.

### 2.2 iana-time-zone-haiku — **accepted**

`grep -c 'name = "iana-time-zone-haiku"' Cargo.lock` → **1**. My §3 enumeration was incomplete. As #283 says, the finding it supports is unaffected, because the crate compiles on no target this project ships — which is why the merged roadmap row's wording is the right repair.

### 2.3 gh-pages "13 behind" — **not accepted, and here is the evidence**

#283 says: *"Section 6 described gh-pages as '13 behind'. It is an orphan branch with no common ancestor with main, so 'behind' does not measure anything there."*

The orphan fact is correct and I did not know it — `git merge-base main gh-pages` returns nothing, confirming no common ancestor. But the conclusion does not follow, because `git branch -v`'s `[behind N]` never compares to `main`. It compares a branch to **its own upstream**:

```
gh-pages upstream:                        origin/gh-pages
git rev-list --count gh-pages..origin/gh-pages   →  13
```

So "13 behind" measures something real and precisely stated: the local `gh-pages` ref is 13 commits behind the remote it tracks. That is exactly what cycle 17 §6 said — *"13 behind locally, which is the publishing job"* — a phrasing that already encodes the cycle-9 E1 lesson about which direction that arrow points. The correction reads the figure as a main-vs-gh-pages comparison I did not make.

I am recording this as rejected rather than quietly accepting it, because a review process where corrections are deferred to rather than verified is the same failure as one where findings are accepted without verification — and this engagement has spent eighteen cycles arguing the opposite. Both prior gh-pages claims (cycle 9's "stale", corrected; this one) came from the same figure being genuinely easy to misread, so the underlying observation — *this number invites misreading* — stands even though this instance of it does not.

---

## 3. New finding

### F — LOW (new) — `codelore-rca` declares two more dependencies it does not use, and a third that only looks unused

Cycle 16 swept `codelore-lib` and `codelore-cli` with a crude method and skipped `codelore-rca` — the crate the whole finding concerned — which is how `petgraph` and `aho-corasick` were missed. This cycle ran the sweep it should have run: **all three crates**, all dependency tables (`dependencies`, `dev-dependencies`, `build-dependencies`), all source roots (`src/`, `tests/`, `benches/`, `examples/`, `build.rs`), with `-`→`_` normalisation.

Result:

| Crate | Declared | Unreferenced in source | Actually removable |
|---|---|---|---|
| `codelore-rca` | 18 + 2 dev | `serde_json`, `num-traits`, `rayon` | **`serde_json`, `rayon`** — `num-traits` is required via derive expansion (see below) |
| `codelore-lib` | 32 + 1 dev + 3 build | none | — |
| `codelore-cli` | 14 + 5 dev | none | — |

The two columns differ, and that gap is the finding's real lesson: "absent from the source text" and "safe to remove" are not the same predicate, and only the second one is checkable by building.

All three have **zero** occurrences anywhere in `codelore-rca/src/` — not in code, not in comments, and the crate contains no aliased imports (`use … as …`) that could hide them.

> **Corrected after validation: `num-traits` is required, and the text-search method above is exactly why I missed it.**
>
> Removing it does not compile:
>
> ```
> error[E0463]: can't find crate for `num_traits`
>  --> crates/codelore-rca/src/languages/language_java.rs:5:39
>   |
> 5 | #[derive(Clone, Debug, PartialEq, Eq, FromPrimitive)]
>   |                                       ^^^^^^^^^^^^^ can't find crate
> ```
>
> `codelore-rca` derives `FromPrimitive` in six generated `language_*.rs`
> files, and `num-derive`'s documentation is explicit that its macros "assume
> that the `num_traits` crate is a **direct dependency**" unless the
> `#[num_traits = "…"]` helper attribute names another path — which this crate
> does not use. The derive expands to bare `num_traits::` paths that must
> resolve in this crate's own extern prelude, so a transitive copy does not
> help. `num-traits` therefore stays.
>
> I checked for aliased imports as the way a dependency could hide, and missed
> the mechanism that was actually hiding one: a proc-macro emitting a path that
> appears nowhere in the source. That is the *converse* of the blind spot this
> very section cites two paragraphs down — and I cited it while walking into it.

*(Corrected after cycle 19: the expansion emits an `extern crate num_traits as
_num_traits;` item, not the bare `num_traits::` paths described above. The
conclusion is unchanged and in fact firmer — an `extern crate` item resolves
only against this crate's own extern prelude, which makes the direct-dependency
requirement structural rather than stylistic. See cycle 19 §2.)*

`serde_json` and `rayon` are genuinely unused — verified by removing both and
running `cargo check -p codelore-rca --all-targets`, which passes with zero
errors. They are the same residue class as `petgraph` and `aho-corasick`:
upstream `rust-code-analysis` used them for JSON output and parallel walking;
this fork's surviving subset does not.

**The bound, restated correctly.** Removing the two genuinely-unused crates changes the build by **nothing**:

- `serde_json` — also a direct dependency of `codelore-lib` *and* `codelore-cli`.
- `rayon` — also a direct dependency of `codelore-lib`.

So this is **manifest hygiene only, with zero build-time or supply-chain reduction** — explicitly not the kind of win #278 delivered. Its value is that a vendored crate's manifest currently overstates what the vendored code needs, which misleads exactly the person trying to work out what the fork still depends on. That person, twice recently, was me.

**Complementary tooling note — and a third class the first draft of this note missed.** #278's CHANGELOG observes that "unused-dependency tooling would not have found any of this: the grammars were referenced, from inside the `mk_langs!` invocation, and merely unreachable." That is right about the grammars, and I paired it with its converse: `cargo-machete` or `cargo-udeps` would find genuinely-unreferenced declarations that humans miss. Both halves hold for `serde_json` and `rayon`.

But there is a third class, and this crate contains a live specimen of it: **required-but-invisible**. `num-traits` appears nowhere in the source and is load-bearing anyway, because a derive macro expands to it. `cargo-machete` is a text-based scanner and would flag it; acting on that flag breaks the build. `cargo-shear`'s own documentation concedes the same limit — it "cannot detect hidden imports from macro expansions" without a nightly `--expand`.

So the recommendation needs a qualifier it did not have: a `cargo-machete` step is still worth wiring, but it must land **with `num-traits` in `package.metadata.cargo-machete.ignored`**, and its output has to be treated as a candidate list rather than a verdict. A gate that is wrong about this crate on day one, and whose wrongness compiles cleanly right up until someone acts on it, is worse than no gate — it manufactures exactly the confident-and-wrong claim the last three cycles have been about.

---

## 4. Residuals

Unchanged: the gitlink differential fixture (still the only item with no decision recorded against it, since cycle 6); `outputSchema` at 1 of 11 MCP tools; M8 cancellation; zizmor not yet a required context. From cycle 13: the tested `cargo publish --no-verify` split for Trusted Publishing without trading Build L3. From cycle 15: P1–P6, with P1 (AI attribution) still the highest-value item and its design in cycle 16 §2.

**Currency:** not re-verified — the delta is two documentation commits and nothing dependency-related moved.

---

## 5. Honesty ledger

- **The sweep in §3 is the one I should have run in cycle 16.** Two cycles and two corrections later, running it across all three crates with a real method took one command. The finding it produced is small; the fact that it exists at all is the point.
- **I stated the "no build change" bound before claiming the finding — and the bound was wrong.** Applying §2.1's lesson, I checked the lockfile to confirm each crate was still pulled elsewhere, and concluded removal was free. It was free for two of three. The check I ran answers "does this crate survive in the graph", which is not the question; the question is "does *this* crate still compile without the declaration", and only a build answers it. Reaching for the lockfile a second time felt like applying the correction. It was applying its form to a different question.
- **The dependency I got wrong was hidden by the exact mechanism I cited in the same section.** §3's tooling note quotes #278 on macro-referenced-but-unreachable grammars, then misses macro-required-but-unreferenced `num-traits` two paragraphs above it. I checked for aliased imports as the hiding mechanism and did not consider proc-macro expansion, which is the one that was operating.
- **I reject one correction and accept two.** The rejection is evidenced with the commands that settle it. If the counter-argument is that `[behind N]` is a confusing figure to cite at all, I agree, and the underlying point survives.
- **Limits.** Nothing compiled. The three unused dependencies are established by exhaustive text search over the crate's sources plus the absence of aliasing; a `cargo-machete` run would confirm independently and is the natural check. The "no build change" claim rests on lockfile consumer analysis, not on two builds compared.

---

## 6. Housekeeping

- Branches: `main` + `gh-pages`. All report branches through cycle 17 have landed.
- `_to_delete/` carries this cycle's artifacts; `rm -rf _to_delete` when convenient. `HANDOFF.md` remains yours.
- **This report** is committed to branch `docs/hardening-cycle-18`, based on `main` (`19a00ec`).

---

## 7. Method

Two commits, both read at source. #284 was checked against the finding it implements, including whether it absorbed the correction made to that finding in the same PR series — it did. #283's three corrections were each re-derived from the artifacts they concern (`Cargo.lock` for two, `git merge-base` plus `rev-list` against the tracking ref for the third) before being accepted or rejected. The new finding came from finally running the sweep whose earlier crude versions produced two of the corrections being adjudicated, and its impact was bounded against the lockfile before it was written down rather than after.
