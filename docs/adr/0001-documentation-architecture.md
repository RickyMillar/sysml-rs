# ADR 0001: Separate language, implementation, and tooling documentation

- **Status:** accepted
- **Date:** 2026-08-22

## Context

sysml-rs has two public documentation sources:

- [`RickyMillar/sysmlv2-book`](https://github.com/RickyMillar/sysmlv2-book), an example-first guide to textual SysML v2; and
- this repository's README files and developer guides, which document a parser, semantic model, runtime, CLI, editors, API, and MCP server.

Some existing Book material describes sysml-rs conventions such as `sysml.toml`, lock files, sessions, and solver behaviour. Conversely, component documentation can describe implementation details without identifying whether the underlying concept is portable SysML or a sysml-rs choice. This risks treating tool behaviour as OMG-standard language behaviour.

## Decision

Create a documentation portal in `website/`, built with Astro Starlight and initially published as `main`/pre-alpha documentation on the repository's GitHub Pages project path.

The portal has three primary content areas:

1. **Learn SysML v2** — portable SysML v2 and KerML concepts. The SysML v2 Book remains the canonical language-prose source during transition.
2. **Use sysml-rs** — user workflows and product references: projects, dependencies, workspaces, CLI, runtime, editors, diagrams, API, and MCP.
3. **Develop sysml-rs** — crate architecture, implementation details, testing, and contribution guidance.

Pages must use the scope/status policy in [`../documentation-policy.md`](../documentation-policy.md). In particular:

- a SysML `import` is language documentation, while a `sysml.toml` dependency is tooling documentation;
- a standard modelling pattern may be portable while its session, solver, renderer, or transport behaviour is sysml-rs-specific;
- the Book may link to tooling documentation but must label any tool-specific callout;
- the physical `crates/lang/` directory does not make every behaviour a normative language claim.

The Book will be consumed at a pinned revision when it is integrated into the portal. Its CC-BY-4.0 prose licence, code/example terms, attribution, and renderer-asset notices must be preserved in the built site.

## Consequences

- A portal scaffold and Pages workflow may be added without immediately moving Book content.
- Existing Book Pages remains usable until portal routes and redirects have been tested.
- Chapter 15, Chapter 16, and tooling-focused appendices will be split gradually; they will not be copied wholesale into language documentation.
- Generated/reference content is preferred for moving product surfaces such as CLI commands, API/MCP catalogues, diagnostics, and support status.
- This is docs-as-code, not an unreviewed editable wiki. Product and language claims require evidence appropriate to their scope.
- Custom-domain and release-versioned documentation decisions are deferred; the initial site documents the pre-alpha `main` branch.
