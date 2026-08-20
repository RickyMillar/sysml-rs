# sysml-project

Project & workspace manifests, file-loading discovery primitives, the embedded standard-library registry, and `.kpar` archive I/O — the leaf crate that turns paths on disk into structured SysML v2 projects.

`Layer 2 · lang` · `project discovery` · `KerML Clause 10` · `crate-type: rlib` · `default features: kpar, lock`

## Overview

`sysml-project` implements **KerML Clause 10 (Model Interchange Projects)** plus the workspace's own file-loading model. It is a pure, leaf-level crate: no salsa database, no service state, no project IDs of its own beyond a transient handle. Higher layers (`sysml-ide-db`, `sysml-service`, `sysml-lsp-server`, `sysml-cli`) call into it to answer two questions: *"what is this directory?"* and *"which source files belong to it?"*

The crate owns four concerns, each in its own module group:

**Manifests (KerML Clause 10).**

`.project.json` → `ProjectInfo`, `.meta.json` → `ProjectMeta` (carries a symbol index), `.workspace.json` → `WorkspaceInfo`. Plain serde over JSON.

**File-loading discovery.**

The `discovery` module: `pick_mode` / `discover` / `peek_neighbours` — Phase-1 primitives over `sysml.toml`-rooted directories. The crate's primary, most-consumed surface.

**Standard library.**

`StdlibRegistry` embeds **10** stdlib projects (3 KerML kernel + 7 SysML domain) via `include_str!` and exposes lookup by URN / HTTPS IRI / name, plus a combined cross-project symbol index.

**`.kpar` + lock files.**

Feature-gated: `kpar` (default) for ZIP-archive read/write, `lock` (default) for the TOML `KparLockFile` resolved-graph format.

## Where it sits

```text
consumers sysml-cli sysml-lsp-server sysml-service sysml-ide-db
▲ pure functions · no salsa, no service state
this crate sysml-project
▼ depends on
lang deps sysml-manifest sysml-id (opt · kpar) tree-sitter-sysml
ext deps serde / serde_json walkdir sha2 · hex semver zip (opt) toml (opt) chrono (opt)
```

The salsa-tracked wrappers (`discover_root`, `peek_neighbours_for`) live in `sysml-ide-db` and call the pure functions here. URL → path normalisation is a transport concern handled by `sysml-service`; this crate deals only in `PathBuf`.

## The two discovery modules

> ⚠  **Naming clash — read carefully.** The crate ships two modules whose names differ by one character. They solve different problems and are *both* actively used downstream. Pick by what you are loading.

**`discovery` (src/discovery.rs · ~33 KB).**

**The active file-loading surface.** Phase-1 primitives for the file-loading rebuild, operating over `sysml.toml` manifests and raw `*.sysml`/`*.kerml` source trees.

- `pick_mode(&OpenTarget) → ModeDecision` — Strict / Discovered / DiscoveredViaManifest, walking up for `sysml.toml`.

- `discover(root, max_files) → DiscoveredProject` — recursive scan, stops at nested `sysml.toml` sub-project boundaries.

- `peek_neighbours(file) → NeighbourIndex` — sibling name index for IM010 diagnostic enrichment.

Consumed by `sysml-service`, `sysml-lsp-server`, `sysml-cli` (12+ files). This is what `open_context` runs.

**`discover` (src/discover.rs · legacy walk-up).**

**KerML-manifest discovery.** Walks up the tree looking for the Clause-10 JSON manifests, not `sysml.toml`.

- `discover_project(dir) → DiscoveryResult`

- `discover_workspace(dir) → DiscoveryResult`

- `DiscoveryResult` = `Workspace(PathBuf)` / `Project(PathBuf)` / `NotFound` — workspace wins over project in the same directory.

Still consumed by `sysml-cli`, `sysml-lsp-server`, `sysml-service` for `.workspace.json`/`.project.json` layouts. Predates the `sysml.toml` file-loading rebuild.

