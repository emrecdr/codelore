# Hardening cycle 19 — a finding that was one-third wrong, and a recommendation that was wholly wrong

**Anchor:** `e01df10` · **Baseline:** `19a00ec` (cycle-18 anchor) · **Delta:** 3 commits (#285–#287), no release cut.

Audited from the live repository read-only; `main` = `origin/main` = `e01df10`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled here.

All three commits are consequences of cycle 18. One upholds my rejection of a correction. One corrects my finding. One **empirically refutes my recommendation** and, in doing so, finds a real defect my sweep missed. The second and third are the substance of this cycle, and both land against me.

---

## 1. The rejection was upheld

Cycle 18 §2.3 rejected the claim that `gh-pages`'s "13 behind" measures nothing, on the evidence that `[behind N]` compares a branch to its own upstream rather than to `main`. #285 withdraws that correction and records why: it had measured `gh-pages..main` (807) — "a comparison the report never made — and generalised from it."

Worth stating plainly because it cuts both ways: this engagement has now had a correction rejected on evidence and retracted, in the same series where three of my own claims were corrected on evidence and stood. That is the process working in both directions, which is the only way it is worth anything.

---

## 2. My finding was one-third wrong, and the lesson is better than the finding

Cycle 18 named three unused dependencies in `codelore-rca`. **Two were removable; the third was load-bearing.**

#285 settled it by building, which is the part I could not do:

```
serde_json + rayon removed  ->  cargo check --all-targets: 0 errors
num-traits removed          ->  error[E0463]: can't find crate for `num_traits`
                                in six language_*.rs files
```

The mechanism, which I verified directly:

| | |
|---|---|
| Files importing `num_derive` | **6** (`use num_derive::FromPrimitive;`) |
| Files naming `num_traits` | **0** |

`num-derive`'s `FromPrimitive` expansion emits bare `num_traits::` paths that must resolve in *this crate's* extern prelude — its documentation says the macros assume `num_traits` is a direct dependency unless the `#[num_traits = "…"]` helper names another path, which this crate does not use. So a transitive copy does not help, and the crate is required while being entirely absent from the source text.

Three of my claims fall with it: "all three unreferenced"; the "removing all three changes the build by nothing" bound; and the tooling note's assertion that cargo-machete would find all three. #285's extracted lesson is sharper than anything in my finding, and I am adopting it verbatim as a standing rule:

> **"Absent from the source text" and "safe to remove" are different predicates, and only the second is checkable by building.**

My cycle-18 report claimed rigour in the exact place it failed — *"zero occurrences anywhere… no aliased imports that could hide them"* — having enumerated one hiding mechanism (aliasing) and missed the one that was actually operating (derive expansion). Bounding the *impact* before claiming the finding, which I did and was pleased with, is worth nothing if the finding's premise is wrong.

---

## 3. My cargo-machete recommendation was refuted by running it

Cycle 18 recommended `cargo-machete` as a guard for the unused-declared class. #287 ran it. On this workspace it reports **six findings: one real, five that name load-bearing code.**

- **Real:** `codelore-cli` declares `dirs` and never calls it. All three mentions in its sources are *comments about `codelore-lib`'s* cache-root resolution; the single real `dirs::cache_dir()` call lives in `codelore-lib/src/cache.rs` beside that crate's own declaration. Removed in #287, verified by deleting and building — and note that **my sweep missed this**, because the name does appear in the crate's text. My method produced both error types: a false positive (`num-traits`, text-absent but required) and a false negative (`dirs`, text-present but unused).
- **False positives:** `num-traits`, plus `tree-sitter-java`, `-javascript`, `-python`, `-rust` — four of the five grammars the product actually dispatches. Acting on that output deletes those languages.

**The tool's own output proves the mechanism, and I verified the natural experiment:**

| Grammar | Named in `langs.rs` (`mk_langs!`) | Named elsewhere | Flagged? |
|---|---|---|---|
| java | 2 | **0** | yes |
| javascript | 1 | **0** | yes |
| python | 1 | **0** | yes |
| rust | 1 | **0** | yes |
| **typescript** | 1 | **3** | **no** |

The only grammar cargo-machete does *not* flag is the only one named outside the macro — it appears in the `get_language!` special case in `macros.rs`. Four named only inside `mk_langs!` are flagged; the one named outside is clean. cargo-machete is a text scanner and macro invocations are opaque to it; cargo-shear concedes the same limit, liftable only with a nightly `--expand`.

Reaching green would take five ignore entries, at which point the gate suppresses more than it reports and each entry is a standing invitation to remove the wrong one. #287 also grounds the rejection in this repo's own precedent: it fails the bar set when zizmor was adopted, where an advisory version was written and discarded because *"a check that is red for the wrong reason teaches people to ignore red checks."*

**Why this refutation should have been mine.** Cycle 18 §3 quoted #278's CHANGELOG — *"the grammars were referenced, from inside the `mk_langs!` invocation, and merely unreachable"* — and then, two paragraphs later, recommended a text scanner as a guard. The same macro opacity that makes the grammars invisible to reachability analysis makes them **false positives** to text analysis. I had the disqualifying fact in my own report and drew the opposite conclusion from it. That is a worse error than the num-traits miss, because it required no new information to avoid.

---

## 4. New finding

### F — LOW (new) — The one dependency that is invisible to text search carries no comment saying so, and has now trapped two independent auditors

I checked whether `num-traits` is an instance of a class or a one-off. Across all three crates, every other non-obvious dependency is named somewhere in source text:

| Dependency | Files naming it |
|---|---|
| `serde` | 93 |
| `clap` | 6 |
| `num-derive` | 6 |
| `schemars` | 3 |
| `thiserror` | 2 |
| `num-format` | 1 |
| **`num-traits`** | **0** |

**`num-traits` is the only text-invisible dependency in the workspace.** Every derive macro in use whose expansion needs a companion crate (`serde`, `clap`, `schemars`, `thiserror`) has that crate named directly somewhere; only `num-derive` → `num_traits` has the derive crate imported by name while the crate its output requires is never written down.

It has now trapped: my cycle-18 sweep, `cargo-machete`, and by #287's reasoning `cargo-shear` as well. Its manifest line carries no rationale (`num-traits  = "0.2"`, `Cargo.toml:44`), while comparable non-obvious dependencies in this workspace do — `leiden-rs` carries four lines explaining its feature choice, `headless_chrome` five explaining its optional gate. The dependency with the strongest claim to needing a comment is the one without one.

**Fix:** one comment on that line — that it is required by `#[derive(FromPrimitive)]` expansion in the six generated `language_*.rs` files, that it appears nowhere in the source text, and that unused-dependency scanners will flag it as a false positive. That is a three-line change which forecloses a trap that has already been sprung twice, and it matches the convention the repo already applies to less confusing dependencies.

**Severity Low**, honestly: nothing is broken and the build is correct. This is documentation of a known trap, not a defect.

---

## 5. Residuals

Unchanged: the gitlink differential fixture (still the only item with no decision recorded against it, since cycle 6); `outputSchema` at 1 of 11 MCP tools; M8 cancellation; zizmor not yet a required context in `protect-main`. From cycle 13: the tested `cargo publish --no-verify` split. From cycle 15: P1–P6, with P1 (AI attribution) still the highest-value open item and its design in cycle 16 §2.

**Closed this cycle:** the unused-declared-dependency thread. Two deps removed from `codelore-rca` (#286), one from `codelore-cli` (#287), `num-traits` correctly retained, and the automation option evaluated and rejected with evidence rather than left open. That thread is done; §4 is its last loose end.

**Currency:** not re-verified; nothing dependency-related moved beyond the removals above.

---

## 6. Honesty ledger

- **The cargo-machete recommendation is the worst kind of error in this engagement**, worse than the cycle-17 misquote: that one needed a truncated quotation to go wrong, this one needed only that I not connect two paragraphs of my own report. A recommendation is a claim about the future and inherits the burden of one — including the burden of checking it against the facts already in the same document.
- **My sweep produced both error types.** False positive (`num-traits`) and false negative (`dirs`). A method that can fail in both directions is not a sweep, it is a heuristic, and cycle 18 presented it as the rigorous version of a method whose crude version had already been corrected twice.
- **The capability gap is real and worth naming.** #285 and #287 settled in minutes, by building, questions I cannot settle here. Where a claim is only checkable by compiling, the honest move is to mark it as unverified rather than to present a text search as equivalent — and cycle 18 did not do that.
- **One rejection upheld.** §1. Recorded because a ledger that only lists my errors is as unbalanced as one that lists none.
- **Limits.** Nothing compiled. §4's claim that `num-traits` is the only text-invisible dependency rests on the same text-search method that failed in cycle 18 — but here it is used to establish *absence of naming*, which is what text search actually measures, rather than *absence of need*, which is what it cannot. The empirical cross-check is cargo-machete's own six findings, which name exactly one text-invisible dependency.

---

## 7. Housekeeping

- Branches: `main` + `gh-pages`. All report branches through cycle 18 have landed.
- `_to_delete/` carries prior cycles' artifacts; `rm -rf _to_delete` when convenient. `HANDOFF.md` remains yours.
- **This report** is committed to branch `docs/hardening-cycle-19`, based on `main` (`e01df10`).

---

## 8. Method

Three commits read at source. The retraction in §1 was checked against the figure it concerns. The correction in §2 was re-derived by counting `num_derive` imports against `num_traits` mentions across the crate, which reproduces the mechanism without a compiler. §3's refutation was verified by the natural experiment its own commit message implies — tabulating where each grammar is named, inside the macro versus outside — and the one grammar the tool does not flag is the one named outside, exactly as claimed. §4 came from asking whether the corrected finding was a one-off or a class, and testing that against every derive macro in the workspace.
