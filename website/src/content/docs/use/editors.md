---
title: Editor setup
description: Set up SysML v2 language support in VS Code or any LSP-capable editor, backed by the sysml-lsp-server binary.
scope:
  - sysml-rs tooling
status: pre-alpha
last_verified_against: fcd1305
source_of_truth:
  - editors/vscode/README.md
  - editors/vscode/package.json
  - editors/vscode/src/serverBinary.ts
  - crates/tooling/sysml-lsp-server/src/lib.rs
  - crates/tooling/sysml-lsp-server/src/main.rs
  - .github/workflows/vscode-extension.yml
---

You want diagnostics, completion, hover, and navigation while you edit
`.sysml` and `.kerml` files. That comes from two pieces: the
**`sysml-lsp-server`** language server (a Rust binary) and, for VS Code, a
thin **client extension** that launches it. Any other editor with a Language
Server Protocol client can use the same server directly.

## Before you start

The extension is **not published to the VS Code Marketplace or Open VSX**,
and no GitHub release has been published yet, so both the extension and the
server are built from a source checkout today. Follow the
[installation guide](/sysml-rs/start-here/install/) first so that `cargo`
builds work in your clone.

## VS Code

### 1. Build the language server

```bash
cargo build --release -p sysml-lsp-server
```

The binary lands at `target/release/sysml-lsp-server`.

### 2. Package and install the extension

The extension lives in `editors/vscode`. Packaging it produces a `.vsix` you
install into your normal VS Code:

```bash
cd editors/vscode
npm ci
npm run package        # runs vsce; produces sysml-0.1.0.vsix
code --install-extension sysml-0.1.0.vsix
```

If a server binary is present at `editors/vscode/server/sysml-lsp-server`
when you package, it is bundled into the VSIX and the extension uses it
automatically. Otherwise, point the extension at the binary you built in
step 1 with the `sysml.server.path` setting (or put `sysml-lsp-server` on
your `PATH`).

Continuous integration also packages per-platform VSIXes (Linux x64/arm64,
macOS arm64/x64, Windows x64) with the server bundled, as workflow artifacts
of `.github/workflows/vscode-extension.yml`; a tagged release will attach
them to a GitHub Release once releases begin.

### 3. Check it works

Open a folder containing `.sysml` files. You should see syntax highlighting
immediately and, once the server has indexed the workspace, squiggles for
real diagnostics and working hover/completion. The **SysML: Show Output**
command opens the server log if something looks wrong.

### How the extension finds the server

In priority order:

1. the `sysml.server.path` setting,
2. a bundled `server/` binary inside the installed extension,
3. `sysml-lsp-server` on the system `PATH`,
4. the **SysML: Install/Update Language Server** command, which downloads
   from GitHub releases — **this cannot work yet**, because no release has
   been published. Use one of the first three until then.

### Useful settings

All settings are under the `SysML` section (`sysml.*`):

| Setting | Default | What it does |
|---|---|---|
| `sysml.server.path` | *(empty)* | Explicit path to `sysml-lsp-server`; empty means auto-detect. |
| `sysml.server.trace` | `off` | LSP wire tracing (`messages`, `verbose`) in the Output channel. |
| `sysml.library.enabled` | `true` | Load the SysML v2 standard library on startup. |
| `sysml.validation.enabled` | `true` | Semantic validation (constraint and type checking). |
| `sysml.inlayHints.enabled` | `true` | Inlay hints for inferred types and multiplicities. |
| `sysml.formatting.enabled` | `true` | Document formatting. |
| `sysml.workspace.maxIndexFiles` | `500` | Cap on indexed workspace files (`0` = unlimited). |

## What you get

The language server advertises these capabilities (verified against the
server's initialize response at the commit above):

- **Diagnostics** — parse and semantic errors pushed as you type.
- **Semantic highlighting** — semantic tokens (full, delta, and ranged), on
  top of the extension's TextMate grammars.
- **Completion** — context-aware, triggered on `:`, `.`, `=`, `[`, and `"`.
- **Hover** and **signature help**.
- **Navigation** — go-to-definition, type definition, implementation, and
  find-references; document and workspace symbols; call hierarchy.
- **Rename** with prepare support, **formatting**, **folding**, selection
  ranges, document links, and code lenses.
- **Code actions** — quick fixes, rewrites, and organize-imports.
- **Inlay hints** for inferred types and multiplicities (toggleable).

The extension additionally ships snippets for common SysML/KerML constructs
and JSON-schema validation for `.project.json` / `.workspace.json` /
`.meta.json` manifests.

**Experimental / partial support** — the extension is deliberately thin
right now: the diagram webview, simulation panels, and debug adapter were
removed during the renderer rework and will return with the new React-SVG
renderer. Diagrams are available today through the
[CLI export and the embeddable viewer](/sysml-rs/use/views-and-diagrams/),
and model execution through the [CLI](/sysml-rs/use/cli-workflows/) and
[runtime](/sysml-rs/use/runtime/).

## Other editors (generic LSP clients)

`sysml-lsp-server` is a standard LSP server: run with no arguments it speaks
the protocol over **stdin/stdout** (logs go to stderr), so any editor with
an LSP client — Neovim, Helix, Emacs, Zed, and the rest — can use it. Point
your client at the binary for the `sysml` and `kerml` filetypes; there is no
sysml-rs-maintained configuration for editors other than VS Code, so
highlighting quality and feature coverage depend on your client.

The server is also reachable over WebSocket at the API server's `/lsp`
endpoint — see [Integrations](/sysml-rs/use/integrations/).
