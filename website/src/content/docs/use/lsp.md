---
title: The language server
description: sysml-lsp-server as an interface — stdio protocol, advertised capabilities, generic editor clients, and the WebSocket bridge.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: 023a5ef
source_of_truth:
  - crates/tooling/sysml-lsp-server/src/main.rs
  - crates/tooling/sysml-lsp-server/src/lib.rs
  - crates/tooling/sysml-api/src/lsp_ws.rs
---

You want diagnostics, completion, hover, and navigation for `.sysml` and
`.kerml` files in *your* editor — or you are building a tool that speaks
the Language Server Protocol itself. **`sysml-lsp-server`** is a standard
LSP server, and this page treats it as an interface in its own right.
If you just want VS Code set up, go straight to
[editor setup](/sysml-rs/use/editors/) — the extension launches this same
server for you.

## The binary

```bash
cargo build --release -p sysml-lsp-server
./target/release/sysml-lsp-server --version
# sysml-lsp-server 0.1.0
```

Run with **no arguments**, the process speaks LSP over **stdin/stdout** and
serves until the client closes the connection. The only accepted arguments
are `--version` and `--help`; anything else exits with an error rather than
silently becoming a server. Launched by hand in a terminal it will look
hung — it is waiting for LSP frames on stdin.

Logging never touches stdout (stdout carries the protocol): detailed logs
go to a file in the user cache directory (on Linux,
`~/.cache/sysml-lsp/lsp.log`; the exact path is printed by
`sysml-lsp-server --help` and announced to the client at startup), panics
are appended to `lsp-panic.log` beside it, and if that directory is not
writable the server logs to stderr and keeps serving. `RUST_LOG` tunes the
filter.

## What it advertises

From the `initialize` response (`ServerCapabilities` is built in
`crates/tooling/sysml-lsp-server/src/lib.rs`, and this list was checked
against a live initialize round-trip at the commit above):

- **Diagnostics** — parse and semantic errors pushed as you type
  (full-document sync).
- **Semantic tokens** — full, delta, and ranged.
- **Completion** — triggered on `:`, `.`, `=`, `[`, and `"`, with resolve
  support.
- **Hover** and **signature help**.
- **Navigation** — definition, type definition, implementation,
  references; document and workspace symbols; call hierarchy.
- **Rename** (with prepare), **formatting**, **folding ranges**, selection
  ranges, document links, and code lenses.
- **Code actions** — quickfix, rewrite, and organize-imports kinds.
- **Inlay hints** (disable with `SYSML_LSP_DISABLE_INLAY_HINTS=1` if your
  client renders them badly).
- **Execute-command** — the `sysml.*` service commands (evaluate, verify,
  simulate, diagram, workspace info, …) callable via
  `workspace/executeCommand`; these are the same commands the
  [HTTP API](/sysml-rs/use/service-api/) catalogues.
- **Workspace folders** with change notifications.

## What the server loads

On `initialize` every workspace folder (or the root URI) is registered as a
project. After `initialized` the server, in the background:

1. runs **project discovery** over each root — a
   [`sysml.toml` manifest](/sysml-rs/use/sysml-toml/) defines a project and
   its [dependencies](/sysml-rs/use/dependencies/); a plain directory of
   `.sysml` files works too, as a synthetic project;
2. loads the **SysML v2 standard library**, so library types resolve;
3. **indexes** the workspace's `.sysml`/`.kerml` files (capped by the
   `maxIndexFiles` setting, default 500) and registers file watchers for
   them and for manifest files.

`sysml.toml` itself gets manifest-specific diagnostics when opened.
Expect diagnostics to firm up as library load and indexing complete — the
server reports progress through `window/logMessage` and `$/progress`.

## Configuring a generic client

Any LSP-capable editor can use the server: point the client at the binary
for the `sysml` and `kerml` filetypes. A worked example for **Helix**
(`~/.config/helix/languages.toml`):

```toml
[language-server.sysml]
command = "/abs/path/to/sysml-rs/target/release/sysml-lsp-server"

[[language]]
name = "sysml"
scope = "source.sysml"
file-types = ["sysml", "kerml"]
roots = ["sysml.toml"]
language-servers = ["sysml"]
```

The `command`/stdio invocation is exactly what the server expects (verified
above); the Helix stanza itself is written from Helix's documented
`languages.toml` shape and has **not been exercised end-to-end** by the
sysml-rs project — treat it as a starting point. Two general notes for any
client: syntax *highlighting* quality depends on your editor's grammar for
SysML (semantic tokens from the server add to it, they don't replace it),
and there is no sysml-rs-maintained configuration for editors other than
VS Code.

Client-side settings the server understands (sent as the `sysml` section
via `workspace/didChangeConfiguration`) are listed on the
[editor setup page](/sysml-rs/use/editors/#useful-settings).

## The WebSocket bridge

For clients that prefer a socket to spawning a subprocess, the
[API server](/sysml-rs/use/service-api/) bridges a full LSP session over
WebSocket at `GET /lsp` — the upgrade handshake was verified against a
running `sysml-api`. The bridged session runs over the **same service
instance** as the HTTP routes, so editor edits and API reads see one model.
This is how the [Simulation App](/sysml-rs/use/simulation-app/)'s embedded
Monaco editor gets language support.
