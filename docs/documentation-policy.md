# Documentation scope and verification policy

This policy applies to public sysml-rs documentation, including future portal pages, repository guides, and sysml-rs-specific callouts in the SysML v2 Book.

See [ADR 0001](adr/0001-documentation-architecture.md) for the documentation architecture.

## Scope labels

State the scope near the start of a page and beside claims that cross a boundary.

| Label | Use for | Do not imply |
|---|---|---|
| **SysML v2 / KerML** | OMG-standard syntax, semantics, libraries, notation, or interchange concepts. | That every SysML tool implements the concept, or that sysml-rs fully supports it. |
| **sysml-rs implementation** | Parser, lowering, semantic graph, validation, coverage, or internal architecture. | OMG conformance or a stable public Rust API. |
| **sysml-rs tooling** | CLI commands, manifests, lock files, caches, sessions, UI, transports, API, MCP, and operational workflows. | A portable SysML requirement. |
| **OMG API subset** | A standard API operation implemented by sysml-rs. | That native sysml-rs API additions are standard. |
| **Experimental / partial support** | A limited implementation, preview surface, known gap, or unstable workflow. | Completeness, certification, or backward compatibility. |

A page can carry more than one label. Labels must be text, not colour alone.

## Required page metadata

New portal pages should use this frontmatter shape. Existing Markdown pages may add it when they are materially revised or migrated.

```yaml
---
title: Projects, dependencies, and workspaces
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 6299655
source_of_truth:
  - crates/lang/sysml-manifest/src/manifest.rs
  - crates/tooling/sysml-cli/tests/project_init.rs
known_limitations: /reference/known-limitations/#dependency-resolution
---
```

- `scope` uses one or more labels above.
- `status` is normally `pre-alpha`, `experimental`, `partial`, or `stable`; do not use `stable` for a surface that lacks a published compatibility commitment.
- `last_verified_against` is a release tag, commit, generated-catalogue revision, or dated external standard reference.
- `source_of_truth` names the code, test, specification clause, or generated artefact that supports the page.
- `known_limitations` is optional, but required when a material limitation changes how a reader should use the feature.

## Evidence required for claims

| Claim | Required evidence |
|---|---|
| SysML v2 / KerML language claim | OMG source or clause reference; checked example where practical. |
| sysml-rs implementation claim | Relevant source plus a focused test, corpus result, or capability evidence. |
| CLI, manifest, API, or MCP behaviour | Executable help, contract/integration test, or generated catalogue. |
| Runtime or solver behaviour | Reproducible scenario/test and documented assumptions. |
| Performance statement | Command, input, hardware/environment, date/commit, and result. |
| Security/default exposure statement | Source/configuration evidence and security review where applicable. |

Avoid absolute claims such as “supports SysML v2” or “conformant” unless their scope and evidence are stated. sysml-rs is a pre-alpha partial implementation and makes no OMG conformance claim.

## Canonical sources during transition

| Topic | Canonical source | Documentation use |
|---|---|---|
| Language teaching and reference | `RickyMillar/sysmlv2-book` | Book content; cite OMG material for load-bearing claims. |
| Installation and pre-alpha positioning | repository `README.md` | Portal installation page may replace it after a tested cutover. |
| CLI syntax and operational behaviour | Clap help and `crates/tooling/sysml-cli` contract/integration tests | Generate or test reference pages; do not hand-maintain flag inventories. |
| `sysml.toml` and `sysml.lock` | `crates/lang/sysml-manifest` schema/parser tests | Validate every published TOML example. |
| Dependency resolution | `crates/tooling/sysml-resolve` provider tests | Identify provider, cache, integrity, and failure-mode scope. |
| Native service/API/MCP catalogue | `sysml-service` registry and transport contract tests | Distinguish native operations from OMG API subset support. |
| Diagnostics | diagnostic registry and tests | Prefer generated reference material. |
| Language support status | `tools/spec-index` evidence plus test results | Publish commit/version and measurement scope. |
| Implementation architecture | `docs/developer_guide/` plus crate source | Keep user journeys separate from crate-layer detail. |

## Authoring rules

1. Start user pages with the reader's goal, not a crate name.
2. Link from language material to tooling documentation instead of duplicating tooling procedures.
3. Explain the difference between a model-level `import` and a project dependency wherever both are relevant.
4. Do not copy hard-coded command, tool, or coverage counts when a catalogue or test can provide the value.
5. Keep static Book diagrams self-contained; documentation rendering must not depend on a live API server.
6. Run the smallest relevant documentation and executable-example checks before review.
