---
title: The Simulation App
description: The SysML v2 Simulation App — a browser workbench over sysml-api for browsing, running, verifying, and analyzing models.
scope:
  - sysml-rs tooling
  - Experimental / partial support
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - editors/simulation-app/README.md
  - editors/simulation-app/package.json
  - editors/simulation-app/vite.config.ts
---

You want to *see* a model — browse its structure, drive a live simulation,
watch verification verdicts land — without writing a query for every step.
The **SysML v2 Simulation App** (in `editors/simulation-app`) is the
project's workbench frontend: a React + TypeScript app that runs in your
browser and talks to a local [`sysml-api` server](/sysml-rs/use/service-api/).
It holds no model state of its own — everything on screen is the same
service layer the CLI, LSP, and MCP interfaces expose.

## What's in it

One model, several focused workbenches:

- **Browse** — the model tree, element details, source, and
  [diagram views](/sysml-rs/use/views-and-diagrams/).
- **Run** — live simulation sessions over the
  [runtime](/sysml-rs/use/runtime/): start an orchestrator, step, inject,
  watch timeseries.
- **Verify** — verification cases and the verdict matrix.
- **Requirements** — the requirements workbench: documents, traces,
  suspect links.
- **Analyze** — analysis cases, sweeps, and trade studies.
- **Compare** — a Simulate mode (reached from the session switcher or
  Cmd-K, not a nav tab) for diffing forked simulation branches.

## Running it from source

Two processes: the backend on **:8080**, the app's Vite dev server on
**:3010**, which proxies API, session-WebSocket, and `/lsp` traffic to the
backend.

```bash
# from the repo root
cargo build --release -p sysml-api
./target/release/sysml-api

# in a second terminal
cd editors/simulation-app
npm install
npm run dev            # http://localhost:3010
```

Then open a workspace by deep link — the `?workspace=` parameter auto-loads
a model folder:

```
http://localhost:3010/run?workspace=/abs/path/to/examples/espresso-production-cell
```

The canonical guided tour is the **espresso production cell** example
(`examples/espresso-production-cell` in the repository): drive a live
orchestrator session in Run, watch group-head temperatures climb into the
brew band, read the verdict matrix in Verify — and the compact
`examples/espresso-pump-hybrid` for breakpoints and fork-and-compare on the
Compare canvas.

Other `package.json` scripts: `npm run build` (production build),
`npm run typecheck`, `npm run test` (vitest), `npm run test:e2e`
(Playwright). A Tauri desktop shell exists behind `npm run dev:desktop` /
`npm run build:desktop`.

## Status: preview

**Experimental / partial support.** The app is developed against the
pre-alpha backend and is not packaged or released — running from a source
checkout as above is the only supported path today. Expect rough edges,
UI churn, and no compatibility commitment; the backend it fronts binds to
loopback by default with the
[security defaults](/sysml-rs/use/service-api/#security-defaults) of a
local development server.

## The embeddable diagram viewer

The same codebase ships `embed.html`, a standalone build of the app's
diagram renderer. It is what powers the interactive diagrams in the
[SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/): export a view
as ViewModel JSON, drop the static embed build next to it, and the diagram
renders with no live server. The export-and-embed workflow is documented on
[views and diagrams](/sysml-rs/use/views-and-diagrams/#rendering-static-vs-interactive).

## Scripting what the app does

Every button in the app dispatches a service command over the
[HTTP API](/sysml-rs/use/service-api/) — the app's dev proxy forwards to
`GET /commands`-catalogued endpoints and the session WebSocket. If you want
to automate a workflow you first clicked through in the app, the
[service API page](/sysml-rs/use/service-api/) shows the same operations
as `curl` calls.
