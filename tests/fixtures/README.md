# Test Fixtures

Shared and frozen test data for cross-crate tests.

## Directory Layout

| Directory | Purpose |
|-----------|---------|
| `shared/` | Fixtures used by multiple crates (LSP, CLI, diagnostics) |
| `book-examples/` | Frozen copies from `the-book/examples/` — do not edit in place |
| `vehicle-project/` | Multi-file project with `.project.json` metadata |
| `multi-workspace/` | Multi-project workspace with `.workspace.json` |

## Conventions

- **Crate-local fixtures** live in `crates/<group>/<crate>/fixtures/{valid,invalid,regression}/`
- **Cross-crate fixtures** live here in `tests/fixtures/shared/`
- **Naming**: `{feature}_{scenario}.sysml`
- **Book examples** are frozen snapshots — update by re-copying from `the-book/` when the book changes
- CLI fixtures in `crates/tooling/sysml-cli/fixtures/` are symlinks to shared where identical

## `shared/` Contents

| File | Origin | Used By |
|------|--------|---------|
| `simple_vehicle.sysml` | CLI fixtures (62 lines) | LSP diagnostic_ux_tests |
| `test_all_features.sysml` | CLI fixtures | LSP diagnostic_ux_tests |
| `test_hover.sysml` | CLI fixtures | LSP diagnostic_ux_tests |
| `test_action.sysml` | CLI fixtures | LSP diagnostic_ux_tests |
| `test_whatif.sysml` | CLI fixtures | LSP diagnostic_ux_tests, protocol_tests |
| `test_flow.sysml` | CLI fixtures | LSP diagnostic_ux_tests |
| `test_health_diagnostics.sysml` | CLI fixtures | LSP diagnostic_ux_tests |
| `sysml-rs-model.sysml` | `model/sysml-rs.sysml` | LSP diagnostic_ux_tests |
| `sensemetry.sysml` | LSP fixtures | parser-batch, CLI inspect |

## Visualization Coverage Fixtures

Files in `../vis-coverage/` (when present) are used by `sysml-diagram` tests and the fixture generation pipeline.
The script `tools/scripts/generate-vis-fixtures.sh` converts these to JSON in `editors/diagram/fixtures/`.

## Complete Catalog

See `MANIFEST.toml` (when present) for a machine-readable catalog of all fixture directories and files across the project.

## When to Add Fixtures

- **Shared fixture** (`tests/fixtures/shared/`): When multiple crates need the same .sysml file
- **Crate-local fixture** (`crates/.../fixtures/`): When only one crate uses the file
- **Invalid fixture** (`crates/.../fixtures/invalid/`): For error recovery and diagnostic testing
- **Vis-coverage fixture** (`tests/vis-coverage/`): For testing diagram view types
