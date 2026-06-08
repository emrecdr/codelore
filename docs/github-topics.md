# GitHub Topic Tags

`CodeLore` configures these topic tags on the GitHub repository for discoverability + matching `code-maat`'s topic set (so users browsing `https://github.com/topics/behavioral-code-analysis` find both tools side by side). The README also displays the most prominent ones as badges (see top of README.md).

## Recommended topic set (18 tags)

GitHub allows up to 20 topics per repository. We use 18 — leaves headroom for future additions without churn.

### Primary discoverability (5)

| Topic | Why |
|---|---|
| `behavioral-code-analysis` | Primary category. The techniques code-maat introduced and CodeLore modernizes. |
| `code-analysis-tool` | Broader category for any tool analyzing source code. |
| `repository-mining` | Industry + academic term for extracting signal from VCS history. |
| `technical-debt` | What hotspots, code-health, and clone-coupling surface. |
| `code-maat` | Direct association with Adam Tornhill's tool that CodeLore modernizes. |

### Tech-stack + capability tags (10)

| Topic | Why |
|---|---|
| `rust` | Language of implementation. Highest-traffic topic for OSS Rust tools. |
| `sarif` | SARIF 2.1.0 is CodeLore's primary CI/CD output format (3 rules ship). |
| `clone-detection` | We ship Type 1 + Type 2 clone detection via tree-sitter AST hashing. |
| `hotspot-analysis` | Headline output of `hotspots` + `code-health` analyses. |
| `change-coupling` | One of the 14 published analyses + the live-clone differentiator. |
| `code-complexity` | Cyclomatic + cognitive + Halstead + MI metrics via vendored rust-code-analysis. |
| `developer-tools` | Broadest umbrella for CLI dev tools. |
| `cli` | Every CLI tool wants this. |
| `git` | Primary (and only) VCS we support. |
| `mining-software-repositories` | Academic-canonical form of "repository-mining" — pulls academic-interest traffic. |

### Backing-store + ecosystem (3)

| Topic | Why |
|---|---|
| `duckdb` | Embedded analytics DB we use as the fact store. |
| `tree-sitter` | Parser layer (powers complexity + clone detection). |
| `code-quality` | Adjacent umbrella; pulls traffic from quality-tool searches. |

## Set the topics on github.com

### One-shot setup via the gh CLI (recommended)

```bash
gh repo edit \
  --add-topic behavioral-code-analysis \
  --add-topic code-analysis-tool \
  --add-topic repository-mining \
  --add-topic technical-debt \
  --add-topic code-maat \
  --add-topic rust \
  --add-topic sarif \
  --add-topic clone-detection \
  --add-topic hotspot-analysis \
  --add-topic change-coupling \
  --add-topic code-complexity \
  --add-topic developer-tools \
  --add-topic cli \
  --add-topic git \
  --add-topic mining-software-repositories \
  --add-topic duckdb \
  --add-topic tree-sitter \
  --add-topic code-quality
```

### Via the GitHub web UI

1. Open the repo's main page on github.com.
2. Click the ⚙️ gear icon next to "About" (top-right of the repo description).
3. Paste the comma-separated topics:
   `behavioral-code-analysis, code-analysis-tool, repository-mining, technical-debt, code-maat, rust, sarif, clone-detection, hotspot-analysis, change-coupling, code-complexity, developer-tools, cli, git, mining-software-repositories, duckdb, tree-sitter, code-quality`
4. Save.

### Via the REST API (CI-friendly)

```bash
gh api -X PUT /repos/<owner>/<repo>/topics \
  -F 'names[]=behavioral-code-analysis' \
  -F 'names[]=code-analysis-tool' \
  -F 'names[]=repository-mining' \
  -F 'names[]=technical-debt' \
  -F 'names[]=code-maat' \
  -F 'names[]=rust' \
  -F 'names[]=sarif' \
  -F 'names[]=clone-detection' \
  -F 'names[]=hotspot-analysis' \
  -F 'names[]=change-coupling' \
  -F 'names[]=code-complexity' \
  -F 'names[]=developer-tools' \
  -F 'names[]=cli' \
  -F 'names[]=git' \
  -F 'names[]=mining-software-repositories' \
  -F 'names[]=duckdb' \
  -F 'names[]=tree-sitter' \
  -F 'names[]=code-quality'
```

Note: this form *replaces* the entire topic list, so include every topic you want to keep.

## Mirror locations

The tag list is mirrored in three places so each discovery channel surfaces CodeLore consistently:

| Channel | What's there | File |
|---|---|---|
| GitHub repo topics | Full 18 (set via `gh repo edit`) | (live on github.com — set manually) |
| README badges | 10 most prominent | `README.md` top-of-file |
| crates.io keywords | 5 (crates.io max) | `Cargo.toml::[workspace.package].keywords` |
| Canonical source-of-truth | All 18 with rationale | this file |

When adding or removing a topic, update this file first, then propagate to the other channels.
