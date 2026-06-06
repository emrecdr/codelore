# CodeLore Plan 4: Analyses + Identity Resolution + Kamei Enrichment + Complete Code Health

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Plan 4 of 6 for v1 Spine release.** Builds on Plans 1–3 (the walking skeleton, RCA vendor, and complexity integration). This is the biggest plan by feature count.

**Goal:** Ship the remaining v1 §1.1 analyses (change-coupling with Fisher exact, code-ownership via Fractal Value, code-age, abs-churn, author-churn, entity-churn, communication, summary), populate the Kamei 14-feature change vector, wire identity resolution (`.mailmap` + bot filtering + AI attribution stub), and complete the Code Health composite by activating its previously-zero inputs (churn / fragmentation / coupling).

**Architecture:** Plan 4 fills out the analytical side of the schema. The 5 stages of the pipeline (Source → Identity → Group → Temporal → Team) need their middle stages implemented (only Source + Ingest exist in Plan 1; Plan 4 adds Identity). The Kamei vector lands as an enrichment pass after ingest. New analyses are SQL views with thin Rust orchestrators per the established pattern. Code Health gets re-formulated to its full §4.6 shape now that all four inputs exist.

**Tech Stack:**
- All Plan 1+2+3 stack
- New: `fishers_exact = "1"` (or vendored: spec §2.2 lists `fishers_exact` — keep the dep small)
- New: `gix-mailmap` (if a separate crate) or use `gix::Repository::mailmap_resolve` if available

**Out of scope for this plan (deferred to Plan 5):**
- SARIF + Markdown + Parquet + SQLite outputs (Plan 5)
- Provenance manifest emission (Plan 5)
- Adaptive complexity sampling (per-revision)
- 12 code-maat parity analyses deferred to v1.5 per §8.1 (main-dev, main-dev-by-revs, refactoring-main-dev, entity-effort, entity-ownership, fragmentation as separate analysis, soc, messages regex, identity-dump, fn-coupling, fn-ownership, fn-hotspot)
- Co-change graph entropy (v1.5)
- RefactoringMiner integration (v1.5)
- Bootstrap CIs + Scott-Knott ESD (v1.5)
- MCP server (also v1.5 per revised spec)

**Definition of Done for Plan 4:**
- `.mailmap` resolution wired in `GixRepo::resolve_alias`
- `bots.toml` + AI attribution stub populated during ingest
- Kamei 14 features populated for each commit (NS, ND, NF, entropy, LA, LD, LT, FIX, NDEV, AGE, NUC, EXP, REXP, SEXP)
- 8 new analyses pass tests:
  - `change-coupling` (with Fisher exact significance, default p < 0.05)
  - `code-ownership` (Fractal Value / HHI)
  - `code-age`
  - `abs-churn`
  - `author-churn`
  - `entity-churn`
  - `communication`
  - `summary`
- Code Health composite now uses all 4 inputs from spec §4.6
- CLI dispatches all 10 analyses (Plan 1 + Plan 3 + Plan 4 = revisions, hotspots, code-health, change-coupling, code-ownership, code-age, abs-churn, author-churn, entity-churn, communication, summary = 11)
- All previous tests pass + new Plan 4 tests pass
- Clippy/fmt/deny clean
- CHANGELOG and README updated

---

## §1 — Plan 3 carry-over (Phase 4.A)

### Task 1: `--complexity-sample head` flag (no-op for Plan 4; sets up Plan 5 infrastructure)

**Files:**
- Modify: `crates/codelore-cli/src/args.rs`
- Modify: `crates/codelore-lib/src/options.rs`

- [ ] **Step 1: Add Options field for complexity sampling**

In `crates/codelore-lib/src/options.rs` `Options` struct, add field:

```rust
pub complexity_sample: ComplexitySample,
```

And the enum (in a sub-module if you want — or in `options.rs` directly):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComplexitySample {
    #[default]
    Head,  // Plan 3 default — parse at HEAD only
    Adaptive,  // Plan 5 — every Nth based on file's revision count
    Full,  // Plan 5 — every revision
}
```

In `Default for Options`, set `complexity_sample: ComplexitySample::Head`.

- [ ] **Step 2: Add CLI flag**

In `crates/codelore-cli/src/args.rs`, add to `AnalyzeArgs`:

```rust
/// Complexity sampling strategy: head (default) | adaptive | full.
/// adaptive and full are Plan 5 work; head matches Plan 3 behavior.
#[arg(long, default_value = "head")]
pub complexity_sample: String,
```

In `main.rs analyze()`, parse this string to `ComplexitySample` and set it on `opts`. For Plan 4, only `head` is supported; bail on adaptive/full with "Plan 5 work."

- [ ] **Step 3: Build + tests**

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

Should be 236 tests passing (Plan 4 baseline).

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/ crates/codelore-cli/
git commit -m "feat(cli): add --complexity-sample flag (Plan 4 ships head only; Plan 5 adds adaptive/full)"
```