>  **Rule of thumb:** opening a folder/file in the modern flow → `discovery::pick_mode`. Resolving a KerML-format `.project.json`/`.workspace.json` layout → `discover_project`. The result types are named apart on purpose: `DiscoveredProject` (the rich scan result) vs `DiscoveryResult` (the legacy 3-variant enum).

## Public API

All re-exported from `lib.rs`. Feature-gated items are marked.

| Symbol | Kind | Module | Feature | Purpose |
|---|---|---|---|---|
| `Project` | struct | project | — | Loaded project: `id`, `info`, `meta`, `root`. `from_directory` / `read_source`. |
| `ProjectHandle` | struct(u32) | project | — | Transient numeric session handle. **Not** `sysml_id::ProjectId` (content-derived, salsa-stable). |
| `ProjectRoot` | enum | project | — | `Directory` / `InMemory` / `Kpar` (kpar variant only with feature `kpar`). |
| `ProjectInfo` | struct | info | — | `.project.json` manifest: name, version, description, topic, usage deps. |
| `ProjectUsage` | struct | info | — | A dependency reference: `resource` IRI + `versionConstraint`. |
| `ProjectMeta` | struct | meta | — | `.meta.json`: `index` (name→file), `created`, `metamodel`, `checksum` map. |
| `FileChecksum` | struct | meta | — | Per-file `value` + `algorithm` entry inside `ProjectMeta.checksum`. |
| `SymbolIndex` | struct | meta | — | Cross-project symbol table: `insert`, `lookup`, `merge`, `conflicts`, `iter`, `len`. |
| `SymbolEntry` | struct | meta | — | One `(symbol, file, project)` row in a `SymbolIndex`. |
| `WorkspaceInfo` | struct | workspace | — | `.workspace.json`: a list of `WorkspaceProject`. |
| `WorkspaceProject` | struct | workspace | — | `path` (relative to workspace root) + `iris`. |
| `StdlibRegistry` | struct | stdlib | — | 10 embedded stdlib projects; `get_by_iri`, `get_by_name`, `kernel_projects`, `symbol_index`. |
| `discover_project` | fn | discover | — | Walk up for `.workspace.json`/`.project.json` → `DiscoveryResult`. |
| `discover_workspace` | fn | discover | — | Walk up for `.workspace.json` only → `DiscoveryResult`. |
| `DiscoveryResult` | enum | discover | — | Legacy: `Workspace` / `Project` / `NotFound`. |
| `compute_checksum` | fn | checksum | — | Hex SHA-256 of bytes. |
| `verify_checksum` | fn | checksum | — | Compare bytes against an expected hex digest. |
| `ChecksumAlgorithm` | enum | checksum | — | Currently only `Sha256`; `from_name` is case-insensitive. |
| `Error` / `Result<T>` | enum / alias | error / lib | — | Crate error type (I/O, JSON, symbol conflict, checksum mismatch, …) and its `Result` alias. |
| `pick_mode` | fn | discovery | — | Decide Strict / Discovered / DiscoveredViaManifest from an `OpenTarget`. |
| `discover` | fn | discovery | — | Recursive source-file scan with sub-project isolation and a file cap. |
| `peek_neighbours` | fn | discovery | — | Sibling-file name index (tree-sitter pass) for IM010 enrichment. |
| `OpenTarget` | enum | discovery | — | User intent: `File` / `Folder` / `Synthetic { uri, content }`. |
| `ModeDecision` | enum | discovery | — | Output of `pick_mode`: `Strict` / `Discovered` / `DiscoveredViaManifest`. |
| `ProjectKind` | enum | discovery | — | Why a project exists: `Strict` / `Discovered` / `DiscoveredViaManifest`. |
| `DiscoveredProject` | struct | discovery | — | Rich scan result: `root`, `manifest`, `files`, `sub_projects`, `warnings`. |
| `NeighbourIndex` | struct | discovery | — | name → declaring files; `lookup` / `is_empty`. |
| `DiagnosticHint` | struct | discovery | — | Soft hint (`code`, `message`, `path`) collected during a scan. |
| `DiscoveryError` | enum | discovery | — | Hard scan error: `CapExceeded` / `Manifest` / `Io`. |
| `KparReader` | struct | kpar | kpar | Streaming reader over a `.kpar` ZIP (generic `Read + Seek`). |
| `KparArchive` | struct | kpar | kpar | In-memory archive (folded in from the former `sysml-kpar` crate). |
| `KparBuilder` | struct | kpar | kpar | Builds a `KparArchive` from a manifest + source dir. |
| `read_kpar` / `write_kpar` | fn | kpar | kpar | In-memory read/write of `.kpar` archives. |
| `KparError` | enum | kpar | kpar | Archive-specific error type. |
| `KparLockFile` | struct | lock | lock | TOML resolved dependency graph (`version` + `projects`). |
| `LockedProject` | struct | lock | lock | One resolved project row with `source` and `dependencies`. |
| `ProjectSource` | enum | lock | lock | `Path` / `Kpar` / `Stdlib` origin of a locked project. |

