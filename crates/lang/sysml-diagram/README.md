# sysml-diagram

Visualization exporters for SysML v2: turns a `ModelGraph` into Sprotty `SGraph` JSON, typed table / tree / geometry payloads, and PlantUML text.

`Layer 3 · lang` · `visualization` · `crate-type: rlib` · `10 view generators` · `server-rendered · no wasm32`

`sysml-diagram` is the Rust side of the visualization pipeline. It reads a semantic `ModelGraph` (from `sysml-core`) and emits diagram payloads that the TypeScript Sprotty library (`editors/diagram/`) renders. There is **no in-browser diagram engine**: diagrams are rendered here, in Rust, and pushed to clients over LSP / REST / MCP.

> ⚠  **Architecture invariant — rlib only, no wasm32.** This crate is `crate-type = ["rlib"]` with no `cdylib` and zero `wasm_bindgen`. It depends on the full `sysml-runtime` (which pulls in the diffsol DAE solver) for the `FlowConnectionIR` type used by sequence diagrams, so it *cannot* target `wasm32-unknown-unknown`. The old in-browser `DiagramEngine` WASM build is gone — anything you find under a committed `pkg/` is dead artifact, not built from this source.

## Pipeline

Generation is a three-stage internal pipeline. Generators only ever produce the renderer-agnostic `DiagramIR`; a single render layer is the only place Sprotty types are constructed.

```text
input sysml-core::ModelGraph
↓
generate ViewGenerator (ir/generator.rs) 10 view generators
↓
IR DiagramIR (ir/types.rs)
↓
render ir::render() (ir/render.rs)
↓
output DiagramPayload Graph(SGraph) Table(TableModel) Geometry(GeometryModel) Tree(TreeModel)
↓
consumers LSP setModel REST /diagram MCP sysml_diagram editors/diagram (Sprotty + ELK)
```

>  **Output contract: `DiagramPayload`.** Consumers deserialize a tagged union — wire format `{ "kind": "graph" | "table" | "geometry" | "tree", "data": … }`. Graph-shaped views (General, Interconnection, ActionFlow, StateTransition, Sequence, Requirements, Parametric) emit a Sprotty `SGraph`; Grid emits a `TableModel`, Geometry a `GeometryModel`, Browser a `TreeModel`. The non-graph families route through dedicated typed builders (`tmodel` / `gmodel` / `tree`) and never round-trip through `SGraph`.

## View generators (10)

Every `ViewType` variant maps to exactly one `ViewGenerator`; dispatch in `ir/generator.rs` is exhaustive, so adding a variant without a generator is a compile error.

| ViewType | Generator file | Payload | ELK layout | Notes |
|---|---|---|---|---|
| General | ir/generators/general.rs | Graph | layered / DOWN | BDD, packages, sub-diagram islands, edge rerouting |
| Interconnection | ir/generators/interconnection.rs | Graph | layered | IBD context frame, proxy ports, port-to-port routing |
| StateTransition | ir/generators/state.rs | Graph | layered / DOWN | Recursive nesting, MAX_STATE_DEPTH=20, hidden cardinal ports |
| ActionFlow | ir/generators/action.rs | Graph | layered / DOWN | Control flow, fork / join / decision / merge nodes |
| Sequence | ir/generators/sequence.rs | Graph | fixed | Lifelines, precomputed routes, message ordering |
| Requirements | ir/generators/requirements.rs | Graph | layered / DOWN | Legacy peer kind — flat requirement list + satisfy / verify edges |
| Parametric | ir/generators/parametric.rs | Graph | layered | Legacy peer kind — constraint blocks, parameters, binding connectors |
| Grid | ir/generators/grid.rs | Table | fixed | to_payload routes to TableModel (traceability matrix) |
| Browser | ir/generators/browser.rs | Tree | layered / DOWN | to_payload routes to TreeModel (containment tree) |
| Geometry | ir/generators/geometry.rs | Geometry | fixed | to_payload routes to GeometryModel (spatial primitives) |

>  **Two legacy peer kinds.** `Requirements` and `Parametric` are not standard spec `ViewDefinition` kinds — the spec models them as `General` / `Interconnection` with a `viewCondition` filter. They are retained as standalone generators for visual fidelity (solver value badges, satisfaction icons, binding-participant discovery) until that intelligence ports onto the filtered path.

## Public API

#### `to_payload(graph, view, expanded_ids) → DiagramPayload`

Canonical entry point. Returns the tagged `DiagramPayload` for the view, routing Grid / Geometry / Browser to their typed builders and everything else to `Graph(SGraph)`. `to_payload_with(graph, &ViewRequest)` is the `ViewRequest`-aware form; `to_payload_with_filter_cache(…)` adds a reusable filter cache.

#### `to_smodel(graph, view, expanded_ids) → SGraph`

Generate the Sprotty `SGraph` for a view. `expanded_ids` selects which nodes show children as nested boxes vs collapsed compartment lines. Thin wrapper over `to_smodel_with(graph, &ViewRequest)`.

#### `to_smodel_with(graph, &ViewRequest) → SGraph`

Honors a structured `ViewRequest` — its `filter` / `hints` flow into the `GeneratorContext`, and `request.overlays` run after generation to add visual fidelity. `to_smodel_with_filter_cache(…)` is the same with a shared cache.

#### `to_smodel_subtree(…) → SGraph children`

Generates SModel children for embedding in a composed view — returns the children of the generated `SGraph` without the root wrapper.

#### `to_framed_smodel(graph, view, expanded_ids, view_name) → SGraph`

