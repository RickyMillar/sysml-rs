# sysml-id

Element identifiers, qualified names, deterministic canonical keys, and project/commit IDs — the Layer-0 identity primitives every other crate builds on.

`Layer 0 · lang` · `Identity` · `crate-type: rlib` · `leaf · no workspace deps`

## Overview

`sysml-id` is the foundation crate of the workspace: it owns the small set of value types used to name and identify model elements. It has **zero workspace dependencies** (only `thiserror` plus optional `uuid`, `serde`, `schemars`), so it sits at the very bottom of the dependency graph and is consumed by nearly every other crate. Changes here propagate everywhere — keep the surface minimal and zero-cost.

**The invariant this crate enforces.**


## Where it sits

```text
consumers sysml-core sysml-parser-trait sysml-parser-incremental sysml-runtime sysml-resolve sysml-store sysml-service …and more
▲ depend on
Layer 0 sysml-id
▼ depends on
external thiserror uuid (opt) serde (opt) schemars (opt)
```

## Identity provenance

How a stable `ElementId` is derived as the parser walks a model tree:

```text
root CanonicalKey::root(project_id)
▼ for_named / for_anonymous / for_relationship
child CanonicalKey → "p::Foo#Package::Bar#PartUsage"
▼ to_element_id()
id ElementId (deterministic UUID)
```

The `kind` is embedded at *every* level (not just the leaf) so two elements that share a path but differ in `ElementKind` still get distinct IDs. Anonymous children disambiguate by zero-based sibling index; relationships key on the `(source, kind, target, sibling_index)` tuple.

## Public API

| Type | Kind | Purpose |
|---|---|---|
| `ElementId` | struct | Unique element identifier. Wraps `uuid::Uuid` with the default `uuid` feature, or a plain `String` without it. |
| `CanonicalKey` | struct | Deterministic structural key (newtype over `String`) that hashes into a stable `ElementId`. The identity backbone for the whole model. |
| `QualifiedName` | struct | `::`-separated hierarchical path (e.g. `Package::Part::Attribute`), backed by `Vec<String>` segments. Unicode-aware, escape-aware. |
| `ProjectId` | struct | Newtype over `String` identifying a project. |
| `CommitId` | struct | Newtype over `String` identifying a commit / snapshot. |
| `IdError` | enum | Parse failures: `InvalidUuid`, `InvalidQualifiedName`. |

#### `ElementId`

Derives `Clone, PartialEq, Eq, PartialOrd, Ord, Hash`. Serde-transparent when the `serde` feature is on.

- `new_v4() -> Self` — fresh random ID (UUID v4 with `uuid`; an atomic counter `elem_{:016x}` without).

- `from_string(impl Into<String>) -> Self` — parses a valid UUID string directly; otherwise derives a deterministic UUID from the bytes (see warning below).

- `as_str(&self) -> String` — the string form (allocates).

- `impl FromStr` — parses a UUID string (`uuid` feature) or accepts any string (without); errors as `IdError::InvalidUuid`.

- `impl Display`

#### `CanonicalKey`

Newtype over the canonical-key string. Derives `Clone, PartialEq, Eq, Hash`. Built recursively as the parser descends containment; pinned by ADR-009.

- `root(project_id: &str) -> Self` — the project-root key; parent of all top-level elements.

- `for_named(parent, kind, name) -> Self` — child with a name. Produces `"{parent}::{name}#{kind}"`.

- `for_anonymous(parent, kind, sibling_index) -> Self` — unnamed child (inline expressions, synthetic literals). Produces `"{parent}/{kind}[{i}]"`.

- `for_relationship(source, kind, target, sibling_index) -> Self` — relationship element keyed on the `(source, kind, target)` triple. Produces `"{source}:{kind}->{target}[{i}]"`.

- `as_str(&self) -> &str` — borrow the underlying key string.

- `to_element_id(&self) -> ElementId` — deterministically derive the ID (routes through `ElementId::from_string`).

- `impl Display`

#### `QualifiedName`

Derives `Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default`. Serde derives the segment vector (not transparent).

- `empty()`, `from_single(name)`, `from_segments(Vec<String>)` — constructors.

- `segments() -> &[String]`, `len()`, `is_empty()`

- `simple_name() -> Option<&str>` — the last segment (`None` if empty).

- `parent() -> Option<QualifiedName>` — all but the last segment (`None` if ≤1 segment).

- `child(name) -> QualifiedName` — append one segment.

- `starts_with(&QualifiedName) -> bool` — prefix test.

