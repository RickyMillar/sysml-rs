# LSP Server Architecture

Where every IDE feature lives, how diagnostics flow, and how the LSP composes with the rest of the stack. Read [00-architecture.md](00-architecture.md) first for the layering context.

## Layering: the LSP is a thin wrapper

The LSP is a Layer 5 transport crate (`crates/tooling/sysml-lsp-server`). It does **not** parse, resolve, validate, elaborate, or execute on its own — those live in `sysml-core` / `sysml-runtime`, behind the salsa-incrementalized query surface in `sysml-ide-db`. The LSP's job is to:

1. Translate LSP protocol calls into ide-db queries or `SysmlService` commands.
2. Cancel/debounce/dedupe in flight work so the editor stays snappy.
3. Manage open-document state, library lifecycle, and project discovery.
4. Format the responses back as LSP types.

[ADR-010](../design/adr/010-lsp-as-thin-wrapper.md) codifies the "LSP holds no domain state" rule: the LSP is the single writer for URIs it has open, but the model itself is owned by `SysmlService` / `AnalysisHost`.

## Module map

The crate is ~57 source files / ~45k lines. Know the map; don't read everything.

### Core server

| Module | Role |
|--------|------|
| `lib.rs` (~2900 lines) | `SysmlLanguageServer` struct + all `LanguageServer` trait impls. Dispatches to feature modules. The "god module". |
| `main.rs` | Binary entry: tracing init, panic hook, `run_stdio()`. |
| `types.rs` | `FeatureFlags`, `LibraryState`, semantic token type/modifier constants, `SYNTHETIC_FILE`. |
| `lsp_types.rs` | Minimal LSP protocol types (formerly the standalone `sysml-lsp` crate, merged in). Converts `sysml-span::Diagnostic` to LSP `Diagnostic`. |
| `utils.rs` | Offset/position conversions, URI parsing, `range_to_lsp_range`. |

### Diagnostic pipeline

| Module | Role |
|--------|------|
| `diagnostic_pipeline.rs` | Per-URI task management. Replaces/aborts stale diagnostic tasks via `DashMap<String, JoinHandle>`. Generates correlation task IDs for telemetry. |
| `diagnostics.rs` | Converts parse/resolve/validate errors to LSP `Diagnostic`. Calls `elaborate()` for elaboration-time diagnostics, runs health checks (import / action / flow / state machine / verification). |
| `manifest_diagnostics.rs` | Diagnostics for `sysml.toml` manifest files. |
| `constraint_monitor.rs` | Live constraint evaluation — publishes constraint pass/fail as diagnostics on every edit. |

Diagnostic sources live in lower crates:

| You see this | Source | Code |
|--------------|--------|------|
| Parse error | `crates/lang/sysml-parser-incremental/src/` | E100 |
| Unresolved name | `crates/lang/sysml-core/src/resolution/` | E200 |
| Missing required property | `crates/lang/sysml-core/src/validation.rs` | V001 |
| Wrong type for property | `crates/lang/sysml-core/src/validation.rs` | V002 |
| Requires at least one value | `crates/lang/sysml-core/src/validation.rs` | V003 |
| Allows at most one value | `crates/lang/sysml-core/src/validation.rs` | V004 |
| Read-only property | `crates/lang/sysml-core/src/validation.rs` | V005 |
| Structural issue (orphan, cycle, dangling membership) | `crates/lang/sysml-core/src/structural_validation.rs` | S001–S004 |
| Semantic rule violation (86 codegen rules) | `crates/lang/sysml-core/src/semantic_checks/`, dispatched by the generated `semantic_validation.generated.rs` in `OUT_DIR` | SM* |
| Import health issue | `crates/lang/sysml-core/src/import_health.rs` | IM* |
| Physics-domain issue | `crates/lang/sysml-core/src/physics/` | PH001–PH006 |
| Constraint pass/fail | `crates/tooling/sysml-service/src/constraint_monitor.rs` | — |

### Resolution tiers (non-blocking UI)

The LSP doesn't block on full cross-file resolution for every keystroke. `background.rs` defines a `ResolutionTier` enum and feature gates are tiered:

| Tier | Latency | Scope | Features it unblocks |
|------|---------|-------|----------------------|
| T1 Syntax | < 50 ms, sync | Current file CST only | Highlighting, outline, syntax errors |
| T2 Local | < 200 ms, debounced | Same file resolved | Goto-def (within file), completion |
| T3 Full | Background | Cross-file + library | Full validation, find-refs, rename, deep completion |

`did_change` schedules diagnostics after a 150 ms debounce (`DID_CHANGE_DIAGNOSTICS_DEBOUNCE_MS`).

