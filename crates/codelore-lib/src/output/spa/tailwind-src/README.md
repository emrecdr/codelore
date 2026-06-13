# Tailwind v4 + DaisyUI 5 — one-time CSS rebuild workflow

The SPA dashboard's CSS is **precompiled and committed** to the repo as
`../tailwind.daisyui.min.css`. `output/spa.rs` then inlines it into the
rendered HTML at template substitution time, exactly the way ECharts
and d3-hierarchy get inlined. This file describes how to regenerate
the compiled CSS when DaisyUI / Tailwind bumps.

## Why precompile

Tailwind v4 dropped the production-grade runtime CDN script that v3
shipped. The recommended distribution path is the standalone CLI which
scans templates at build time and emits the pruned utility set. For
`CodeLore`'s offline-first build flow, precompiling locally + checking
the result in is the simplest option that preserves both:

- the single-file `--format spa` output (no runtime CDN dependency), and
- a `git diff`-reviewable supply-chain audit trail (the compiled CSS
  is committed, not regenerated on every contributor's machine).

The trade-off is a periodic chore (this README) when DaisyUI / Tailwind
ships a meaningful release.

## What's checked in

```
tailwind-src/
├── README.md            ← this file
├── input.css            ← `@import "tailwindcss"; @plugin "./daisyui.mjs"; @source ...`
├── daisyui.mjs          ← DaisyUI 5 plugin source (committed; see "Update DaisyUI" below)
└── daisyui-theme.mjs    ← DaisyUI 5 theme plugin (committed)
```

The two `.mjs` files are DaisyUI's plugin source code consumed by the
Tailwind CLI at *build* time only — they never reach the browser. The
output (`../tailwind.daisyui.min.css`) does. Committing them keeps the
build reproducible without any internet access once the Tailwind CLI
is on `$PATH`.

## Install the Tailwind standalone CLI

Download the Tailwind v4 standalone executable from
<https://github.com/tailwindlabs/tailwindcss/releases/latest> for your
platform. As of writing the latest is `v4.3.1`:

| Platform | Asset |
|---|---|
| macOS arm64 | `tailwindcss-macos-arm64` (~76 MB) |
| macOS x64 | `tailwindcss-macos-x64` (~79 MB) |
| linux arm64 | `tailwindcss-linux-arm64` (~104 MB) |
| linux x64 | `tailwindcss-linux-x64` (~106 MB) |

```bash
mkdir -p ~/.local/bin
curl -fSL -o ~/.local/bin/tailwindcss \
    https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-macos-arm64
chmod +x ~/.local/bin/tailwindcss
# ensure ~/.local/bin is on $PATH so the `just` recipe finds the binary
```

## Run the rebuild

From the repository root:

```bash
just spa-css-rebuild
```

That recipe runs (effectively):

```bash
tailwindcss \
    -i crates/codelore-lib/src/output/spa/tailwind-src/input.css \
    -o crates/codelore-lib/src/output/spa/tailwind.daisyui.min.css \
    --minify
```

Expected output: a `74 KB`-ish minified CSS file containing Tailwind
v4 core + the DaisyUI classes actually referenced in `template.html`.
Review the diff (`git diff crates/codelore-lib/src/output/spa/tailwind.daisyui.min.css`)
to confirm the regeneration is the expected shape, then commit.

## When to rebuild

- A `template.html` edit introduces classes Tailwind hasn't seen yet —
  the `@source "../template.html"` directive at the top of `input.css`
  scopes the scan, so untouched classes don't bloat the output.
- The Tailwind standalone CLI bumps to a release we want to pick up.
- DaisyUI ships a new version with components / changes we want — see
  "Update DaisyUI" below.

If you forget to rebuild after adding new utility classes, the classes
won't render — the missing styles surface immediately on the next
`codelore analyze --format spa` run.

## Update DaisyUI

DaisyUI 5 distributes its plugin code as two `.mjs` files that need
to sit next to `input.css`. We commit the pinned versions to keep the
build reproducible.

To bump DaisyUI:

```bash
# pick the new tag from https://github.com/saadeghi/daisyui/releases
DAISY_TAG=v5.5.23
BASE="https://github.com/saadeghi/daisyui/releases/download/$DAISY_TAG"
SPADIR=crates/codelore-lib/src/output/spa/tailwind-src

curl -fSL -o "$SPADIR/daisyui.mjs"        "$BASE/daisyui.mjs"
curl -fSL -o "$SPADIR/daisyui-theme.mjs"  "$BASE/daisyui-theme.mjs"

# record the new SHA-256s + version in this README's "Pinned versions"
# table, then run `just spa-css-rebuild` and commit everything together.
```

## Pinned versions

| Layer | Version | Source | SHA-256 |
|---|---|---|---|
| Tailwind v4 standalone CLI | 4.3.1 | <https://github.com/tailwindlabs/tailwindcss/releases/tag/v4.3.1> | (not pinned — user installs via the curl above) |
| `daisyui.mjs` | 5.5.23 | <https://github.com/saadeghi/daisyui/releases/tag/v5.5.23> | `c41cd218e07899f85005ba6da07d59ac38028c6999a6a76cbb2fe3edb6ac1e3f` |
| `daisyui-theme.mjs` | 5.5.23 | <https://github.com/saadeghi/daisyui/releases/tag/v5.5.23> | `72efb4cbf0d1f205988b2f09d917c174a233784ca5c9cc70db729e2712548bb6` |

Bump these together when a release looks worth picking up — there is
no automation watching upstream. Verify SHAs with
`shasum -a 256 daisyui*.mjs` after downloading; they should match the
table above before you commit.
