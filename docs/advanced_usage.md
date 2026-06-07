# CodeLore — Advanced Usage & Reference Guide

This guide provides deep technical details on configuring, running, and understanding the behavioral code analysis metrics in **CodeLore**.

---

## 1. Core Analyses Reference

CodeLore supports 11 distinct analyses. Each can be executed by passing `--analysis <NAME>` to the CLI:

### 1.1 `revisions` (Revision Frequency)
* **Purpose**: Tracks how many times each file has been modified in the git history.
* **Output columns**: `entity` (file path), `n-revs` (revision count).
* **Use Case**: Simple identification of high-activity files.

### 1.2 `hotspots` (Hotspot Detection)
* **Purpose**: Locates files that are both highly complex and frequently changed.
* **Math Formula**:
  $$\text{hotspot\_score} = \text{percentile\_rank}(\text{revisions}) \times \text{percentile\_rank}(\text{cognitive\_complexity}) \times \frac{100 - \text{code\_health}}{10.0}$$
* **Output columns**: `path`, `name`, `revisions`, `cognitive`, `code_health`, `hotspot_score`.
* **Use Case**: Prioritizing refactoring targets. High hotspot scores flag files with high maintenance risk.

### 1.3 `code-health` (Code Health Composite)
* **Purpose**: Calculates an aggregate score between `0` (worst) and `100` (healthiest) based on four weighted dimensions:
  1. **Cognitive Complexity** ($w_{cx} = 0.40$) — nested loops, complex conditions, logic branching.
  2. **Churn Rate** ($w_{cn} = 0.25$) — lines of code added or deleted over time.
  3. **Author Fragmentation** ($w_{au} = 0.15$) — Fractal Value (1 - HHI) of contributor commits.
  4. **Coupling Centrality** ($w_{cp} = 0.20$) — degree centrality in the logical coupling network.
* **Output columns**: `path`, `name`, `cognitive`, `score`.

### 1.4 `change-coupling` (Logical Coupling)
* **Purpose**: Finds pairs of files that tend to change together in the same commits.
* **Filtering Invariants**:
  1. **Max Changeset Pre-filter**: Excludes commits modifying more than `max_changeset_size` files (default 30) to eliminate massive refactoring or license sweeps.
  2. **Min Shared Revs**: Requires at least `min_shared_revs` commits touching both files.
  3. **Fisher Exact Test**: Computes a two-tailed significance test. If $p \ge 0.05$, the coupling is dropped as statistically insignificant.
* **Output columns**: `entity_a`, `entity_b`, `shared`, `revs_a`, `revs_b`, `average_revs`, `degree`, `fisher_p`.

### 1.5 `code-ownership` (Ownership & Fragmentation)
* **Purpose**: Calculates contributor fragmentation and identifies the primary owner of each file.
* **Metric**: Fractal Value ($1 - \text{HHI}$) where:
  $$\text{HHI} = \sum_{i} \left(\frac{\text{commits by author}_i}{\text{total commits}}\right)^2$$
* **Output columns**: `path`, `main_author`, `total_revs`, `fractal_value`.

### 1.6 Churn Views (`abs-churn`, `author-churn`, `entity-churn`)
* **`abs-churn`**: Global code changes (added/deleted LOC) aggregated by date.
* **`author-churn`**: Code changes attributed to each developer.
* **`entity-churn`**: Lines of code added/deleted per file.

### 1.7 `communication` (Shared-Work Pairs)
* **Purpose**: Maps Conway's law by finding pairs of authors who regularly modify the same files.
* **Output columns**: `author_a`, `author_b`, `shared` (number of shared files), `average` (mean total commits), `strength` (coupling percentage).

### 1.8 `code-age` (Temporal Stability)
* **Purpose**: Measures the number of months since each file was last modified.
* **Output columns**: `path`, `age_months`.

### 1.9 `summary` (Repository Overview)
* **Purpose**: A quick 4-row overview of total commits, file changes, unique entities, and active authors.

---

## 2. CLI Options & Threshold Configuration

You can customize the pipeline thresholds via CLI arguments.

### 2.1 Threshold Settings

| Flag | Default | Purpose |
| :--- | :--- | :--- |
| `--min-revs <u32>` | `5` | Ignore files with fewer than N revisions. |
| `--min-shared-revs <u32>` | `5` | Ignore coupling pairs with fewer than N shared commits. |
| `--rows <u32>` | `None` | Limit output to the top N rows (sorted descending by risk/frequency). |

### 2.2 Date and Range Filters *(Coming in Plan 6.5)*

* `--after <YYYY-MM-DD>`: Analyze only commits created on or after this date.
* `--before <YYYY-MM-DD>`: Analyze only commits created on or before this date.
* `--commit-range <revisions>`: Restrict commit walking to a git range (e.g. `main..feature`).

---

## 3. Socio-Technical Identity Resolution

A primary strength of CodeLore is tracking the human developer narratives behind commits.

### 3.1 Mailmap Consolidation
If a developer commits using multiple emails, CodeLore automatically uses the project's `.mailmap` file to resolve their different identities to a single canonical name and email address.

### 3.2 Bot Filtering
To avoid skewing ownership and communication metrics with automated CI scripts, dependency bump bots, and linters, CodeLore checks emails and names against a default-deny bot list (e.g., `dependabot[bot]`, `github-actions[bot]`, `renovate[bot]`).

### 3.3 AI Authorship Tracking
CodeLore scans commit structures, committer patterns, and signed-by trailers to classify commits into three categories stored in the fact database:
1. `human` — Human-authored.
2. `ai-assisted` — Commit carries LLM signatures (e.g., `Co-Authored-By: Claude` or `Copilot`).
3. `ai-authored` — Authored entirely by an automated AI agent or bot.

---

## 4. Kamei Change Metrics (14-Feature Vector)

Every commit ingested by CodeLore is enriched with the Kamei JIT-SDP (Just-In-Time Software Defect Prediction) change vector. These parameters capture the shape of code modifications:

1. **NS**: Number of modified subsystems (top-level directories).
2. **ND**: Number of modified directories.
3. **NF**: Number of modified files.
4. **Entropy**: The distribution of changes across files (high entropy = tangled changes).
5. **LA**: Lines of code added.
6. **LD**: Lines of code deleted.
7. **LT**: Average size of files touched (pre-change).
8. **Fix**: Boolean flag indicating if the commit message matches bug/fix regex patterns.
9. **NDEV**: Number of developers who previously modified the touched files.
10. **Age**: Average time (days) since the last modification of touched files.
11. **NUC**: Number of unique changes (historical commit count) of touched files.
12. **EXP**: Developer experience (previous commits).
13. **REXP**: Developer experience weighted by age (recent commits have higher weight).
14. **SEXP**: Developer experience in the same subsystem.
