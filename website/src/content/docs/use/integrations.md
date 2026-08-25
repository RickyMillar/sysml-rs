---
title: Integrations
description: One service layer, many doors — choose the sysml-rs interface for your job, with the security defaults that govern the networked ones.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - crates/tooling/sysml-api/README.md
  - crates/tooling/sysml-mcp/README.md
  - SECURITY.md
---

You want something other than a human at a keyboard to work with your
models — a script, an editor, a CI job, a web UI, or an AI agent. sysml-rs
exposes **one service layer through several transports**: every interface
dispatches into the same command registry, so an operation behaves
identically whichever door you come in through. Pick the door, and this
page points you at its documentation.

## Which interface for which job

| Interface | It's for | Page |
|---|---|---|
| **CLI** (`sysml`) | Terminal and CI workflows: parse, check, export, simulate, verify | [CLI workflows](/sysml-rs/use/cli-workflows/) |
| **Language server** (`sysml-lsp-server`) | Diagnostics, completion, and navigation in any LSP-capable editor | [The language server](/sysml-rs/use/lsp/) · [VS Code setup](/sysml-rs/use/editors/) |
| **REST / WebSocket / SSE** (`sysml-api`) | Scripting over HTTP, custom frontends, streaming progress and session events | [The service API](/sysml-rs/use/service-api/) |
| **MCP** (`sysml-mcp`) | AI agents — every service command as a callable tool over stdio | [MCP for AI agents](/sysml-rs/use/mcp/) |
| **Simulation App** | A browser workbench: browse, run, verify, requirements, analyze | [The Simulation App](/sysml-rs/use/simulation-app/) |

The machine-readable inventory of what all of these can do is the API
server's `GET /commands` (its MCP twin is the `sysml_command_catalog`
tool); a rendered snapshot is published as the
[API & MCP catalogue](/sysml-rs/reference/api-mcp-catalog/).

Combinations work too: `sysml-api --mcp` runs the HTTP server and an MCP
handler over the **same live model**, so a human in the Simulation App and
an AI agent can share one session — see
[MCP for AI agents](/sysml-rs/use/mcp/).

## Security defaults, in brief

The networked interface is a **local development server**, and its
defaults are deliberately narrow:

- `sysml-api` binds **`127.0.0.1:8080` — loopback only** — and warns at
  startup if you bind wider (twice, if you do it with no auth token set).
- **CORS admits loopback origins only** (`localhost`, `127.0.0.1`, `[::1]`);
  `--permissive-cors` restores allow-any and belongs behind a trusted
  proxy, nowhere else.
- **Authentication is off by default.** Setting `SYSML_API_TOKEN` gates
  write and command routes behind `Authorization: Bearer <token>`; read
  routes are never authenticated. There is no rate limiting.
- The **MCP server has no network surface** — stdio only, with the
  privileges of whatever launched it.

Anyone who can reach the API port can read and modify the loaded model and
run simulations on your machine, so keep it on loopback or behind your own
authenticating proxy. The full defaults, with the verification behind each
claim, are on [the service API page](/sysml-rs/use/service-api/#security-defaults);
the threat model and private vulnerability reporting are in
[SECURITY.md](https://github.com/RickyMillar/sysml-rs/blob/main/SECURITY.md).
