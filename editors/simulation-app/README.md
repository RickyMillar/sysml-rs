# SysML v2 Simulation App

The workbench frontend for `sysml-rs`: a React + TypeScript app over the
unified service backend (`sysml-api`). One model, several focused
workbenches — **Browse · Run · Verify · Requirements · Analyze** as front
doors, with **Compare** as a Simulate mode (reached from the frame session
switcher or Cmd-K, not a nav tab).

## Quickstart

Two processes — the backend on **:8080**, this app's Vite dev server on
**:3010** (which proxies `/api`, `/sessions`, … to the backend):

```bash
# from the repo root
cargo build --release -p sysml-api
./target/release/sysml-api

# in a second terminal
cd editors/simulation-app && npm install && npm run dev   # http://localhost:3010
```

Open a workspace via deep link (the `?workspace=` param auto-loads and
always wins):

```
http://localhost:3010/run?workspace=<abs path to a model folder>
```

## The demo

**The canonical guided demo is the espresso production-cell walkthrough:**
— load `examples/espresso-production-cell`, drive a live orchestrator session,
watch each station's group-head temperature climb into the brew band, read the
verification verdict matrix in Verify, then switch to the compact
`examples/espresso-pump-hybrid` to drive its event-driven safety-relief latch on
a breakpoint and fork-and-compare branches on the Compare diff canvas. §4 of that
recipe is the current in-app beat-by-beat; §2 keeps the equivalent wire-level `curl`s.

A second, static-model tour for the Requirements workbench lives at
`examples/espresso-production-cell`
(`/requirements?workspace=…/examples/espresso-production-cell`).

## Development

- `npm run dev` — Vite dev server on :3010
- `npx tsc --noEmit` — typecheck
- `npx vitest run` — unit/component suites
- `npx playwright test --project=ninebar` — the (CI-blocking) new-shell e2e
  project; legacy projects run pinned flag-off until the Phase 8 deletion.

Architecture notes live in the ninebar plan
docs beside it; the session API contract is
