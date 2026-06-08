# CodeLore — Bugfix Sprint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close 8 validated correctness / silent-failure bugs surfaced by the two audits committed earlier today (`docs/updated_analysis_report.md` + `docs/modernization_audit_2026-06-08.md`). Every item in this plan is a real bug shipped against `main` that produces wrong output, drops data silently, or undermines a stated CodeLore value proposition. Each finding was validated by grep against the live source before being added to this plan.

**Architecture:** Single-fix-per-task. Each task is small (~5–50 LOC + tests + 1 commit). Most fixes are localized — only one bug (cache key + provenance manifest) is a joint fix because the two surface bugs share a root cause. Land each as an atomic commit; run `cargo test --workspace --all-features && cargo clippy --workspace --all-targets --all-features -- -D warnings` before every commit.

**Tech Stack:** No new dependencies. All fixes use existing crates (`duckdb`, `serde`, `serde_json`, `regex`, `time`, `tracing`).

---

## Non-Goals (deliberate scope cuts)

- **Code-maat feature gaps** — covered by `docs/superpowers/plans/2026-06-08-codelore-code-maat-parity.md`. This plan is bugfix-only.
- **Modernization items** — `format!` SQL sweep, ENUM types, SARIF coverage expansion, etc. — covered by `docs/superpowers/plans/2026-06-08-codelore-modernization-sprint.md`.
- **Already-tracked v1.x backlog** — rename tracking, Options builder, CSV crate, parallel clone walk. Listed in `docs/codebase_analysis_report.md`; touched only if a bugfix here naturally subsumes one (none currently do).
- **`code-maat` compat behaviors** — preserving legacy quirks is explicitly out of scope per [[feedback-modernize-dont-migrate]]. Bugs get fixed; the modern surface IS the spec.

---

## File Structure (modifications)

**Modified files:**
- `crates/codelore-lib/src/analyses/clone_coupling.rs` — Task 1 (p_value fix)
- `crates/codelore-lib/src/analyses/hotspots.rs` — Task 2 (drop empty name)
- `crates/codelore-lib/src/analyses/code_health.rs` — Task 2 (drop empty name)
- `crates/codelore-lib/src/output/csv.rs` — Task 2 (CSV header sync)
- `crates/codelore-lib/src/output/json.rs` — Task 2 (struct serialization)
- `crates/codelore-lib/src/output/markdown.rs` — Task 2 (header sync)
- `crates/codelore-lib/src/output/sarif.rs` — Task 2 (fingerprint impact) + Task 5 (CODELORE-MISSING-COCHANGE rule)
- `crates/codelore-lib/src/analyses/churn.rs` — Task 3 (author_churn tertiary sort)
- `crates/codelore-lib/src/cache.rs` — Task 4 (canonical opts serialization)
- `crates/codelore-lib/src/provenance/mod.rs` — Task 4 (Manifest uses same helper)
- `crates/codelore-cli/src/diff_output.rs` — Task 5 (emit coupling absences to SARIF)
- `crates/codelore-lib/src/identity/bots.rs` — Task 6 (AI pattern extension)
- `crates/codelore-cli/src/diff.rs` — Task 7 (worktree prune on startup)
- `crates/codelore-cli/src/main.rs` — Task 7 (startup hook)

**New test files:**
- `crates/codelore-lib/tests/clone_coupling_p_value_test.rs` — Task 1
- `crates/codelore-lib/tests/cache_clone_options_test.rs` — Task 4
- `crates/codelore-lib/tests/provenance_clone_options_test.rs` — Task 4
- `crates/codelore-cli/tests/diff_sarif_absence_test.rs` — Task 5
- `crates/codelore-lib/tests/ai_attribution_modern_test.rs` — Task 6
- `crates/codelore-cli/tests/diff_worktree_prune_test.rs` — Task 7

---

## Task ordering (intentional — each builds on the prior)

1. **Task 1: `clone_coupling.rs` p_value=0.0** — most explosive (silently wrong differentiator output). Smallest fix.
2. **Task 2: Drop empty `name` column from hotspots + code_health** — visible in every CSV/JSON/SARIF; cleanest as one task because both analyses share the pattern and same emitters.
3. **Task 3: `author_churn` tertiary sort** — 1 line. Eliminates one source of test flake.
4. **Task 4: Canonical Options serialization for cache key + provenance manifest (joint)** — root cause for two reported bugs; fixes both via one helper. Prevents recurrence.
5. **Task 5: SARIF `CODELORE-MISSING-COCHANGE` rule + emit coupling absences** — the strategic differentiator finally reaches Code Scanning.
6. **Task 6: AI-attribution patterns updated** — adds Cursor / Aider / Cody / Continue / Codeium / Windsurf / Devin patterns; case-insensitive substring match.
7. **Task 7: Worktree prune on startup** — hygiene; eliminates the silent-orphan failure mode.

Stop after Task 7. The remaining audit items go to the modernization plan.

---

## Task 1: `clone_coupling.rs` ships fake `p_value` — use real `cp.fisher_p`

**Validation evidence (2026-06-08):** `crates/codelore-lib/src/analyses/clone_coupling.rs:186` has `let approximated_p = 0.0;` with a comment claiming "already passed Fisher filter". The `cp` (CouplingRow) in scope at the HashMap probe carries `fisher_p` (verified at `coupling.rs:31` — the field is `pub fisher_p: f64`). Every live-clone row currently ships `p_value: 0.0` in CSV/JSON/SARIF output. The combined_score ranking is `similarity × degree_pct × (1 − 0.0)` = `similarity × degree_pct`, meaning p-value contributes nothing to ranking despite being the whole point of carrying it.