---

## §2 — Identity resolution (Phase 4.B)

### Task 2: `.mailmap` resolution in `GixRepo::resolve_alias`

**Files:**
- Modify: `crates/codelore-lib/src/repo/gix_repo.rs`
- Create: `crates/codelore-lib/tests/mailmap_test.rs`
- Modify: `crates/codelore-lib/Cargo.toml`

- [ ] **Step 1: Write failing test**

```rust
// tests/mailmap_test.rs
use codelore_lib::repo::{GixRepo, Repo};
use codelore_lib::test_support::tiny_repo;

#[test]
fn mailmap_maps_email_to_canonical() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    // Build a tiny repo with a .mailmap file
    std::fs::write(path.join(".mailmap"),
        "Alice Real <alice@real.com> <alice@old.com>\n"
    ).unwrap();
    // ... init + 1 commit via the tiny_repo helper pattern
    // (or just open and verify resolve_alias works)

    let repo = GixRepo::open(path).expect("open");
    // Note: resolve_alias takes an email, returns canonical email
    let canonical = repo.resolve_alias("alice@old.com");
    assert_eq!(canonical, "alice@real.com");

    let unchanged = repo.resolve_alias("bob@example.com");
    assert_eq!(unchanged, "bob@example.com", "unknown email should pass through");
}
```

Add `[[test]] name = "mailmap_test" required-features = ["test-support"]`.

- [ ] **Step 2: Implement `resolve_alias` via gix mailmap**

In `crates/codelore-lib/src/repo/gix_repo.rs`, replace the stub:

```rust
fn resolve_alias(&self, email: &str) -> String {
    use std::sync::OnceLock;
    // Load mailmap lazily and cache for repo lifetime
    static MAILMAP: OnceLock<gix::mailmap::Snapshot> = OnceLock::new();
    // Actually we want per-instance caching; use Self field with OnceLock<...>
    // For Plan 4, simpler: load every call (mailmap files are small)
    let repo = self.inner.to_thread_local();
    let mailmap = match repo.open_mailmap() {
        Ok(m) => m,
        Err(_) => return email.to_string(),
    };
    let signature = gix::actor::SignatureRef {
        name: "".into(),
        email: email.as_bytes().into(),
        time: Default::default(),
    };
    mailmap.try_resolve(signature)
        .map(|s| std::str::from_utf8(&s.email).unwrap_or(email).to_string())
        .unwrap_or_else(|| email.to_string())
}
```

The exact gix API may vary. Read `gix`'s mailmap module documentation. Adapt to whatever works on gix 0.84.

- [ ] **Step 3: Run, iterate, commit**

```bash
cargo test -p codelore-lib --test mailmap_test --all-features
git add crates/codelore-lib/
git commit -m "feat(lib): GixRepo::resolve_alias via gix mailmap"
```

---

### Task 3: `bots.toml` + AI attribution stub during ingest

**Files:**
- Create: `crates/codelore-lib/src/identity/mod.rs`
- Create: `crates/codelore-lib/src/identity/bots.rs`
- Modify: `crates/codelore-lib/src/facts/ingest.rs` (use identity::*)
- Modify: `crates/codelore-lib/src/lib.rs` (add pub mod identity)
- Update: spec §6.6 if any error variants need adjusting

- [ ] **Step 1: Default bot patterns**

```rust
// src/identity/bots.rs
pub const DEFAULT_BOT_PATTERNS: &[&str] = &[
    "dependabot[bot]",
    "github-actions[bot]",
    "claude-code[bot]",
    "copilot[bot]",
    "renovate[bot]",
    "pre-commit-ci[bot]",
];

pub fn is_bot(email: &str, name: &str) -> bool {
    DEFAULT_BOT_PATTERNS.iter().any(|p| email.contains(p) || name.contains(p))
}

pub fn ai_attribution(email: &str, name: &str, message: &str) -> &'static str {
    if is_bot(email, name) {
        "ai-authored"
    } else if message.contains("Co-Authored-By: Claude") ||
              message.contains("Co-Authored-By: Copilot") {
        "ai-assisted"
    } else {
        "human"
    }
}
```

