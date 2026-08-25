---
title: Integrations
description: Drive sysml-rs from other software — the REST/WebSocket API server, the MCP server for AI agents, and the security defaults that govern both.
scope:
  - sysml-rs tooling
  - OMG API subset
status: pre-alpha
last_verified_against: fcd1305
source_of_truth:
  - crates/tooling/sysml-api/src/main.rs
  - crates/tooling/sysml-api/README.md
  - crates/tooling/sysml-mcp/README.md
  - SECURITY.md
---

You want something other than a human at a keyboard to work with your
models — a script, a web frontend, a CI job, or an AI agent. sysml-rs
exposes one service layer through several transports, so the same
operations are available whichever door you come in: the CLI, the LSP
server, the **REST/WebSocket/SSE API server**, and the **MCP server**.
This page covers the last two.

## The REST API server (`sysml-api`)

Build and start it from a source checkout:

```bash
cargo build --release -p sysml-api
./target/release/sysml-api
# SysML API server listening on 127.0.0.1:8080
```

By default it binds **`127.0.0.1:8080` — loopback only**. To bind elsewhere,
pass the address as a positional argument (`sysml-api 0.0.0.0:8080`); the
server prints a warning explaining what a non-loopback bind exposes, and a
second warning if you do it without an auth token set. Read
[security defaults](#security-defaults) before widening anything.

Two endpoints to try first:

```bash
curl http://127.0.0.1:8080/health
# {"status":"ok","version":"0.1.0"}

curl http://127.0.0.1:8080/commands
# [{"category":"Visualization","deprecated":false,
#   "description":"Re-project a diagram with the given expanded-node set, …",
#   "name":"sysml.diagram.expand","params":[…]}, …]
```

`GET /commands` is the **machine-readable catalogue** of every registered
service command, with parameters and descriptions. It is the authoritative
inventory — the docs deliberately never copy a command count out of it.
Each command is callable as `POST /api/commands/{name}` (or through the
generic `POST /api/command` dispatcher) with a JSON body.

On top of the command catalogue there are explicit REST routes for common
reads — model trees, element navigation, diagnostics, queries, view
rendering, trace matrices — and for loading files and driving simulation
sessions. The crate README in `crates/tooling/sysml-api` carries the full
route table.

### Streaming: WebSocket and SSE

- `GET /v1/progress` — **Server-Sent Events** stream of progress
  (`library_load`, `workspace_index`, `dependency_fetch`, `refresh`,
  `ready`), with a keep-alive comment every 15 seconds. Long-running loads
  are observable instead of silent.
- `GET /api/sessions/:id/events` — **WebSocket** stream of live
  simulation/action session events, used by the desktop workbench.
- `GET /lsp` — bridges a full LSP session over WebSocket, for editors that
  prefer a socket to spawning the [server binary](/sysml-rs/use/editors/).

### OMG Systems Modeling API subset vs the native API

**OMG API subset** — the project/commit snapshot routes follow the resource
shape of the OMG Systems Modeling API and Services specification: projects
contain commits, and a commit addresses a model snapshot.

```
GET  /projects
GET  /projects/:project_id/commits
GET  /projects/:project_id/commits/:commit_id/model
POST /projects/:project_id/commits/:commit_id/model
```

This is a small, shape-level subset; sysml-rs makes **no conformance claim**
against the OMG API specification.

Everything else — `/models/:uri/*`, `/commands`, `/api/command(s)`, the
session and streaming endpoints — is the **native sysml-rs API**, not OMG
standard, and can change without a deprecation period while the version
stays `0.x`.

## The MCP server (`sysml-mcp`)

AI agents connect over the
[Model Context Protocol](https://modelcontextprotocol.io): the client
launches `sysml-mcp` as a subprocess and speaks JSON-RPC over stdio. Every
service command is exposed as a tool — loading models, querying the graph,
diagnostics, rendering views, running simulations and trade studies.

```bash
cargo build --release -p sysml-mcp
```

Register it with any MCP client — Claude Code / Claude Desktop
(`.mcp.json` / `claude_desktop_config.json`) or another client's
equivalent:

```json
{
  "mcpServers": {
    "sysml": {
      "command": "/abs/path/to/sysml-rs/target/release/sysml-mcp",
      "args": [],
      "env": { "RUST_LOG": "sysml_mcp=info" }
    }
  }
}
```

Once connected, the agent should call the **`sysml_command_catalog`** tool
for the live tool inventory (the stdio twin of `GET /commands`), and can
start with `sysml_load_workspace` or `sysml_load_source` followed by
`sysml_query`. Responses that touch a model URI carry a `_readiness` field
describing library/workspace load state, so an agent can tell "still
indexing" apart from "actually empty".

Two operational notes: the binary's stdout **is** the transport (all logs
go to stderr), and the server holds models in memory for the lifetime of
the subprocess.

### One process for both: `sysml-api --mcp`

`sysml-api --mcp` runs the HTTP server and an MCP stdio handler over the
**same service instance**: a file loaded through HTTP (say, by the desktop
app) is immediately visible to the agent, and vice versa. Use it when a
human UI and an agent should share one live model; use plain `sysml-mcp`
when the agent should have its own.

## Security defaults

The API server is a **local development server**, and its defaults are
deliberately narrow. Both widen only when you explicitly say so:

- **Bind address `127.0.0.1:8080`** — reachable from this machine only.
  Passing a non-loopback address makes the server warn on startup, and warn
  again if no token is set.
- **Browser origins restricted to loopback** — CORS admits `localhost`,
  `127.0.0.1`, and `[::1]` on any port; every other origin gets no
  allow-origin header. `--permissive-cors` (or
  `SYSML_API_CORS=permissive`) restores allow-any, which is appropriate
  behind a trusted proxy and nowhere else.

Authentication is **off by default**: with `SYSML_API_TOKEN` unset, write
and command routes are open. Set it, and those routes require
`Authorization: Bearer <token>`; read routes are never authenticated.
There is no rate limiting, and the 50 MB request body limit is a resource
guard, not a security control.

Anyone who can reach the port can read and modify the loaded model and run
simulations on your machine — so keep the server on loopback, or put it
behind your own authenticating proxy. The project's
[SECURITY.md](https://github.com/RickyMillar/sysml-rs/blob/main/SECURITY.md)
states the full threat model and how to report a vulnerability privately.

The MCP server has no network surface of its own — it is stdio-only, with
the privileges of whatever launched it (it reads files your user can read).
The `--mcp` combined mode carries the HTTP surface above, with the same
defaults.