**Files:**
- Modify: `crates/codelore-lib/src/analyses/clone_coupling.rs` (lines ~180-195)
- Test: `crates/codelore-lib/tests/clone_coupling_p_value_test.rs` (new)

- [ ] **Step 1: Write the failing test**

  ```rust
  // crates/codelore-lib/tests/clone_coupling_p_value_test.rs
  use codelore_lib::{Options, analyses::clone_coupling::run_clone_coupling};
  use codelore_lib::test_support::differential_repo;

  #[test]
  fn clone_coupling_carries_real_fisher_p_value() {
      let repo = differential_repo::build();
      let db = /* ingest fixture */;
      let opts = Options { repo_path: repo.dir.path().into(), ..Options::default() };

      let rows = run_clone_coupling(&db, &opts).unwrap();
      assert!(!rows.is_empty(), "fixture should produce at least one live-clone pair");

      // The bug: every row had p_value=0.0 regardless of the underlying coupling.
      // After the fix: at least one row must have a non-zero p_value (the Fisher
      // gate is `p < 0.05` so surviving rows have p in (0, 0.05)).
      let nonzero_p_values: Vec<_> = rows.iter().filter(|r| r.p_value > 0.0).collect();
      assert!(
          !nonzero_p_values.is_empty(),
          "all rows had p_value=0.0 — Fisher p-value was not propagated from CouplingRow"
      );

      // combined_score must now factor in p-value, not just similarity*degree
      for row in &rows {
          let expected = row.similarity * row.degree_pct * (1.0 - row.p_value);
          assert!((row.combined_score - expected).abs() < 1e-9,
              "combined_score does not match similarity * degree_pct * (1 - p_value)");
      }
  }
  ```

- [ ] **Step 2: Run the test to confirm it fails**

  `cargo test -p codelore-lib --test clone_coupling_p_value_test` → FAIL on the nonzero-p-values assertion.

- [ ] **Step 3: Apply the fix**

  In `crates/codelore-lib/src/analyses/clone_coupling.rs`, replace the relevant lines:

  ```rust
  // BEFORE:
  let approximated_p = 0.0; // already passed Fisher filter
  let combined_score = p.similarity * degree_pct * (1.0 - approximated_p);

  // AFTER:
  // Carry the real Fisher p-value through from the coupling probe — earlier
  // versions zeroed this out, claiming the gate made it redundant, but the
  // value matters both for ranking (lower p ⇒ stronger signal) and for output
  // honesty (the field is in every CSV/JSON/SARIF row).
  let fisher_p = cp.fisher_p;
  let combined_score = p.similarity * degree_pct * (1.0 - fisher_p);
  ```

  And update the row construction:

  ```rust
  // ... in the rows.push(CloneCouplingRow { ... })
  p_value: fisher_p,
  combined_score,
  ```

- [ ] **Step 4: Run test, then full suite**

  `cargo test -p codelore-lib --test clone_coupling_p_value_test` → PASS.
  `cargo test --workspace --all-features` → all 350 tests pass (we added one).
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` → clean.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/codelore-lib/src/analyses/clone_coupling.rs crates/codelore-lib/tests/clone_coupling_p_value_test.rs
  git commit -m "$(cat <<'EOF'
  fix(lib): clone-coupling ships real fisher_p instead of hard-coded 0.0

  Every live-clone row in CSV/JSON/SARIF output has been shipping p_value=0.0
  since clone-coupling first shipped. The clone_coupling::run code zeroed out
  the p-value with the comment "already passed Fisher filter" and used the
  zero in combined_score = similarity * degree_pct * (1 - 0.0).

  This is wrong on two axes: (1) output honesty — we advertise the field in
  every row of every format and the value is fake; (2) ranking — pairs at
  p=0.001 and pairs at p=0.04 score identically when the whole point of
  passing through Fisher is that lower p = stronger signal.

  The real fisher_p is in scope as cp.fisher_p (CouplingRow carries it; the
  HashMap probe table threads it through). Fix is one line in the
  let-binding plus updating the row construction.

  Headline live-clone output now ranks pairs by Fisher significance as
  designed; CSV/JSON/SARIF p_value fields are now accurate.
  EOF
  )"
  ```

---

## Task 2: Drop empty `name` column from `hotspots` + `code_health` rows

**Validation evidence (2026-06-08):** `crates/codelore-lib/src/analyses/hotspots.rs:99` has `SELECT path, '' AS name, ...`. Same pattern at `crates/codelore-lib/src/analyses/code_health.rs:113`. `HotspotRow` struct carries `pub name: String` (verified in earlier session). CSV header from `output/csv.rs` is `entity,name,revisions,cognitive,code-health,hotspot-score` — advertises a column we always populate with `""`. Same for code-health.