### IDE features

| Module | LSP method | Key detail |
|--------|------------|------------|
| `hover.rs` | `textDocument/hover` | Markdown: signature, type info, supertypes, inherited members, evaluated values via `evaluation::try_evaluate_value` |
| `completion.rs` (~2300 lines) | `textDocument/completion` | Keywords, element names, types from scope, snippet completions |
| `navigation.rs` | `textDocument/definition`, `references` | Goto-def via resolution, find-refs via graph traversal |
| `rename.rs` | `textDocument/rename` | Cross-file rename with workspace edits |
| `semantic_tokens.rs` | `textDocument/semanticTokens` | Server-side semantic highlighting (complements tree-sitter highlighting in the editor) |
| `code_actions.rs` | `textDocument/codeAction` | Quick-fixes, refactoring, extract/inline |
| `code_lens.rs` | `textDocument/codeLens` | Constraint-evaluation lenses, verification-run lenses, physics-domain classification lenses |
| `inlay_hints.rs` | `textDocument/inlayHint` | Evaluated value hints (gated by `SYSML_LSP_DISABLE_INLAY_HINTS=1`) |
| `formatting.rs` | `textDocument/formatting` | Auto-indentation, walks tree-sitter CST |
| `symbols.rs` | `textDocument/documentSymbol` | Document outline / breadcrumb symbols |
| `kinds.rs`, `syntax_context.rs`, `type_hierarchy.rs` | — | `ElementKind` → `SymbolKind`, cursor context detection, type hierarchy navigation |
| `advanced_features.rs` | `documentLink`, `signatureHelp`, `callHierarchy` | Document links, signature help, call hierarchy |
| `manifest_language_features.rs` | (sysml.toml editing) | Completion, diagnostics, code actions, links for `sysml.toml` |

### Execution integration

LSP execution commands route through `SysmlService` (S2) wherever possible:

| Module | Role |
|--------|------|
| `commands.rs` (~3400 lines) + `command_dispatch.rs` | All `sysml.*` command handlers + routing |
| `evaluation.rs` | `try_evaluate_value()` for hints/hover, `evaluate_constraints()` for lenses |
| `simulation.rs` / `action_session.rs` | `SimulationSession` (state machines), `ActionSession` (action flows) |
| `workspace_verify.rs`, `whatif.rs`, `aggregation.rs` | Verification runner, trade studies, model metrics |
| `execution_runtime.rs` | Session limits (`MAX_SESSIONS`, expiry timeout) |

`command_dispatch.rs` has a single registry; `dispatch_table_has_all_commands` keeps the registry honest against the `#[service_command]` set. When you add a `#[service_command]` to `sysml-service`, the LSP picks it up via the dispatch table — same handler, four transports (LSP, REST, MCP, CLI).

### Infrastructure

| Module | Role |
|--------|------|
| `workspace.rs`, `workspace_index.rs`, `workspace_snapshot.rs` | Library loading, file discovery, cross-file index, immutable snapshots |
| `library_cache.rs`, `library_manager.rs` | Stdlib caching (`~/.cache/sysml-rs/`, 5s cold start → <500ms warm) and lifecycle |
| `parser_cache.rs` | Tree-sitter tree cache and parser instances |
| `project_discovery.rs`, `project_registry.rs` | `sysml.toml` discovery, workspace members, loaded manifest tracking |
| `diagram.rs`, `diagram_manager.rs`, `diagram_edit.rs` | Diagram generation, state tracking, `sysml/diagram/setModel` notification |
| `pending_requests.rs` | Request deduplication (prevents duplicate work on concurrent LSP requests) |
| `telemetry_control.rs`, `telemetry_events.rs` | Rate-limited structured telemetry |
| `ux_messages.rs` | **All** `window/logMessage` emission routes through here — see [08-logging-contract.md](08-logging-contract.md) |

### Test modules

| Module | What it tests |
|--------|---------------|
| `protocol_tests.rs` | Full LSP protocol (initialize, hover, completion, goto-def, rename, etc.) |
| `integration_tests.rs` | Feature integration (evaluation, constraints, simulation, workspace verify) |
| `diagnostic_ux_tests.rs` | Diagnostic UX: error messages, severity, code actions, related information |
| `snapshot_tests.rs` | `insta` snapshot tests over LSP responses |
| `feature_tests.rs`, `utils_tests.rs`, `ux_workflow_tests.rs` | Unit / utility / workflow tests |
| `test_harness.rs` | Mock tower-lsp server, shared test infra |

## Key design patterns / invariants

