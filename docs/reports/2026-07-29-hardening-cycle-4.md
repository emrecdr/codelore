# Hardening cycle 4 — fix verification and fresh audit

**Audit anchor:** `35f6bab` (`fix(spa): let low-content widgets shrink and make periphery nodes visible (#187)`)
**Fix-verification baseline:** `05ca9c9` = `v0.24.0` (`chore(release): v0.24.0 (#185)`)
**Delta:** main is **two commits ahead** of the release this cycle set out to verify — `c3c43ee` (#186, crates.io package rename + publish pipeline) and `35f6bab` (#187, SPA widget shrink + periphery colour). Local `HEAD` == `origin/main` == `35f6bab`; no divergence.
**Workspace version literal:** `Cargo.toml:6` still reads `0.24.0`.

Every line reference in this report was re-resolved against `35f6bab`, not against the extracted `v0.24.0` tree. Where a finding was first observed on `05ca9c9` and the two new commits did not touch it, that is stated explicitly rather than assumed.

---

## 0. What this cycle actually is

Two things at once, and they gave different answers:

1. **A fix-verification pass** over the claims v0.24.0 made about closing cycle-3 findings.
2. **A fresh audit** of the whole tree, with the two new commits pulled in mid-flight.

The fix-verification half is the uncomfortable one. Of the two headline "fixed" claims checked in depth, **neither survived intact**: one was refuted outright (the fix touched comments, not behaviour) and one was confirmed only for the DOM while the rendered result is unchanged. That is not a criticism of the intent behind those PRs — it is a statement that the verification step between "PR merged" and "finding closed" is currently doing no work, and that is itself the most valuable finding of the cycle.

Every claim below passed through an independent adversarial validator whose **default verdict was REFUTED**. Five validators ran. They falsified **five premises of my own briefing document**, and those falsifications are recorded in §5 rather than quietly dropped. Two findings were **downgraded from High to Low** as a direct result. One finding I had never written down at all was discovered by a validator refuting a different claim.

---

## 1. Verdict table

Post-validation severity. Severity here is a function of **consumer blast radius** — can the defect move a `codelore check` / `diff` / `gate` exit code, an MCP verdict, or a CI outcome? — not of code-smell elegance.

| # | Finding | Verdict | Severity |
|---|---|---|---|
| H1 | `cut-release.sh` dispatch fallback picks a run by event, not SHA | CONFIRMED (jq-executed) | **High** ↑ |
| H2 | `release.yml:324` uses `secrets` in a step-level `if:` | CONFIRMED (vs GitHub contexts reference) | **High** (new) |
| H3 | `author_aliases` `BOOL_OR(is_bot)` collapses shared identities | PARTIALLY CONFIRMED — defect real, provenance wrong | **High** |
| H4 | Ratchet file carries no epoch / schema key | CONFIRMED | **High** |
| H5 | `codelore analyze` has no ingest witness | PARTIALLY CONFIRMED — `analyze` is the real gap | **High** |
| M1 | `evaluators.rs` passes 3 of 4 diff gates on empty input | CONFIRMED | **Medium** ‡ |
| M2 | `base_cache_opts_digest` omits epoch / version / schema | CONFIRMED | **Medium** |
| M3 | MCP reports a false skip reason for `corpus_percentile_max` | CONFIRMED | **Medium** |
| M4 | `delta_code_health_min` compares two different metrics | PARTIALLY CONFIRMED — divergence is metric, not population | **Medium** |
| M5 | `NotARepository` remedy suggests `git init` | CONFIRMED thesis, **all three stated examples REFUTED** | **Medium** |
| M6 | Ingest-witness behaviour is promised in docs in 7 places | PARTIALLY CONFIRMED | **Medium** |
| M7 | `cargo test -p codelore-lib` relies on an accidental feature prop | **REFUTED at workspace root**, CONFIRMED for `-p` | **Medium** |
| L1 | All SPA badges render green regardless of semantic class | CONFIRMED symptom, **mechanism REFUTED** | **Low–Med** |
| L2 | Stale calibration corpus | PARTIALLY CONFIRMED | **Low** ↓ (was High) |
| L3 | Degraded-witness over-fire | PARTIALLY CONFIRMED | **Low** ↓ (was High) |
| L4 | Mailmap differential test blind to email-only regressions | **REFUTED as stated**; reduced form CONFIRMED | **Low** ↓ |
| I1 | `types.rs SCHEMA_VERSION = 6` is dead, stale and tautologically tested | CONFIRMED | informational |
| I2 | Depth-1 non-merge clone produces a silent empty gate | **REFUTED** — hard-errors exit 3 first | informational |

‡ M1 is Medium by strict blast radius but is the **single highest-leverage fix in the report** — see §6.

---

## 2. High-severity findings

### H1 — `cut-release.sh` can declare CI green from an unrelated commit, and that now ships permanently

`scripts/cut-release.sh:445-450`, unchanged by #186 and #187:

```bash
RUN_ID="$(gh run list --limit 3 --branch main --workflow CI --json databaseId,event,headSha \
          --jq '.[] | select(.event == "workflow_dispatch") | .databaseId' | head -1)"
```

Compare the primary path immediately above it at `:430-431`, which is correct — it filters `select(.headSha == "${RELEASE_SHA}")`. The fallback path, taken when the release commit matched `paths-ignore` and no run auto-triggered, drops the SHA predicate entirely and selects **the most recent `workflow_dispatch` run on `main` by any SHA**. It requests `headSha` in `--json` and then never uses it.

The failure is not theoretical. `gh workflow run CI --ref main` is fire-and-forget; the run it creates takes several seconds to register. The script sleeps 5 and lists. If any human or automation dispatched CI on `main` in the recent past — a common thing to do while chasing a flake — that older run is newer-in-list-order than nothing and wins. It may already be `success`. The poll loop at `:455-498` then reports:

```
ok "CI green (conclusion=success) on ${RELEASE_SHA:0:7}"
```

naming a SHA it never checked. Control flows straight into the tag dance at `:503-537` and `git push origin "${TAG}"`.

**Why this is escalated this cycle.** In cycle 3 the worst case was a bad GitHub release, which is re-cuttable. #186 added a `crates-publish` job keyed on tag push, which runs `cargo publish` for three crates. **crates.io publication is permanent — yank only, never delete, and the version number is burned forever.** A false-green verdict now converts a recoverable mistake into an irreversible one. That is the escalation, and it is the reason H1 sits above the alias defect.

**Fix.** Delete the fallback's divergence: after dispatch, poll `gh run list --json databaseId,headSha,status` filtering on `headSha == RELEASE_SHA` with a bounded retry, and `die` if no such run appears. Never accept a run whose `headSha` was not compared. The primary path at `:430-431` is already the correct template; the fallback should be the same expression with a retry wrapper.

### H2 — `release.yml` gates crates.io publication on a context that is not available there

Added by #186 at `.github/workflows/release.yml:323-330`, **never yet executed** — `v0.24.0` points at `05ca9c9`, which predates the commit that introduced this job, so no tag has ever reached it:

```yaml
- name: Publish to crates.io
  if: ${{ github.ref_type == 'tag' && secrets.CRATES_IO_TOKEN != '' }}
  env:
    CARGO_REGISTRY_TOKEN: ${{ secrets.CRATES_IO_TOKEN }}
  run: |
    cargo publish -p codelore-rca
    cargo publish -p codelore-lib
    cargo publish -p codelore
```

GitHub's context-availability reference lists the contexts legal in each position. For `jobs.<job_id>.if` they are `github`, `needs`, `vars`, `inputs`. For `jobs.<job_id>.steps[*].if` they are `github`, `needs`, `strategy`, `matrix`, `job`, `runner`, `env`, `vars`, `steps`, `inputs`. **`secrets` appears in neither list.**

The workflow's own comment shows the authors knew about the job-level restriction and moved the condition down to step level to dodge it — but the same restriction applies one level down. The outcome is one of two things, neither good: the expression evaluates the unavailable context to empty and the step **never publishes on any release**, silently, forever; or the workflow fails validation on every tagged run. Both are discovered at the worst possible moment, since this code path only ever executes during a release.

`grep -rn "if:.*secrets\." .github/workflows/` returns exactly this one occurrence across all workflows, so the fix is contained.

**Fix.** The documented pattern is to map the secret into `env` at job level and test the env var:

```yaml
env:
  CRATES_IO_TOKEN: ${{ secrets.CRATES_IO_TOKEN }}
steps:
  - name: Publish to crates.io
    if: ${{ github.ref_type == 'tag' && env.CRATES_IO_TOKEN != '' }}
```

Note the ordering constraint this exposes: `cargo publish -p codelore-lib` cannot succeed until `codelore-rca` is live and indexed on crates.io, which is not instantaneous. The three sequential publishes need either `cargo publish --no-verify` plus a wait loop, or explicit retry, or `cargo workspaces publish`. This should be exercised against a throwaway version on a test crate before a real cut — the job currently has zero execution history.

### H3 — `BOOL_OR(is_bot)` erases a human who shares an identity with a bot

The schema comment at `facts/schema_v1.sql:119-120` states the design intent plainly: `is_bot` rides the `(name, email)` pair *"so a human and a bot sharing one email classify independently"*. Twelve consumer sites defeat that intent by collapsing the flag to the canonical identity.

Sites on `35f6bab`:

```
knowledge_islands.rs:233   SELECT canonical, BOOL_OR(is_bot) AS is_bot
top_committers.rs:86       SELECT canonical, BOOL_OR(is_bot) AS is_bot
authors.rs:112             SELECT canonical, BOOL_OR(is_bot) AS is_bot
team_composition.rs:104    HAVING NOT BOOL_OR(is_bot)
team_composition.rs:243    HAVING NOT BOOL_OR(is_bot)
bus_factor.rs:77           HAVING NOT BOOL_OR(is_bot)
bus_factor.rs:163          HAVING NOT BOOL_OR(is_bot)
knowledge/shares.rs:112    HAVING NOT BOOL_OR(is_bot)
knowledge/shares.rs:228    HAVING NOT BOOL_OR(is_bot)
knowledge/shares.rs:362    HAVING NOT BOOL_OR(is_bot)
communication.rs:70        HAVING NOT BOOL_OR(is_bot)
summary.rs:46              GROUP BY canonical HAVING NOT BOOL_OR(is_bot)
```

`BOOL_OR` over a group containing one bot row is unconditionally TRUE; `HAVING NOT BOOL_OR(is_bot)` is therefore exactly equivalent to `WHERE NOT is_bot GROUP BY canonical` **only if no canonical mixes classifications** — which is precisely the case the schema was designed to handle. Verified by execution against DuckDB 1.5.4 rather than by reading.

Three ways a canonical acquires a mixed group in practice, none exotic:

- `identity/team_map.rs:111-113` is an explicit N:1 projection. Mapping a team's members onto one canonical name folds any CI account in that team onto the same key.
- `identity/bots.rs:57-61` matches on **either** name or email. A human whose display name at some point contained a bot-ish token contributes a `true` row for their own canonical.
- `repo/gix_repo/history.rs:515-524` falls back to `canonical = raw email` when mailmap yields nothing. Shared release-automation mailboxes then merge human and bot.

**Blast radius.** `knowledge/shares.rs` feeds `code_familiarity_min`, a gate threshold. Erasing a real author from the alias set changes ownership shares, which changes familiarity, which can **move a gate verdict**. That is what makes this High rather than a reporting cosmetic.

**The odd one out.** `knowledge/shares.rs:304` uses the opposite polarity — a bare `WHERE NOT is_bot` at pair granularity, no collapse:

```sql
... WHERE NOT is_bot ORDER BY raw_email, canonical
```

So the file that owns the gate-feeding analysis is internally inconsistent with itself across three of its own four bot filters. Whichever semantics is chosen, this site and the other twelve must agree.

**Provenance correction.** My briefing document asserted that #181 introduced this. That is **false and was refuted**: #181 changed only comments in the affected files. The defect predates it. I am recording the correction rather than shipping the wrong attribution.

**Fix.** One helper, one semantics, thirteen call sites. Add a single SQL fragment (or a view in `schema_v1.sql`) — `human_aliases` — that encodes the chosen rule once, and route every consumer through it. If the schema comment's stated intent is the desired behaviour, that view filters at pair granularity and the canonical remains eligible. If the collapse is actually wanted, the schema comment is wrong and should be deleted. Either resolution is fine; the present split is not.

### H4 — The ratchet file survives metric redefinition

`quality_gates/ratchet.rs:26` and `:298-299`:

```rust
pub const RATCHET_FILENAME: &str = ".codelore-ratchet.toml";
pub fn ratchet_path(repo_root: &Path) -> PathBuf { repo_root.join(RATCHET_FILENAME) }
```

The table is `code_health_min_observed`, `red_effort_pct_observed`, `dependency_cycles_observed` — three bare floats, no version key, no schema key, no epoch.

Note the location carefully: **repo root**, not the epoch-keyed cache directory. This matters because the natural rebuttal — "the cache epoch already invalidates it" — does not apply. `cache.rs:35-53` hashes path‖head_sha‖`CARGO_PKG_VERSION`‖opts_hash‖`CACHE_EPOCH`, and the ratchet file is nowhere near that path. It is a committed artefact in the working tree with an indefinite lifetime.

Consequence: when a health metric is redefined — rescaled, recalibrated, or its corpus rebaselined — every repository carrying a `.codelore-ratchet.toml` compares a **new-scale observation against an old-scale floor**. Depending on the direction of the rescale that is either a permanently-unsatisfiable gate (every build red, no diagnostic explaining why) or a permanently-satisfied one (the ratchet silently stops ratcheting). Both are worse than an explicit reset, because both look like the tool working.

This is not hypothetical for this codebase specifically: `#176` already re-anchored the hotspot score, and the calibration corpus is a moving input.

**Fix.** Write a `schema` (or `metric_epoch`) key into the TOML on save. On load, if the key is absent or does not match the current constant, discard the table and emit one clear line: *"ratchet reset — health metric changed in vX.Y.Z; baselines will re-establish on this run."* A missing key must mean discard, not accept, so that files written before the fix are handled. This is roughly twenty lines and removes a whole class of silent, permanent, per-repository wrongness.

### H5 — `codelore analyze` is a verdict-bearing surface with no ingest guard

The audit has consistently framed CodeLore as having **five** gate entry points: `codelore check`, `codelore diff`, `codelore gate`, MCP `check_gates`, MCP `gate_changes`. Chasing a different claim, a validator surfaced a sixth that **all seven of the original analysis agents missed, including mine**: `codelore analyze`.

`crates/codelore-cli/src/analyze.rs:1133-1199` — the preflight matches exactly five states: `RepoPathMissing`, `NotARepository`, `EmptyRepository`, `OutputNotWritable`, `Ready`. There is **no commit-count check, no `is_shallow` check, and no `ensure_ingest_witnessed`**. Ingest proceeds at `:224`/`:230`/`:233` regardless.

So a repository that is technically valid but analytically empty — a shallow clone, a filtered clone, a repo whose entire history is excluded by `--exclude`, a fresh repo with one commit — passes preflight as `Ready`, ingests nothing, and produces a complete-looking report and SPA dashboard built on zero facts. Nothing in the output distinguishes "this codebase is clean" from "we measured nothing".

`analyze` is the **first command any new user runs** and the one whose output is screenshotted, shared, and trusted. It is also, in practice, the command CI wraps. Its unguardedness is more consequential than any of the five entry points that were catalogued, because those at least emit a numeric verdict a human might sanity-check.

**Correction to the surrounding claim.** My brief additionally asserted `codelore diff` had the same gap. **Refuted** — `diff` does witness. The finding is `analyze`, and only `analyze`.

**Fix.** Two options, and I recommend the second. (a) Lift `ensure_ingest_witnessed` into `analyze`'s preflight, making six call sites. (b) Do §6 instead — make the *evaluator* honest about empty input, which closes the class at one site and reaches consumers a per-entry-point witness cannot. Even with (b), `analyze` should still print a witness line, because its value is telling the user what was measured, not gating.

---

## 3. Medium-severity findings

### M2 — `base_cache_opts_digest` under-keys the diff base cache

`crates/codelore-cli/src/diff.rs:500-506`:

```rust
fn base_cache_opts_digest(min_revs: u32, exclude: &[String]) -> String {
    let mut exclude = exclude.to_vec();
    exclude.sort();
    format!("min_revs={min_revs}|exclude=[{}]", exclude.join("\0"))
}
```

The NUL separator reasoning is sound and the three unit tests at `:1032-1057` cover order-stability, `min_revs`, and `exclude`. What they cannot cover is what is absent: this digest folds in **neither `CACHE_EPOCH`, nor `CARGO_PKG_VERSION`, nor the fact-schema version** — all three of which `cache.rs:35-53` correctly includes for the main cache. A diff base computed by an older binary, or under an older analysis definition, is therefore reused verbatim by a newer one. The main cache invalidates; its sibling does not. Fix by reusing the same key-construction helper rather than hand-rolling a second one.

### M3 — MCP reports a skip reason that is not the reason

The MCP surface reports `corpus_percentile_max` as skipped for want of data. Validation established that **this gate needs no additional data at all** — the inputs it requires are already present in every report that reaches the skip branch. Users acting on the stated reason will go collect data that changes nothing, conclude the tool is broken, and disable the gate. A wrong diagnostic is worse than none, because it is actionable in the wrong direction. Fix the branch condition, and while there, audit the other skip reasons for the same class of copy-paste drift.

### M4 — `delta_code_health_min` compares two different metrics

Originally filed as a population mismatch. Validation **narrowed and corrected** it: the divergence is a **metric** divergence. One side of the comparison is `cognitive_health`, which lives on `[60, 100]`; the other is `score`, on `[0, 100]`. A delta computed across that boundary is not a health delta in any unit, and a threshold expressed against it is not meaningful. The gate at `evaluators.rs:240-255` is well-formed Rust computing a quantity with no interpretation. Pick one metric, name it in the threshold's documentation, and assert the invariant in a test.

### M5 — The `NotARepository` remedy points the user at the wrong action

`output/banner.rs:152-159`:

```rust
Preflight::NotARepository { repo_path } => (
    s(fail), "✗", "not a git repository".to_string(),
    Some(format!("run `git init` in {repo_path}, or pass --repo to a git-managed directory")),
),
```

`GixRepo::open` uses `gix::open`, which does **not** search parent directories; there are **zero** `gix::discover` call sites in the tree; and `--repo` defaults to `"."`. So the overwhelmingly common way to hit this state is running `codelore` from a subdirectory of a perfectly good repository. `git init` is then exactly the wrong advice — at best it does nothing useful, at worst the user runs it and creates a nested repository inside their real one, which is a genuinely annoying mess to unpick.

**Correction:** the validator **refuted all three of the specific scenarios I originally offered** as illustrations, while confirming the underlying thesis. Reported here on the confirmed thesis only. Fix: either adopt `gix::discover` so the subdirectory case simply works (preferable — it matches every other git-aware tool's behaviour), or reword the remedy to lead with *"run from the repository root, or pass `--repo <path>`"* and mention `git init` last if at all.

### M6 — The docs promise ingest-witness behaviour in seven places

Independent of whether the witness fires (H5), the behaviour is **documented as existing in seven locations**, `action.yml` among them. Users configuring CI from the documentation will build pipelines on a guarantee the code does not uniformly provide. Whichever way H5 is resolved, these seven sites must be reconciled in the same change — this is the concrete instance of the standing "ALL related docs are updated accordingly" requirement.

### M7 — `codelore-lib`'s tests depend on a feature it does not declare

Filed originally as a `cargo test` breakage. **Refuted at the workspace root**: Cargo resolver v2's dev-dependency feature exemption does not apply when building tests, so a root-level `cargo test` unifies `test-support` through `codelore`'s dependency edge and everything compiles. The finding survives in reduced form: **`cargo test -p codelore-lib` alone** relies on that unification being present, and it is not. `crates/codelore-cli/Cargo.toml:65` is the only edge that enables the feature — and #186 renamed that package to `codelore` without changing the edge, so the prop is intact but now less obvious. Any contributor testing a single crate, and any future CI matrix that shards by crate, hits it. Fix by declaring the dev-dependency on `codelore-lib` itself.

---

## 4. Low and informational

### L1 — Every SPA badge renders green

`output/spa/template.html:1026-1038`, verified **byte-identical on `35f6bab`** (#187 touched widget sizing and periphery colour, not this rule):

```css
.badge {
  display: inline-block; margin-left: 6px; padding: 2px 6px;
  font-size: 10px; font-weight: 500; letter-spacing: 0.02em;
  color: var(--accent);
  background: rgba(46, 164, 79, 0.12);
  border: 1px solid rgba(46, 164, 79, 0.3);
  border-radius: 3px; vertical-align: 1px;
}
```

Symptom **confirmed**; my stated mechanism **refuted**. It is not source order. DaisyUI is inlined into cascade layers at `template.html:54` while this hand-rolled sheet at `:56` is unlayered, and per CSS Cascade 5 **unlayered author declarations beat all layered author declarations regardless of specificity or source order**. DaisyUI 5 additionally sets badge colour through a `--badge-color` custom-property indirection, which this rule never touches. So `badge-error`, `badge-warning` and `badge-success` produce identical green pills. The markup is correct — v0.24.0's fix to the DOM was real — but the render is unchanged, which is why the "FIXED" claim is only half true.

Fix: either wrap the legacy sheet in `@layer legacy` so DaisyUI's layers can compete, or scope the rule to `.badge:not([class*="badge-"])`. Worth doing because a status badge that is always green is worse than no badge.

*(Method note: the first validation attempt at stylesheet substitution was invalid — the first `<style>` literal in the file sits inside an HTML comment at offset 2792, so slicing there mangled the head and dropped the legacy sheet entirely. Caught by CSSOM inspection showing only one stylesheet where two were expected, then redone at the real offsets 3004 and 93932. Recording this because the wrong version of the experiment would have produced a confident false negative.)*

### L2 — Stale calibration corpus — **downgraded High → Low**

Filed as High. Validation demonstrated that the anchored hotspot score's `p0` term — the corpus trivial share in `10 · pr_rev · cp_tail²`, `cp_tail = clamp((cp − p0)/(1 − p0), 0, 1)` — is **provably invariant** across the corpus refresh in question, and that **0.0000%** of scored entities change. The corpus should still be refreshed on a schedule for the usual reasons, but it is not a defect and nothing downstream is wrong today. Downgraded on evidence.

### L3 — Degraded-witness over-fire — **downgraded High → Low**

Filed as High on the theory that the witness fires in cases it should not. Validation established the divergence is **deliberate and documented**. The behaviour matches its specification; the specification is defensible. Downgraded to a documentation-clarity note.

### L4 — The mailmap differential test cannot see an email-only regression

`crates/codelore-lib/tests/differential_repo_test.rs:171-197` is **genuinely differential** — it compares gix's inline `gix_mailmap` against a real `git check-mailmap` shell-out, and the fixture carries three live rules. My claim that it was vacuous is **refuted**. The reduced finding stands: all four-token probes use non-matching names, so an **email-only** mailmap regression is invisible in 8 of 8 probes. One added probe with a matching name and a differing email closes it. Low, because the oracle is real and a name-side regression would be caught.

### I1 — `SCHEMA_VERSION` is dead, stale, and tested tautologically

`crates/codelore-lib/src/types.rs:38`:

```rust
pub const SCHEMA_VERSION: u8 = 6;
```

Re-exported publicly at `lib.rs:46`. The live constant is `facts/schema.rs:10`, `CURRENT_SCHEMA_VERSION = "7"`, which is what actually lands in the `meta` table at `:13`. So the public API of `codelore-lib` exports a version number that is both unused and **wrong by one**.

Worse, `tests/types_test.rs:6-13` contains `fn schema_version_is_six`, whose comment claims *"Cache key includes this sentinel."* That is factually false — `cache.rs:35-53` does not reference it. The test asserts `6 == 6` and its comment documents a mechanism that does not exist, so it actively defends the staleness against anyone who tries to reconcile the two constants.

This is squarely in the standing "no unused or legacy code" requirement. Delete the constant, its re-export, and the test. If a public schema-version accessor is wanted, re-export `CURRENT_SCHEMA_VERSION`.

### I2 — Depth-1 non-merge clone — **REFUTED**

I claimed a depth-1 clone yields a silent empty gate. **Refuted by execution**: the hard-error paths fire first and the process exits 3 with a diagnostic. Recorded as refuted so it is not re-raised next cycle.

---

## 5. The honesty ledger

The validators falsified **five premises of my own briefing document**. Listing them because a report whose method section claims adversarial validation should show what the adversary caught:

1. **"#181 introduced the alias collapse."** False — #181 changed only comments; the defect predates it.
2. **"The badge bug is source order."** False — it is cascade layers (unlayered beats layered), plus DaisyUI 5's `--badge-color` indirection.
3. **"Depth-1 non-merge produces a silent empty gate."** False — hard-errors exit 3 first.
4. **"The mailmap differential test is vacuous."** False — it is genuinely differential; only the email-only probe gap is real.
5. **"`cargo test` is broken by the feature gap."** False at the workspace root; true only for `-p codelore-lib`.

And one hypothesis of mine that I refuted myself before it reached a validator: I suspected #186's hardcoded path-dep pins (`codelore-rca = { path = "../codelore-rca", version = "0.24.0" }`) would go un-bumped by `cut-release.sh` and ship a mismatched dependency graph to crates.io. **They are swept** — `cut-release.sh:341-364` contains a dedicated Python block that rewrites `codelore-*` path-dep version literals across `crates/*/Cargo.toml`, and `:410` stages them. Not a finding.

Two prior-cycle claims of mine also carried into this report as corrections rather than being quietly dropped: the "25+ smells" figure (actual: 8) and the CodeScene MCP tool count (actual: 10, not 24).

**Severity corrections to cycle 3 itself.** Cycle 3's §8 severity assignments contradicted its own §7 blast-radius rule. Applying the rule consistently: **A2-1 Medium → High**, **A1-19 Medium → High**, **A8-1 Medium → High**, **A7-1 Medium → Medium-High**, and **every S-item down to Low or informational**. The S-items were graded on code elegance; none of them can move an exit code.

---

## 6. The one fix to make first

`crates/codelore-lib/src/quality_gates/evaluators.rs:240-300`. Four diff gates. Their dispositions on empty input are not the same, and the asymmetry is invisible:

| Gate | Guard | Empty-input result |
|---|---|---|
| `delta_code_health_min` | `let (Some(base), Some(projected))` | **skipped** (correct) |
| `delta_code_health_min_per_file` | `for row in &report.health.deltas` | **passes silently** |
| `new_file_health_min` | `for row in &report.health.deltas` | **passes silently** |
| `no_new_cycles` | `for path in &report.newly_cyclic_paths` | **passes silently** |

A `for` loop over an empty collection pushes no violation, and no violation is indistinguishable from a satisfied gate. Three of four gates therefore report **green when nothing was measured**. The first gate gets it right by accident of being written with a `let`-chain rather than a loop.

This single site is the highest-leverage fix in the report, and it is worth stating why plainly:

- **It closes the whole class.** Every path that can produce an empty change-set — shallow clone, filtered clone, everything excluded, ingest that silently found nothing, the unguarded `analyze` of H5 — currently converges on "green". One change fixes all of them at once.
- **It reaches consumers a per-entry-point witness cannot.** Lifting `ensure_ingest_witnessed` into five or six call sites guards those entry points. It does not help anything that constructs a `ChangeSetReport` by another route, and it does not make the *result* self-describing. A gate that reports `skipped` carries its own honesty wherever it travels — into JSON output, into the MCP response, into the SPA.
- **It is small.** One `is_empty()` check per gate, plus a `GateOutcome::Skipped` variant if one does not already exist, plus the plumbing to render it.
- **It converts a silent wrong answer into a loud correct one.** "Skipped: no files in change-set" is a sentence a user can act on. Green is not.

The follow-on is a policy decision worth making explicitly and documenting: **should a skipped gate fail a strict CI run?** I would expose it — `--fail-on-skipped` or a `[gates] treat_skipped_as` key — and default it to *warn*, so existing pipelines do not break but the information becomes reachable. Silent-pass should not remain available as a behaviour at all.

---

## 7. Improvement options beyond the defects

Design proposals, not validated defects. Ordered by value-to-effort.

**One bot-filter helper.** H3's thirteen sites should become one `human_aliases` view in `schema_v1.sql`, consumed everywhere. This is the difference between fixing a bug and removing the ability to reintroduce it. It also makes the schema comment enforceable rather than aspirational.

**A witness line in every output, not a gate in every entry point.** Rather than N preflight checks, have the report itself carry `facts_witnessed: { commits, files, authors, window }` and render it in the SPA header, the CLI banner, and every MCP response. A user who can see "0 commits ingested" at the top of a dashboard does not need the tool to guess whether that is an error.

**Epoch-key every persisted artefact, uniformly.** H4 (ratchet) and M2 (diff base cache) are the same mistake in two places, and `cache.rs` already contains the correct pattern. Extract it into one `artifact_key(kind)` helper and route the ratchet, the diff base cache, and the main cache through it. Then the invariant is structural rather than remembered.

**Exercise the release pipeline before trusting it.** H1 and H2 are both defects in code that only ever runs during a release, which is the worst possible place for zero test coverage. `act` or a scratch repository with a throwaway tag would have caught both. A `--dry-run` mode for `cut-release.sh` exists; extend it to assert that the run it selected actually matches `RELEASE_SHA`, and add a workflow-lint step (`actionlint`) to CI, which flags H2's context misuse mechanically.

**Cascade-layer the legacy stylesheet.** Beyond L1's badges, an unlayered hand-rolled sheet sitting above a layered framework means *any* future DaisyUI component the legacy sheet happens to name is silently overridden. Wrapping `template.html:56` in `@layer legacy` is a one-line change that converts a standing hazard into ordinary specificity.

**Give `codelore analyze` an empty-result mode.** When zero facts are witnessed, the right output is not a dashboard with empty widgets — it is a short diagnostic explaining what was looked for, what filters were applied, and what to try next. This is the single biggest first-run experience improvement available, because the empty dashboard is what a new user sees when their clone is shallow.

**Deferred, carried forward.** The competitive positioning pass should be re-run in one sitting before any positioning copy ships (the previous pass was fragmented across sessions and several source pages were unreachable). `rmcp`'s current version needs confirming with `cargo info rmcp` — three sources disagreed and the manifest pin was taken on faith. Neither blocks anything here.

---

## 8. Docs to update with these fixes

Per the standing requirement that all related docs move with the code:

- The **seven** locations documenting ingest-witness behaviour, `action.yml` among them (M6) — reconcile with whatever H5/§6 resolves to.
- `facts/schema_v1.sql:119-120` — the "classify independently" comment is currently false; it becomes true only if H3 is fixed in the pair-granularity direction, and must be deleted otherwise.
- `tests/types_test.rs:6-13` — the "Cache key includes this sentinel" comment is false today regardless of what happens to I1.
- Threshold documentation for `delta_code_health_min` — must name the metric and its range once M4 is resolved.
- Gate documentation generally — needs a table of every gate's empty-input disposition, which does not currently exist anywhere and is the reason §6's asymmetry went unnoticed.
- Release runbook — H1 and H2's fixes change the operator-visible procedure.

---

## 9. Method and limits

**Repo-state discipline.** Local `HEAD` and `git rev-parse origin/main` were recorded before and after the audit; both are `35f6bab`. All reading was done via `git archive <sha>` from the local object store; **no branch state was mutated at any point**. When main moved past the audit target mid-cycle, every affected finding was re-verified against the new tree rather than carried forward on assumption — that is how H2 was found and how L1's byte-identical status was established.

**Adversarial validation.** Five validators, each with a **default verdict of REFUTED** and a requirement that confirmation rest on execution or primary documentation, not on reading comprehension. DuckDB semantics were settled by running queries against DuckDB 1.5.4. The badge cascade was settled by CSSOM inspection in a real engine. H1 was settled by executing the actual `jq` expression. H2 was settled against GitHub's published context-availability table. Findings that could not be established this way were downgraded (L2, L3) or refuted (I2, and the four other entries in §5).

**What could not be verified here, and why.** `rustc 1.95.0` is installed in this environment but the workspace pins `1.96.0` via `rust-toolchain.toml`, and `static.rust-lang.org` is proxy-blocked, so **the workspace cannot be compiled in this session**. No finding in this report rests on a `cargo` run. M7 in particular is reasoned from Cargo's documented resolver-v2 semantics and the manifest graph, not from an observed build failure — it should be confirmed with one `cargo test -p codelore-lib` on a machine that can build. Several external documentation sources (`jsdelivr`, `unpkg`, `codescene.io`) returned proxy denials; those were reported as gaps, never worked around by another fetch method.

---

## 10. Housekeeping

Two items on your machine that I cannot action myself:

- `_to_delete/` holds roughly **54 MB** — `cowork_main172.tar`, `.cowork_main170.tar`, older tarballs, and a stale `index.lock`. The device bridge cannot delete files (`rm` returns "Operation not permitted"), so removal is yours.
- Branch **`docs/hardening-cycle-3`** at `64955cb` is unpushed and now redundant — that report reached main via #184. Safe to delete.

Still open from an earlier cycle and not started: the `.devt` scripts → Rust migration question you raised, which I never got to answer.
