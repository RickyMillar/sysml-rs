# sysml-core

The semantic model database for sysml-rs: `Element`, `Relationship`, and `ModelGraph`, plus name resolution, elaboration, validation, query, and canonical JSON. Every upper layer depends on it.

`Layer 1 · lang` · `semantic model` · `name resolution` · `crate-type: rlib` · `182 ElementKind` · `build-time codegen`

## What this crate owns

`sysml-core` is the universal in-memory model database. It holds the three types every other crate is built on — `Element`, `Relationship`, and the `ModelGraph` that indexes them — and the algorithms that turn a freshly parsed syntax tree into a fully resolved, elaborated, validated model.

**The invariant it enforces.**

One canonical, in-memory representation of a SysML v2 model. Identity is stable (`ElementId` from `CanonicalKey`), ownership is membership-based, and every name in the model resolves through a single deterministic precedence order.

**The pipeline it implements.**

Parse output → **elaborate** (additive, idempotent) → **resolve** (two-pass name resolution) → **validate** (structural + semantic) → **query / serialize**. The graph is mutated in place by each stage.

## Where it sits

```text
consumers sysml-runtime sysml-resolve sysml-query sysml-ide-db sysml-diagram sysml-service
▲ depend on the model types
Layer 1 sysml-core Element · Relationship · ModelGraph
▼ built on
Layer 0 sysml-id sysml-span sysml-codegen (build-dep)
```

>  **Parser note.** The sole parser is tree-sitter (`sysml-parser-incremental`), which implements `sysml_parser_trait::Parser` and returns a `ModelGraph`-backed `ParseResult`. The old Pest PEG parser (`sysml-parser-batch`) has been deleted. `sysml-core` consumes only the resulting graph — it does not parse text itself (the parser crates appear only as *dev-dependencies* for tests).

## Core types

#### `struct Element` — *element.rs*

A single model element. Fields: `id: ElementId`, `kind: ElementKind`, `name: Option<String>`, `owning_membership: Option<ElementId>`, `owner: Option<ElementId>` (derived shortcut), `qname: Option<QualifiedName>`, `props: BTreeMap<Cow<'static, str>, Value>`, `spans: Vec<Span>`, `name_span: Option<Span>`.

Construct with `Element::new(id, kind)`, `Element::new_with_kind(kind)` (fresh UUID, synthetic), or `Element::new_with_key(..)` (reparse-stable identity from a `CanonicalKey`, ADR-009).

#### `enum ElementKind` — *generated · 182 variants*

Generated at build time from the official SysML v2 spec (`element_kind.generated.rs`), one variant per concrete element type — e.g. `PartUsage`, `ActionDefinition`, `StateUsage`. Carries spec-derived hierarchy and constraint helpers (subtype tests, usage/definition predicates, allowed-relationship checks). Never hand-edit; change the spec TTL/XMI inputs or the codegen instead.

#### `struct Relationship + enum RelationshipKind` — *relationship.rs · 37 kinds*

A directed, typed link between elements. `RelationshipKind` covers the SysML v2 relationship vocabulary — `Owning`, `Specialize`, `Redefine`, `Subsetting`, `TypeOf` (FeatureTyping), `Satisfy`, `Verify`, `Derive`, `Trace`, `Reference`, `Flow`, `Transition`, and more. `ModelGraph` keeps a per-kind reverse index so `relationships_by_kind` is O(1).

#### `struct ModelGraph` — *graph.rs*

The model database. Stores `elements: FxHashMap<ElementId, Element>` and `relationships: FxHashMap<ElementId, Relationship>` plus a battery of reverse indexes (owner→children, source/target→rels, kind index, name index, relationship-kind index, membership indexes, library name index). Indexes are *not* serialized and are rebuilt by `rebuild_indexes()` after deserialization.

Key methods: `add_element`, `add_relationship`, `get_element`, `get_element_mut`, `elements_by_kind` / `element_ids_by_kind` (O(1)), `rebuild_indexes`. Namespace-aware lookup (`owned_members`, `resolve_name_in`, `resolve_qualified`) lives in the resolution/namespace modules.

## Public modules

| Module | Responsibility | Key types / functions |
|---|---|---|
| `element` | The `Element` struct + constructors | `Element` (re-exported at crate root) |
| `graph` | The `ModelGraph` database + reverse indexes | `ModelGraph` (re-exported at crate root) |
| `relationship` | Typed links between elements | `Relationship`, `RelationshipKind` (re-exported) |
| `resolution` | Two-pass name resolution: scope tables, import/inheritance/library lookup | `resolve_references`, `resolve_with_library`, `resolve_qualified` |
| `elaborate` | Additive, idempotent derivation of implicit relationships/properties | `elaborate`, `elaborate_with_library`, `ElaborationReport` |
| `semantic_checks` | Hand-written semantic check functions (dispatched by generated code) | actions, cardinality, connectors, distinguishability, ownership, requirements, specialization, states, typing, variation |
| `structural_validation` | Structural well-formedness checks | `StructuralError` |
| `query` | Higher-level queries over the graph | find-by-name, trace matrix, requirement queries |
| `metadata` | ToolExecution / ToolVariable metadata queries | metadata accessor functions |
| `membership` | SysML v2 membership-based ownership views | `MembershipView`, `OwningMembershipView`, `MembershipBuilder` |
| `factory` | Constructs elements with type-appropriate defaults | `ElementFactory` (`package`, `part_definition`, `part_usage`, …) |
| `meta` | Property value model (formerly the sysml-meta crate) | `Value`, `Applicability`, `ClauseKind` |
| `json` | Canonical JSON serialization (feature-gated on `serde`) | canonical export/import |
| `physics` | Static physics classification of attributes/ports (ISQ dimensions, conservation laws, bond-graph roles) | `classify_port_definition`, `classify_part_attributes`, `PhysicsDomain`, `VariableRole`, `DimensionVector` |
| `occurrence` | Occurrence / individual semantics | occurrence helpers |
| `spatial` | Spatial geometry helpers | geometry helpers |
| `view_filter` | View membership filtering (views-first-class surface) | `ViewFilter`, `FilterCombine` |
| `view_index` | Index of declared views / viewpoints / exposes | `build_view_index`, `views_by_viewpoint`, `ViewSummary` |
| `import_health` | Model-level import diagnostics | `import_health_diagnostics` |
| `element_ordering` | Deterministic element ordering for canonical output | ordering helpers |
| `expression_pretty` | Pretty-printing of expression elements | expression formatters |
| `error_codes` | Diagnostic error-code registry | error-code constants |
| `crossrefs` | Cross-reference registry generated from the Xtext grammar | generated cross-ref tables |

