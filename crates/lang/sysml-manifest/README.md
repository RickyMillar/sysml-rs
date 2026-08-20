# sysml-manifest

The `Cargo.toml` equivalent for SysML v2 projects: parse, build, serialize and discover `sysml.toml` manifests and `sysml.lock` lock files.

`Layer 2 · lang` · `project metadata` · `crate-type: rlib` · `TOML · serde`

## Overview

`sysml-manifest` owns the on-disk project description for a SysML workspace. It is a small, pure-data crate: TOML in, strongly-typed structs out (and back). It defines the schema of `sysml.toml` (project identity, package IRI, standard-library selection, dependencies, workspace layout) and `sysml.lock` (resolved package pins), plus the directory walk-up search that locates them. It performs *no* dependency resolution, fetching, or model parsing — those live downstream in `sysml-resolve` and `sysml-project`.

**What it owns.**

- **Manifest schema** — the canonical shape of `sysml.toml`.

- **Lock-file schema** — resolved package pins + checksums.

- **Dependency grammar** — path / git / kpar / registry source forms.

- **Stdlib selection** — which of the 10 OMG standard libraries are active.

- **Discovery** — walk-up search for manifests and workspace roots.

**What it does *not* do.**

- No SemVer math — versions round-trip as plain strings.

- No network / git / archive fetching.

- No transitive dependency resolution (that is `sysml-resolve`).

- No model/CST parsing (that is the parser + `sysml-project`).

- No element identity types — purely filesystem + TOML data.

## Where it sits

```text
downstream sysml-project sysml-resolve sysml-cli sysml-lsp-server sysml-service
▲ depend on
this crate sysml-manifest
▼ depends on
upstream serde toml thiserror
```

A leaf data crate. It has **no first-party (workspace) dependencies** — only the three third-party crates above. The `walk_up` directory-search helper is re-exported and reused by `sysml-project` so both crates share one termination-correct walk loop.

## The `sysml.toml` manifest

A representative manifest exercising every section:

```
[project]
name = "coffee-machine"
version = "0.1.0"
description = "Smart coffee machine system model"
license = "MIT"
sysml-edition = "2025"          # default "2025"; omitted on save when default
authors = ["Alice <alice@example.com>"]

[package]
iri = "urn:acme:coffee-machine" # else auto-generated as urn:sysml:<name>

[stdlib]
include_only = ["systems", "analysis", "geometry"]   # allowlist
exclude      = ["analysis"]                          # removal pass

[dependencies]
beverage-types = { path = "../beverage-types" }
thermal-model  = { git = "https://github.com/acme/thermal-model", tag = "v1.0.0" }
sensor-defs    = { kpar = "https://example.com/sensors-v2.kpar" }
base-types     = "1.0"          # short form → Dependency::Registry

[workspace]
members         = ["beverage-types", "coffee-machine"]
exclude         = ["legacy"]
default-members = ["coffee-machine"]

[workspace.project]            # shared config members can inherit
sysml-edition = "2025"
license       = "MIT"
```

## Public API

Everything below is re-exported from the crate root (`sysml_manifest::*`). Sort by any column; filter to find a symbol.

| Symbol | Kind | Module | Purpose |
|---|---|---|---|
| `SysmlManifest` | struct | manifest | Top-level parsed `sysml.toml` (`project`, `package`, `stdlib`, `dependencies`, `workspace`). |
| `ProjectConfig` | struct | manifest | `[project]` — name, version, description, license, `sysml-edition`, authors. |
| `PackageConfig` | struct | manifest | `[package]` — optional canonical `iri` for publishing / KPAR generation. |
| `WorkspaceConfig` | struct | manifest | `[workspace]` — members, exclude, default-members, shared `project`. |
| `Dependency` | enum | dependency | Untagged: `Registry(String)` (short form) or `Detailed(..)` (table form). |
| `GitRef` | enum | dependency | Resolved git ref: `Tag` / `Branch` / `Rev` / `DefaultBranch`. |
| `StdlibConfig` | struct | stdlib | `[stdlib]` — `include_only` / `exclude` over 10 OMG libraries. |
| `LockFile` | struct | lock | Parsed `sysml.lock` — `lock_version` + resolved `packages`. |
| `LockedPackage` | struct | lock | One resolved pin: name, version, prefixed `source`, optional checksum / requested. |
| `ManifestError` | enum | error | Error type: `Io`, `Parse`, `Serialize`, `InvalidVersion`, `MissingField`, `Other`. |
| `walk_up` | fn | path_walk | Generic walk-up-dir helper; also used by `sysml-project`. |
| `find_manifest` | fn | discovery | Walk up from a dir; return first `(PathBuf, SysmlManifest)`. |
| `find_workspace` | fn | discovery | Walk up; return first manifest carrying a `[workspace]` section. |
| `load_manifest` | fn | manifest | Read + parse a `sysml.toml` from a path. |
| `save_manifest` | fn | manifest | Serialize a manifest to a `sysml.toml` file (pretty TOML). |
| `load_lock` | fn | lock | Read + parse a `sysml.lock` from a path. |
| `save_lock` | fn | lock | Serialize a lock file to disk (pretty TOML). |
| `MANIFEST_FILENAME` | const | lib | `"sysml.toml"` |
| `LOCK_FILENAME` | const | lib | `"sysml.lock"` |