**Files:**
- Modify: `crates/codelore-lib/src/analyses/hotspots.rs` (SQL + Row struct)
- Modify: `crates/codelore-lib/src/analyses/code_health.rs` (SQL + Row struct)
- Modify: `crates/codelore-lib/src/output/csv.rs` (drop `name` from headers)
- Modify: `crates/codelore-lib/src/output/json.rs` (struct serialization auto-updates)
- Modify: `crates/codelore-lib/src/output/markdown.rs` (drop column from tables)
- Modify: `crates/codelore-lib/src/output/sarif.rs` (verify `name` not used in fingerprints or location)
- Modify: existing snapshot tests if any (search `entity,name,revisions`)

- [ ] **Step 1: Run failing-test sweep first**

  Grep for any test asserting against the `name` column literally:

  ```bash
  rg -t rust 'entity,name,revisions|HotspotRow\s*\{[^}]*name' crates/codelore-lib/
  ```

  Catalogue each match — these tests need their assertions updated.

- [ ] **Step 2: Drop `name` from `HotspotRow` struct**

  ```rust
  // BEFORE:
  pub struct HotspotRow {
      pub path: String,
      pub name: String,   // always "" — drop
      pub revisions: u32,
      pub cognitive: f64,
      pub code_health: f64,
      pub hotspot_score: f64,
  }

  // AFTER:
  pub struct HotspotRow {
      pub path: String,
      pub revisions: u32,
      pub cognitive: f64,
      pub code_health: f64,
      pub hotspot_score: f64,
  }
  ```

  And drop the SQL's `'' AS name,` projection at hotspots.rs:99. Same for code_health.rs:113 + struct field.

- [ ] **Step 3: Update CSV emitter**

  `write_hotspots_csv` header `entity,name,revisions,...` → `entity,revisions,...`. Loop body drops the empty middle column. Same for code_health.

- [ ] **Step 4: Update Markdown emitter**

  Drop the `| Name |` column header + body cell. Same for code_health.

- [ ] **Step 5: Verify SARIF impact**

  Read `sarif.rs::build_hotspot_result` — confirm `name` field isn't used in `partialFingerprints` or `physicalLocation`. If it is, we need a new fingerprinting scheme (versioned migration). Most likely it's not used (the location uses `path`); confirm by reading the function.

- [ ] **Step 6: Update existing snapshot/golden tests**

  Tests found in Step 1 — update each. `code_maat_parity_test.rs` may have an expected CSV body that includes the empty column.

- [ ] **Step 7: Run all tests + commit**

  ```bash
  cargo test --workspace --all-features
  ```

  Should be green. Commit:

  ```bash
  git commit -m "$(cat <<'EOF'
  fix(lib): drop always-empty `name` column from hotspots + code-health rows

  HotspotRow and CodeHealthRow carry a `name: String` field that the SQL
  populates with `'' AS name` — every row in every output format ships an
  empty middle column that the CSV header advertises as if it contained data.

  This was a code-maat-era hangover: code-maat's tuple-typed datasets used the
  name slot for sub-file entity names (function, class) but our analyses are
  per-path and the slot was never wired. In a Rust codebase with typed row
  structs there's no reason to carry the dead column.

  Drop the field from the struct, the SQL projection, and the CSV/Markdown
  emitters. SARIF unaffected (location uses `path`; fingerprints don't
  reference `name`). Snapshot tests updated to match the new schema.

  Output schema change — users reading hotspots/code-health CSV verbatim
  will see one fewer column. Documented in CHANGELOG.
  EOF
  )"
  ```

---

## Task 3: `author_churn` deterministic tertiary sort

**Validation evidence (2026-06-08):** `crates/codelore-lib/src/analyses/churn.rs:81` SQL is `ORDER BY added DESC, commits DESC{limit}` — two columns. No tertiary `author ASC` sort. Two authors with identical `added`+`commits` flip non-deterministically. Other 11 analyses in the same workspace have tertiary sorts; this one is an outlier.

**Files:**
- Modify: `crates/codelore-lib/src/analyses/churn.rs` (one-line SQL edit)
- Test: extend existing `crates/codelore-lib/tests/churn_test.rs` with a tie-breaker assertion

- [ ] **Step 1: Write the failing test**

  Build a fixture where two authors have identical churn (e.g., Alice and Bob both add 100 lines in 1 commit each). Assert that across 10 runs the output ordering is stable (Alice before Bob, since `A < B` alphabetically).

- [ ] **Step 2: Apply the fix**

  `ORDER BY added DESC, commits DESC{limit}` → `ORDER BY added DESC, commits DESC, author ASC{limit}`.

  Also check `abs_churn` (line ~47 per audit). Even though dates are grouped and thus unique in practice, add the tertiary sort to match the project's stated style: `ORDER BY date DESC, added DESC, deleted DESC`.

- [ ] **Step 3: Test + commit**

  ```bash
  cargo test -p codelore-lib --test churn_test
  cargo test --workspace --all-features
  ```

  ```bash
  git commit -m "$(cat <<'EOF'
  fix(lib): author_churn + abs_churn deterministic tertiary sort

  author_churn SQL was `ORDER BY added DESC, commits DESC` — two columns,
  no tertiary tie-break. Two authors with identical churn flip positions
  between runs. The other 11 analyses in this workspace all have a
  deterministic tertiary sort; author_churn was the outlier.

  Add `, author ASC` to author_churn and `, added DESC, deleted DESC` to
  abs_churn (the latter is safe today — dates are GROUP-BY-unique — but
  matching the project style prevents future regression if grouping
  changes).

  Eliminates one source of golden-test flake and ensures SARIF
  partialFingerprints based on row order are stable across runs.
  EOF
  )"
  ```

