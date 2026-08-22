# sysml-spec-tests

Conformance and regression harness for the SysML v2 implementation: parser corpus coverage, spec-direct element/property gates, cross-transport identity baselines, frozen service-command fixtures, and the full parse → elaborate → execute pipeline.

`Layer 5 · testing` · `conformance + regression harness` · `crate-type: lib (test-only)` · `parser: tree-sitter`

## What this crate owns

`sysml-spec-tests` is the workspace's spec-conformance and cross-transport regression net. It does not ship runtime code — its `src/` is a small set of coverage primitives (corpus discovery, spec-derived expected-value loaders, report rendering) and the heavy lifting lives in `tests/`: 37 integration test files plus 3 examples that drive the real engine, the real `SysmlService`, and the in-process LSP/REST transports against committed fixtures and the official SysML v2 corpus.

The invariant it enforces, in one line:

>  **Authority, not opinion.** Every expected value is derived from the spec (TTL vocab, xtext grammar) via `sysml-codegen`, or frozen from a real run into a committed fixture / `insta` snapshot. The crate never hand-asserts "what SysML looks like" — it asserts that the parser/service reproduce the authority's own examples, and that two transports produce *identical* results.

**77.**

constructible `ElementKind` variants tracked (`data/constructible_kinds.txt`)

**56.**

"Advent of SysML v2" lesson files (`corpus/advent/`)

**138.**

pilot-implementation JSON dump fixtures (`fixtures/pilot-dumps/`)

**25.**

vendored book example fixtures (`examples/the-book-corpus/` at the workspace root — source repo, commit, and resync procedure in its README)

**188.**

`insta` snapshots (`tests/snapshots/`)

**37.**

integration test files in `tests/`

## The parser under test: tree-sitter

> ⚠  **The Pest parser is gone.** `sysml-parser-batch` (Pest PEG) and the `rule_coverage.rs` Pest-as-oracle equivalence tests were deleted. The sole parser is `TreeSitterParser` from `sysml-parser-incremental`, which implements the `sysml_parser_trait::Parser` trait and returns a real `ModelGraph`-backed `ParseResult`. All corpus parsing routes through it (see `src/corpus.rs`). Any "(pest-based)" wording in older docs is stale.

## Where it sits

```text
corpus in references/sysmlv2 (Pilot + Models) corpus/advent fixtures/*.sysml · pilot-dumps
▼ feed source / fixtures into
harness sysml-spec-tests
▼ drives
exercises TreeSitterParser sysml-runtime (elaborate · compile · execute) SysmlService
▼ through transports
transports in-process tower-lsp in-process axum router direct service dispatch
▼ checked against
oracles spec-derived expected sets committed fixtures insta snapshots expected-failure allowlists
```

## Test suites

The integration files group into the five layers of
need (spec-obligation gate files elided — see `spec-obligations/README.md`).