## Build-time codegen

`build.rs` (~35 KB) drives `sysml-codegen` to generate **five** files into `OUT_DIR`, each `include!`-ed back into `lib.rs`. It also enforces spec coverage: the build *fails* if TTL/XMI/JSON type sets diverge or if cross-reference coverage drops below 100%.

| Generated file | Contents |
|---|---|
| `element_kind.generated.rs` | `ElementKind` enum (182 variants) + hierarchy and relationship-constraint methods |
| `enums.generated.rs` | Value enums (`FeatureDirectionKind`, `VisibilityKind`, …) |
| `properties.generated.rs` | Typed property accessors + per-type validation |
| `semantic_validation.generated.rs` | Dispatcher routing to hand-written checks in `semantic_checks/` |
| `crossrefs.generated.rs` | Cross-reference registry from the Xtext grammar (`pub mod crossrefs`) |

> ⚠  **Adding a cross-reference property?** Update `IMPLEMENTED_CROSSREFS` or `SKIPPED_CROSSREFS` in `build.rs` — the 100% coverage gate will otherwise fail the build with an opaque codegen error.

## Resolution & elaboration

**Name resolution precedence.**

Names resolve in a fixed order:

- Owned

- Inherited

- Imported

- Parent (enclosing namespace)

- Global

- Library

It runs in two passes: **Pass 1** resolves types (Specialization, FeatureTyping); **Pass 2** resolves features (Subsetting, Redefinition).

**Elaboration.**

The elaboration pass is **additive and idempotent** — it only adds derived relationships/properties, never removes or overwrites. One file per domain: `actions`, `states`, `flows`, `connectors`, `constraints`, `ports`, `requirements`, `successions`, `imports`, `implicit_generalization`.

## Usage

```
use sysml_core::{ModelGraph, Element, ElementKind, ElementFactory};
use sysml_core::resolution::resolve_references;
use sysml_core::elaborate::elaborate;

let mut graph = ModelGraph::new();

// Build a tiny model via the factory (type-appropriate defaults).
let pkg = graph.add_element(ElementFactory::package("Vehicles"));
let mut car = ElementFactory::part_definition("Car");
car.owner = Some(pkg);
let car_id = graph.add_element(car);

// Derive implicit structure, then resolve names.
elaborate(&mut graph);
let report = resolve_references(&mut graph);

// Query by kind via the O(1) index.
for part in graph.elements_by_kind(&ElementKind::PartDefinition) {
    println!("{:?}", part.name);
}
let _ = report; // ResolutionResult: diagnostics + counts
```

JSON serialization lives behind the `serde` feature (`sysml_core::json`).

## Features & dependencies

**Cargo features.**

| Feature | Effect |
|---|---|
| `parallel` *(default)* | Enables `rayon` for parallel passes |
| `serde` | Serialize/Deserialize + the `json` module |
| `schemars` | JSON-Schema derivation |
| `resolution-tracing` | `tracing` output for debugging resolution |

**Dependencies.**

- **Runtime:** `sysml-id`, `sysml-span`, `thiserror`, `rustc-hash`; optional `rayon`, `serde`/`serde_json`, `schemars`, `tracing`.

- **Build:** `sysml-codegen`.

- **Dev (tests only):** `sysml-parser-trait`, `sysml-parser-incremental`, `criterion`, `proptest`.

## Pitfalls

> ⚠  **Never bincode anything touching `meta::Value`.** `Value` uses `#[serde(untagged)]`, which bincode cannot round-trip. Use `serde_json` for any serialization involving `Value`, `Element`, or `ModelGraph`.

- **Rebuild indexes after deserialization.** The reverse indexes are `#[serde(skip)]`; call `rebuild_indexes()` on a freshly deserialized graph.

- **Ownership is membership-based.** `Element.owner` is a cached shortcut derived from `owning_membership → OwningMembership → namespace`; do not treat it as authoritative on its own.

- **Don’t hand-edit generated code.** `ElementKind` and the other four generated files come from the spec — edit the TTL/XMI inputs or the codegen.

- **Register new semantic checks.** A new check function in `semantic_checks/` must also be registered in `codegen/src/semantic_rules.toml`.

Part of the [sysml-rs](../../../README.md) workspace · agent guidance in `CLAUDE.md` · regenerated 2026-06-03
