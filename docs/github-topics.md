# GitHub Topic Tags

CodeLore configures these topic tags on the GitHub repository for discoverability. The README also displays them as badges (see top of README.md).

## Configured topics

| Topic | Why | Discovery URL |
|---|---|---|
| `behavioral-code-analysis` | Primary category — the techniques code-maat introduced and CodeLore modernizes (hotspots, change-coupling, ownership, code-age, etc.) | https://github.com/topics/behavioral-code-analysis |
| `code-analysis-tool` | Broader category for any tool that analyzes source code | https://github.com/topics/code-analysis-tool |
| `repository-mining` | Academic + industry term for extracting signal from VCS history | https://github.com/topics/repository-mining |
| `technical-debt` | What hotspots + code-health + clone-coupling surface | https://github.com/topics/technical-debt |
| `code-maat` | Direct association with Adam Tornhill's tool that CodeLore modernizes | https://github.com/topics/code-maat |

## Additional topics worth considering

These weren't in the user's initial list but match CodeLore's surface area; add via the GitHub repo settings UI or `gh repo edit --add-topic ...` if they make sense:

- `rust` — language of implementation
- `sarif` — primary CI/CD output format (3 SARIF rules ship)
- `git-history-analysis` — narrower than repository-mining
- `software-quality` — broader umbrella
- `developer-tools` — broadest umbrella
- `duckdb` — fact-store backend (some users search by data-store choice)
- `tree-sitter` — parser layer (used in clone detection)

## How to set them

### Via the GitHub web UI

1. Open the repo's main page on github.com
2. Click the ⚙️ gear icon next to "About" (top-right of the repo description)
3. In the "Topics" field, add tags as space- or comma-separated values
4. Press Enter / save

### Via the gh CLI

```bash
gh repo edit \
  --add-topic behavioral-code-analysis \
  --add-topic code-analysis-tool \
  --add-topic repository-mining \
  --add-topic technical-debt \
  --add-topic code-maat
```

Multiple `--add-topic` flags can be combined in one invocation. The CLI also accepts `--remove-topic` for removals.

### Via the REST API

```bash
gh api -X PUT /repos/<owner>/<repo>/topics \
  -F 'names[]=behavioral-code-analysis' \
  -F 'names[]=code-analysis-tool' \
  -F 'names[]=repository-mining' \
  -F 'names[]=technical-debt' \
  -F 'names[]=code-maat'
```

Note: this replaces the entire topic list, so include every topic you want to keep.

## Where the same tags also appear

- **`Cargo.toml` keywords**: same five tags configured at the workspace level so crates.io search surfaces CodeLore when users search those terms. See `Cargo.toml::keywords`.
- **README badges**: rendered at the top of README.md using shields.io. Each badge links to the corresponding `github.com/topics/<tag>` page.
- **This file**: human-readable canonical source of the tag list. Update here first when adding/removing.
