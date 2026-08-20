# view-showcase

Test bed for every spec view kind and every `view def` / `view`
feature recognised by the Phase 5 backend.

## Structure

- `Model.sysml` — the underlying domain model. Carries one element of
  every kind a `view def` might want to expose (parts with ports +
  connections, states, actions, requirements, constraints, spatial
  attributes).
- `Views.sysml` — 17 view declarations, each annotated with which
  feature it exercises.

## What each view tests

View kind is declared with `:>` (specialize a standard view definition).
There is no name-suffix inference — view names are free-form.

| # | View | Feature exercised |
|---|---|---|
| 1 | `OverviewView` | Default General view (no `:>`), single Expose |
| 2 | `PowertrainView` | `:> Interconnection` |
| 3 | `DriveModesView` | `:> StateTransition`, Expose a StateDefinition |
| 4 | `StartFlowView` | `:> ActionFlow` |
| 5 | `AllReqsView` | `:> Requirement` + multiple Expose |
| 6 | `CommsView` | `:> Sequence` |
| 7 | `CatalogView` | `:> Browser` + recursive `Foo::*` Expose |
| 8 | `TraceMatrixView` | `:> Grid` (traceability matrix) |
| 9 | `LayoutView` | `:> Geometry` (uses x/y/w/h attributes) |
| 10 | `Parametric` | Legacy peer kind — reached only by own-name (no std-lib def) |
| 11 | `MixedExposeView` | Multiple Expose clauses, mixed element kinds (→ General) |
| 12 | `AllPartsView` | `filter true` — viewCondition pass-through |
| 13 | `NothingView` | `filter false` — viewCondition exclude-all |
| 14 | `TypedAsView` | `:> PowertrainView` → transitive walk to Interconnection |
| 15 | `RenderedView` | `render :>> StandardRendering` |
| 16 | `scratch` (anonymous) | ViewUsage (not ViewDefinition) |
| 17 | `EmptyExposeView` | View with no Expose / no body |

## Loading

```bash
# CLI smoke test
cargo run --release -p sysml-cli -- inspect examples/view-showcase

# Backend (HTTP API)
curl -s "http://localhost:8080/models/__workspace__/views" \
     | python3 -m json.tool

# Render one view by id
curl -s "http://localhost:8080/models/__workspace__/views/<view-id>/render" \
     | python3 -m json.tool
```

## Known gaps the showcase surfaces

- **Filter expressions referencing element fields don't evaluate.**
  Only `filter true;` and `filter false;` reliably affect the diagram
  — predicates like `kind == "PartUsage"` silently fall through to the
  safe-default `true` path because the runtime evaluator can't yet
  project Element fields off `Value::Ref(self)`.
- **Multi-filter views drop all but the last `filter` clause.** The
  composer keeps only `summary.filters.last()` when stuffing into
  `ViewFilter::expression`.
- **(FIXED) View-kind resolution follows the `:>` chain.** Declaring
  `view def Foo :> Interconnection` (or transitively, `:> Bar` where
  `Bar :> Interconnection`) resolves to the right `ViewType`. Resolution
  matches the author-written supertype name, so it works even when the
  std-lib alias isn't merged into the workspace graph.
- **`expose` narrows nodes but not edges.** Rendering a view whose
  exposed subject has any inbound or outbound relationship in the full
  graph still emits every other relationship in the workspace as a
  dangling edge. Top-level nodes count is correct (1 — the subject)
  but the SGraph payload carries 11k+ stale edges. The dangling-
  endpoint pruner runs against `passes_filter` only, not against
  `is_canvas_root`. Sprotty's client-side renderer hides them, but
  payload size is wrong.