---

## Task 4: Canonical Options serialization — joint fix for cache key + provenance manifest

**Validation evidence (2026-06-08):** `cache.rs:81 opts_hash` serializes 11 named fields; `provenance/mod.rs Manifest` carries 18 named fields. Neither captures the 5 clone-detection options (`min_clone_node_count`, `exclude_patterns`, `min_clone_shared_revs`, `clone_similarity_floor`, `clone_skip_same_dir`) or other Options fields that grew after the original lists were authored (`max_coupling_pct`, `group_file`, `team_map_file`, `temporal_period_days`, `strict_grouping`). Root cause: hand-curated subsets that drift as `Options` grows.

**Strategy:** Replace BOTH surface sites with a single helper `Options::canonical_serialization()` that JSON-serializes the whole `Options` struct (with sorted Vec fields for stability). cache.rs hashes the result; provenance/mod.rs records it verbatim.

**Files:**
- Modify: `crates/codelore-lib/src/options.rs` — add `canonical_serialization()` method
- Modify: `crates/codelore-lib/src/cache.rs` — replace `opts_hash` body with `canonical_serialization().hash()`
- Modify: `crates/codelore-lib/src/provenance/mod.rs` — Manifest carries the canonical JSON
- Test: new `crates/codelore-lib/tests/cache_clone_options_test.rs`
- Test: new `crates/codelore-lib/tests/provenance_clone_options_test.rs`

- [ ] **Step 1: Add `Serialize` to `Options` if not already**

  Verify `Options` derives `Serialize`. If not, add it. Check that all field types are `Serialize`-able (`PathBuf` is, `Vec<String>` is, `time::Date` needs `serde` feature — already enabled per Cargo.toml).

- [ ] **Step 2: Write the failing cache-collision test**

  ```rust
  // crates/codelore-lib/tests/cache_clone_options_test.rs
  use codelore_lib::{Options, cache::cache_key};
  use std::path::PathBuf;

  fn opts(min_clone_node_count: u32) -> Options {
      Options {
          repo_path: PathBuf::from("/tmp/repo"),
          min_clone_node_count,
          ..Options::default()
      }
  }

  #[test]
  fn cache_key_changes_when_min_clone_node_count_changes() {
      let k1 = cache_key(&opts(30).repo_path.clone(), "deadbeef", &opts(30));
      let k2 = cache_key(&opts(50).repo_path.clone(), "deadbeef", &opts(50));
      assert_ne!(k1, k2, "cache_key MUST change when min_clone_node_count changes");
  }

  #[test]
  fn cache_key_changes_when_exclude_patterns_change() {
      let mut a = opts(30); a.exclude_patterns = vec!["vendor/**".into()];
      let mut b = opts(30); b.exclude_patterns = vec!["target/**".into()];
      let ka = cache_key(&a.repo_path.clone(), "x", &a);
      let kb = cache_key(&b.repo_path.clone(), "x", &b);
      assert_ne!(ka, kb, "cache_key MUST change when --exclude patterns change");
  }

  // ... 3 more tests for clone_similarity_floor, min_clone_shared_revs, clone_skip_same_dir
  ```

  Run: all 5 tests FAIL (cache_key is identical for the different option values — the bug is shipped).

- [ ] **Step 3: Add canonical serialization helper**

  ```rust
  // crates/codelore-lib/src/options.rs

  impl Options {
      /// Stable JSON serialization of the full Options for cache-keying and
      /// provenance recording. Vec fields are sorted so insertion-order
      /// doesn't perturb the output.
      pub fn canonical_serialization(&self) -> String {
          // Clone + sort Vec fields so the serialized form is order-independent
          let mut snapshot = self.clone();
          snapshot.exclude_patterns.sort();
          // Sort any other Vec fields as they're added
          serde_json::to_string(&snapshot)
              .expect("Options serialization cannot fail (all fields are Serialize)")
      }
  }
  ```

- [ ] **Step 4: Replace `opts_hash` in cache.rs**

  ```rust
  fn opts_hash(opts: &Options) -> String {
      opts.canonical_serialization()
      // Note: cache_key() further hashes this with SHA-256, so the string
      // length doesn't matter and we don't need to truncate.
  }
  ```

  Delete the 30-line hand-curated field list above.

- [ ] **Step 5: Update `Manifest::capture` to record the canonical serialization**

  ```rust
  pub struct Manifest {
      pub codelore_version: String,
      pub gix_version: String,
      pub arrow_version: String,
      pub duckdb_version: String,
      pub run_started_at: String,
      pub repo_path: String,
      pub analysis: String,
      /// Canonical JSON of every Options field at run time. Replaces the
      /// previous hand-curated 11-field subset that silently omitted clone
      /// options and other fields added after the original schema.
      pub options: serde_json::Value,
  }

  impl Manifest {
      pub fn capture(_db: &FactsDb, opts: &Options, analysis: &str) -> Result<Self> {
          // ...
          Ok(Manifest {
              codelore_version: env!("CARGO_PKG_VERSION").to_string(),
              // ... other version fields ...
              analysis: analysis.to_string(),
              options: serde_json::from_str(&opts.canonical_serialization())
                  .expect("canonical_serialization produces valid JSON"),
          })
      }
  }
  ```

  This is a breaking change to the provenance JSON schema. Old: 18 named fields. New: nested `options: { ... }` object. Document in CHANGELOG as a schema version bump.