Note: `parse_manifest` / `parse_lock` exist but are *not* re-exported at the crate root; use `load_manifest` / `load_lock` for path-based loading.

## Dependency source forms

`Dependency` is `#[serde(untagged)]`. The short string form parses to `Registry`; any inline table parses to `Detailed`. Classification predicates inspect the populated fields:

| Source | TOML form | Variant | Predicate | Accessor |
|---|---|---|---|---|
| path | `{ path = "../lib" }` | Detailed | `is_path()` | `as_path()` |
| git (tag) | `{ git = "…", tag = "v1" }` | Detailed | `is_git()` | `as_git_url()` · `git_ref()` |
| git (branch) | `{ git = "…", branch = "main" }` | Detailed | `is_git()` | `as_git_url()` · `git_ref()` |
| git (rev) | `{ git = "…", rev = "abc123" }` | Detailed | `is_git()` | `as_git_url()` · `git_ref()` |
| kpar | `{ kpar = "…/lib.kpar" }` | Detailed | `is_kpar()` | `as_kpar_url()` |
| registry (short) | `dep = "1.0"` | Registry | `is_registry()` | `as_registry_requirement()` |
| registry (table) | `{ version = "1.0", registry = "…" }` | Detailed | `is_registry()` | `as_registry_requirement()` · `registry_backend()` |

> ⚠  **Predicates are not mutually exclusive.** `is_registry()` is true for any `Detailed` dependency that carries a `version` — so a git dep with a `version` field reports both `is_git() == true` and `is_registry() == true`. Callers that route on source kind should check `is_path` / `is_git` / `is_kpar` *before* falling back to registry.

## Standard-library selection

`StdlibConfig` filters the 10 known OMG libraries. Default (both lists empty) enables all. `include_only` is an allowlist; `exclude` is a removal pass applied after it; an `exclude` entry of `"all"` or `"*"` disables everything. Each library maps to a versioned OMG `20250201` KPAR URL.

| Library name | Spec | Kind | Version |
|---|---|---|---|
| `semantic` | KerML | core | 1.0.0 |
| `data-type` | KerML | core | 1.0.0 |
| `function` | KerML | core | 1.0.0 |
| `systems` | SysML | core | 1.0.0 |
| `analysis` | SysML | domain | 2.0.0 |
| `cause-and-effect` | SysML | domain | 2.0.0 |
| `geometry` | SysML | domain | 2.0.0 |
| `metadata` | SysML | domain | 2.0.0 |
| `quantities-and-units` | KerML | domain | 2.0.0 |
| `requirement-derivation` | SysML | domain | 2.0.0 |

Accessors: `StdlibConfig::known_library_names()`, `enabled_libraries()`, `has_domain_libraries()`, and the static lookups `library_kpar_url(name)` / `library_version_constraint(name)`.

## Modules

| Module | Visibility | Responsibility | Key items |
|---|---|---|---|
| `lib` | root | Re-exports + filename constants. | `MANIFEST_FILENAME`, `LOCK_FILENAME` |
| `manifest` | private (re-exported) | Manifest schema + load/save/parse. | `SysmlManifest`, `ProjectConfig`, `PackageConfig`, `WorkspaceConfig` |
| `dependency` | private (re-exported) | Dependency grammar + classification. | `Dependency`, `DetailedDependency`, `GitRef` |
| `lock` | private (re-exported) | Lock-file schema + load/save/parse. | `LockFile`, `LockedPackage` |
| `stdlib` | private (re-exported) | Standard-library selection + KPAR URLs. | `StdlibConfig` |
| `discovery` | private (re-exported) | Walk-up search for manifests / workspaces. | `find_manifest`, `find_workspace` |
| `path_walk` | **pub** | Generic walk-up-dir loop (shared with `sysml-project`). | `walk_up` |
| `error` | private (re-exported) | Error type + constructors. | `ManifestError` |

