---
title: Choose your path
description: Where to go after your first model — learn the language, work the CLI, manage projects, set up an editor, integrate, or contribute.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 0b1dc5c
source_of_truth:
  - README.md
---

You know [what sysml-rs is](/sysml-rs/start-here/what-is-sysml-rs/) and have
[run it once](/sysml-rs/start-here/first-model/). What you do next depends on
what you came for.

## Learn the language

If SysML v2 itself is new to you, start with
**[The SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/)** — a
practical guide to the textual notation that builds one model across sixteen
chapters and cites the specification where a claim is load-bearing. Its
samples are parse-checked against this implementation before it ships. It is a
pre-1.0 draft, and the OMG specification remains the authority it defers to.

## Work with models from the command line

The CLI is one of the two stable surfaces of the project (the language server
is the other). [CLI workflows](/sysml-rs/use/cli-workflows/) covers the
day-to-day loop — checking, querying, executing, and exporting models — beyond
the first taste this section gave you. `sysml --help` is always the
authoritative command list.

## Set up a project with dependencies

Once a model outgrows one file, [sysml.toml](/sysml-rs/use/sysml-toml/)
explains the project manifest, and the pages around it cover
[dependencies](/sysml-rs/use/dependencies/) and
[workspaces](/sysml-rs/use/workspaces/). This machinery is a sysml-rs tooling
convention — SysML v2 defines the language, not a package manager — and it is
young: expect the workflow to be functional and the corners to be rough.

## Edit with language support

[Editors](/sysml-rs/use/editors/) covers the language server and the VS Code
extension — diagnostics, semantic highlighting, completion, hover, and
go-to-definition. Any LSP-capable editor can use the server. The desktop
workbench, by contrast, is in active rework; treat it as a preview and keep
your models in files the CLI and LSP can see.

## Integrate via API or MCP

[Integrations](/sysml-rs/use/integrations/) covers the REST/WebSocket server
and the MCP server that exposes models to AI agents. Both dispatch through the
same service layer as the CLI, so behaviour matches. The HTTP server is a
local development server — loopback-only by default, writes unauthenticated
unless you set a token — not something to put on a network as-is.

## Contribute

Bug reports with a minimal reproducing `.sysml` file are genuinely valuable —
the project is pre-alpha and you *will* find constructs it does not handle.
[CONTRIBUTING.md](https://github.com/RickyMillar/sysml-rs/blob/main/CONTRIBUTING.md)
covers developer setup, testing, and the standing rule that language behaviour
follows the OMG specification, with citations. Licensing is MIT/Apache-2.0
dual — see [licensing](/sysml-rs/about/licensing/).
