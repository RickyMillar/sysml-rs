# Visualisation pipeline architecture

## Scope

`sysml-diagram` produces a renderer-neutral `ViewModel` from a resolved
`ModelGraph`. The simulation app renders its graph scene with React-SVG.
Grid, Browser, and Geometry view families carry typed `non_graph` data in the
same ViewModel and use dedicated table, tree, and geometry components.

## Pipeline

```text
ModelGraph → ViewRequest → DiagramIR → ViewModel → React-SVG / typed non-graph renderer
```

- **`ViewRequest`** identifies one of the standard view families, expansion,
  filter, expose, frame, and overlay inputs.
- **`DiagramIR`** expresses semantic nodes, ports, edges, compartments, and
  layout intent without browser-implementation details.
- **`ViewModel`** joins the scene to design tokens, text-map, interaction map,
  optional frame, and optional non-graph model.
- **Sidecars** (`SimOverlay`, `VerdictOverlay`, `DiagnosticOverlay`) join a
  ViewModel by element id rather than mutating the cached scene.

## Product transport

`sysml-service` resolves and caches ViewModels through `sysml-ide-db` Salsa
queries. Declared views use `sysml.views.render` or
`sysml.diagram.viewmodel`; ad-hoc diagram commands also return ViewModels. The
API, MCP, and LSP carry this contract directly. The LSP notification method is
`sysml/diagram/setViewModel`.

The CLI supports `sysml export viewmodel --workspace <dir> --view <name>`;
export prunes text-map and interaction sidecars to elements referenced by the
view.

## Extending a view family

1. Add or reuse a `ViewType` in `src/view_type.rs`.
2. Implement the `ViewGenerator` in `src/ir/generators/` and register it in
   `get_generator`.
3. Keep view-specific semantics in typed IR fields and tags.
4. Add focused DiagramIR/ViewModel tests and, where appropriate, React-SVG
   rendering coverage.
5. Update product-facing documentation from the generated or contract-tested
   inventory rather than copying command lists.

## Invariants

- ViewModel is the only diagram wire contract.
- Renderers do not receive an unresolved `ModelGraph` as an alternative scene
  format.
- Model-language semantics remain distinct from sysml-rs product behavior.
- Browser layout and styling are frontend concerns; Rust describes the scene
  and semantic tokens.