Wraps the generated graph in a spec-style view frame (e.g. `gv`, `iv`, `stv`) with an optional view name.

#### `to_smodel_json(graph, view) → String`

Convenience: `to_smodel` with no expanded nodes, serialized as pretty JSON.

#### `generate_action_named(graph, action_name) → SGraph`

ActionFlowView for a single named `ActionDefinition` (used by the LSP for code-lens / command targeting).

#### `generate_sequence_from_flows(flows: &[FlowConnectionIR], graph) → SGraph`

SequenceView from pre-compiled `sysml_runtime::flows::FlowConnectionIR` (e.g. captured during action simulation).

#### `to_plantuml / to_plantuml_state_view / to_plantuml_sequence`

PlantUML text export (secondary output format). `SequenceEvent` is the event type for the sequence variant.

## Modules & re-exports

| Module | Vis | Responsibility & key types |
|---|---|---|
| smodel | pub | Sprotty model + dispatch. `ViewType`, `SGraph`, all `to_*` entry points. |
| ir | pub | Internal pipeline: `generator.rs` (ViewGenerator + dispatch), `types.rs` (DiagramIR), `render.rs` (IR→SGraph), `generators/*`. |
| payload | pub | `DiagramPayload` tagged union (graph / table / geometry / tree) — the consumer wire contract. |
| tmodel | pub | `TableModel`, `TableRow`, `TableColumn`, `TableColumnKind`, `TableCell` — Grid payload. |
| tree | pub | `TreeModel`, `TreeNode` — Browser payload. |
| gmodel | pub | `GeometryModel`, `GeometryPrimitive`, `Viewport` — Geometry payload. |
| view_request | pub | `ViewRequest` (view + filter + hints + overlays), `DiagramRequestKey`. |
| visual_kind | pub | `VisualKind` (38 variants), `CompartmentKind` (69), `Shape`, `EdgeStyle`, `ArrowHead`, `LineStyle`, `GraphicalKind` alias. |
| plantuml / action_plantuml | priv (re-exported fns) | PlantUML text export — secondary format. Re-exports `to_plantuml*`, `SequenceEvent`. |

## Usage

Generate JSON for a view, then a tagged payload, then a sequence diagram from compiled flows. This compiles against the current API.

```
use std::collections::HashSet;
use sysml_core::ModelGraph;
use sysml_diagram::smodel::{to_payload, to_smodel_json, ViewType};
use sysml_diagram::DiagramPayload;

let graph = ModelGraph::new();

// 1. Simplest: pretty SGraph JSON for a view.
let json: String = to_smodel_json(&graph, ViewType::General);

// 2. Tagged payload — Grid yields Table, Browser yields Tree, etc.
let payload: DiagramPayload = to_payload(&graph, ViewType::Grid, &HashSet::new());
assert_eq!(payload.kind(), "table");

// 3. Sequence diagram from pre-compiled flows.
use sysml_runtime::flows::FlowConnectionIR;
let flows: Vec<FlowConnectionIR> = Vec::new();
let sgraph = sysml_diagram::smodel::generate_sequence_from_flows(&flows, Some(&graph));
let _ = sgraph;
```

## Dependencies

**Upstream (runtime).**

- `sysml-core` (feature `serde`) — `ModelGraph`, `Element`, `ElementKind` (182 variants), `Relationship`

- `sysml-runtime` — `flows::FlowConnectionIR` for sequence diagrams. **Pulls in diffsol → blocks wasm32.**

- `serde` + `serde_json` — payload serialization

- `tracing` — diagnostics

**Build & dev.**

- `toml` (build-dep) — `build.rs` reads `pipeline_coverage.toml`

- `sysml-parser-incremental` (feature `semantic`), `sysml-parser-trait`, `sysml-span` — dev-only, parse `.sysml` fixtures in tests

**Downstream consumers.**

- `sysml-lsp-server` — `sysml/diagram/setModel` push

- `sysml-api` (REST), `sysml-mcp` — serve `DiagramPayload`

- `sysml-cli` — `export smodel --view <kind>`

## Invariants & pitfalls

**IR decoupling.**

Generators produce `DiagramIR` only — never import `SNode`/`SEdge` in generator code. `ir/render.rs` is the single place Sprotty structures are built. New SModel field? Add it to `DiagramIR` and handle it in render.

**VisualKind exhaustiveness.**

The 38-variant `VisualKind` maps from the generated `ElementKind` (182 variants). Every `match` is exhaustive — adding a variant forces `css_class()`, `node_type()`, `shape()`, `compartment_kind()` and predicates to update. Intentional compile-time coverage enforcement.

**build.rs coverage gate.**

`build.rs` validates `src/pipeline_coverage.toml` against the `VisualKind` / `CompartmentKind` variant sets at compile time and prints coverage stats + crash-risk warnings. A missing entry surfaces as a `cargo:warning`.

**ELK options are advisory in JSON.**

ELK reads layout from `ILayoutConfigurator.apply()` on the TS side; the `layout_options` field on `SNode` exists only for non-ELK consumers. Edge routing runs in the browser (Sprotty + ELK), not here.

## Testing

```
# All diagram tests (13 per-view integration files under tests/)
cargo test -p sysml-diagram

# A single view's tests
cargo test -p sysml-diagram --test smodel_general

# Generate JSON for every view (examples/all_views.rs)
cargo run -p sysml-diagram --example all_views
```

Part of the [sysml-rs](../../../README.md) workspace · regenerated 2026-06-03
