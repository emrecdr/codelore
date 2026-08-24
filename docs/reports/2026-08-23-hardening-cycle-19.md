# Hardening cycle 19 — a finding that was one-third wrong, and a recommendation that was wholly wrong

**Anchor:** `e01df10` · **Baseline:** `19a00ec` (cycle-18 anchor) · **Delta:** 3 commits (#285–#287), no release cut.

Audited from the live repository read-only; `main` = `origin/main` = `e01df10`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled here.

All three commits are consequences of cycle 18. One upholds my rejection of a correction. One corrects my finding. One **empirically refutes my recommendation** and, in doing so, finds a real defect my sweep missed. The second and third are the substance of this cycle, and both land against me.

> **Revised after an independent validation pass.** The headline finding survived
> a harder test than the one it applied: a census of every declaration in all four
> manifests, across normal, dev and build sections, confirms `num-traits` is the
> only text-invisible dependency. The `extern crate` mechanism was read out of
> num-derive's own source, and `cargo-machete` was run directly — five findings,
> matching this report's six minus the one already removed. Four claims did not
> survive and are corrected in place: the grammar table's java row (§3, a
> substring double-count), the census table's omission of `num` (§4, the most
> relevant comparator), two comment-length figures (§4), and the branch status
> (§7 — this report was **not** landed when it claimed to be). Corrections are
> inline. §4 also gained a second finding: the stale-comment defect turned out to
> be a class of seven, two of them user-facing.

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

Six generated files carry `use num_derive::FromPrimitive;`; none names `num_traits` anywhere.

`num-derive`'s `FromPrimitive` expansion emits `extern crate num_traits as _num_traits;` — verified in the proc-macro's own source, which documents that the macros assume `num_traits` is a direct dependency unless the `#[num_traits = "…"]` helper names another ident, a helper this crate does not use. An `extern crate` item resolves only against *this crate's* extern prelude, which is why a transitive copy cannot serve and why the failure surfaces as `E0463` — a crate-resolution error rather than a path error. The crate is required while being entirely absent from the source text.

*(Correction: this section originally described the expansion as emitting "bare `num_traits::` paths". The conclusion is unchanged, but the mechanism is an `extern crate` item, which is what makes the direct-dependency requirement structural rather than stylistic.)*

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
| java | 1 | **0** | yes |
| javascript | 1 | **0** | yes |
| python | 1 | **0** | yes |
| rust | 1 | **0** | yes |
| **typescript** | 1 | **3** | **no** |

*(Correction: the java row originally read 2. `tree_sitter_java` is a substring of `tree_sitter_javascript`, so the search counted the javascript declaration twice — a false positive of exactly the kind this section is about, inside the table demonstrating it. The mechanism and the conclusion are unaffected.)*

The only grammar cargo-machete does *not* flag is the only one named outside the macro — it appears in the `get_language!` special case in `macros.rs`. Four named only inside `mk_langs!` are flagged; the one named outside is clean. cargo-machete is a text scanner and macro invocations are opaque to it; cargo-shear concedes the same limit, liftable only with a nightly `--expand`.

Reaching green would take five ignore entries, at which point the gate suppresses more than it reports and each entry is a standing invitation to remove the wrong one. #287 also grounds the rejection in this repo's own precedent: it fails the bar set when zizmor was adopted, where an advisory version was written and discarded because *"a check that is red on every pull request teaches people to ignore red checks, which is worse than not running the tool."* A machete gate fails that bar on its own terms — five false positives make it red on every pull request until they are suppressed.

**Why this refutation should have been mine.** Cycle 18 §3 quoted #278's CHANGELOG — *"the grammars were referenced, from inside the `mk_langs!` invocation, and merely unreachable"* — and then, two paragraphs later, recommended a text scanner as a guard. The same macro opacity that makes the grammars invisible to reachability analysis makes them **false positives** to text analysis. I had the disqualifying fact in my own report and drew the opposite conclusion from it. That is a worse error than the num-traits miss, because it required no new information to avoid.

