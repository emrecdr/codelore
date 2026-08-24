# Hardening cycle 20 — the fourth substring error, and one deferral that bundles a free fix with an expensive one

**Anchor:** `d3ff722b` · **Baseline:** `e01df10` (cycle-19 anchor) · **Delta:** 3 commits (#288–#290), no release cut.

Audited from the live repository read-only; `main` = `origin/main` = `d3ff722b`; tree clean but for untracked `HANDOFF.md` and `_to_delete/`. Nothing compiled here.

All three commits come from validating cycle 19, and all three found things that cycle missed. The corrections are in §1; the new finding in §3 concerns the one item #288 explicitly deferred, which I think was deferred for a reason that only applies to half of it.

---

## 1. Corrections to cycle 19, verified

### 1.1 The java/javascript substring error — the fourth of this exact class

Cycle 19 §3 tabulated where each grammar is named, to demonstrate the macro-opacity mechanism. The java row said "2" occurrences in `langs.rs`. **`tree_sitter_java` is a substring of `tree_sitter_javascript`**, so the java count silently absorbed the javascript line.

This is the same error I have now made four times: `<th` matching `<thead` (cycle 13), `ccomment` matching `DocCommentMarker` (cycle 17, caught in-draft), inflated `Preproc` file counts (corrected in #277), and now `java` matching `javascript`. Three of the four were in tables presented as evidence. The mechanism never varies — an unanchored substring search over identifiers that share prefixes — and neither does the fix, which is to anchor the pattern or count distinct matches.

The finding it supported survives untouched: the conclusion rested on typescript being the *only* grammar named outside `mk_langs!` and the only one unflagged, and that contrast is unaffected by the java row's inflation. But an evidence table with a wrong cell is a bad advertisement for the argument it carries.

### 1.2 The census omitted `num` — the most relevant comparator

Cycle 19 §4 tabulated how many files name each non-obvious dependency, to establish `num-traits` as the only text-invisible one. It listed seven crates and omitted **`num`**, which sits directly beside `num-derive` and `num-traits` in the same manifest. The num arrangement is three crates, not the two my table implied: `num` serves the generated call sites, `num-derive` the attribute, `num-traits` that attribute's expansion.

### 1.3 My description of the mechanism was imprecise

Cycle 19 said the expansion emits "bare `num_traits::` paths." #289 records the actual emission: **`extern crate num_traits as _num_traits;`**. My phrasing was close enough to be believed and wrong enough to be superseded — and #289 notes it had already propagated into a CHANGELOG entry, where two `[Unreleased]` entries described the mechanism two contradictory ways six lines apart, both due to ship in one release section.

### 1.4 Also accepted

Two comment-length figures undercounted; a bare `Cargo.toml:44` did not say which of four manifests it meant; and the report claimed to be committed to a branch with no remote and no pull request. That last one is a fair hit on a line I have written every cycle: creating a local ref via plumbing is not the same as landing a report, and only the maintainer's push makes it so.

### 1.5 What the project found in itself, which is the better half

#289 opens: *"A cleanup review of the previous commit found that its own enumeration was incomplete, which is the finding defending itself."* A seventh stale claim survived in `UPSTREAM.md` — twenty lines above a paragraph the previous commit had just rewritten, on a page published to crates.io — so a table enumerating six and calling them complete was wrong when written. The extracted lesson is the same one cycle 19 took from `num-traits`, turned on its author: **a sweep bounded by what its author thought to look for is not a sweep either.**

And #290 restores something #287's rejection took with it. Cycle 18's corrected recommendation had two halves — a machete *gate* and a machete *ignore declaration*. Rejecting the gate silently dropped the declaration, leaving a manifest comment saying "do not act on that report" that only reaches someone who opens the manifest, "when every report of this dependency so far arrived as tool output." `[package.metadata.cargo-machete] ignored = ["num-traits"]` now ships; five findings drop to four. The four grammar false positives are kept deliberately, because they are the evidence that a text scanner cannot see through a macro, and a green scanner would erase the argument for rejecting the gate. That is a sharper disposition than either my recommendation or its refutation reached alone.

---

## 2. Verified clean

The manifest marker sweep (#290) covers seven sites across four manifests plus `UPSTREAM.md` — which `codelore-rca` sets as `readme`, making it that crate's published package page, outside every guard's reach twice over (by extension and by path). The num-family now carries the comment cycle 19 asked for, widened to all three crates, with the facade-aliasing alternative considered and rejected on the grounds that the other num crates arrive via arrow and duckdb anyway.

---

## 3. New finding

### F — LOW (new) — The deferred `codelore-rca` public-surface item bundles a free documentation fix with an expensive API change, and only the second deserves the deferral

#288 records, rather than fixes, that *"codelore-rca's public surface still describes languages it cannot parse — an orphaned `SpaceKind` variant and a crate rustdoc listing six languages never vendored"*, and classifies it as *"a breaking API change and an upstream divergence, which is a decision about the fork's budget rather than a defect fix."*

That classification is right for one of the two items and wrong for the other, and bundling them blocks a free fix behind a budget decision it does not need.

**The rustdoc.** `crates/codelore-rca/src/lib.rs` advertises eleven languages; the crate parses five:

| Advertised | Status |
|---|---|
| Java, JavaScript, Python, Rust, Typescript | **parse** (5) |
| C++ | vendored, removed in #278 |
| "The JavaScript used in Firefox internal" (Mozjs) | vendored, removed at fork time |
| C#, CSS, Go, HTML | **never vendored by this fork** — `UPSTREAM.md` has no hit for any of them |

So the split is four never-present plus two removed, not "six never vendored" — a small precision worth having, because the four have been wrong since the fork was created, inherited verbatim from upstream, while the two became wrong through this project's own excisions.

Fixing this is a **doc-comment edit**. It changes no type, no signature, no behaviour; it is not a breaking change and the "upstream divergence" it creates is a comment that describes this fork instead of a different codebase. Meanwhile it is live right now on the docs.rs page for `codelore-rca 0.28.0`, telling anyone who lands there that the crate handles C++, C#, CSS, Go and HTML. Of everything in this repository, a published package page advertising six capabilities that do not exist is the item with the widest audience and the lowest fix cost.

**The `SpaceKind` variant** is the genuinely expensive one, and more entangled than #288's phrasing suggests. `SpaceKind::Namespace` — "A `C/C++` namespace" — can no longer be constructed, but it is still *matched* in two places, and one of them is in the product, not the vendored crate:

- `codelore-rca/src/spaces.rs:58` — its own `Display`
- `codelore-lib/src/complexity/mod.rs:60` — `space_kind_str`, mapping it to `"namespace"`

So the product carries a live match arm for a variant nothing can produce. Removing the variant is a breaking change to a published enum *and* touches the product's dispatch; that genuinely is a fork-budget decision, and deferring it is reasonable.

**Recommendation:** split the item. Ship the rustdoc correction now — it is minutes of work, needs no API decision, and is the only part with a public audience. Keep the `SpaceKind` removal deferred on its own merits, and while it waits, note the dead arm at `complexity/mod.rs:60` so the next reader knows it is unreachable rather than merely rare.

**Severity Low.** Nothing computes a wrong number and no exit code moves. It is documentation — but documentation on a published package page, which is the same class as F287 (the `@v1` ref the docs promised and nothing provided), and that one was rated by its audience rather than its mechanism.

**My own stake in this, stated plainly:** I quoted that exact rustdoc block in cycle 16 §1 — the "## Supported Languages — C++, C#, CSS, Go, HTML…" list — while building the case that C++ and Kotlin were unreachable. I read the crate's own claim that it supports C++, used the passage as context, and never noticed the claim was false. Cycle 17 then declared the excision "complete and clean" on the strength of an identifier sweep that by construction could not see prose. Both misses are mine, and #288 found them.

---

## 4. Residuals

Unchanged: the gitlink differential fixture (still the only item with no decision recorded against it, since cycle 6); `outputSchema` at 1 of 11 MCP tools; M8 cancellation; zizmor not yet a required context. From cycle 13: the `cargo publish --no-verify` split. From cycle 15: P1–P6, with P1 (AI attribution) still the highest-value open item.

**Newly open:** the `SpaceKind::Namespace` removal (§3), deferred on its own merits, with a dead match arm in the product while it waits.

**Currency:** not re-verified; the delta is documentation and manifest metadata.

---

## 5. Honesty ledger

- **Four substring errors, three of them in evidence tables.** The pattern is now the most reliable defect I produce. The remedy is mechanical and I have not been applying it: anchor the pattern, or count distinct matches, whenever the search term can prefix another identifier in the same namespace. `java`/`javascript` was foreseeable from the same table it appeared in.
- **My mechanism description was imprecise and propagated.** "Bare `num_traits::` paths" was close enough to be adopted and wrong enough to need superseding, and it reached a CHANGELOG before it was corrected. Getting a mechanism *approximately* right is worse than saying it is unverified, because approximations get quoted.
- **I quoted the stale rustdoc and did not see it** (§3). Cycle 16 had the evidence in the report; cycle 17 declared the surface clean using a method that could not have seen it. The lesson is the same one the project extracted for itself in #289 — a sweep is bounded by what its author thought to look for — and identifier sweeps are bounded, by construction, to identifiers.
- **The project's self-correction is now finding more than my audits are.** #288, #289 and #290 each found real defects, including one that invalidated the previous commit's own completeness claim. That is the right direction for this engagement to be heading, and worth saying rather than competing with.
- **Limits.** Nothing compiled. §3's claims are established from source: the rustdoc block, the `SpaceKind` enum, its two match sites, and `UPSTREAM.md`'s silence on C#/CSS/Go/HTML. The claim that `SpaceKind::Namespace` cannot be constructed rests on the C++ getter having been removed in #278 and no remaining site producing it — a reachability argument of exactly the kind #285 showed can fail when macros are involved, so it should be confirmed by removing the variant and building before anyone acts on it.

---

## 6. Housekeeping

- Branches: `main` + `gh-pages`. Cycle 19 landed via #288.
- `_to_delete/` carries prior artifacts; `rm -rf _to_delete` when convenient. `HANDOFF.md` remains yours.
- **This report** is written to `docs/reports/2026-08-24-hardening-cycle-20.md` and committed to a local branch `docs/hardening-cycle-20` based on `main` (`d3ff722b`). Per §1.4, that is a local ref only — it needs a push and a PR to land.

---

## 7. Method

Three commits read at source. Each correction to cycle 19 was re-derived before acceptance: the java/javascript overlap by re-running the count with the substring relationship in view, the num-family arrangement by reading the manifest, the emission mechanism from #289's own statement of it. The new finding came from taking the one item #288 deferred and asking whether its two halves share a cost — they do not — which required enumerating the advertised languages against the parseable ones and tracing `SpaceKind::Namespace` to its remaining match sites, including the one in the product that #288's phrasing did not mention.
