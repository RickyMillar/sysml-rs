# SysML v2 — VS Code (LSP-only)

Lightweight VS Code extension for SysML v2 / KerML. User-facing setup and
feature documentation lives on the
[documentation portal](https://rickymillar.github.io/sysml-rs/use/editors/);
this README is the developer reference. It provides:

- **Syntax highlighting** — TextMate grammars (`syntaxes/*.tmLanguage.json`) plus
  semantic highlighting from the language server.
- **Language server features** — diagnostics, hover, completion,
  go-to-definition, semantic tokens, inlay hints, formatting — served by the
  `sysml-lsp-server` binary (`crates/tooling/sysml-lsp-server`).
- **Snippets** for common SysML/KerML constructs and the standard view kinds.
- **Manifest validation** for `.project.json` / `.workspace.json` / `.meta.json`.

## Status

This is a **deliberately thin** build. The diagram webview, simulation panels,
and debug adapter were removed in the renderer rework (Bucket 3.1) and return
once the new React-SVG renderer ships. See

## Server binary

The extension launches `sysml-lsp-server`, located (in priority order) via:

1. `sysml.server.path` setting,
2. a bundled `server/` binary next to the extension,
3. the system `PATH`,
4. `SysML: Install/Update Language Server` (downloads from GitHub releases).

## Develop

```bash
npm install
npm run compile      # esbuild → dist/extension.js
npm run typecheck
npm run lint
npm run test:grammar # TextMate grammar tests (2 known pre-existing action-keyword failures)
```

Press F5 in VS Code to launch an Extension Development Host.