---

## 4. New findings

### F — LOW (new) — The one dependency that is invisible to text search carries no comment saying so, and has now trapped two independent auditors

I checked whether `num-traits` is an instance of a class or a one-off. Across all three crates, every other non-obvious dependency is named somewhere in source text:

| Dependency | Files naming it |
|---|---|
| `serde` | 93 |
| `clap` | 6 |
| `num-derive` | 6 |
| **`num`** | **6** |
| `schemars` | 3 |
| `thiserror` | 2 |
| `num-format` | 1 |
| **`num-traits`** | **0** |

**`num-traits` is the only text-invisible dependency in the workspace.** Every derive macro in use whose expansion needs a companion crate (`serde`, `clap`, `schemars`, `thiserror`) has that crate named directly somewhere; only `num-derive` → `num_traits` has the derive crate imported by name while the crate its output requires is never written down.

*(Correction: the `num` row was missing, and it is the most relevant comparator in the table — same crate, same manifest, same family. Restoring it shows the arrangement is three crates rather than two, and that the report described two of them.* `num` *is required by the generated **call sites**, which read* `num::FromPrimitive::from_u16(x)` *in the same six files;* `num-derive` *supplies the attribute;* `num-traits` *is required by the attribute's expansion. The facade is greppable and the leaf is not, which is a better account of the trap than "it happens to be the only invisible one". The zero and the six are exact; the remaining counts are the section's original figures and were not re-derived, since none of them carry argumentative weight.)*

It has now trapped: my cycle-18 sweep, `cargo-machete`, and by #287's reasoning `cargo-shear` as well. Its manifest line carries no rationale (`num-traits  = "0.2"`, `crates/codelore-rca/Cargo.toml:44`), while comparable non-obvious dependencies in this workspace do — `leiden-rs` carries twelve lines explaining its feature choice, `headless_chrome` eight explaining its optional gate. The dependency with the strongest claim to needing a comment is the one without one.

