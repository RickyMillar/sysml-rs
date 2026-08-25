---
title: What sysml-rs is (and is not)
description: Honest pre-alpha positioning for sysml-rs, a partial Rust implementation of OMG SysML v2.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 8d4ceb3
source_of_truth:
  - README.md
---

sysml-rs is a Rust implementation of the OMG **SysML v2** specification: an
incremental parser, a semantic model, an execution and physics runtime, and the
tooling that sits on top — a CLI, a language server, REST and MCP services, a
VS Code extension, and a desktop workbench.

Text goes in and a queryable, executable model comes out. The pipeline is
**source → tree-sitter CST → semantic graph → name resolution and validation →
execution, queries, and diagrams**, and every transport (CLI, LSP, HTTP, MCP)
dispatches through the same service layer, so the same operation behaves
identically everywhere.

## What it is not

sysml-rs implements a **substantial subset** of SysML v2, tracked against the
OMG specification. It is not a complete or certified implementation, it has not
been through any OMG conformance process, and it makes no conformance claim.
Expect constructs that do not parse yet, semantics implemented for the common
cases but not every corner, and interfaces that change without a deprecation
period while the version stays `0.x`.

It is useful today for reading, checking, querying, and executing models you
write against the subset it supports. It is not ready to be the system of
record for a programme you cannot afford to migrate.

## Where it is rough

- **Language coverage is partial.** The grammar is derived from the OMG Xtext
  grammars, but not every production is wired through to full semantics.
  Unsupported syntax surfaces as a parse diagnostic rather than silently doing
  something else.
- **The execution runtime is the youngest part.** Continuous dynamics, hybrid
  models, and verification cases work on the repository examples, which the
  regression suite locks down. Novel model shapes will find gaps.
- **The desktop workbench and diagram surfaces are in active rework.** The CLI
  and LSP are the stable surfaces.
- **No stable Rust API yet.** Workspace crates are not on crates.io and depend
  on each other by path.
- **The HTTP API is a local development server.** It binds loopback only and
  writes are unauthenticated unless a token is set; widening either is opt-in.

## Where to go next

- Learn the language in the
  [SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/).
- Build the tool from source using the
  [repository README quick start](https://github.com/RickyMillar/sysml-rs#readme)
  (a portal installation guide is on its way).
- Contribute via
  [CONTRIBUTING](https://github.com/RickyMillar/sysml-rs/blob/main/CONTRIBUTING.md).
