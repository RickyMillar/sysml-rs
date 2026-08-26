---
title: Editor setup
description: Set up SysML v2 language support in VS Code — the client extension, the bundled language server, and the settings that tune it.
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
`.sysml` and `.kerml` files in VS Code. That comes from two pieces: the
**`sysml-lsp-server`** language server (a Rust binary) and a thin
**client extension** that launches it. This page covers the VS Code setup;
for the server as an interface in its own right — its capabilities, generic
LSP clients like Helix or Neovim, and the WebSocket bridge — see
[the language server](/sysml-rs/use/lsp/).

## Before you start

The extension is **not published to the VS Code Marketplace or Open VSX**.
Install it from the `.vsix` attached to the
[latest release](https://github.com/RickyMillar/sysml-rs/releases/latest) —
one per platform, each with the language server bundled — or build it from a
source checkout.

```bash
# Linux x86-64; substitute your platform's asset name
curl -L -O https://github.com/RickyMillar/sysml-rs/releases/latest/download/sysml-linux-x64-0.1.0.vsix
code --install-extension sysml-linux-x64-0.1.0.vsix
```

The published packages are `sysml-{linux-x64,linux-arm64,darwin-arm64,darwin-x64,win32-x64}-0.1.0.vsix`.
They carry the extension's own version (`0.1.0`), which is not the release tag —
check the release you downloaded from rather than the filename.

To build both from source instead, follow the
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

Continuous integration packages the same per-platform VSIXes (Linux x64/arm64,
macOS arm64/x64, Windows x64) with the server bundled, via
`.github/workflows/vscode-extension.yml`; a tagged build attaches them to the
GitHub Release, which is where the packages above come from.

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
4. the **SysML: Install/Update Language Server** command, which downloads a
   prebuilt server from GitHub releases. Releases now publish those server
   binaries, so this path is live for Linux x64/arm64, macOS arm64/x64, and
   Windows x64.

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

Everything the language server provides — diagnostics as you type,
semantic highlighting on top of the extension's TextMate grammars,
completion, hover, navigation, rename, formatting, code actions, and inlay
hints. The full capability list, checked against the server's live
initialize response, is on [the language server page](/sysml-rs/use/lsp/#what-it-advertises).

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

## Other editors

`sysml-lsp-server` is a standard LSP server, so any editor with an LSP
client — Neovim, Helix, Emacs, Zed, and the rest — can use it directly.
Invocation, a worked client configuration, and the WebSocket bridge are on
[the language server page](/sysml-rs/use/lsp/).