*(Correction: those two figures originally read four and five. Both were undercounts, which strengthens the point rather than weakening it. The original also cited a bare `Cargo.toml:44` without saying which of the workspace's four manifests it meant.)*

**Fix:** comment the num-family as a group, since the confusion is relational rather than local — naming which crate each of the three serves, that `num-traits` appears nowhere in the source text, and that unused-dependency scanners will therefore flag it as a false positive. Commenting only `num-traits` would leave the next reader still wondering why a crate and its own facade are both declared.

**Considered and rejected:** aliasing the facade away with `num = { package = "num-traits" }`, which would make `num-traits` text-visible and drop the facade without touching a file marked `// Code generated; DO NOT EDIT.`. Measurement kills it — `num-integer`, `num-complex`, `num-rational`, `num-iter` and `num-bigint` are all pulled in by `arrow` and `duckdb` independently, so the compile-graph saving is the facade alone. Trading a documented dependency for an aliased manifest key sets a subtler trap than the one it closes, which is the failure mode this whole cycle is about.

**Severity Low**, honestly: nothing is broken and the build is correct. This is documentation of a known trap, not a defect.

### F — MEDIUM (new, found in validation) — the removal commits left a trail of stale claims, and two of them are user-facing

Added by the validation pass rather than the audit. It began as the small finding below and did not stay small: asking whether the stale comment was a one-off or a class returned **a class**, spanning three commits, with the two worst instances well outside comment prose.

| | where | what it claims | severity |
|---|---|---|---|
| **`codelore profile` output** | `codelore-cli/src/main.rs` | advertises a C++ tree-sitter grammar | **printed to users** |
| **`codelore-rca`'s crates.io page** | `UPSTREAM.md` pinned-grammar table | four grammars the manifest no longer declares, plus a paragraph routing C++ through a deleted macro arm | **published** |
| forward reference | `UPSTREAM.md` layout notes | a `macros.rs` mozcpp reference that is gone, and work "to be resolved" that was | doc |
| stale rationale | `codelore-lib/src/analyses/import_graph.rs` | avoids `petgraph` because of a version conflict that no longer exists | comment |
| self-contradiction | `docs/roadmap-v1.x-and-beyond.md` | says the `leiden-rs` `petgraph` feature is off *solely* over that conflict — contradicting the manifest comment the same commit wrote saying the opposite | doc |
| the same attribution again | `UPSTREAM.md` excision narrative | repeats that *solely*-over-the-conflict claim, twenty lines from a paragraph this pass rewrote | **published** |
| the original | `codelore-cli/Cargo.toml` | documents a dependency the manifest no longer declares | comment |

The first two matter most and are the ones no comment-hygiene process would have caught. `codelore profile` is the command a user runs to find out what the tool is built from, and it names a language the binary cannot parse. `UPSTREAM.md` is set as `readme` in `codelore-rca`'s manifest, so its grammar table *is* the crates.io page — telling anyone evaluating the crate that it pins four grammars it does not have, in a section written as present-tense operating guidance for the next person upgrading them.

All seven are fixed in the landing commit. Two further instances in the vendored fork — an orphaned `SpaceKind` variant documented as C/C++, and a crate rustdoc listing six languages that were never vendored — are recorded rather than fixed, because one is a breaking API change and the other means diverging from upstream for cosmetic gain.

An eighth is excluded on purpose, and naming the reason is what actually closes this class. `CHANGELOG.md`'s released `[0.28.0]` section carries the same *solely* attribution the roadmap row and `UPSTREAM.md` were corrected for. It stays. A released changelog section records what was believed when it shipped, and the `[Unreleased]` entry directly above it already discloses the correction — rewriting the released text would delete that disclosure rather than add to it. The criterion this table should have carried from the start is therefore not a count but a boundary: **prose giving present-tense guidance** gets swept, **prose recording what was believed at a point in time** does not. Manifests, module rustdocs, published READMEs and the roadmap are the first kind; dated audit reports and released changelog sections are the second. Seven was never a property of the codebase — it was a property of where the author looked. Stating the rule closes the class; counting instances only ever closes the instances someone thought to count.

The seventh is worth its own sentence, because of how it was found. The first six came from a sweep. The seventh came from a **cleanup review of the fix for the first six**, and it sat twenty lines above a paragraph that fix had already rewritten — in the published README, carrying the exact attribution the roadmap row was corrected for. A table that enumerated six and declared them complete was wrong at the moment it was written. That is the class defending itself: a sweep bounded by what its author thought to look for is not a sweep either, and the only thing that caught the remainder was a second pass with a different brief.

**What this says about the excision.** #278 was a large, careful, well-reasoned change, and its own narrative in `UPSTREAM.md` is accurate. What it did not do was sweep for *other* text asserting the state it had just changed. The removals were verified by building, and every one of these survives a build.

---

The original instance, which found the class:

```toml
# Plan …: dirs (cache root for diff worktrees), serde/serde_json
# (DiffOutput + --base-cache serialization), tempfile (worktree tempdirs).
-dirs = "6"                                    ← removed by #287
 serde = { version = "1", features = ["derive"] }
```

The comment survived the deletion, so `crates/codelore-cli/Cargo.toml` documented a dependency it no longer declared. This is not the same defect as the `dirs` declaration itself — that one was a manifest claiming more than the code used; this one is a manifest explaining a line that is gone.

It also sharpens the cycle's own lesson. The removal was verified by building, which is the right method and the one this cycle argues for — but a build cannot see a stale comment. **"Verified by building" bounds the change to the compiler's field of view, and prose is outside it.** The report's §6 names a capability gap in the other direction (claims only checkable by compiling); this is the complement.

**Fix:** rewrite the comment to describe what the block actually declares. Done in the landing commit, since a comment that is wrong is not deferrable in the way an absent comment is.

**Related, recorded not fixed:** the comment also carried a plan-number marker, of the kind `comment_hygiene_test` forbids. That guard scans `.rs`/`.sql` under `src`/`tests` and never reads manifests, so two further markers survive in the other two manifests. Logged as a finding rather than fixed here — it needs a widened guard plus a self-test, not a quiet edit.

---

## 5. Residuals

Unchanged: the gitlink differential fixture (still the only item with no decision recorded against it, since cycle 6); `outputSchema` at 1 of 11 MCP tools; M8 cancellation; zizmor not yet a required context in `protect-main`. From cycle 13: the tested `cargo publish --no-verify` split. From cycle 15: P1–P6, with P1 (AI attribution) still the highest-value open item and its design in cycle 16 §2.

*(Two residuals were spot-checked during validation rather than carried forward unverified. **zizmor**: confirmed open — `protect-main` requires nine contexts (`cargo-deny`, `clippy`, `dogfood`, `rustfmt`, `self-gate`, `spa-browser`, and the three `test` matrix legs) and zizmor is not among them. **`outputSchema`**: the "1 of 11" figure looks overstated — there are eleven `#[tool(` declarations in `mcp.rs` and no occurrence of `output_schema`, `outputSchema`, or structured-content plumbing anywhere in the CLI crate, so the honest reading is zero of eleven. Flagged rather than rewritten, because the figure predates this cycle and the residual is open either way.)*

*(**Retracted in cycle 20.** The `outputSchema` refutation above is wrong, and "1 of 11" was right all along. `check_gates` returns `Result<Json<GateSummary>, ErrorData>` — the only tool of the eleven that does — and `rmcp`'s `#[tool]` macro derives the output schema from a `Json<T>` return type, so the schema is published without `output_schema` ever appearing in this repository's source. `tests/mcp_test.rs` already asserts the running server emits it. The refutation searched for a literal that a proc-macro makes unnecessary, which is the exact macro-opacity mechanism §3 of this same report establishes against `cargo-machete` — committed here, two sections later, against the report's own residual. A claim of the form "feature X is not configured" is unfalsifiable by text search whenever the framework can infer X from a type signature; the check that works is reading the return types.)*

**Closed this cycle:** the unused-declared-dependency thread. Two deps removed from `codelore-rca` (#286), one from `codelore-cli` (#287), `num-traits` correctly retained, and the automation option evaluated and rejected with evidence rather than left open. That thread is done — §4's first finding was its last loose end, closed by the comment the landing commit adds and by the `[package.metadata.cargo-machete] ignored` entry a later cleanup pass put beside it. That entry is the half of the fix a scanner can act on, and cycle 18 §3 had already prescribed it; rejecting the *gate* here silently took the non-gate declaration with it, which is how the comment came to ship alone. §4's second finding belongs to a different thread.

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

- Branches: `main` + `gh-pages` on the remote. All report branches through cycle 18 have landed.
- `_to_delete/` carries prior cycles' artifacts; `rm -rf _to_delete` when convenient. `HANDOFF.md` remains yours.
- **This report** is committed to the local branch `docs/hardening-cycle-19`, based on `main` (`e01df10`).

*(Correction: the first and third bullets contradicted each other — a local `docs/hardening-cycle-19` did exist when this was written, so the branch list was incomplete. More usefully: that branch had no remote and no pull request, so unlike every cycle through 18 this report was **not** landed at the time it claimed to be committed. It lands with the validation pass that produced these corrections.)*

---

## 8. Method

Three commits read at source. The retraction in §1 was checked against the figure it concerns. The correction in §2 was re-derived by counting `num_derive` imports against `num_traits` mentions across the crate, which reproduces the mechanism without a compiler. §3's refutation was verified by the natural experiment its own commit message implies — tabulating where each grammar is named, inside the macro versus outside — and the one grammar the tool does not flag is the one named outside, exactly as claimed. §4 came from asking whether the corrected finding was a one-off or a class, and testing that against every derive macro in the workspace.
