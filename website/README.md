# sysml-rs documentation portal

The public documentation portal, built with [Astro Starlight](https://starlight.astro.build/)
and deployed to GitHub Pages at <https://rickymillar.github.io/sysml-rs/> by
`.github/workflows/docs-portal.yml`.

The portal follows the repository's
[documentation scope and verification policy](../docs/documentation-policy.md)
and [ADR 0001](../docs/adr/0001-documentation-architecture.md). Read both
before authoring pages.

## Local development

Requires the Node version in `.nvmrc`.

```bash
cd website
npm ci
npm run dev       # live-reload dev server
npm run build     # production build; fails on broken internal links
npm run preview   # serve the production build locally
npm run check     # astro type-check
npm run a11y      # accessibility smoke check over dist/
```

CI runs `check`, `build`, and `a11y` on every push and pull request that
touches `website/`; deploys happen only from `main`.

## Authoring a page

Pages live under `src/content/docs/`. Every page that makes a language,
implementation, or tooling claim carries the documentation-policy frontmatter;
the scope/status badges and the evidence footer render automatically from it.

```markdown
---
title: Projects, dependencies, and workspaces
description: One-sentence summary used in search and link previews.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: <commit or tag the claims were checked against>
source_of_truth:
  - crates/lang/sysml-manifest/src/manifest.rs
  - crates/tooling/sysml-cli/tests/project_init.rs
known_limitations: /sysml-rs/reference/known-limitations/#dependency-resolution
---

Start with the reader's goal, not a crate name. Label anything that is a
sysml-rs convention rather than OMG-standard behaviour.
```

Valid `scope` labels (see the policy for meanings): `SysML v2 / KerML`,
`sysml-rs implementation`, `sysml-rs tooling`, `OMG API subset`,
`Experimental / partial support`.

MDX pages can use the callout components:

```mdx
import ToolNote from '../../components/ToolNote.astro';
import Experimental from '../../components/Experimental.astro';
import KnownLimitation from '../../components/KnownLimitation.astro';
```

## Rules that keep the site honest

- Every command snippet must be executed against a real build before
  publishing, or explicitly labelled illustrative.
- No hand-maintained counts (commands, tools, crates) — generate them or write
  "the live catalogue is authoritative".
- External content is consumed only at revisions pinned in
  `content-lock.json`; no build may fetch an unpinned `main` of another
  repository.
- Internal links are validated at build time; the a11y smoke check enforces
  titles, single `h1`, iframe titles, and image alt text.