1. **Salsa-first.** All file content and queries go through `AnalysisHost`. Lock the `Arc<Mutex<AnalysisHost>>` briefly to set inputs or hand out an `Analysis` snapshot, then drop the lock. Snapshots are cheap; locks are not.
2. **Cancellation safety.** Wrap salsa queries with `Cancelled::catch()`. Background work must be cancellation-safe — the user's next keystroke invalidates the world.
3. **Debounced diagnostics.** `did_change` waits 150 ms (`DID_CHANGE_DIAGNOSTICS_DEBOUNCE_MS`) before scheduling diagnostics. `DiagnosticPipeline::replace_diagnostics_task` aborts the prior task for that URI.
4. **Diagnostic fingerprinting.** `last_published_diagnostics: DashMap<String, u64>` maps URI to a hash — skip the `publishDiagnostics` if the result hash is unchanged.
5. **UX messages routed.** Never call `client.log_message()` directly. Route through `ux_messages.rs`. See [08-logging-contract.md](08-logging-contract.md) for the field vocabulary and rate-limit rules.
6. **Single writer for open URIs.** While a URI is `textDocument/didOpen` → `didClose`, the LSP owns its content via `set_file_content_in_project(uri, source, ProjectHandle)`. Other writers (CLI, REST) MUST go through `SysmlService::load_workspace_source` instead of writing salsa directly.
7. **Feature flags.** `FeatureFlags` (in `types.rs`) gates expensive resolution/validation passes per workspace. Updated via `did_change_configuration`.

## Two highlighting systems

The LSP and the editor each contribute syntax colors. They don't interact.

### 1. Tree-sitter syntax highlighting (editor-side)

| Where | File |
|-------|------|
| Captures | `editors/vscode/...` |
| Source grammar | `crates/lang/sysml-parser-incremental/tree-sitter/rules/*.js` |
| Node types | `crates/lang/sysml-parser-incremental/tree-sitter/src/node-types.json` |

Matches tree-sitter CST node types to `@keyword` / `@type` / `@variable` captures. Fast, runs on every keystroke in the editor, no LSP roundtrip. If a token isn't colored, check:

1. Does the node type exist in `node-types.json`?
2. Is there a matching rule in `highlights.scm`?
3. Run the extension's validate script.

### 2. LSP semantic tokens (server-side)

| File | `crates/tooling/sysml-lsp-server/src/semantic_tokens.rs` |

Provides **richer** highlighting on top of tree-sitter — uses the resolved `ModelGraph` to distinguish definitions from usages, mark unresolved references, etc. The editor merges these with tree-sitter highlights.

## Configuration

| Setting | Where | Effect |
|---------|-------|--------|
| `SYSML_LSP_DISABLE_INLAY_HINTS=1` | env var | Disables inlay-hint evaluation (Zed extension sets this by default) |
| `SYSML_LIBRARY_PATH` | env var | Override standard library location |
| `lsp.sysml-lsp.binary.path` | Zed `settings.json` | Override LSP binary location |
| `sysml.featureFlags.*` | `did_change_configuration` | Per-workspace gate of expensive passes |

## Common pitfalls

- **`lib.rs` is the god module.** Most protocol handler logic lives there; feature modules contain the computation. Don't try to grep for "where hover is handled" — start in `lib.rs::hover` and follow the call to `hover.rs`.
- **URI normalization.** Use `canonical_file_uri()` / `uri_aliases()` for URI comparison. Raw string equality misses symlinks and tilde expansion.
- **Inlay hints disabled by default.** Zed sets `SYSML_LSP_DISABLE_INLAY_HINTS=1` — check the env var before debugging "missing hints".
- **Library loading is async.** Features degrade gracefully before stdlib is loaded. Check `LibraryState` before assuming library types are available.
- **Don't reinstate transport bypasses.** S2 collapsed CLI/REST/LSP/MCP duplicates onto `SysmlService`. New commands belong in `sysml-service` with `#[service_command]`, not as LSP-only handlers.

## Related documentation

- [00-architecture.md](00-architecture.md) — overall layering.
- [03-resolution.md](03-resolution.md) — what the LSP's resolution queries do.
- [08-logging-contract.md](08-logging-contract.md) — `ux_messages` channels and field vocabulary.
- [20-sysml-service-design.md](20-sysml-service-design.md) — the service the LSP dispatches to.
- [ADR-010](../design/adr/010-lsp-as-thin-wrapper.md) — "LSP holds no domain state".
- [ADR-013](../design/adr/013-monaco-editor-transport.md) — Monaco-over-WebSocket transport (the in-browser sibling of the stdio LSP).
- `crates/tooling/sysml-lsp-server/CLAUDE.md` — quick-reference module map and pitfall list.