- [ ] **Step 6: Write provenance test**

  ```rust
  // crates/codelore-lib/tests/provenance_clone_options_test.rs
  #[test]
  fn manifest_records_clone_detection_options() {
      let opts = Options {
          min_clone_node_count: 42,
          exclude_patterns: vec!["foo/**".into()],
          clone_similarity_floor: 0.85,
          ..Options::default()
      };
      let manifest = Manifest::capture(&db, &opts, "clones").unwrap();
      let opts_json = &manifest.options;
      assert_eq!(opts_json["min_clone_node_count"], 42);
      assert_eq!(opts_json["exclude_patterns"], serde_json::json!(["foo/**"]));
      assert_eq!(opts_json["clone_similarity_floor"], 0.85);
  }
  ```

- [ ] **Step 7: Update existing tests that asserted against the old Manifest schema**

  Grep for `Manifest {` / `manifest.min_revs` / similar — there are existing tests that construct Manifest by struct literal or assert against individual fields. Update each.

- [ ] **Step 8: Run all tests + commit**

  ```bash
  cargo test --workspace --all-features
  ```

  ```bash
  git commit -m "$(cat <<'EOF'
  fix(lib): canonical Options serialization — fixes cache collision + stale provenance

  Two reported bugs shared a root cause: cache.rs opts_hash and provenance/
  mod.rs Manifest both used hand-curated subsets of Options fields. The
  subsets were authored before clone detection landed and never updated,
  silently omitting:
    - min_clone_node_count
    - exclude_patterns
    - min_clone_shared_revs
    - clone_similarity_floor
    - clone_skip_same_dir
    - max_coupling_pct, group_file, team_map_file, temporal_period_days,
      strict_grouping (added by other plans, same omission pattern)

  Impact:
    - Cache key collision: changing --exclude or --min-clone-node-count
      hit the cache against the old DB → silently wrong output. The cache
      is supposed to be invisible; users had no way to detect the staleness.
    - Provenance manifest gap: .provenance.json claimed reproducibility but
      omitted the thresholds that produced clone output → undermines the
      README's "every config knob" pitch.

  Systemic fix: single Options::canonical_serialization() helper that JSON-
  serializes the whole struct (Vec fields sorted for order-independence).
  Both cache.rs and provenance/mod.rs delegate to it. Adding a new Options
  field automatically propagates to both the cache key AND the provenance
  manifest with zero per-field maintenance.

  BREAKING: provenance JSON schema changed — 18 named fields became a
  nested `options: { ... }` object. Provenance schema version bumped;
  consumers parsing the old shape must migrate.
  EOF
  )"
  ```

---

## Task 5: SARIF `CODELORE-MISSING-COCHANGE` rule + emit coupling absences from diff

**Validation evidence (2026-06-08):** `crates/codelore-cli/src/diff_output.rs::emit_sarif` (line 259+) emits SARIF for `hotspots.rank_entrants`, `hotspots.score_increased`, and `clones.new_families` only. Coupling absences are iterated in text (line 78, 84), Markdown (196, 200, 213), and JSON output, but the SARIF emit function has no `coupling_absences` reference anywhere. The CodeScene-signature "did you forget to update X?" signal — marketed in `advanced-usage.md` as a strategic differentiator — never reaches GitHub Code Scanning.

**Files:**
- Modify: `crates/codelore-cli/src/diff_output.rs` (extend `emit_sarif` + add rule definition)
- Test: `crates/codelore-cli/tests/diff_sarif_absence_test.rs` (new)

- [ ] **Step 1: Write the failing test**

  ```rust
  // crates/codelore-cli/tests/diff_sarif_absence_test.rs
  // Constructs a DiffOutput with one CouplingAbsence, calls emit_sarif,
  // parses the JSON, asserts the result contains a `CODELORE-MISSING-COCHANGE`
  // rule + a corresponding `results[]` entry mentioning both file paths.

  #[test]
  fn emit_sarif_includes_missing_cochange_results() {
      let output = DiffOutput {
          coupling_absences: vec![CouplingAbsence {
              touched: "src/auth/login.rs".into(),
              missing: "src/auth/session.rs".into(),
              shared: 12,
              fisher_p: 0.001,
          }],
          ..DiffOutput::default()
      };
      let mut buf = Vec::new();
      emit_sarif(&mut buf, &output).unwrap();
      let sarif: serde_json::Value = serde_json::from_slice(&buf).unwrap();

      // Rule defined
      let rules = sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
      assert!(rules.iter().any(|r| r["id"] == "CODELORE-MISSING-COCHANGE"));

      // Result emitted
      let results = sarif["runs"][0]["results"].as_array().unwrap();
      let absence_results: Vec<_> = results.iter()
          .filter(|r| r["ruleId"] == "CODELORE-MISSING-COCHANGE")
          .collect();
      assert_eq!(absence_results.len(), 1);
      let msg = absence_results[0]["message"]["text"].as_str().unwrap();
      assert!(msg.contains("src/auth/login.rs") && msg.contains("src/auth/session.rs"));
  }
  ```

