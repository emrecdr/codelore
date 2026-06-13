# Tailwind v4 + DaisyUI 5 — one-time CSS rebuild workflow

The SPA dashboard's CSS is **precompiled and committed** to the repo as
`../tailwind.daisyui.min.css`. `build.rs` then SHA-pins it as a build
asset and inlines it into the rendered HTML the same way ECharts and
d3-hierarchy are inlined. This file describes how to regenerate the
compiled CSS when DaisyUI / Tailwind bumps.

## Why precompile

Tailwind v4 dropped the production-grade runtime CDN script that v3
shipped. The recommended distribution path is the standalone CLI which
scans templates at build time and emits the pruned utility set. For
`CodeLore`'s offline-first build flow, precompiling locally + checking
the result in is the simplest option that preserves both:

- the single-file `--format spa` output (no runtime CDN dependency), and
- the `build.rs` SHA-pin trust manifest (the compiled CSS gets the same
  SHA-256 audit treatment as ECharts).

The trade-off is a periodic chore (this README) when DaisyUI / Tailwind
ships a meaningful release.

## Install the standalone CLI

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
# Pick your platform's URL from the table above
curl -sL -o ~/.local/bin/tailwindcss \
    https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.1/tailwindcss-macos-arm64
chmod +x ~/.local/bin/tailwindcss
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

Review the diff (`git diff crates/codelore-lib/src/output/spa/tailwind.daisyui.min.css`)
to confirm the regeneration is the expected shape, then commit.

## When to rebuild

- DaisyUI bumps with new component vocabulary you want to adopt.
- Tailwind core bumps with utility names we use.
- A `template.html` edit introduces classes Tailwind hasn't seen yet —
  the `@source "../template.html"` directive at the top of `input.css`
  scopes the scan, so untouched classes don't bloat the output.

If you forget to rebuild after adding new utility classes, the
classes won't render — the missing styles surface immediately on the
next `codelore analyze --format spa` run.

## Pinned versions

| Layer | Version | Notes |
|---|---|---|
| Tailwind v4 standalone CLI | 4.3.1 | tracked manually here; CLI version comes from the GitHub releases page |
| DaisyUI | 5 (bundled with the CLI) | the standalone CLI ships with the DaisyUI plugin built in |

Bump these together when a release looks worth picking up — there is
no automation watching upstream.