- `to_escaped_string()` / `parse_escaped(&str)` — round-trip segments containing literal `:` (`\:`) and `\` (`\\`).

- `impl FromStr` — splits on `::`, trims whitespace, rejects empty segments (`IdError::InvalidQualifiedName`).

- `impl Display` — joins segments with `::` (unescaped).

#### `ProjectId · CommitId`

Thin newtypes over `String`. Both derive the full ordering/hash set and are serde-transparent.

- `new(impl Into<String>) -> Self`

- `as_str(&self) -> &str`

- `impl FromStr` — infallible (`Err = Infallible`), `impl Display`.

## Usage

```
use sysml_id::{ElementId, CanonicalKey, QualifiedName, ProjectId, CommitId};

// Random vs. deterministic-from-string element IDs.
let random = ElementId::new_v4();
let a = ElementId::from_string("my-element");
let b = ElementId::from_string("my-element");
assert_eq!(a, b);                         // same input → same ID

// CanonicalKey: stable identity as the parser walks containment.
let project = CanonicalKey::root("my-project");
let pkg     = CanonicalKey::for_named(&project, "Package", "Foo");
let part    = CanonicalKey::for_named(&pkg, "PartUsage", "Bar");
assert_eq!(part.as_str(), "my-project::Foo#Package::Bar#PartUsage");
assert_eq!(part.to_element_id(), part.to_element_id());

// QualifiedName navigation (note: simple_name returns Option<&str>).
let qn: QualifiedName = "Vehicle::Engine::Cylinder".parse().unwrap();
assert_eq!(qn.len(), 3);
assert_eq!(qn.simple_name(), Some("Cylinder"));
assert_eq!(qn.parent().unwrap().to_string(), "Vehicle::Engine");

// Project / commit identifiers.
let project_id = ProjectId::new("my-project");
let commit_id  = CommitId::new("abc123");
assert_eq!(project_id.as_str(), "my-project");
assert_eq!(commit_id.as_str(), "abc123");
```

## Features & dependencies

| Feature | Default | Enables |
|---|---|---|
| `uuid` | yes | Back `ElementId` with `uuid::Uuid` (v4). Without it, `ElementId` is a `String` backed by an atomic counter. |
| `serde` | no | `Serialize`/`Deserialize` for all types (transparent where applicable; pulls `uuid/serde`). |
| `schemars` | no | `JsonSchema` derivation for `ElementId`, `ProjectId`, `CommitId`, `QualifiedName` — used by API-schema generation. |

Runtime deps: `thiserror` (always), `uuid` / `serde` / `schemars` (each optional). Dev-only: `serde_json`, `criterion` (benches), `proptest` (property tests).

## Pitfalls & invariants

>  **Two compile shapes for `ElementId`.** With `uuid` (default) it is a `Uuid`; without it, a counter-backed `String`. Both expose the same API, but identity semantics differ — production builds keep `uuid` on.

> ⚠  **`from_string` derives a deterministic UUID via a bespoke byte-level XOR/add fold** (not UUIDv5/SHA-1), with v4 version/variant bits set. Collision resistance is weaker than a standard namespaced UUID. Because `CanonicalKey::to_element_id` routes every model element's identity through this, it is the chosen hashing path — treat it as a stable contract and do not change the byte layout without re-keying existing data.

> ⚠  **This is a Layer-0 leaf depended on by ~14 crates** (sysml-core, sysml-parser-trait, sysml-parser-incremental, sysml-runtime, sysml-resolve, sysml-store, sysml-service, sysml-api, sysml-ide-db, sysml-lsp-server, sysml-mcp, sysml-query, sysml-manifest, sysml-project). Any API or identity change ripples through the whole workspace.

- `QualifiedName::simple_name()` returns `Option<&str>`, not a bare `&str` — handle the empty case.

- `FromStr` for `QualifiedName` trims whitespace around `::` and rejects empty/trailing segments; `parse_escaped` is the round-trippable form when a segment contains a literal `:`.

- `CanonicalKey` embeds `kind` at every level and keys relationships on `(source, kind, target, sibling_index)` — preserve this when minting synthetic elements so IDs stay distinct.

## Testing

```
cargo test  -p sysml-id                        # unit + proptest (uuid on by default)
cargo test  -p sysml-id --no-default-features  # String-backed ElementId path
cargo test  -p sysml-id --features serde       # serde round-trip tests
cargo bench -p sysml-id                        # criterion benchmarks
```

Property tests cover `ElementId` round-trip and uniqueness, `QualifiedName` round-trip, `starts_with` transitivity, and segment-count invariants. `CanonicalKey` has a dedicated suite asserting determinism and that distinct names / kinds / sibling indices / projects / relationship tuples all yield distinct IDs.

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