- [ ] **Step 2: Use in ingest**

In `ingest_loop`, when appending the commit row:
- Call `resolve_alias(author_email)` to get `canonical_author`
- Call `bots::ai_attribution(email, name, message)` for `ai_attribution`
- Mark `is_bot` in the `author_aliases` table when the canonical author matches a bot pattern

- [ ] **Step 3: Tests + commit**

Add a test that creates a fixture commit with a bot author (e.g. `dependabot[bot]@noreply.github.com`) and verifies it gets `ai_attribution = "ai-authored"`.

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): identity resolution — mailmap + bots.toml + AI attribution stub"
```

---

## §3 — Kamei vector enrichment (Phase 4.C)

### Task 4: Compute Kamei 14 features per commit

**Files:**
- Create: `crates/codelore-lib/src/kamei/mod.rs`
- Modify: `crates/codelore-lib/src/facts/ingest.rs` (call kamei::enrich after main ingest)
- Create: `crates/codelore-lib/tests/kamei_test.rs`

The Kamei 14 features (spec §3.1, Kamei et al. JIT-SDP canonical):

| Group | Field | Definition |
|---|---|---|
| **Diffusion** | NS | distinct top-level dirs touched |
| | ND | distinct dir paths touched |
| | NF | distinct files touched |
| | entropy | normalized entropy of LOC across files |
| **Size** | LA | total lines added |
| | LD | total lines deleted |
| | LT | mean total LOC of touched files (pre-change) |
| **Purpose** | FIX | commit message matches bug-fix regex |
| **History** | NDEV | distinct developers who previously touched these files |
| | AGE | mean days since last change to these files |
| | NUC | unique changes (commits) to these files |
| **Experience** | EXP | total prior commits by author |
| | REXP | weighted recent commits (decay by year) |
| | SEXP | subsystem experience (commits touching same top-level dir) |

- [ ] **Step 1: Skeleton enrichment**

`src/kamei/mod.rs`:

```rust
//! Kamei 14-feature JIT-SDP canonical vector enrichment.
//! See spec §3.1 + Kamei et al. 2013.
//! Computed as an UPDATE pass after main ingest using SQL aggregates
//! over the commits + changes tables.

use crate::facts::FactsDb;
use crate::{CodeLoreError, Result};

pub fn enrich(db: &FactsDb) -> Result<()> {
    // Run an UPDATE pass that computes each Kamei field for every row
    // via correlated subqueries. Specifically:
    let sql = "
        -- LA / LD / NF / NS / ND
        UPDATE commits SET
          la = (SELECT COALESCE(SUM(loc_added), 0) FROM changes WHERE changes.rev = commits.rev),
          ld = (SELECT COALESCE(SUM(loc_deleted), 0) FROM changes WHERE changes.rev = commits.rev),
          nf = (SELECT COUNT(DISTINCT path) FROM changes WHERE changes.rev = commits.rev),
          ns = (SELECT COUNT(DISTINCT SPLIT_PART(path, '/', 1)) FROM changes WHERE changes.rev = commits.rev),
          nd = (SELECT COUNT(DISTINCT SUBSTR(path, 1, LENGTH(path) - LENGTH(SPLIT_PART(path, '/', -1)) - 1))
                FROM changes WHERE changes.rev = commits.rev);

        -- FIX
        UPDATE commits SET
          fix = CASE WHEN REGEXP_MATCHES(LOWER(message), 'bug|fix|defect|issue|error') THEN TRUE ELSE FALSE END;

        -- entropy: H = -Σ p_i log2 p_i over LOC distribution
        UPDATE commits SET
          entropy = (
              WITH dist AS (
                  SELECT loc_added FROM changes WHERE changes.rev = commits.rev AND loc_added > 0
              ),
              total AS (SELECT SUM(loc_added) AS t FROM dist),
              probs AS (SELECT loc_added::DOUBLE / NULLIF(total.t, 0) AS p FROM dist, total)
              SELECT COALESCE(-SUM(p * LOG2(NULLIF(p, 0))), 0) FROM probs
          );

        -- NDEV, AGE, NUC, EXP, REXP, SEXP: each requires a window over prior commits.
        -- Computed via CTE + UPDATE on a per-row basis. See full SQL in implementation.
    ";
    db.conn().execute_batch(sql)
        .map_err(|e| CodeLoreError::Analysis(format!("kamei enrich: {e}")))?;

    // NDEV/AGE/NUC/EXP/REXP/SEXP need richer SQL — see Step 2 + 3.
    enrich_history_features(db)?;
    enrich_experience_features(db)?;
    Ok(())
}