`KERNEL_PROJECT_URNS` and `StdlibProject` are also public from the `stdlib` module but are not re-exported at crate root; reach them via `sysml_project::StdlibRegistry`'s methods.

## Embedded standard library (10 projects)

Each is embedded at compile time from `src/stdlib_assets/<name>.{project,meta}.json` via `include_str!`. The 3 kernel libraries form a circular dependency cluster, tracked in `KERNEL_PROJECT_URNS`.

| URN name | Group | Kernel? | HTTPS IRI tail |
|---|---|---|---|
| `semantic-library` | KerML kernel | yes | KerML/…/Kernel-Semantic-Library |
| `data-type-library` | KerML kernel | yes | KerML/…/Kernel-Data-Type-Library |
| `function-library` | KerML kernel | yes | KerML/…/Kernel-Function-Library |
| `systems-library` | SysML domain | — | SysML/…/SysML-Systems-Library |
| `analysis-library` | SysML domain | — | SysML/…/SysML-Analysis-Library |
| `cause-and-effect-library` | SysML domain | — | SysML/…/SysML-Cause-and-Effect-Library |
| `geometry-library` | SysML domain | — | SysML/…/SysML-Geometry-Library |
| `metadata-library` | SysML domain | — | SysML/…/SysML-Metadata-Library |
| `quantities-and-units-library` | SysML domain | — | SysML/…/SysML-Quantities-and-Units-Library |
| `requirement-derivation-library` | SysML domain | — | SysML/…/SysML-Requirement-Derivation-Library |

## Key modules

