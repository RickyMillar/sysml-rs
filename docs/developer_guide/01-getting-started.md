# Getting Started

This guide covers setting up your development environment for sysml-rs.

## Prerequisites

### Required

- **Rust 1.75+** (2024 edition features)
- **Git** for version control

### Optional

- **PostgreSQL 14+** for the `sysml-store` PostgreSQL backend tests (behind the `sqlx` feature)
- **Python 3.10+** for benchmark report generation
- **rust-gdb** or **rust-lldb** for debugging

## Building

```bash
# Clone the repository
git clone https://github.com/your-org/sysml-rs.git
cd sysml-rs

# Build all crates
cargo build

# Build with release optimizations
cargo build --release
```

## Running Tests

### Quick Test (no corpus)

```bash
# Run unit tests only (fast, ~30 seconds)
cargo test
```

### Full Test Suite (with corpus)

```bash
# Set the corpus path
export SYSML_CORPUS_PATH=/path/to/sysml-rs/references/sysmlv2

# Run all tests including ignored corpus tests
cargo test -- --ignored
```

### Specific Test Categories

```bash
# Parsing coverage only
SYSML_CORPUS_PATH=/path/to/sysml-rs/references/sysmlv2 \
cargo test -p sysml-spec-tests corpus_coverage -- --ignored --nocapture

# Resolution tests (slower, ~3 min)
SYSML_CORPUS_PATH=/path/to/sysml-rs/references/sysmlv2 \
cargo test -p sysml-spec-tests corpus_resolution_multi_file -- --ignored --nocapture

# Quick smoke test (fast sanity check)
SYSML_CORPUS_PATH=/path/to/sysml-rs/references/sysmlv2 \
cargo test -p sysml-spec-tests corpus_smoke_test -- --ignored --nocapture
```

### Diagnostic & Highlighting Quality Checks

```bash
# Diagnostic regression check (all example collections vs baseline)
./scripts/diagnostic_sweep.sh

# Quick check (book examples only, < 30s)
./scripts/diagnostic_sweep.sh --quick

# Update baseline after intentional changes
./scripts/diagnostic_sweep.sh --update

# Keyword highlight coverage (grammar vs highlights.scm)
./scripts/highlight_coverage.sh

# Diagnostic severity enforcement
./scripts/severity_audit.sh

# Snapshot tests for book examples (diagnostic + token output)
cargo test -p sysml-lsp-server snapshot_diagnostics_book -- --nocapture

# False-positive guard tests
cargo test -p sysml-lsp-server entry_succession_no_sm002 return_param_no_e004 -- --nocapture

# Keyword coverage Rust test
cargo test -p sysml-spec-tests highlights_scm_covers -- --nocapture
```

These scripts live in `scripts/`; run them with `--help` for options. The
diagnostic sweep compares every example collection against a locked baseline,
so it is the canonical regression gate for diagnostic/highlighting quality.

## IDE Setup

### VS Code

Recommended extensions:
- **rust-analyzer** - Rust language support
- **Even Better TOML** - Cargo.toml editing
- **Error Lens** - Inline error display

Settings (`.vscode/settings.json`):
```json
{
    "rust-analyzer.cargo.features": "all",
    "rust-analyzer.checkOnSave.command": "clippy"
}
```

### IntelliJ / CLion

- Install the Rust plugin
- Import as Cargo project
- Enable clippy in settings

## Project Structure

The workspace is split into three crate groups. There are **10 lang crates**,
**11 tooling crates**, and **1 test crate**. (Verified against the `[workspace]`
members in the root `Cargo.toml`.)

