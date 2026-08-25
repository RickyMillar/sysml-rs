---
title: The service API
description: Script against sysml-rs over HTTP — one command registry behind REST, WebSocket, and SSE, served by sysml-api.
scope:
  - sysml-rs tooling
  - OMG API subset
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - crates/tooling/sysml-api/src/main.rs
  - crates/tooling/sysml-api/README.md
  - crates/tooling/sysml-api/src/lib.rs
  - SECURITY.md
---

You want to drive sysml-rs from a script, a CI job, a web frontend, or any
program that speaks HTTP. That is what **`sysml-api`** is for: an HTTP
server that exposes the sysml-rs service layer over REST, WebSocket, and
Server-Sent Events.

## One hub, many doors

Every sysml-rs interface — the [CLI](/sysml-rs/use/cli-workflows/), the
[language server](/sysml-rs/use/lsp/), this API server, and the
[MCP server](/sysml-rs/use/mcp/) — is a thin transport over the same
service-layer **command registry**. A command like `sysml.load_workspace`
or `sysml.verify` behaves identically whichever door you call it through;
the transports only translate wire formats. So anything you see an agent
or the [Simulation App](/sysml-rs/use/simulation-app/) do, you can do
with `curl`.

## Starting the server

```bash
cargo build --release -p sysml-api
./target/release/sysml-api
# SysML API server listening on 127.0.0.1:8080
```

By default it binds **`127.0.0.1:8080` — loopback only**. To bind elsewhere,
pass the address as a positional argument (`sysml-api 0.0.0.0:8080`); the
server warns on startup about what a non-loopback bind exposes, and warns
again if you do it with no auth token set. Read
[security defaults](#security-defaults) before widening anything.

Check it is alive:

```bash
curl http://127.0.0.1:8080/health
# {"status":"ok","version":"0.1.0"}
```

## The command catalogue

`GET /commands` is the machine-readable inventory of every registered
service command — name, category, description, and a parameter schema for
each. It is the authoritative catalogue; these docs deliberately never copy
a command count out of it. A rendered snapshot is published as the
[API & MCP catalogue](/sysml-rs/reference/api-mcp-catalog/) reference page.

```bash
curl http://127.0.0.1:8080/commands
# [{"category":"Execution","deprecated":false,
#   "description":"Sweep an ODE parameter across a range, …",
#   "name":"sysml.trade_study.ode_sweep",
#   "params":[{"name":"uri","required":true,"ty":"string", …}, …]}, …]
```

## Calling a command

Every catalogued command is callable as `POST /api/commands/{name}` with a
JSON body of its parameters (there is also a generic `POST /api/command`
dispatcher taking `{"command": …, "params": …}`). A real round-trip —
load a model from source, then search it:

```bash
curl -X POST http://127.0.0.1:8080/api/commands/sysml.load_source \
  -H 'Content-Type: application/json' \
  -d '{"uri":"demo.sysml","source":"package Demo { part def Vehicle { attribute mass : ScalarValues::Real; } part car : Vehicle; }"}'
# null    (success; the model is now held in server memory as demo.sysml)

curl -X POST http://127.0.0.1:8080/api/commands/sysml.find \
  -H 'Content-Type: application/json' \
  -d '{"uri":"demo.sysml","pattern":"Vehicle"}'
# [{"id":"b174e4cd-…","kind":"PartDefinition","name":"Vehicle",
#   "name_span":{"col":24,"end":31,"file":"demo.sysml","line":1, …}, …}]
```

Parameter errors come back as `{"error": "invalid input: …"}` naming the
missing or malformed field, so the fastest way to learn a command's shape
is to read its `/commands` entry and let the server correct you.

## Convenience REST routes

On top of the command registry there are explicit REST routes for common
reads, so simple lookups don't need a POST body: model trees, element
navigation (`/models/:uri/elements/:id`, `…/children`, `…/ancestors`),
statistics, diagnostics, trace matrices, view rendering, and JSON export.
For example:

```bash
curl http://127.0.0.1:8080/models/demo.sysml/stats
# {"elements_by_kind":{"AttributeUsage":1,"FeatureTyping":2, …},
#  "total_elements":11,"total_relationships":0}
```

The full route table lives in the crate README at
`crates/tooling/sysml-api/README.md`. Reads are unauthenticated; writes and
command dispatch are token-gated when a token is configured (below). New
structured reads are added to the command registry rather than as new REST
aliases, so the catalogue is the surface that grows.

## Streaming: WebSocket and SSE

- `GET /v1/progress` — **Server-Sent Events** stream of progress events
  (`library_load`, `workspace_index`, `dependency_fetch`, `refresh`,
  `ready`), with a keep-alive comment every 15 seconds. Long-running loads
  are observable instead of silent. The companion snapshot endpoint
  `GET /v1/readiness/:uri` reports the current library/project/file load
  state for one model URI.
- `GET /api/sessions/:id/events` — **WebSocket** stream of live
  simulation/action session events, used by the
  [Simulation App](/sysml-rs/use/simulation-app/).
- `GET /lsp` — bridges a full LSP session over WebSocket, for editors that
  prefer a socket to spawning the server binary — see
  [the language server](/sysml-rs/use/lsp/#the-websocket-bridge).

## OMG Systems Modeling API subset vs the native API

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
`Authorization: Bearer <token>` — a POST without the header gets `401`,
with it `200`; read routes are never authenticated. There is no rate
limiting, and the 50 MB request body limit is a resource guard, not a
security control.

Anyone who can reach the port can read and modify the loaded model and run
simulations on your machine — so keep the server on loopback, or put it
behind your own authenticating proxy. The project's
[SECURITY.md](https://github.com/RickyMillar/sysml-rs/blob/main/SECURITY.md)
states the full threat model and how to report a vulnerability privately.

## Sharing the instance with an AI agent

`sysml-api --mcp` runs this HTTP server and an MCP stdio handler over the
**same service instance**, so a human UI and an agent see one live model —
details on [the MCP page](/sysml-rs/use/mcp/).
