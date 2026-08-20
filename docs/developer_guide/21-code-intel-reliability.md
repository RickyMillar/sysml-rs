# Code-Intelligence Tooling Reliability

The living home for MCP / code-intel tooling reliability notes: which tool to
trust for which question, and the measurements behind that guidance. Update THIS
file when a new tooling gotcha surfaces.

The routing rules that use these findings — rust-analyzer for Rust semantics,
SocratiCode for conceptual search, grep for exact text — are summarised in the
root `CLAUDE.md`.

## Tooling notes

- **SocratiCode** v1.8.6, plugin install (indexed at the repository root):
  - `codebase_search` (hybrid semantic + BM25) — ✅ excellent for Rust.
    Use liberally; better than grep for conceptual queries.
  - `codebase_impact` / `codebase_graph_query` / `codebase_graph_circular`
    — ❌ unreliable on Rust. Investigated 2026-05-06: SocratiCode's graph
    is built with ast-grep (syntactic), but Rust's symbol resolution
    requires semantic analysis (cross-crate `use` re-exports, trait
    dispatch, generic inference). Result on this workspace:
    1163 dep edges across 1235 files (avg 0.9/file — should be 5–10×
    higher), and `codebase_graph_status` reports **81% unresolved call
    edges**. No env-var knob fixes this.
  - **Policy**: for Rust impact / call-graph / blast-radius, use grep,
    rust-analyzer, `cargo-modules`, or `cargo-call-stack`. SocratiCode's
    role is search only.
  - Re-evaluate when an upstream fix lands or SocratiCode ships a
    rust-analyzer-backed graph builder. Worth filing the 81%-unresolved
    finding as an upstream bug at github.com/giancarloerra/socraticode.

---

## Historical baselines (archived)

The identity/salsa-migration capture records that used to live here — the S0
service-command, reparse-identity, Pest↔tree-sitter, perf, and cross-transport
baselines, the S1 progress log, and the sprint-era coverage-gate list — moved to
They are dated records, not current instructions: two of the harnesses they
describe no longer exist, and the identity invariants they were gating have since
landed.

Of the gates named there, these test targets are still live:

```bash
cargo test --release -p sysml-spec-tests --test service_command_baseline
cargo test --release -p sysml-spec-tests --test cross_transport_identity_baseline
cargo test --release -p sysml-spec-tests --test perf_baseline -- --ignored --nocapture --test-threads=1
```
