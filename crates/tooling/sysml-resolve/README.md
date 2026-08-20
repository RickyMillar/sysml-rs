# sysml-resolve

Resolves a `sysml.toml` manifest into a topologically-ordered dependency graph — path, git, KPAR and registry sources — with cycle detection, diamond dedup, SHA-256 integrity checks and lock-file generation. SysML's answer to Cargo's resolver.

`Layer 3 · tooling` · `dependency resolution` · `crate-type: rlib` · `4 source providers` · `1 registry backend`

## What it owns

`sysml-resolve` turns a project's root `sysml.toml` into a fully materialised, build-ready set of source directories. It walks the declared dependencies depth-first, fetches and caches anything remote, verifies artifact integrity, and emits a deterministic `ResolvedGraph` ordered so that every package appears *before* the packages that depend on it. It also generates and validates `sysml.lock` for reproducible builds.

**Resolution.**

Post-order DFS over transitive deps with cycle detection and diamond dedup. The root project is never added to the graph — only its dependencies.

**Fetch & cache.**

Git mirrors, KPAR archives and registry artifacts materialise into a per-source cache under the platform cache dir; path deps resolve in place.

**Lock files.**

`generate_lock` / `is_lock_up_to_date` produce a `LockFile` sorted by `(name, source)` — independent of resolution order.

## Where it sits

```text
consumers sysml-cli · sysml-service · sysml-lsp-server
▲ resolve() · source_paths() · generate_lock()
this crate sysml-resolve → resolverprovidersregistrygraphlockerror
▼ depends on
upstream sysml-manifest sysml-project sysml-id semver sha2 ureq directories
```

Sits one layer above `sysml-manifest` (which parses `sysml.toml` / `sysml.lock`) and reuses `sysml-project`'s `kpar` module to read archives.

## Public API

The crate surface is small and re-exported from `lib.rs`: two top-level resolution functions, two lock helpers, three registry-metadata helpers, and the graph/error types.

#### `fn resolve(manifest, manifest_dir) -> Result<ResolvedGraph, ResolveError>`

Resolve the full transitive dependency graph from a parsed manifest and its directory. Drives the `ResolverEngine` (post-order DFS). The root project is excluded from the returned graph.

#### `fn source_paths(graph: &ResolvedGraph) -> Vec<PathBuf>`

Collect every `.sysml` source file path from all resolved packages, recursing into each package's `source_dir`. Files and subdirs are sorted for deterministic output.

#### `fn generate_lock(graph: &ResolvedGraph) -> LockFile`

Produce a `sysml_manifest::LockFile` from a resolved graph. Entries are sorted by `(name, source)` for determinism — **not** by graph/topological order.

#### `fn is_lock_up_to_date(graph: &ResolvedGraph, lock: &LockFile) -> bool`

Returns true if the (re-sorted) lock matches the resolved graph. Used to decide whether `sysml.lock` needs regenerating.

#### `fn resolve_registry_release_metadata(backend, package, version) -> Result<RegistryReleaseMetadata, ResolveError>`

Look up the metadata (resolved version, KPAR artifact URL, checksum) for one exact registry release without fetching it.

#### `fn resolve_latest_registry_release_metadata(backend, package, requirement) -> Result<RegistryReleaseMetadata, ResolveError>`

Find the highest registry release matching a semver requirement and return its metadata.

#### `struct RegistryReleaseMetadata`

Metadata for a registry release: resolved version, artifact URL, and checksum. Returned by the two metadata-lookup helpers above.

#### `struct ResolvedGraph`

The fully resolved graph: `packages: Vec<ResolvedPackage>` in topological order (deps before dependents). Helpers: `find(name)`, `contains(name)`, `len()`, `is_empty()`, `new()`.

#### `struct ResolvedPackage`

One resolved dependency: `name`, `version`, `source: PackageSource`, and `source_dir: PathBuf` — the directory holding the package's `.sysml` sources.

#### `enum PackageSource — Path | Git | Kpar | Stdlib | Registry`

The origin of a resolved package. Carries a `to_lock_source() -> String` producing the lock-file descriptor for each variant (see the variant table below).

#### `enum ResolveError`

Resolution failure modes: `Io`, `Manifest`, `Cycle`, `MissingDependency`, `ChecksumMismatch`, `UnsupportedSource` (see error table below).

## Key modules

| Module | Responsibility | Key items |
|---|---|---|
| `resolver.rs` | The resolution engine: post-order DFS with cycle detection and diamond-dep dedup. Entry point for the whole crate. | `resolve`, `source_paths`, `ResolverEngine`, `PackageIdentity` |
| `providers.rs` | Source-family dispatch. The `SourceProvider` trait plus four impls and the cache-path helpers. Largest module (~60 KB incl. tests). | `SourceProvider`, `SourceProviderRegistry`, `PathProvider`, `GitProvider`, `KparProvider`, `RegistryProvider` |
| `registry.rs` | Registry index resolution, artifact fetch and semver matching. The `RegistryBackend` trait plus the single Sysand backend. | `RegistryBackend`, `RegistryBackendRegistry`, `SysandRegistryBackend`, `RegistryReleaseMetadata` |
| `graph.rs` | Resolved-graph data types and the lock-source string encoding. | `ResolvedGraph`, `ResolvedPackage`, `PackageSource` |
| `lock.rs` | Deterministic lock-file generation and change detection from a resolved graph. | `generate_lock`, `is_lock_up_to_date` |
| `error.rs` | The crate's error enum and its constructor helpers. | `ResolveError` |