fn enrich_history_features(db: &FactsDb) -> Result<()> {
    // NDEV: distinct developers who previously touched the files in this commit
    // AGE: mean days since last commit to these files
    // NUC: unique commits to these files prior to this one
    //
    // Per-row UPDATE with correlated subqueries:
    let sql = "
        UPDATE commits AS c SET
          ndev = (
              SELECT COUNT(DISTINCT prev.canonical_author)
              FROM commits prev
              INNER JOIN changes pchg ON pchg.rev = prev.rev
              INNER JOIN changes cchg ON cchg.rev = c.rev AND cchg.path = pchg.path
              WHERE prev.date < c.date
          ),
          nuc = (
              SELECT COUNT(DISTINCT prev.rev)
              FROM commits prev
              INNER JOIN changes pchg ON pchg.rev = prev.rev
              INNER JOIN changes cchg ON cchg.rev = c.rev AND cchg.path = pchg.path
              WHERE prev.date < c.date
          ),
          age = (
              SELECT COALESCE(AVG(DATE_DIFF('day', prev.date, c.date)), 0.0)
              FROM (
                  SELECT MAX(prev.date) AS date, cchg.path
                  FROM commits prev
                  INNER JOIN changes pchg ON pchg.rev = prev.rev
                  INNER JOIN changes cchg ON cchg.rev = c.rev AND cchg.path = pchg.path
                  WHERE prev.date < c.date
                  GROUP BY cchg.path
              ) prev
          );
    ";
    db.conn().execute_batch(sql)
        .map_err(|e| crate::CodeLoreError::Analysis(format!("kamei history: {e}")))?;
    Ok(())
}

fn enrich_experience_features(db: &FactsDb) -> Result<()> {
    // EXP: prior commit count by this author
    // REXP: weighted by 1/(1+age_in_years)
    // SEXP: prior commits by this author touching the same subsystem (top-level dir)
    let sql = "
        UPDATE commits AS c SET
          exp = (
              SELECT COUNT(*)
              FROM commits prev
              WHERE prev.canonical_author = c.canonical_author AND prev.date < c.date
          ),
          rexp = (
              SELECT COALESCE(SUM(1.0 / (1.0 + DATE_DIFF('year', prev.date, c.date)::DOUBLE)), 0.0)
              FROM commits prev
              WHERE prev.canonical_author = c.canonical_author AND prev.date < c.date
          ),
          sexp = (
              SELECT COUNT(DISTINCT prev.rev)
              FROM commits prev
              INNER JOIN changes pchg ON pchg.rev = prev.rev
              INNER JOIN changes cchg ON cchg.rev = c.rev
              WHERE prev.canonical_author = c.canonical_author
                AND prev.date < c.date
                AND SPLIT_PART(pchg.path, '/', 1) = SPLIT_PART(cchg.path, '/', 1)
          );
    ";
    db.conn().execute_batch(sql)
        .map_err(|e| crate::CodeLoreError::Analysis(format!("kamei experience: {e}")))?;
    Ok(())
}
```

- [ ] **Step 2: Wire into ingest**

After the existing channel-based ingest + complexity pass in `FactsDb::ingest`:

```rust
crate::kamei::enrich(self)?;
```

- [ ] **Step 3: Test**

```rust
// tests/kamei_test.rs
#[test]
fn kamei_vector_populated_for_tiny_repo() {
    // ... build tiny_repo, ingest, assert kamei fields are populated
    let la: String = db.query_one_value(
        "SELECT CAST(SUM(la) AS TEXT) FROM commits"
    ).expect("query");
    assert!(la.parse::<u32>().unwrap() > 0, "tiny repo should have LA > 0");

    let exp_max: String = db.query_one_value(
        "SELECT CAST(MAX(exp) AS TEXT) FROM commits"
    ).expect("query");
    // tiny_repo has 5 commits by same author; max EXP should be 4 (commits 1-4 are prior to commit 5)
    assert!(exp_max.parse::<u32>().unwrap() >= 4);
}
```

- [ ] **Step 4: Commit**

```bash
git add crates/codelore-lib/
git commit -m "feat(lib): Kamei 14-feature change vector enrichment via SQL UPDATE pass"
```

---

## §4 — code-age + churn analyses (Phase 4.D)

### Task 5: `code-age` analysis

**Files:**
- Create: `crates/codelore-lib/src/analyses/code_age.rs`
- Update: `analyses/mod.rs`
- Test: `crates/codelore-lib/tests/code_age_test.rs` + Cargo.toml

**Spec §1.1**: "age — entity → months since last modification."

```rust
// SQL key idiom:
// SELECT path,
//   DATE_DIFF('month', MAX(date), :age_time_now) AS age_months
// FROM commits
// INNER JOIN changes USING(rev)
// GROUP BY path
// ORDER BY age_months ASC
```

`opts.age_time_now` is `Option<Date>`; default to today via `time::OffsetDateTime::now_utc().date()`.

Commit: `feat(lib): code-age analysis`.

---

### Task 6: `abs-churn` / `author-churn` / `entity-churn` analyses

**Files:**
- Create: `crates/codelore-lib/src/analyses/churn.rs` (3 functions in one file: all share the same churn aggregation)
- Update: `analyses/mod.rs`
- Test: `crates/codelore-lib/tests/churn_test.rs`

**3 functions:**

```rust
pub fn run_abs_churn(db: &FactsDb, _opts: &Options) -> Result<Vec<AbsChurnRow>>;
pub fn run_author_churn(db: &FactsDb, _opts: &Options) -> Result<Vec<AuthorChurnRow>>;
pub fn run_entity_churn(db: &FactsDb, _opts: &Options) -> Result<Vec<EntityChurnRow>>;
```

**SQL idioms:**

```sql
-- abs-churn (by date)
SELECT date, SUM(loc_added) AS added, SUM(loc_deleted) AS deleted
FROM commits INNER JOIN changes USING(rev)
GROUP BY date ORDER BY date;

