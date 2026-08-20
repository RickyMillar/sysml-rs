# crates/testing

Spec-conformance, regression, cross-transport-parity and performance baselines for the SysML v2 implementation. A directory grouping (no `Cargo.toml`) holding the workspace's full-stack test suite.

`Group · testing` · `conformance & baselines` · `1 crate` · `parser: tree-sitter`

This directory is a **grouping convention**, not a Cargo crate — there is no `Cargo.toml` here and nothing is published under a `testing` name. It currently holds a single crate, [`sysml-spec-tests`](sysml-spec-tests/README.md), which began as parser-coverage tracking and has since grown into the workspace's de-facto full-stack integration suite: corpus & element-kind coverage, tree-sitter grammar validation, spec/property conformance against the OMG pilot implementation, **cross-transport identity** (LSP vs REST byte-for-byte), service-command baselines, and an in-process performance harness.

## Where it sits

Test crates are leaves: they depend *downward* on the production crates and are never depended on. `sysml-spec-tests` reaches across both [lang](../lang/README.md) and [tooling](../tooling/README.md) — its `[dependencies]` pin the parser/model layer, while its `[dev-dependencies]` pull in nearly the whole backend so the integration suites can drive the real `SysmlService` and in-process transports.

```text
test sysml-spec-tests
▼ [dependencies] — parser + model layer
lang sysml-parser-incremental sysml-parser-trait sysml-core sysml-codegen sysml-span
▼ [dev-dependencies] — full backend for integration suites
runtime sysml-runtime sysml-id
tooling sysml-service sysml-lsp-server sysml-api
transport tower-lsp axum tower
```

>  **Parser fact:** the corpus harness drives `sysml_parser_incremental::TreeSitterParser` — the *sole* parser, which implements `sysml_parser_trait::Parser` and returns a real `ModelGraph`-backed `ParseResult`. The old Pest parser (`sysml-parser-batch`) and its `rule_coverage.rs` Pest-as-oracle tests were deleted (TS-3.7a). Any doc mentioning Pest, `sysml-parser-batch`, or `sysml-text-pest` is stale.

## Crates in this group

| Crate | Role | ~LOC (src) | Key dependencies |
|---|---|---|---|
| [**sysml-spec-tests**](sysml-spec-tests/README.md) | Corpus coverage, element-kind / tree-sitter validation, spec & pilot conformance, cross-transport identity, service-command + perf baselines | ~4.1k | deps: sysml-parser-incremental, sysml-parser-trait, sysml-core, sysml-codegen, sysml-span · dev: sysml-runtime, sysml-service, sysml-lsp-server, sysml-api, tower-lsp, axum, insta |

## What the suite covers

`sysml-spec-tests` splits into two halves: reusable **coverage primitives** in `src/` (importable as a library), and 37 **integration test files** in `tests/` (count refreshed 2026-07-30; layer map in `sysml-spec-tests/CLAUDE.md`) that drive the real engine and transports against committed fixtures and the official corpus.

**Coverage primitives (`src/`).**

- **corpus** — rayon-parallel parse of real `.sysml` files

- **element_coverage** — which of the **77** constructible `ElementKind`s are produced

- **treesitter_validation** — node types / enums vs spec TTL + xtext

- **pilot_normalise** — canonicalise OMG pilot JSON dumps for equivalence

- **report** — human-readable coverage / failure rendering


**Fixture corpora.**

- `data/constructible_kinds.txt` — **77** kinds (curated, spec-aligned)

- `fixtures/pilot-dumps/` — **138** OMG pilot JSON dumps

- `fixtures/service-baseline/` — **182** per-command JSON baselines

- `fixtures/cross-transport-*/` — LSP/REST parity + identity baselines

- `fixtures/reparse-identity-baseline/` — round-trip stability

- `corpus/advent/` — **56** lesson files

- `data/expected_failures.txt` — allowlist (must shrink over time)

## Integration test layers

per-file map lives in [sysml-spec-tests/README.md](sysml-spec-tests/README.md).

| Layer | Representative files | What it gates |
|---|---|---|
| L1 Provenance & codegen | `spec_drop_manifest`, `derived_indexes`, `grammar_spec_conformance`, `spec_kind/property_conformance` | Generated tables + derived indexes faithful to the pinned spec sources (`references/sysmlv2/spec-drop.toml`) |
| L2 Spec obligations | `*_spec_conformance` (12) + `obligation_matrix_consistency` | Runtime behaviour matches the cited obligations in `spec-obligations/*.md` |
| L3 Pilot oracle | `pilot_impl_conformance`, `pilot_dump_fixtures_loadable` | `ModelGraph` shape == OMG pilot elaborated JSON (ADR-015, external truth) |
| L4 Identity & transport | `identity_invariants`, `cross_transport_identity_baseline`, `semantic_tokens_invariants` | Deterministic IDs reparse-stable + transport-identical (diff-correlation gate lives in `sysml-core/tests/diff_identity.rs`) |
| L5 Corpus regression | `corpus_regression` (registry: full-corpus/stdlib/advent/xpect), `service_command_baseline`, `perf_baseline`, RSC baselines, `project_diagnostics_tests`, `scoping_tests`, `orchestrator_archive_watermark` | The real corpus still parses/resolves/elaborates/executes; baselines unmoved (no-regression signal, not conformance proof) |

>  **In-process transport harness.** Cross-transport tests do not spawn binaries. They build one `SysmlService` and exercise it three ways — through `tower-lsp` (LSP), through an `axum` router via `tower::ServiceExt` (REST), and via direct dispatch — then assert byte-identical results. This is why dev-deps reach into `sysml-lsp-server` (a path dep) and `sysml-api`.

## Running the suite

```
# Unit tests + all fixture-backed gates (no corpus needed)
cargo test -p sysml-spec-tests

# Full corpus tests (requires reference materials, #[ignore] by default)
SYSML_CORPUS_PATH=references/sysmlv2 \
  cargo test -p sysml-spec-tests -- --ignored

# Review / accept insta snapshot changes
cargo insta review
```

Part of the [sysml-rs](../../README.md) workspace · crate docs: [sysml-spec-tests](sysml-spec-tests/README.md) · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