## Usage

Build a manifest in code, save it, then discover and load it back:

```
use std::path::Path;
use sysml_manifest::{
    find_manifest, load_manifest, save_manifest,
    Dependency, SysmlManifest,
};

// Build a manifest programmatically.
let mut manifest = SysmlManifest::new("coffee-machine", "0.1.0");
manifest.add_dependency("beverage-types", Dependency::path("../beverage-types"));
manifest.add_dependency(
    "thermal-model",
    Dependency::git_tag("https://github.com/acme/thermal-model", "v1.0.0"),
);

// urn:sysml:coffee-machine (auto-generated; no [package] set)
let iri = manifest.effective_iri();
// All 10 stdlib libraries enabled (no [stdlib] set)
let libs = manifest.effective_stdlib().enabled_libraries();

// Round-trip through disk.
save_manifest(Path::new("sysml.toml"), &manifest)?;
let reloaded = load_manifest(Path::new("sysml.toml"))?;
assert_eq!(manifest, reloaded);

// Discover the nearest manifest by walking up from a working dir.
if let Some((path, found)) = find_manifest(Path::new("./src/models"))? {
    println!("found {} at {}", found.project.name, path.display());
}
# Ok::<(), sysml_manifest::ManifestError>(())
```

## Invariants & pitfalls

**Untagged enum order.**

`Dependency::Registry` **must** precede `Detailed` in the enum so serde's untagged matching tries the string form first.

**Stringly-typed lock sources.**

`LockedPackage.source` is a prefixed string — `path:`, `git:<url>#<commit>`, `kpar:`, `registry:`. There is no typed enum; consumers parse the prefix (helpers `is_path` / `is_git` / `is_kpar` exist).

**Defaults are skipped on save.**

`sysml-edition` defaults to `"2025"` and is omitted when serializing if unchanged. Empty maps / vecs and `None` options are likewise skipped.

**Strict `[stdlib]` schema.**

`StdlibConfig` uses `deny_unknown_fields`, so legacy boolean-keyed configs like `systems = true` are rejected at parse time.

**Prefer the effective accessors.**

Use `effective_stdlib()` (all-enabled default) and `effective_iri()` rather than reading `.stdlib` / `.package` directly.

**Workspace discovery skips non-workspace manifests.**

`find_workspace` keeps walking up past any `sysml.toml` that lacks a `[workspace]` section.

## Dependencies

| Crate | Direction | Why |
|---|---|---|
| `serde` | upstream | Derive Serialize/Deserialize on all schema types. |
| `toml` | upstream | The sole on-disk format; parse + pretty-serialize. |
| `thiserror` | upstream | `ManifestError` derivation. |
| `pretty_assertions` | dev-only | Readable diffs in round-trip tests. |
| `sysml-project` | downstream | Project discovery; reuses `walk_up`. |
| `sysml-resolve` | downstream | Dependency resolution over the manifest graph. |
| `sysml-cli` | downstream | Init / build / lock commands. |
| `sysml-lsp-server` | downstream | Workspace + project awareness. |
| `sysml-service` | downstream | Unified service layer project state. |

>  **Known doc-vs-Cargo drift (2026-06-03).** `Cargo.toml` still declares `sysml-id` and `semver` as dependencies, but *neither is referenced anywhere in `src/`* — versions are stored and round-tripped as plain strings, and no element-id types are used. Both are dead deps slated for removal; the `ManifestError::InvalidVersion` / `MissingField` variants are likewise dormant (no constructors or call sites). They are intentionally omitted from the upstream table above.

## Tests

Unit tests live inline in each module (no external test dir):

```
cargo test -p sysml-manifest
```

Coverage: minimal / full / workspace manifest parsing, manifest & lock round-trips, lock sort, dependency-type detection, walk-up discovery (current dir, walk-up, none, workspace), `walk_up` first-match / root-termination, stdlib filtering (include_only / exclude / exclude-all), and rejection of legacy boolean-keyed `[stdlib]`.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