-- author-churn (by canonical_author)
SELECT canonical_author AS author, SUM(loc_added) AS added, SUM(loc_deleted) AS deleted, COUNT(DISTINCT rev) AS commits
FROM commits INNER JOIN changes USING(rev)
GROUP BY canonical_author ORDER BY added DESC;

-- entity-churn (by path)
SELECT path, SUM(loc_added) AS added, SUM(loc_deleted) AS deleted, COUNT(DISTINCT rev) AS commits
FROM changes
GROUP BY path
HAVING commits >= :min_revs
ORDER BY added DESC;
```

Commit: `feat(lib): abs-churn + author-churn + entity-churn analyses`.

---

## §5 — communication analysis (Phase 4.E)

### Task 7: `communication` analysis

**Files:**
- Create: `crates/codelore-lib/src/analyses/communication.rs`
- Update: `analyses/mod.rs`
- Test: `crates/codelore-lib/tests/communication_test.rs`

**Spec §1.1**: "communication — author pair → shared changes, average commits, strength."

**SQL** (uses self-join on authors via shared entities):

```sql
WITH author_files AS (
    SELECT DISTINCT changes.path, commits.canonical_author AS author
    FROM commits INNER JOIN changes USING(rev)
),
pairs AS (
    SELECT a.author AS author_a, b.author AS author_b, COUNT(DISTINCT a.path) AS shared
    FROM author_files a
    INNER JOIN author_files b ON a.path = b.path AND a.author < b.author
    GROUP BY a.author, b.author
),
totals AS (
    SELECT canonical_author AS author, COUNT(DISTINCT rev) AS commits
    FROM commits
    GROUP BY canonical_author
)
SELECT
    p.author_a, p.author_b, p.shared,
    (ta.commits + tb.commits) / 2 AS average,
    100.0 * p.shared / NULLIF((ta.commits + tb.commits) / 2, 0) AS strength
