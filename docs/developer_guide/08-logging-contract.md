# Logging Contract (LSP + Runtime)

## Purpose
Define a consistent contract for:
- telemetry/event tracing (`tracing::*`)
- user-facing client messages (`window/logMessage`)

This keeps operational logs queryable and prevents user message spam.

## Channels

### 1) Telemetry channel (`tracing`)
Use `tracing` for:
- request/handler lifecycle
- background jobs
- parser/resolution/execution internals
- diagnostics, timing, counters, failures

Rules:
- prefer structured fields over interpolated strings
- include correlation IDs (`task_id`, `parent_task_id`, `session_id`) when available
- use `trace`/`debug` for noisy internals, `info` for high-level lifecycle, `warn`/`error` for actionable failures

### 2) UX channel (`window/logMessage`)
Use only for user-visible status or action-required warnings/errors.

Policy:
- all `window/logMessage` emission must go through `sysml-lsp-server/src/ux_messages.rs`
- do not call `client.log_message(...)` directly in feature/engine code
- if a message is operational telemetry, log to `tracing` instead

## Field Vocabulary
Use these canonical field names where applicable:
- `document_uri`
- `workspace_root`
- `line`
- `character`
- `element_id`
- `session_id`
- `task_id`
- `parent_task_id`
- `command`
- `elapsed_ms`
- `diagnostic_count`
- `result`

Notes:
- prefer `document_uri` over generic `uri` in new code
- prefer `elapsed_ms` over mixed duration formatting

## Event Naming
Use short action-oriented event messages:
- `"spawned background task"`
- `"workspace indexing finished"`
- `"verification complete"`
- `"constraint evaluation error"`

Avoid embedding dynamic context in the event message; put it in fields.

## Correlation Rules
- Every spawned task should have a `task_id`.
- Child work should include `parent_task_id` or a stable suffix (`<parent>:<phase>`).
- `spawn_blocking` sections should emit the same `task_id` lineage.

## Volume Control Rules
- For high-frequency events (per-step, per-file, per-token), apply sampling or cooldown controls.
- Use `sysml-lsp-server/src/telemetry_control.rs` helpers:
- `should_log_every_n(key, n)` for repetitive debug paths.
- `should_log_after_cooldown(key, duration)` for repeated user-facing warnings.
- Always keep first-occurrence visibility even when sampling.

## Feature-Flag Behavior (Important For Agents)
Current behavior:
- tracing in runtime/parser/core crates is mostly behind optional features (`tracing`, `resolution-tracing`)
- these features are **not enabled by default** across the workspace

Implication:
- generic agent runs (`cargo check`, `cargo test`) usually do **not** compile those optional tracing paths unless features are explicitly enabled

Recommended debug commands:
- `cargo check -p sysml-core --features resolution-tracing`
- `cargo check -p sysml-parser-trait --features tracing`
- `cargo check -p sysml-runtime --features tracing`

If we want "agent-default" tracing compilation in the future, add workspace aliases/scripts that always pass these features.
