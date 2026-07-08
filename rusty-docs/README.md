# Rusty Docs

The Rusty documentation site — built with [Astro](https://astro.build) +
[Starlight](https://starlight.astro.build), themed to match the Rusty brand:
Bricolage Grotesque headings, Hanken Grotesk body, JetBrains Mono code,
rust-orange accents, warm-paper (light) / ember (dark) palettes, and dark
"terminal" code blocks.

## Prerequisites

- **Node.js 18.20.8+, 20.3.0+, or 22+** (Astro 5 requirement)
- npm (or pnpm / yarn / bun)

## Local development

```bash
npm install
npm run dev        # live preview at http://localhost:4321
```

Other scripts:

```bash
npm run build      # static site → ./dist
npm run preview    # serve the built ./dist locally
```

> **After changing `astro.config.mjs`** (fonts, code-block theme, i18n), restart
> the dev server — Expressive Code compiles its styles at startup and a
> hot-reload won't pick those up:
>
> ```bash
> rm -rf node_modules/.astro .astro dist && npm run dev
> ```


## Project structure

```
rusty-docs/
├── astro.config.mjs        # Starlight config: title, logo, sidebar, code theme
├── package.json
├── tsconfig.json
├── public/
│   └── favicon.svg
└── src/
    ├── content.config.ts   # docs content collection
    ├── assets/
    │   └── rusty-mark.svg   # header logo (mascot)
    ├── styles/
    │   └── rusty.css        # ← the entire Rusty theme lives here
    └── content/docs/        # your Markdown / MDX pages
        ├── index.mdx
        ├── getting-started/
        ├── guides/
        ├── configuration/
        ├── tools/
        └── reference/
```

## Writing content

Pages are Markdown (`.md`) or MDX (`.mdx`) in `src/content/docs/`. Each needs
frontmatter with at least a `title`:

```md
---
title: My Page
description: One-line summary for SEO + the page header.
---

## First section
...
```

- **Admonitions** use Starlight asides: `:::note`, `:::tip`, `:::caution`,
  `:::danger` (with an optional custom title: `:::caution[Heads up]`).
- **Tabs** and **cards** need MDX — import them at the top of an `.mdx` file:
  `import { Tabs, TabItem, CardGrid, LinkCard } from '@astrojs/starlight/components';`
- **Code blocks** render as a dark terminal panel. Add a title with
  ` ```bash title="terminal" `; shell languages get terminal chrome automatically.
- **Sidebar order** is defined explicitly in `astro.config.mjs` → `sidebar`.
  Add new pages there.

## Languages (i18n)

The site ships bilingual: **English** (default, served at `/`) and
**简体中文** (served at `/zh-cn/`). Starlight shows a language picker next to the
theme selector automatically.

- Locales are declared in `astro.config.mjs` under `locales`, and every sidebar
  entry has a `translations['zh-CN']` label.
- English pages live in `src/content/docs/…`; their Chinese counterparts live in
  `src/content/docs/zh-cn/…` with the same file names.
- **All pages are translated into 简体中文.** Add a new page in both
  `src/content/docs/<path>` and `src/content/docs/zh-cn/<path>`; if a Chinese
  version is missing, Starlight automatically falls back to the English content.
- To add/remove a language, edit `locales` in `astro.config.mjs` (and the
  `translations` maps on the sidebar).

## Customizing the look

Everything visual is in **`src/styles/rusty.css`**, driven by `--rz-*` tokens at
the top (one block for dark/Ember, one for light/Daylight). Change the palette
there and the whole site follows. Code-block colors live in `astro.config.mjs`
under `expressiveCode.styleOverrides`.

To make code blocks follow light/dark instead of always-dark, set
`expressiveCode.themes: ['github-light', 'github-dark']` in `astro.config.mjs`
and remove the `codeBackground` override.

## Deploy to Cloudflare Pages

Starlight builds a fully static site, so no adapter is needed.

**Option A — Git integration (recommended)**

1. Push this folder to a GitHub/GitLab repo.
2. In the Cloudflare dashboard: **Workers & Pages → Create → Pages →
   Connect to Git**, and pick the repo.
3. Set the build settings:
   - **Framework preset:** `Astro`
   - **Build command:** `npm run build`
   - **Build output directory:** `dist`
   - **Node version:** set env var `NODE_VERSION` = `22` (Pages defaults can be old)
4. **Save and Deploy.** Every push redeploys automatically.

**Option B — Direct upload with Wrangler**

```bash
npm install
npm run build
npx wrangler pages deploy dist --project-name rusty-docs
```

### Custom domain

In the Pages project → **Custom domains**, add `docs.rustycli.com` (or your
domain) and point the DNS `CNAME` as instructed. Then update `site:` in
`astro.config.mjs` to that URL for correct canonical links + sitemap.

---

Built to match [rustycli.com](https://rustycli.com). Theme lives in
`src/styles/rusty.css`.
