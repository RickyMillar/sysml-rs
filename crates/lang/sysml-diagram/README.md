# sysml-diagram

Renderer-neutral visualization support for SysML v2. The crate turns a semantic
`ModelGraph` into a serializable `ViewModel`, typed table/tree/geometry data for
non-graph views, and PlantUML text exports.

## Contract

`ViewModel` is the sole diagram wire artifact. It contains:

- `DiagramIR` scene structure and geometry;
- design tokens, interaction descriptors, text-map, and optional frame data;
- `non_graph` typed data for Grid, Browser, and Geometry views.

The simulation app renders graph scenes with React-SVG and dispatches non-graph
models to its table, tree, and geometry components.

```rust
use sysml_core::ModelGraph;
use sysml_diagram::{to_view_model, ViewRequest, ViewType};

let graph = ModelGraph::new();
let request = ViewRequest::new(ViewType::General);
let view_model = to_view_model(&graph, &request);
```

## Public modules

| Module | Purpose |
|---|---|
| `view_model` | `ViewModel`, frames, and pure builders. |
| `view_request` / `view_type` | Standard view-family dispatch and scoped render requests. |
| `ir` | Renderer-neutral scene types, generators, filters, and overlays. |
| `non_graph` | Typed `Table`, `Tree`, and `Geometry` view data. |
| `design_tokens`, `interaction`, `text_map` | Renderer sidecars joined by element id. |
| `tmodel`, `tree`, `gmodel` | Non-graph data builders. |
| `plantuml`, `action_plantuml` | Textual export helpers. |

## Consumers

- `sysml-service` exposes cached ViewModels to the API, MCP, and LSP.
- `sysml-ide-db` caches ViewModels and their sidecars through Salsa.
- `editors/simulation-app` renders ViewModels with React-SVG.
- `sysml-cli export viewmodel` writes a declared view with pruned sidecars.

## Development

Generators produce `DiagramIR`; renderer concerns belong in frontend adapters.
Keep volatile product contracts covered by focused ViewModel and frontend tests.

```bash
cargo test -p sysml-diagram --lib
```