| Layer | Test file | What it gates | Needs corpus? |
|---|---|---|---|
| L1 Provenance | `spec_drop_manifest` | SHA-256 of every consumed spec source vs `references/sysmlv2/spec-drop.toml`; pilot-jar SHA vs CI workflow. | spec files |
| L1 Provenance | `derived_indexes` | Regen-diff + checksum tie for `references/sysmlv2/derived/` (spec plaintext, xtext rule index). | spec files |
| L1 Provenance | `grammar_spec_conformance` | Tree-sitter node-type / enum-value coverage cross-referenced against spec TTL + xtext (was `treesitter_tests`). | spec files |
| L1↔L2 seam | `spec_kind_conformance` | Every TS-emitted `ElementKind` ⊆ the TTL vocab. | spec files |
| L1↔L2 seam | `spec_property_conformance` | Axis-aware property conformance on emitted graphs. | spec files |
| L2 Obligations | `*_spec_conformance` (12 files) | Runtime/elaboration behaviour vs the cited obligations in `spec-obligations/*.md`. | no (purpose-built fixtures) |
| L2 Obligations | `obligation_matrix_consistency` | `// OBL:` markers ↔ obligation-matrix rows can't drift. | no |
| L3 Pilot oracle | `pilot_impl_conformance` | TS-2.7: parse SysML, project `ModelGraph`, diff against committed pilot dumps (strict + allowlist). ADR-015; external truth. | no (fixtures) |
| L3 Pilot oracle | `pilot_dump_fixtures_loadable` | TS-2.5 smoke: every pilot-dump fixture loads as JSON and the manifest is well-formed. | no (fixtures) |
| L4 Identity | `identity_invariants` | Deterministic IDs reparse-stable (S0.T2) + LSP/REST-identical (S0.T5), one transport-parameterised gate. | no (fixtures) |
| L4 Identity | _(relocated)_ `sysml-core/tests/diff_identity.rs` | B3: `diff_graphs` correlates strictly by `ElementId` (ADR-009) — lives with its algorithm (steward-ruled). | no |
| L4 Identity | `cross_transport_identity_baseline` | S2.T19: byte-identical command responses across CLI/MCP/REST transports. | no (fixtures) |
| L4 Identity | `semantic_tokens_invariants` | Phase 1.7: semantic-tokens foundation invariants via the in-process LSP. | no (fixtures) |
| L5 Corpus | `corpus_regression` | Registry-driven: full-corpus + stdlib + advent + xpect d1–d5 × parse/resolve/elaborate/execute (was 5 driver files). | `SYSML_CORPUS_PATH` (advent committed) |
| L5 Baselines | `service_command_baseline` | S0.T1: frozen request/response fixtures for load-bearing `#[service_command]`s. | no (fixtures) |
| L5 Baselines | `perf_baseline` | S0.T4: LSP keystroke / REST cold-warm / sim-start latency baselines. | no (fixtures) |
| L5 Baselines | `rsc2/rsc3/rsc5_*_baseline` | RSC behavioural baselines (exchange plane, quantities, value binding). | no (fixtures) |
| L5 Regression | `project_diagnostics_tests`, `scoping_tests`, `contract_b1_derive_refine_trace`, `gap_repros`, `exchange_plane_fixture`, `orchestrator_archive_watermark` | Workspace diagnostics, scoping strategies, traceability contract, pinned gap repros, RW-4 memory watermark. | no |

## In-process transport harness

The cross-transport and perf suites don't spawn binaries. They construct one `SysmlService` and exercise it three ways — through the in-process `tower-lsp` server, through the in-process `axum` router via `tower::ServiceExt`, and via direct service dispatch — then assert the results are identical. This is why the dev-dependency set reaches up into `sysml-lsp-server` and `sysml-api`: it rebuilds a small driver that mirrors the module-private harness in `sysml-lsp-server`'s own protocol tests.

```text
one coreSysmlService (salsa-backed)
▲ dispatched identically through ▲
drivers tower-lsp (in-proc) axum router (in-proc) direct dispatch
▼ assert
gatebyte-identical responses → insta snapshot
```

## Library modules (`src/`)

These are the coverage primitives the integration tests build on.

| Module | Responsibility | Key items |
|---|---|---|
| `corpus` | Discover `.sysml` files in the reference corpus and parse them with `TreeSitterParser` (rayon-parallel). | `discover_corpus_files`, `parse_all_corpus_files`, `collect_element_kinds` |
| `element_coverage` | Track which of the 77 constructible `ElementKind` variants the parser produces; expected set is spec-derived. | coverage diff vs `data/constructible_kinds.txt` |
| `treesitter_validation` | Cross-reference tree-sitter node types and enum values against the spec TTL / xtext files. | node + enum validators |
| `pilot_normalise` | Normalise OMG pilot-implementation JSON dumps to canonical form for equivalence testing. | `parse_pilot_json`, `normalize`, `to_canonical_json` |
| `report` | Human-readable coverage / failure report rendering. | report formatters |