FROM pairs p
INNER JOIN totals ta ON ta.author = p.author_a
INNER JOIN totals tb ON tb.author = p.author_b
WHERE p.shared >= :min_shared_revs
ORDER BY strength DESC, p.author_a, p.author_b;
```

Commit: `feat(lib): communication analysis (Conway's law shared-work pairs)`.

---

## §6 — code-ownership (Fractal Value) analysis (Phase 4.F)

### Task 8: `code-ownership` analysis with Fractal Value

**Files:**
- Create: `crates/codelore-lib/src/analyses/ownership.rs`
- Update: `analyses/mod.rs`
- Test: `crates/codelore-lib/tests/ownership_test.rs`

**Spec §1.1 + §1.1 in deeper detail** (D'Ambros/Gall/Lanza/Pinzger formula, 1 - HHI):

```
FV(entity) = 1 - Σᵢ (aᵢ / nc)²
```

where `aᵢ` is contribution of author i and `nc` is total contribution. FV ∈ [0, 1); higher = more fragmented ownership.

**SQL**:

```sql
WITH author_revs AS (
    SELECT changes.path, commits.canonical_author AS author, COUNT(DISTINCT changes.rev) AS revs
    FROM changes INNER JOIN commits ON changes.rev = commits.rev
    GROUP BY changes.path, commits.canonical_author
),
totals AS (
    SELECT path, SUM(revs) AS total FROM author_revs GROUP BY path
)
SELECT
    ar.path,
    -- Σᵢ (aᵢ/total)²
    SUM(POWER(ar.revs::DOUBLE / NULLIF(t.total, 0), 2)) AS hhi,
    1.0 - SUM(POWER(ar.revs::DOUBLE / NULLIF(t.total, 0), 2)) AS fractal_value,
    -- Also report the main developer (highest revs)
    FIRST(ar.author ORDER BY ar.revs DESC) AS main_author,
    t.total
FROM author_revs ar
INNER JOIN totals t ON ar.path = t.path
GROUP BY ar.path, t.total
HAVING total >= :min_revs
ORDER BY fractal_value DESC;
```

`FIRST(... ORDER BY ...)` is a DuckDB aggregate. If unavailable, use `ARG_MAX(ar.author, ar.revs)`.

Commit: `feat(lib): code-ownership analysis with Fractal Value (HHI complement)`.

---

## §7 — change-coupling with Fisher exact significance (Phase 4.G)

### Task 9: `change-coupling` analysis with `fishers_exact`

**Files:**
- Modify: `crates/codelore-lib/Cargo.toml` (add `fishers_exact = "1"`)
- Create: `crates/codelore-lib/src/analyses/coupling.rs`
- Update: `analyses/mod.rs`
- Test: `crates/codelore-lib/tests/coupling_test.rs`

**Spec §1.1**: "change-coupling (Fisher exact, default p < 0.05) — pair → degree, average revs."

**Algorithm** (per spec §3.2.1 correctness invariants):

1. Pre-filter changeset size: drop commits touching > `max_changeset_size` files (default 30 — prevents 500-file refactor commits from dominating).
2. Generate pairs from each remaining commit's file list.
3. Count shared revisions per (path_a, path_b) pair.
4. For each pair, compute Fisher exact test using a 2x2 contingency:
   - a = shared revs of both
   - b = revs of a only (not b)
   - c = revs of b only (not a)
   - d = total revs - a - b - c
5. Compute degree: 100 * shared / avg(revs_a, revs_b)
6. Filter: only include pairs where `degree >= min_coupling_pct`, `shared >= min_shared_revs`, and Fisher p-value < `opts.fisher_significance`.

**SQL** (heavy use of CTEs):

```sql
WITH filtered_commits AS (
    SELECT rev FROM (
        SELECT rev, COUNT(*) AS files
        FROM changes
        GROUP BY rev
    ) WHERE files <= :max_changeset_size
),
file_revs AS (
    SELECT path, COUNT(DISTINCT rev) AS revs
    FROM changes
    INNER JOIN filtered_commits USING(rev)
    GROUP BY path
    HAVING revs >= :min_revs
),
pairs AS (
    SELECT a.path AS path_a, b.path AS path_b,
           COUNT(DISTINCT a.rev) AS shared
    FROM changes a
    INNER JOIN changes b ON a.rev = b.rev AND a.path < b.path
    INNER JOIN filtered_commits ON filtered_commits.rev = a.rev
    GROUP BY a.path, b.path
    HAVING shared >= :min_shared_revs
)
SELECT
    p.path_a, p.path_b,
    fr_a.revs AS revs_a,
    fr_b.revs AS revs_b,
    p.shared,
    (fr_a.revs + fr_b.revs) / 2 AS average_revs,
    100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2, 0) AS degree
FROM pairs p
INNER JOIN file_revs fr_a ON fr_a.path = p.path_a
INNER JOIN file_revs fr_b ON fr_b.path = p.path_b
WHERE 100.0 * p.shared / NULLIF((fr_a.revs + fr_b.revs) / 2, 0) >= :min_coupling_pct
ORDER BY degree DESC, average_revs DESC;
```

Then in Rust, for each row, compute Fisher exact p-value using the `fishers_exact` crate:

```rust
use fishers_exact::fishers_exact;

let total_commits = total_filtered_commits_count;  // computed once
let a = shared;
let b = revs_a - shared;
let c = revs_b - shared;
let d = total_commits - a - b - c;
let p = fishers_exact(&[a as u32, b as u32, c as u32, d as u32])?.greater_pvalue;
if p < opts.fisher_significance {
    // include this pair
}
```

Commit: `feat(lib): change-coupling analysis with Fisher exact significance per spec §3.2.1`.

---

## §8 — summary analysis (Phase 4.H)

### Task 10: `summary` analysis

**Files:**
- Create: `crates/codelore-lib/src/analyses/summary.rs`
- Update: `analyses/mod.rs`
- Test: `crates/codelore-lib/tests/summary_test.rs`

**Spec §1.1**: summary is a 4-row count overview.

```sql
SELECT 'commits' AS metric, COUNT(*) FROM commits
UNION ALL SELECT 'changes', COUNT(*) FROM changes
UNION ALL SELECT 'entities', COUNT(*) FROM entities
UNION ALL SELECT 'authors', COUNT(DISTINCT canonical_author) FROM commits;
```

Returns `Vec<SummaryRow { metric: String, value: i64 }>`.

Commit: `feat(lib): summary analysis (4-row repo overview)`.

---

## §9 — Code Health composite — full §4.6 formula (Phase 4.I)

### Task 11: Wire churn / fragmentation / coupling into Code Health

**Files:**
- Modify: `crates/codelore-lib/src/analyses/code_health.rs`
- Modify: `crates/codelore-lib/tests/code_health_test.rs`

**Goal**: replace Plan 3's cognitive-only formula with the full §4.6:

```
codehealth(entity) = 100 × (1
    - w_cx · normalize(cognitive_complexity)
    - w_cn · normalize(entity_churn_rate)
    - w_au · normalize(fractal_value)
    - w_cp · normalize(coupling_centrality)
)
```

Defaults: w_cx=0.40, w_cn=0.25, w_au=0.15, w_cp=0.20.

**SQL update** — join with churn, ownership, and coupling tables:

```sql
WITH file_complexity AS (
    SELECT path, MAX(cognitive) AS cognitive FROM complexity_metrics GROUP BY path
),
file_churn AS (
    SELECT path, SUM(loc_added) + SUM(loc_deleted) AS churn FROM changes GROUP BY path
),
file_fv AS (
    -- copy from ownership analysis CTE
    ...
),
file_coupling AS (
    -- centrality = number of pairs this file appears in
    SELECT path, COUNT(*) AS centrality FROM (
        SELECT path_a AS path FROM (...coupling pairs CTE) UNION ALL
        SELECT path_b FROM (...coupling pairs CTE)
    ) GROUP BY path
),
norm AS (
    SELECT
        fc.path,
        fc.cognitive,
        CASE WHEN MAX(fc.cognitive) OVER () > 0 THEN fc.cognitive / MAX(fc.cognitive) OVER () ELSE 0 END AS n_cx,
        CASE WHEN MAX(fch.churn) OVER () > 0 THEN COALESCE(fch.churn, 0) / MAX(fch.churn) OVER () ELSE 0 END AS n_cn,
        COALESCE(ffv.fractal_value, 0) AS n_au,
        CASE WHEN MAX(fcp.centrality) OVER () > 0 THEN COALESCE(fcp.centrality, 0)::DOUBLE / MAX(fcp.centrality) OVER () ELSE 0 END AS n_cp
    FROM file_complexity fc
    LEFT JOIN file_churn fch ON fc.path = fch.path
    LEFT JOIN file_fv ffv ON fc.path = ffv.path
    LEFT JOIN file_coupling fcp ON fc.path = fcp.path
)
SELECT
    path, cognitive,
    GREATEST(0.0, LEAST(100.0, 100.0 * (1.0 - 0.40 * n_cx - 0.25 * n_cn - 0.15 * n_au - 0.20 * n_cp))) AS score
FROM norm
ORDER BY score ASC;
```

Update test to assert score still in [0, 100], and that paths with high churn now have lower scores than they did in Plan 3.

Commit: `feat(lib): wire churn + fragmentation + coupling into Code Health composite per spec §4.6`.

---

## §10 — CLI dispatch (Phase 4.J)

### Task 12: Add all new analyses to CLI dispatch + CSV emitters

**Files:**
- Modify: `crates/codelore-lib/src/output/csv.rs` (add emitters for each new analysis)
- Modify: `crates/codelore-cli/src/main.rs` (add match arms)
- Modify: `crates/codelore-cli/tests/cli_test.rs` (add 8 new CLI tests, one per analysis)

For each new analysis:
1. Add `write_X_csv` emitter in csv.rs
2. Add match arm in main.rs
3. Add CLI test in cli_test.rs

The 8 new analyses + their schemas:

| analysis | CSV header |
|---|---|
| code-age | `entity,age-months` |
| abs-churn | `date,added,deleted` |
| author-churn | `author,added,deleted,commits` |
| entity-churn | `entity,added,deleted,commits` |
| communication | `author-a,author-b,shared,average,strength` |
| code-ownership | `entity,main-dev,total,fractal-value` |
| change-coupling | `entity-a,entity-b,degree,average-revs` |
| summary | `metric,value` |

Commit: `feat(cli): expose 8 new analyses (code-age, churn family, communication, ownership, coupling, summary)`.

---

## §11 — Docs + Plan 4 Done (Phase 4.K)

### Task 13: Update CHANGELOG and README

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `README.md`

Insert Plan 4 section above Plan 3 with full list of new analyses + Kamei + identity resolution + complete Code Health.

Update README "What works today" to mention all 11 analyses now ship. Mark Plan 4 ✅ in roadmap.

Commit: `docs: CHANGELOG + README for Plan 4 analytical fill-out`.

---

## Plan 4 Definition of Done

- [ ] `.mailmap` resolution works in `GixRepo::resolve_alias`
- [ ] `bots.toml` patterns filter known bots; AI attribution column populated
- [ ] All 14 Kamei features populated for each commit
- [ ] 8 new analyses pass tests: change-coupling, code-ownership, code-age, abs-churn, author-churn, entity-churn, communication, summary
- [ ] Code Health composite uses all 4 inputs from §4.6
- [ ] CLI dispatches all 11 analyses
- [ ] All previous tests pass + new Plan 4 tests pass
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- [ ] `cargo fmt --all --check` clean
- [ ] `cargo deny check` clean (`fishers_exact` license should pass)
- [ ] CHANGELOG and README updated

After Plan 4: author **Plan 5** (SARIF + Markdown + Parquet + SQLite + provenance manifest).

---

## Self-Review

### Spec coverage check

| Spec section | Plan 4 coverage |
|---|---|
| §1.1 v1 analyses | ✓ Tasks 5–10 (8 new analyses + Plans 1+3 = 11 total) |
| §3.1 Kamei vector | ✓ Task 4 |
| §3.2.1 correctness invariants (max-changeset-size, mirrored dedup, Fisher significance) | ✓ Task 9 (change-coupling) |
| §4.6 Code Health composite full formula | ✓ Task 11 |
| §1.1 identity resolution (.mailmap + bots) | ✓ Tasks 2-3 |
| §1.1 AI attribution column | ✓ Task 3 (stub; full attribution Plan 5+) |

### Known soft spots

- **Kamei NDEV/AGE/NUC/EXP/REXP/SEXP**: SQL is computationally expensive (O(N²) per row via correlated subqueries). For Plan 4 walking-skeleton against tiny_repo (5 commits) and CodeLore's repo (~50 commits), fine. For Linux kernel scale, will need optimization (window functions over time-ordered rows). Document for Plan 6 perf work.
- **change-coupling Fisher loop**: computed in Rust over each SQL row. For repos with 100k+ pair candidates, this is slow. Plan 6 may push Fisher into DuckDB via UDF.
- **Code Health normalization**: spec §4.6 says "repo's empirical 95th percentile"; Plan 4 still uses MAX. Plan 5 (when we add provenance manifest with reproducibility commitments) should switch to `PERCENTILE_DISC(0.95)`.
- **Communication analysis self-pair**: SQL excludes self-pairs via `a.author < b.author` — correct for code-maat parity (code-maat does same).

---

*End of Plan 4.*