- [ ] **Step 2: Add the rule to `emit_sarif`'s rule registry**

  Find where the existing rules (`CODELORE-HOTSPOT`, `CODELORE-CLONE`) are defined in `emit_sarif` and add:

  ```rust
  json!({
      "id": "CODELORE-MISSING-COCHANGE",
      "name": "MissingCoChange",
      "shortDescription": { "text": "Coupled file omitted from changeset" },
      "fullDescription": {
          "text": "A file historically and significantly coupled with the file you modified was not modified in this PR. The 'absent change' pattern often indicates an oversight (a forgotten test, a forgotten migration script, a forgotten doc update)."
      },
      "helpUri": "https://github.com/.../advanced-usage.md#4-pr-mode-codelore-diff",
      "defaultConfiguration": { "level": "note" },
      "properties": {
          "tags": ["behavioral", "coupling", "absent-change-pattern", "pr-diff"]
      }
  })
  ```

- [ ] **Step 3: Emit `results[]` entries for each `CouplingAbsence`**

  After the existing `for c in &output.clones.new_families` loop, add:

  ```rust
  for a in &output.coupling_absences {
      hotspot_results.push(json!({
          "ruleId": "CODELORE-MISSING-COCHANGE",
          "level": "note",
          "message": {
              "text": format!(
                  "PR modifies '{}' but historically '{}' co-changes with it (shared={}, fisher_p={:.4}). Verify the absence is intentional.",
                  a.touched, a.missing, a.shared, a.fisher_p
              )
          },
          "locations": [{
              "physicalLocation": {
                  "artifactLocation": { "uri": a.touched },
                  "region": { "startLine": 1 }
              }
          }],
          "partialFingerprints": {
              "couplingPair/v1": format!("{}::{}", a.touched.min(&a.missing), a.touched.max(&a.missing))
          },
          "properties": {
              "codelore/diff-classification": "missing-cochange",
              "codelore/shared-revs": a.shared,
              "codelore/fisher-p": a.fisher_p,
              "codelore/missing-partner": &a.missing,
              "tags": ["behavioral", "coupling", "absent-change-pattern", "pr-diff"]
          }
      }));
  }
  ```

- [ ] **Step 4: Run test + commit**

  ```bash
  cargo test -p codelore-cli --test diff_sarif_absence_test
  cargo test --workspace --all-features
  ```

  ```bash
  git commit -m "$(cat <<'EOF'
  feat(cli): SARIF CODELORE-MISSING-COCHANGE rule for `codelore diff`

  Coupling absences — the "you changed X but historically Y always changes
  with it, did you forget?" signal — were emitted in text, JSON, and
  Markdown output of `codelore diff`, but the SARIF emitter silently
  skipped them. SARIF is the format that auto-renders on PRs via GitHub
  Code Scanning, so the project's stated strategic differentiator (the
  "absent change pattern") never reached the audience it was designed for.

  Add CODELORE-MISSING-COCHANGE rule to the SARIF tool driver and emit
  one `results[]` entry per CouplingAbsence row. Severity = note (advisory
  — the developer may have intentionally decoupled). Versioned
  partialFingerprints keep identity stable across runs.

  The diff text/JSON/Markdown emitters are unchanged; only SARIF was
  missing this surface.
  EOF
  )"
  ```

---

## Task 6: AI-attribution patterns — add Cursor / Aider / Cody / Continue / Codeium / Windsurf / Devin

**Validation evidence (2026-06-08):** `crates/codelore-lib/src/identity/bots.rs:27` body matches only `Co-Authored-By: Claude`, `Co-Authored-By: Copilot`, `Co-Authored-By: GitHub Copilot`. Substring contains, case-sensitive. Every Cursor / Aider / Cody / Continue / Codeium / Devin repo currently reads as 100% human in the `commits.ai_attribution` column.

**Files:**
- Modify: `crates/codelore-lib/src/identity/bots.rs` (extend pattern list + case-insensitive)
- Test: `crates/codelore-lib/tests/ai_attribution_modern_test.rs` (new)

- [ ] **Step 1: Write the failing test**

  ```rust
  // crates/codelore-lib/tests/ai_attribution_modern_test.rs
  use codelore_lib::identity::bots::ai_attribution;

  #[test]
  fn detects_modern_ai_assistants() {
      for trailer in &[
          "Co-Authored-By: Cursor",
          "Co-Authored-By: Sourcegraph Cody",
          "Co-Authored-By: Continue",
          "Co-Authored-By: Codeium",
          "Co-Authored-By: Windsurf",
      ] {
          let msg = format!("feat: add foo\n\n{trailer}");
          assert_eq!(
              ai_attribution("alice@example.com", "Alice", &msg),
              "ai-assisted",
              "trailer should be detected: {trailer}"
          );
      }
  }

  #[test]
  fn detects_aider_in_message_body() {
      // Aider doesn't use Co-Authored-By; it tags the message body with (aider)
      let msg = "refactor: extract helper (aider)";
      assert_eq!(ai_attribution("alice@example.com", "Alice", msg), "ai-assisted");
  }

  #[test]
  fn detects_devin_via_bot_email() {
      assert_eq!(
          ai_attribution(
              "devin-ai-integration[bot]@users.noreply.github.com",
              "Devin",
              "implement feature"
          ),
          "ai-authored"
      );
  }

  #[test]
  fn co_authored_by_is_case_insensitive() {
      let msg = "feat: x\n\nCO-AUTHORED-BY: cursor";
      assert_eq!(ai_attribution("alice@example.com", "Alice", msg), "ai-assisted");
  }
  ```