## Crate root API (`lib.rs`)

Expand all Collapse all

#### `find_references_dir() -> PathBuf`

Locate the `references/sysmlv2` directory: checks `SYSML_CORPUS_PATH`, then `SYSML_REFS_DIR`, then common relative paths. **Panics** with a clear message if not found (fail-fast over silent stale data). `try_find_references_dir() -> Option<PathBuf>` is the non-panicking variant for tests that should skip gracefully.

#### `CoverageConfig { corpus_path, corpus_subdirs }`

Configuration for corpus runs. `CoverageConfig::from_env()` reads `SYSML_CORPUS_PATH` (returns `None` if unset); `local_dev()` uses a relative fallback. Subdirs default to the Pilot standard library and the example `SysML-v2-Models/models`.

#### `CoverageSummary { total_files, passed_files, expected_failures, unexpected_failures, … }`

Aggregate result of a corpus run. `pass_percentage()` returns passed/total × 100. `unexpected_failures` > 0 means a regression (a file failed that is not in the allowlist).

#### `load_allow_list(&str) -> HashSet<String> · load_constructible_kinds(&str) -> HashSet<String>`

Parse the comment-tolerant allowlist / constructible-kinds files (one entry per line, `#` comments and blanks skipped). Loaded at compile time via `include_str!` in the test files.

## Invariants

- **The allowlist only shrinks.** `data/expected_failures.txt` lists corpus files expected to fail. Entries are removed as parser bugs are fixed; new entries need a comment explaining why. A failure not on the list is a regression.

- **Expected values are spec-derived.** Element kinds, operators and node types come from TTL / xtext via `sysml-codegen` — never hardcoded.

- **Corpus tests are `#[ignore]` by default.** They need an env var pointing at the corpus and the `--ignored` flag. Unit tests and fixture-backed tests run with a plain `cargo test`.

- **Two transports, one truth.** LSP and REST must return identical bytes for the same command (`cross_transport_identity_baseline`).

- **Reports under `reports/` are generated output**, committed as diff baselines — not authored docs.

## Running the suites

```
# Fast: unit tests + all fixture-backed gates (no corpus needed)
cargo test -p sysml-spec-tests

# Full reference-corpus coverage
SYSML_CORPUS_PATH=references/sysmlv2 \
  cargo test -p sysml-spec-tests -- --ignored

# Review / accept insta snapshot changes (identity, service-command, …)
cargo insta review
```

>  **Examples.** `cargo run -p sysml-spec-tests --example categorize_failures` (bucket corpus failures by class), `--example resolution_debug` (inspect name resolution on a file), and `--example normalize_pilot_dump` (canonicalise an OMG pilot JSON dump into a fixture).

## Dependencies

**Library deps.**

- `sysml-parser-incremental` (feature `semantic`) — the tree-sitter parser under test

- `sysml-parser-trait` — `Parser` / `ParseResult` contract

- `sysml-core` — `ModelGraph`, `ElementKind`

- `sysml-codegen` — spec-derived operator / kind extraction

- `sysml-span` (serde), `walkdir`, `rayon`, `serde`/`serde_json`

**Dev deps (test drivers).**

- `sysml-runtime` — elaborate / compile / execute (now owns the full IR + physics layer)

- `sysml-service` — the unified command hub under test

- `sysml-lsp-server` (path dep) + `tower-lsp` — in-process LSP transport

- `sysml-api` + `axum` + `tower` + `http-body-util` — in-process REST transport

- `insta` (json, filters) — snapshot testing; `pretty_assertions`, `regex`, `tokio`, `chrono`, `futures`

> ⚠  **Removed:** `sysml-parser-batch` is no longer a dependency — the Pest parser and its rule-coverage consumer were deleted (see the comment at the top of `Cargo.toml`). It is a test-only crate, so nothing depends on it downstream.

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