## Provider dispatch

The `SourceProviderRegistry` holds four providers in a fixed order and delegates each dependency to the first whose `supports()` returns true. Each remote provider materialises its source into a per-source cache directory under the platform cache root (overridable via `SYSML_RS_CACHE_DIR`).

```text
specDependency (from sysml.toml)
▼ first supports() wins
1PathProvider→resolves in place (relative/abs path)
2GitProvider→…/dependencies/git/<hash(url)>
3KparProvider→…/dependencies/kpar/<hash(src)>
4RegistryProvider→…/dependencies/registry/<…>
```

### PackageSource variants

| Variant | Fields | `to_lock_source()` |
|---|---|---|
| `Path` | `String` (original, possibly relative, path) | `path:<p>` |
| `Git` | `url`, resolved `commit` | `git:<url>#<commit>` |
| `Kpar` | `url` | `kpar:<url>` |
| `Stdlib` | *(none)* — the built-in SysML standard library | `stdlib` |
| `Registry` | `backend`, `package`, `requested`, resolved `version` | `registry:<backend>:<package>@<version>` |

### ResolveError variants

| Variant | Display message |
|---|---|
| `Io` | `I/O error at {path}: {source}` |
| `Manifest` | `manifest error: {0}` — wraps `sysml_manifest::ManifestError` |
| `Cycle` | `dependency cycle detected: {a -> b -> …}` |
| `MissingDependency` | `dependency '{name}' not found at path '{path}'` |
| `ChecksumMismatch` | `checksum mismatch at {path}: expected {expected}, got {actual}. {hint}` |
| `UnsupportedSource` | `dependency source type not yet supported: {dep_type} (dependency '{name}')` |

## Usage

Resolve a project, list its source files, and refresh the lock file if stale:

```
use std::path::Path;
use sysml_manifest::{load_manifest, load_lock, save_lock};
use sysml_resolve::{resolve, source_paths, generate_lock, is_lock_up_to_date};

let dir = Path::new("/my/project");
let manifest = load_manifest(&dir.join("sysml.toml")).unwrap();
let graph = resolve(&manifest, dir).unwrap();

println!("{} dependencies resolved", graph.len());
for path in source_paths(&graph) {
    println!("  {}", path.display());
}

// Regenerate sysml.lock only when it no longer matches the graph.
let lock_path = dir.join("sysml.lock");
if lock_path.exists() {
    let existing = load_lock(&lock_path).unwrap();
    if !is_lock_up_to_date(&graph, &existing) {
        save_lock(&lock_path, &generate_lock(&graph)).unwrap();
    }
} else {
    save_lock(&lock_path, &generate_lock(&graph)).unwrap();
}
```

## Dependencies

| Crate | Role |
|---|---|
| `sysml-manifest` | Parses `sysml.toml` / `sysml.lock`; supplies `Dependency`, `LockFile`, `ManifestError`. |
| `sysml-project` | `kpar` module — reading and extracting `.kpar` archives for KPAR / registry deps. |
| `sysml-id` | Shared identifier types. |
| `semver` | Semantic-version parsing and range matching for registry deps. |
| `sha2` | SHA-256 checksums for artifact integrity and cache-dir hashing. |
| `ureq` | Blocking HTTP client for remote registry index and KPAR/artifact downloads. |
| `directories` | `ProjectDirs` / `BaseDirs` — platform-appropriate cache directory. |
| `serde` / `serde_json` | Registry index / API response parsing. |
| `thiserror` | Derives the `ResolveError` enum. |
| `tracing` | Structured logging of resolution progress. |

**Downstream:** `sysml-cli` (resolve / build pipeline), `sysml-service` and `sysml-lsp-server` (workspace dependency resolution).

## Invariants & pitfalls

- **Post-order DFS, root excluded.** A package is appended only after its own deps, guaranteeing topological order; the root project is never in the graph.

- **Cycle detection canonicalises.** `in_stack` holds `canonicalize()`-d dirs in the current DFS path — symlinks and `././` aliases to the same dir are caught. Distinct relative-path strings to the same dir are treated as distinct declarations.

- **Git needs the system `git` CLI.** `GitProvider` shells out to `git` for clone/fetch/checkout — there is no libgit2. A runtime/portability dependency.

- **Integrity.** KPAR and registry artifacts are SHA-256 verified; a `.sysml-kpar-checksum` marker is written into extracted dirs for lock-file use.

- **Cache layout.** Base cache root resolves via `SYSML_RS_CACHE_DIR` override → `ProjectDirs::from("","","sysml-rs")` → `BaseDirs` fallback (segment `sysml-rs`) → `/tmp/sysml-rs-cache`. Sources live under `dependencies/{git,kpar,registry}/…`; the Sysand remote index is cached at `dependencies/registry/sysand/index-cache` with a 300 s TTL (1 s under `cfg(test)`).

- **Single registry backend today.** `RegistryBackendRegistry` wires exactly one impl, `SysandRegistryBackend`; the `RegistryBackend` trait is the extension point for future backends.

- **Lock order ≠ graph order.** Lock files sort by `(name, source)` for determinism — don't assume they mirror resolution order.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