- [ ] **Step 2: Extend the pattern list + use case-insensitive regex**

  ```rust
  // crates/codelore-lib/src/identity/bots.rs

  use regex::Regex;
  use std::sync::OnceLock;

  /// Modern AI-assistant detection. Case-insensitive. Captures every
  /// published Co-Authored-By trailer used by AI coders in 2024-2026,
  /// plus Aider's (aider) message-body tag.
  static AI_ASSISTED_RE: OnceLock<Regex> = OnceLock::new();

  fn ai_assisted_regex() -> &'static Regex {
      AI_ASSISTED_RE.get_or_init(|| {
          Regex::new(r"(?i)(co-authored-by:\s*(claude|copilot|github\s+copilot|cursor|sourcegraph\s+cody|cody|continue|codeium|windsurf|devin|tabnine|amazon\s+q)|\(aider\))")
              .expect("static regex compiles")
      })
  }

  // Add Devin's bot email to DEFAULT_BOT_PATTERNS:
  pub const DEFAULT_BOT_PATTERNS: &[&str] = &[
      "dependabot[bot]",
      "github-actions[bot]",
      "claude-code[bot]",
      "copilot[bot]",
      "renovate[bot]",
      "pre-commit-ci[bot]",
      "devin-ai-integration[bot]",  // NEW
  ];

  #[must_use]
  pub fn is_bot(email: &str, name: &str) -> bool {
      let e = email.to_lowercase();
      let n = name.to_lowercase();
      DEFAULT_BOT_PATTERNS
          .iter()
          .any(|p| e.contains(&p.to_lowercase()) || n.contains(&p.to_lowercase()))
  }

  #[must_use]
  pub fn ai_attribution(email: &str, name: &str, message: &str) -> &'static str {
      if is_bot(email, name) {
          "ai-authored"
      } else if ai_assisted_regex().is_match(message) {
          "ai-assisted"
      } else {
          "human"
      }
  }
  ```

- [ ] **Step 3: Run all tests** — old bot detection + new patterns. `cargo test --workspace --all-features`.

- [ ] **Step 4: Update `docs/advanced-usage.md` §6.3** to reflect the expanded pattern list. (Quick edit; the doc currently lists 3 trailers — update to the full set with a "case-insensitive" callout.)

- [ ] **Step 5: Commit**

  ```bash
  git commit -m "$(cat <<'EOF'
  fix(identity): AI-attribution detects 2024-2026 AI coders

  ai_attribution() previously detected only Claude, Copilot, and GitHub
  Copilot via case-sensitive substring match on the Co-Authored-By
  trailer. Every repo using Cursor / Aider / Cody / Continue / Codeium /
  Windsurf / Devin / Tabnine / Amazon Q has been ai_attribution = "human"
  in commits.ai_attribution since the analyzer shipped.

  Switch to a case-insensitive regex covering every published trailer
  format for the modern AI-coding assistant lineup. Aider gets special
  handling because it tags the commit message body with "(aider)" rather
  than a Co-Authored-By trailer. Devin gets a bot-email entry in
  DEFAULT_BOT_PATTERNS (devin-ai-integration[bot]) so it classifies as
  ai-authored rather than ai-assisted.

  Bot-email matching also lowercased on both sides to fix Dependabot[Bot]
  / DEPENDABOT[bot] / mixed-case variants returned by some GitHub paths.

  Future-proof: pattern list is one place to extend as new assistants
  ship. A bots.toml extension hook is on the modernization plan to make
  this user-configurable without a release.

  Documented in advanced-usage.md §6.3.
  EOF
  )"
  ```

---

## Task 7: Worktree prune on startup — eliminate orphan accumulation

**Validation evidence (2026-06-08):** `crates/codelore-cli/src/diff.rs::Worktree::Drop` removes the worktree directory + runs `git worktree remove --force` on the `Drop` path. No `git worktree prune` or directory-age sweep anywhere on the startup path. SIGKILL, OOM, or disk-full leaves orphan directories under `$XDG_CACHE_HOME/codelore/diff-worktrees/` AND orphan entries in `.git/worktrees/`. Subsequent `git worktree add` calls then silently skip with "already exists" against a directory git has registered but the filesystem has deleted.

**Files:**
- Modify: `crates/codelore-cli/src/diff.rs` (add `prune_stale_worktrees()` function)
- Modify: `crates/codelore-cli/src/main.rs` (call prune from `run_diff_cmd` startup)
- Test: `crates/codelore-cli/tests/diff_worktree_prune_test.rs` (new)

- [ ] **Step 1: Write the failing test**

  Build a fixture repo, create a stale worktree directory under the cache (mock SIGKILL-aborted run), then call `run_diff_cmd` and assert:
  - `git worktree prune` runs in the repo (verify via `git worktree list` after)
  - The stale directory is removed (only if > 24h old; for the test, use a fake old mtime via `filetime` crate or by touching the file with a past timestamp)