| Module | File | Responsibility · key types |
|---|---|---|
| `discovery` *(pub)* | src/discovery.rs | File-loading rebuild Phase-1 primitives. `pick_mode`, `discover`, `peek_neighbours`; `OpenTarget`, `ModeDecision`, `ProjectKind`, `DiscoveredProject`, `NeighbourIndex`, `DiagnosticHint`, `DiscoveryError`. |
| `discover` | src/discover.rs | Legacy KerML-manifest walk-up. `discover_project`, `discover_workspace`, `DiscoveryResult`. |
| `project` | src/project.rs | Loaded-project shape. `Project`, `ProjectHandle`, `ProjectRoot`. |
| `info` | src/info.rs | `.project.json` parsing. `ProjectInfo`, `ProjectUsage`. |
| `meta` | src/meta.rs | `.meta.json` + symbol index. `ProjectMeta`, `SymbolIndex`, `SymbolEntry`, `FileChecksum`. |
| `workspace` | src/workspace.rs | `.workspace.json` parsing. `WorkspaceInfo`, `WorkspaceProject`. |
| `stdlib` | src/stdlib.rs | Embedded registry. `StdlibRegistry`, `StdlibProject`, `KERNEL_PROJECT_URNS`. |
| `checksum` | src/checksum.rs | SHA-256 helpers. `compute_checksum`, `verify_checksum`, `ChecksumAlgorithm`. |
| `error` | src/error.rs | Crate `Error` enum (feature-gated variants for kpar / lock). |
| `kpar` *(pub · feat)* | src/kpar/*.rs | `.kpar` I/O. `reader.rs` (streaming `KparReader`), `archive_read/write.rs` (in-memory), `schema.rs` (kpar-specific JSON types). |
| `lock` *(feat)* | src/lock.rs | TOML lock file. `KparLockFile`, `LockedProject`, `ProjectSource`. |

## Usage

Load a project directory and read a declared source file:

```
use sysml_project::{Project, ProjectHandle};

// ProjectHandle is a transient session index — NOT sysml_id::ProjectId.
let project = Project::from_directory(ProjectHandle(0), "path/to/my-project")?;
println!("loaded {} v{}", project.info.name, project.info.version);

let src = project.read_source("Parts.sysml")?;
println!("{} bytes", src.len());
# Ok::<(), sysml_project::Error>(())
```

Decide how to open a target, then scan it (the modern file-loading flow):

```
use std::path::PathBuf;
use sysml_project::discovery::{pick_mode, discover, OpenTarget, ModeDecision};

let target = OpenTarget::Folder(PathBuf::from("path/to/workspace"));
match pick_mode(&target) {
    ModeDecision::DiscoveredViaManifest { root, .. } | ModeDecision::Discovered { root } => {
        let project = discover(&root, 10_000)?;
        println!("{} source files, {} sub-projects",
            project.files.len(), project.sub_projects.len());
    }
    ModeDecision::Strict { path } => {
        println!("strict single-file mode: {path:?}");
    }
}
# Ok::<(), sysml_project::discovery::DiscoveryError>(())
```

Query the embedded standard library and build a combined symbol index:

```
use sysml_project::StdlibRegistry;

let registry = StdlibRegistry::new()?;          // parses all 10 embedded manifests
assert_eq!(registry.len(), 10);
assert_eq!(registry.kernel_projects().len(), 3);

let systems = registry.get_by_iri("urn:kpar:systems-library").unwrap();
println!("{}", systems.info.name);              // "SysML Systems Library"

let index = registry.symbol_index();            // merged across all 10 projects
assert!(!index.lookup("Parts").is_empty());
assert!(index.conflicts().is_empty());          // stdlib has no name clashes
# Ok::<(), sysml_project::Error>(())
```

## Features & dependencies

**Features.**

- `kpar` *(default)* — enables the `kpar` module and the `ProjectRoot::Kpar` variant. Pulls in `zip`, `chrono`, and `sysml-id`.

- `lock` *(default)* — enables `KparLockFile` & the TOML lock format. Pulls in `toml`.

**Notable dependencies.**

- `sysml-manifest` — `sysml.toml` parsing + `walk_up` (used by both discovery modules).

- `tree-sitter` + `tree-sitter-sysml` — the lightweight parse in `peek_neighbours`.

- `walkdir` — recursive scan in `discover`.

- `sha2` + `hex` — checksums; `semver` — version handling.

## Pitfalls & invariants

- **`ProjectHandle` ≠ `sysml_id::ProjectId`.** `ProjectHandle(u32)` is a transient session index that does not survive across runs; `sysml_id::ProjectId` is the content-derived, salsa-stable id. Do not conflate them — this crate deliberately keeps its own handle type.

- **Two discovery modules.** `discovery` (sysml.toml file-loading) vs `discover` (KerML JSON-manifest walk-up). Both live and both are consumed — choose by manifest format, not by guessing.

- **Two `ProjectInfo` types.** `crate::ProjectInfo` (the `.project.json` manifest) vs `kpar::schema::ProjectInfo` (kpar-specific, has extra fields). The kpar module re-exports its own; they are not interchangeable.

- **Three project-shaped result types.** `Project` (loaded), `DiscoveredProject` (scan result), `StdlibProject` (registry entry). They carry different data — match the one your call returns.

- **`Project::read_source` rejects kpar roots.** For `ProjectRoot::Kpar` (and `InMemory`) it returns an error — use `KparReader` for archive sources.

- **`discover` isolates nested sub-projects.** A nested `sysml.toml` marks a Cargo-style boundary: its files are excluded from `files` and recorded in `sub_projects` instead.

- **Path form is preserved.** `discover` does not canonicalise or follow symlinks — relative roots yield relative paths, keeping URIs aligned with what the caller passed.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
