---
title: MCP for AI agents
description: Connect Claude Code, Claude Desktop, or any MCP client to sysml-rs — every service command as a callable tool over stdio.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - crates/tooling/sysml-mcp/README.md
  - crates/tooling/sysml-mcp/src/lib.rs
  - crates/tooling/sysml-api/src/main.rs
---

You want an AI agent to load SysML v2 models, query the model graph, run
simulations, and read verification verdicts. **`sysml-mcp`** is the
project's [Model Context Protocol](https://modelcontextprotocol.io) server:
the client launches it as a subprocess and speaks JSON-RPC over stdio, and
every command in the sysml-rs service registry is exposed as a named tool.
The tool names are mechanical translations of the command names
(`sysml.load_workspace` → `sysml_load_workspace`), so anything you can do
over the [HTTP API](/sysml-rs/use/service-api/) an agent can do through a
tool call.

## Build and register it

```bash
cargo build --release -p sysml-mcp
# binary at target/release/sysml-mcp
```

Register the binary with your MCP client. For **Claude Code**, a project
`.mcp.json`; for **Claude Desktop**, the same object inside
`claude_desktop_config.json`:

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

Any other MCP client configures the same way: an absolute command path, no
arguments, stdio transport. The server advertises the `tools` and `logging`
capabilities on initialize and holds loaded models in memory for the
lifetime of the subprocess.

## What the handshake looks like

You never speak the protocol by hand — the client does — but it is worth
seeing once. This exchange was run against the real binary with a scripted
stdio client (newline-delimited JSON-RPC):

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05", …}}
← {"protocolVersion":"2024-11-05","serverInfo":{"name":"rmcp","version":"1.8.0"},
   "capabilities":{"logging":{…},"tools":{…}}}

→ {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sysml_load_source",
   "arguments":{"uri":"demo.sysml","source":"package Demo { part def Vehicle; part car : Vehicle; }"}}}
← {"content":[{"type":"text","text":"null"}],"isError":false}

→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sysml_find",
   "arguments":{"uri":"demo.sysml","pattern":"Vehicle"}}}
← content text: [{"id":"b174e4cd-…","kind":"PartDefinition","name":"Vehicle", …}]
```

Service errors come back as tool results with `isError: true` and a
readable message — never as protocol failures — so an agent can read the
complaint and correct its own call.

## A first session

The pattern that works, in order:

1. **Discover the surface.** Call `sysml_command_catalog` — the live
   inventory of every tool with parameters and descriptions, and the stdio
   twin of the HTTP `GET /commands`. It is authoritative; a rendered
   snapshot is published as the
   [API & MCP catalogue](/sysml-rs/reference/api-mcp-catalog/).
2. **Load something.** `sysml_load_workspace` for a project directory on
   disk, or `sysml_load_source` to pass source text inline.
3. **Query before hydrating.** `sysml_query` with a cheap projection
   (`count`, `ids`) first, then `sysml_element` / `sysml_children` for the
   elements you actually need.
4. **Then the heavy tools** — `sysml_diagnostics`, `sysml_verify`,
   `sysml_simulate_start` and the session family, trade studies.

## The `_readiness` envelope

Loading a workspace kicks off library loading and indexing in the
background, and an early query can race it. Tool responses that touch a
model URI carry a **`_readiness`** field describing library/project/file
load state:

```json
"_readiness": { "file": "parsed_only", "library": {"state": "unloaded"},
                "project": {"state": "not_indexed"}, "project_kind": null }
```

An agent uses it to tell *"still indexing — retry"* apart from *"actually
empty"*. `sysml_readiness` polls the same snapshot directly, and progress
is also streamed as MCP `notifications/message` log entries (logger
`sysml_mcp.progress`); the notification stream can drop events under lag,
so polling `sysml_readiness` is the reliable fallback.

## stdio hygiene

The binary's **stdout is the transport** — a single stray print corrupts
the JSON-RPC stream. All logging goes to stderr (tune it with `RUST_LOG`
in the client config's `env`). If you wrap the binary in a launcher
script, make sure the wrapper writes nothing to stdout either.

## One process for both: `sysml-api --mcp`

Plain `sysml-mcp` gives the agent its own private in-memory service. When a
human UI and an agent should share one live model, run the
[API server](/sysml-rs/use/service-api/) with the MCP handler attached:

```bash
./target/release/sysml-api --mcp
```

HTTP and MCP then dispatch into the **same service instance**: a workspace
loaded by the [Simulation App](/sysml-rs/use/simulation-app/) is
immediately visible to the agent's tools, and an agent-side edit lands in
the model the UI reads. The HTTP side keeps its normal
[security defaults](/sysml-rs/use/service-api/#security-defaults); the MCP
side has no network surface of its own — it is stdio-only, with the
privileges of whatever launched it (it reads files your user can read).