```
sysml-rs/
├── crates/
│   ├── lang/                       # SysML v2 spec implementation (10 crates)
│   │   ├── sysml-id/               # ElementId, QualifiedName, ProjectId, CommitId
│   │   ├── sysml-span/             # Span, Diagnostic, severity
│   │   ├── sysml-project/          # Project / workspace identity, .kpar archives
│   │   ├── sysml-manifest/         # sysml.toml / sysml.lock parsing
│   │   ├── codegen/                # build-time codegen library (used by sysml-core/build.rs)
│   │   ├── sysml-core/             # Element, Relationship, ModelGraph, resolution,
│   │   │                           #   validation, elaboration, physics, canonical JSON
│   │   │                           #   (absorbed sysml-meta + sysml-canon)
│   │   ├── sysml-parser-trait/     # Parser trait, SysmlFile, ParseResult
│   │   ├── sysml-parser-incremental/ # Tree-sitter — the SOLE parser (impl Parser)
│   │   ├── sysml-runtime/          # Execution engine + full analysis IR
│   │   │                           #   (absorbed sysml-analysis-ir; diffsol physics)
│   │   └── sysml-diagram/          # Visualization IR → Sprotty SModel (rlib, server-rendered)
│   ├── tooling/                    # Developer tools consuming the lang crates (10 crates)
│   │   ├── sysml-resolve/          # Multi-package dependency resolution
│   │   ├── sysml-query/            # Transport-agnostic structured-query engine
│   │   ├── sysml-ide-db/           # Salsa incremental database (AnalysisHost / Analysis)
│   │   ├── sysml-store/            # Store trait + InMemory + PostgreSQL backend
│   │   ├── sysml-service/          # Unified command surface (133 #[service_command])
│   │   ├── sysml-service-macros/   # #[service_command] / #[service_impl] proc-macros
│   │   ├── sysml-lsp-server/       # tower-lsp server (thin wrapper over the service)
│   │   ├── sysml-cli/              # sysml <subcommand> CLI
│   │   ├── sysml-api/              # axum REST + WebSocket
│   │   └── sysml-mcp/              # rmcp MCP server (125 tools)
│   └── testing/
│       └── sysml-spec-tests/       # OMG spec corpus coverage tests
├── docs/developer_guide/           # This documentation
├── benchmarks/                     # Performance benchmarks
└── references/sysmlv2/             # Spec files (vendored)
```

> **Came from older docs/code?** `sysml-meta` and `sysml-canon` were folded into
> `sysml-core`; `sysml-store-postgres` into `sysml-store`; `sysml-analysis-ir`
> into `sysml-runtime`; and `sysml-parser-batch` (Pest) was deleted. See the
> consolidation table in [00-architecture.md](00-architecture.md).

## Reference Materials

The `references/sysmlv2/` directory contains the SysML v2 specification materials:

- **Grammar files**: `SysML.xtext`, `KerML.xtext`
- **Vocabulary**: `SysML-vocab.ttl`, `KerML-vocab.ttl`
- **API specs**: `OpenAPI.json`, `SysmlAPISchema.json` (untracked local-optional since 2026-07-30; on fresh clones fetch from https://www.omg.org/spec/SystemsModelingAPI/)
- **Standard library**: `library.kernel/`, `library.systems/`
- **Test corpus**: `org.omg.sysml.xpect.tests/`

See each crate's `README.md` for detailed reference mappings.

## Common Tasks

### Adding a New Element Type

1. Research the type in `references/sysmlv2/SysML-vocab.ttl`
2. `ElementKind` is **generated** from the spec TTL by `sysml-core/build.rs` —
   add the type to the vocab/shapes inputs rather than hand-editing the enum
   (never edit `*.generated.rs`). See [04-codegen.md](04-codegen.md).
3. Add syntax support in the tree-sitter grammar
   (`crates/lang/sysml-parser-incremental/tree-sitter/rules/*.js`) if needed —
   see [02-parsing.md](02-parsing.md) for the grammar-edit constraints.
4. Add the CST → `ModelGraph` conversion in
   `crates/lang/sysml-parser-incremental/src/ast_builder/`.
5. Update tests (`sysml-spec-tests`, plus the tree-sitter corpus).

### Debugging Parse Failures

See [Parsing Guide](02-parsing.md) for detailed debugging workflow.

### Running Benchmarks

Benchmarks are criterion targets living in each crate's `benches/` directory —
there is no top-level benchmark runner.

```bash
cargo bench -p sysml-core          # core_benchmarks, resolution_* suites
cargo bench -p sysml-runtime       # espresso cell/pump, orchestrator, dense constraints
cargo bench -p sysml-service       # bench_check_constraints, bench_sim_start

cargo bench -p sysml-runtime --bench bench_dense_constraints   # one target
```

Criterion writes its own baseline comparison to `target/criterion/` and prints
the regression/improvement verdict against the previous run.

## Getting Help

- Check existing documentation in `developer_guide/`
- Read `tools/fetch-references/README.md` for how the OMG spec references are fetched and where they land
- Open an issue on GitHub for bugs or questions
