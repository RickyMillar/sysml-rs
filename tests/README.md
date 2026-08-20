# Testing Quick Reference

## Running Tests

```bash
# All unit tests (~2100)
cargo test --workspace --lib

# Specific crate
cargo test -p sysml-core --lib

# LSP server tests (~630)
cargo test -p sysml-lsp-server --lib

# Integration tests for a crate
cargo test -p sysml-cli --test '*'

# All integration tests
cargo test --workspace --test '*'
```

## Corpus Tests

The spec corpus validates our parser against 56+ SysML files from the Advent of SysML series.

```bash
# Quick: use the cargo alias
cargo test-corpus

# With feature flag (no --ignored needed)
cargo test -p sysml-spec-tests --features corpus

# Convenience script (auto-sets SYSML_CORPUS_PATH)
./tools/scripts/run-corpus-tests.sh

# Run specific corpus test
cargo test-corpus -- advent_tree_sitter
```

Corpus tests require `SYSML_CORPUS_PATH` to point at the reference materials (defaults to `references/sysmlv2/`).

## Test Organization

### Overview

- **Unit tests**: `#[cfg(test)] mod tests` within each crate's `src/`
- **Integration tests**: `crates/<group>/<crate>/tests/`
- **Spec corpus**: `crates/testing/sysml-spec-tests/`
- **Shared fixtures**: `tests/fixtures/shared/`
- **Book examples**: `tests/fixtures/book-examples/`

### Integration Test Files by Crate

Counts below refreshed 2026-07-29 (derive with `ls crates/<group>/<crate>/tests/*.rs | wc -l`):

| Crate | Unit Tests | Integration Test Files | Pattern |
|-------|-----------|----------------------|---------|
| sysml-cli | inline | 11 files | One file per subcommand |
| sysml-diagram | inline | 14 files | One file per view type |
| sysml-spec-tests | library | 40 files | Tier map: `crates/testing/sysml-spec-tests/README.md` |
| sysml-lsp-server | ~630 inline | - | Tests alongside implementation |
| sysml-ide-db | inline | 3 files | project_integration, id_stability, eval_context_seed |

## Fixtures

See `tests/fixtures/README.md` for fixture conventions and locations.

See `tests/fixtures/MANIFEST.toml` (when present) for a machine-readable catalog of all fixture directories and files across the project.

Note: crate-local fixtures stay in their crate (`crates/<group>/<crate>/fixtures/`), while cross-crate fixtures go in `tests/fixtures/shared/`.

## Snapshots (insta)

```bash
# Review pending snapshot changes
cargo insta review

# Update snapshots in-place
cargo insta test --review
```

## Benchmarks

```bash
# Core performance
cargo bench -p sysml-core

# All benchmarks
cargo bench --workspace
```