- [ ] **Step 2: Implement `prune_stale_worktrees`**

  ```rust
  // crates/codelore-cli/src/diff.rs

  /// Best-effort cleanup of orphan worktrees from prior aborted runs.
  ///
  /// Runs git's own worktree-list pruning (removes registry entries that
  /// point to deleted directories) and removes any subdirectory under
  /// $XDG_CACHE_HOME/codelore/diff-worktrees/ that is older than 24h.
  ///
  /// The 24h grace window avoids racing with a concurrent in-progress run.
  /// All errors are logged at warn level — pruning should never fail the
  /// caller.
  pub fn prune_stale_worktrees(repo_root: &Path) {
      // 1. git worktree prune in the user's repo — removes orphan registry entries
      if let Err(e) = std::process::Command::new("git")
          .args(["-C", &repo_root.display().to_string(), "worktree", "prune"])
          .output()
      {
          tracing::warn!("git worktree prune failed during startup cleanup: {e}");
          // continue — non-fatal
      }

      // 2. Sweep $XDG_CACHE_HOME/codelore/diff-worktrees/ for old directories
      let cache_root = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
          .join("codelore").join("diff-worktrees");
      if !cache_root.exists() {
          return;
      }
      let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);
      let Ok(entries) = std::fs::read_dir(&cache_root) else { return; };
      for entry in entries.filter_map(std::result::Result::ok) {
          let Ok(meta) = entry.metadata() else { continue };
          if !meta.is_dir() { continue }
          let Ok(modified) = meta.modified() else { continue };
          if modified < cutoff {
              if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                  tracing::warn!("failed to remove stale worktree {}: {e}", entry.path().display());
              } else {
                  tracing::info!("pruned stale worktree directory: {}", entry.path().display());
              }
          }
      }
  }
  ```

- [ ] **Step 3: Wire into `run_diff_cmd` startup**

  In `crates/codelore-cli/src/main.rs`, in the diff dispatch:

  ```rust
  Command::Diff(args) => {
      // Best-effort cleanup of orphans from prior aborted runs before we add
      // a new worktree — runs once per `diff` invocation; errors logged only.
      diff::prune_stale_worktrees(&args.repo);
      diff::run_diff_cmd(args)
  }
  ```

- [ ] **Step 4: Run tests + commit**

  ```bash
  cargo test -p codelore-cli --test diff_worktree_prune_test
  cargo test --workspace --all-features
  ```

  ```bash
  git commit -m "$(cat <<'EOF'
  fix(cli): prune stale git worktrees on `codelore diff` startup

  Worktrees created by `codelore diff` are removed on the Drop path —
  but SIGKILL, OOM, or disk-full aborts bypass Drop, leaving orphan
  directories under $XDG_CACHE_HOME/codelore/diff-worktrees/ AND
  orphan registry entries in .git/worktrees/. Symptoms accumulate
  silently: ballooning cache directory, `git worktree list` cluttered
  with dead branches, subsequent worktree-add calls silently skipping
  because git registered a dir we'd already cleaned from disk.

  Add prune_stale_worktrees() that runs on every diff invocation:
    1. `git -C <repo> worktree prune` — clears stale git registry entries
       (removes the "already exists" failure mode on the next add)
    2. Sweeps the cache directory for subdirs older than 24h and removes
       them. 24h grace window avoids racing with concurrent runs.

  Both operations are idempotent + best-effort; errors are warn-logged
  and never fail the diff command.
  EOF
  )"
  ```

---

## Effort estimate

| Task | Files touched | LOC est. | Test LOC | Wall-clock |
|---|---|---|---|---|
| 1. clone-coupling p_value | 1 + 1 test | 5 | 50 | 30 min |
| 2. drop empty `name` column | 6 + ~3 tests | 25 | 80 | 1.5 hrs |
| 3. author_churn tertiary sort | 1 + 1 test | 2 | 40 | 30 min |
| 4. canonical Options serialization | 3 + 2 tests | 70 | 120 | 2 hrs |
| 5. SARIF MISSING-COCHANGE rule | 1 + 1 test | 80 | 90 | 1.5 hrs |
| 6. AI-attribution patterns | 1 + 1 test + doc | 40 | 100 | 1 hr |
| 7. worktree prune startup | 2 + 1 test | 60 | 80 | 1.5 hrs |
| **Total** | **~17 files** | **~280 LOC** | **~560 LOC** | **~9 hrs (1.5 days)** |

After completion: 349 tests + 7 new = 356 tests. All 7 commits are small enough that the **squash-or-keep-separate decision** is the user's. Recommend keeping separate — each commit message documents one specific bug + its impact, and any single fix could be reverted independently.

---

## Self-review

**Spec coverage:** All 7 bugs from the audits make it to a task (4 from `updated_analysis_report.md`, 3 of the 8 🔴 from `modernization_audit_2026-06-08.md`). The remaining audit items (#11 format!→bind, #12 indexes, #18 min_revs split, #19 diff string→enum) are correctness/quality at the modernization level — they go to the modernization plan, not this one.

**Placeholder scan:** No TBD/TODO. Every step has either code or a specific command. Test bodies are complete enough that the implementer can write them without re-deriving the assertion. SQL changes include both BEFORE and AFTER blocks.

**Type consistency:** `HotspotRow` field removed (Task 2) consistently across struct + SQL + emitter + JSON deserialization. `Manifest` schema change (Task 4) marked as BREAKING in the commit. `CouplingAbsence` struct fields referenced (Task 5) exist in current code (verified during validation).

**Dependency order:** Task 4 unblocks all future plans that add Options fields (no more hand-curated lists to update). Task 6 must precede the bots.toml extension hook in the modernization plan. Other tasks are independent.
