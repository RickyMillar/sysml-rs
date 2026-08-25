---
title: How this documentation is maintained
description: The verification policy, generation workflow, and review cadence behind the sysml-rs documentation portal.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: c5e9e06
source_of_truth:
  - docs/documentation-policy.md
  - docs/adr/0001-documentation-architecture.md
  - website/content-lock.json
---

This portal is docs-as-code: pages live in the
[sysml-rs repository](https://github.com/RickyMillar/sysml-rs/tree/main/website),
change through review, and are gated in CI. This page says what keeps them
honest.

## The rules pages follow

- Every page that makes a language, implementation, or tooling claim carries
  **scope badges** and an **evidence footer** (source of truth, the commit it
  was last verified against) — rendered from required frontmatter, per the
  repository's
  [documentation policy](https://github.com/RickyMillar/sysml-rs/blob/main/docs/documentation-policy.md).
- **Commands are run before they are published.** A snippet on a guide page
  was executed against a real build and shows real output, or it is labelled
  illustrative.
- **No hand-maintained inventories.** Command lists, catalogues, and counts
  are generated from the tool or referred to the live catalogue.
- **Defects are listed only while they reproduce.** Every entry on
  [Known limitations](/sysml-rs/reference/known-limitations/) was re-verified
  against the build named in its footer.

## External content is pinned

The Learn section is the SysML v2 Book, built from its
[own repository](https://github.com/RickyMillar/sysmlv2-book) at the exact
commit recorded in
[`website/content-lock.json`](https://github.com/RickyMillar/sysml-rs/blob/main/website/content-lock.json).
No documentation build fetches an unpinned revision of another repository.
Advancing the pin is a deliberate, reviewed change.

## What CI enforces

Every push and pull request touching `website/` builds the site, validates
every internal link and anchor, and runs an accessibility smoke check (page
titles, heading structure, iframe titles, image alt text). Deploys happen
only from `main`.

## Cadence

- **On release:** regenerate the generated reference artifacts and refresh
  install instructions.
- **Monthly:** link and drift review — external links, the Book pin, and
  generated catalogues against the tool.
- **Quarterly:** boundary audit — do any pages present sysml-rs conventions
  as SysML v2 language rules, or vice versa.

## Reporting problems

Documentation issues are welcome in the
[issue tracker](https://github.com/RickyMillar/sysml-rs/issues). For a
behavioural claim, say what you ran and what you saw; for a language claim,
an OMG citation settles it.
