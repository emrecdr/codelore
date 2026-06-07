# CodeLore — Behavioral Code Analyzer

> **Read the lore of your codebase.**

Every codebase tells a story that static linters can never see. Behind the syntax and structure lies a human narrative: who wrote the code, who understands it, which corners hold tribal knowledge, and where the historical scars are hidden. Every commit is a piece of this tribal lore. 

**CodeLore** mines your repository's git history and projects it into behavioral insights—detecting hotspots, mapping logical coupling, measuring knowledge fragmentation, and scoring code health.

---

## 1. Why CodeLore?

Standard static code analysis tools (like SonarQube or ESLint) only analyze code structure at a single point in time. They can tell you if code is poorly formatted, but they cannot tell you:
* Which complex file is written by an author who left the company yesterday (**Knowledge Loss Risk**).
* Which files are implicitly coupled and always modified together (**Hidden Architectural Debt**).
* Which highly complex files are actively changing, and which are stable (**Refactoring ROI**).

CodeLore focuses on the **socio-technical dimension** of software engineering. It reads the legends a codebase tells about itself to help you focus refactoring efforts where they yield the highest return.

---

## 2. Key Differentiators: What CodeLore Does Differently

Unlike other code analysis tools, CodeLore is built around transparency, standardization, and modern data architecture:

### 2.1 Behavioral SARIF (The CI Differentiator)
CodeLore formats its organizational and behavioral findings (such as high-risk hotspots and ownership risks) directly into **SARIF 2.1.0** (Static Analysis Results Interchange Format). This allows you to plug CodeLore warnings directly into standard CI scanners (like GitHub Code Scanning, GitLab security dashboards, or DefectDojo), showing behavioral alerts directly inline on pull requests.

### 2.2 Transparency vs. Opaque ML Models
Many proprietary behavioral tools use complex, closed-source machine learning models to rank hotspots. CodeLore is built on absolute transparency, utilizing **published, deterministic mathematical formulas** (such as the Herfindahl-Hirschman Index for ownership andSonarsource-derived cognitive complexity models) that you can inspect and audit.

### 2.3 Solving the "Inter-Tool Disagreement" Problem
Academic research (Spadoni et al., 2025) has shown up to a 500% disagreement rate between behavioral code analysis tools, caused by differing default configurations and hidden thresholds. 
To solve this, CodeLore outputs a **Provenance Manifest** (`.provenance.json`) with every analysis run. The manifest records every single config knob, version pin, and runtime filter, ensuring that reports are exactly reproducible and mathematically verifiable.

### 2.4 Embedded SQL Fact Store (DuckDB)
CodeLore does not lock your data in a proprietary format. It maps your git repository history into a relational database using **DuckDB**. You can export your data as a standard SQLite database (`facts.db`) or run custom, ad-hoc SQL queries directly from the command line, turning CodeLore into a database tool for your git metadata.

---

## 3. Architecture: Why We Chose Our Stack

CodeLore is designed as a lightweight, zero-dependency command-line binary. We chose our toolchain strictly for performance, memory efficiency, and developer control:

```
┌─────────────────────────────────────────────────────────┐
│                    User Repository                      │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼  [gix (Gitoxide) walk]
┌─────────────────────────────────────────────────────────┐
│                    codelore-lib                         │
│   ┌─────────────────────────────────────────────────┐   │
│   │               tree-sitter RCA                   │   │
│   └────────────────────────┬────────────────────────┘   │
└────────────────────────────┼────────────────────────────┘
                             │  [Stream<CommitEvent>]
                             ▼
┌─────────────────────────────────────────────────────────┐
│                   DuckDB Fact Store                     │
│    (commits, changes, hunks, complexity, aliases)       │
└────────────────────────────┬────────────────────────────┘
                             │
                             ▼  [SQL Query Views]
┌─────────────────────────────────────────────────────────┐
│                     Output Emitters                     │
│      (CSV, JSON, SARIF, Markdown, Parquet, SQLite)      │
└─────────────────────────────────────────────────────────┘
```

* **`gix` (Gitoxide)**: We use pure-Rust `gix` for repository walking instead of `libgit2` (C-bindings). This allows us to walk git object databases with extreme speed, complete memory safety, and native multithreading support.
* **DuckDB**: Serves as our analytical fact store. DuckDB provides columnar vector processing and disk-spill capabilities, enabling CodeLore to analyze repositories at the scale of the **Linux kernel (~1.4M commits)** in **under 10 minutes** using **less than 4 GB of RAM**.
* **tree-sitter & `codelore-rca`**: We run a customized fork of Mozilla's `rust-code-analysis` to perform AST-based cognitive and cyclomatic complexity parsing. This ensures we measure real logical weight, not just superficial lines of code.
* **`fishers_exact`**: Used during Logical Coupling analysis to run Fisher's Exact Significance tests, filtering out random co-changes and focusing only on statistically significant architectural coupling.

---

## 4. Quick Start Guide

### 4.1 Build from Source
Ensure you have the Rust toolchain installed, then compile the workspace:
```bash
cargo build --release -p codelore-cli
```

### 4.2 Run Your First Analysis
Run a basic revision analysis against your repository (or any target path):
```bash
# Get the top 10 most revised files in the current repository
./target/release/codelore analyze --analysis revisions --repo . --rows 10
```

*Output (CSV format):*
```csv
entity,n-revs
src/main.rs,42
src/lib.rs,38
tests/differential_repo_test.rs,15
```

---

## 5. Supported Outputs

CodeLore supports exporting reports to multiple formats:

```bash
# Export Hotspots to JSON
codelore analyze --analysis hotspots --format json --output hotspots.json

# Export Hotspots to SARIF for CI scanning
codelore analyze --analysis hotspots --format sarif --output hotspots.sarif

# Pipe a Markdown summary directly to GitHub Step Summary
codelore analyze --analysis hotspots --format markdown >> "$GITHUB_STEP_SUMMARY"

# Export raw database facts to SQLite for ad-hoc GUI querying
codelore analyze --format sqlite --output facts.db
```

---

## 6. Advanced Usage & Reference
For detailed explanations of all 11 core analyses, command-line thresholds, mailmap resolution rules, and the Kamei change vector specifications, please refer to the [Advanced Usage & Reference Guide](file:///Users/emrec/Projects/playground/codescene/docs/advanced_usage.md).

---

## 7. License
CodeLore is licensed under **GPL-3.0-only**. It bundles a fork of Mozilla's `rust-code-analysis` under the **MPL-2.0** license.
