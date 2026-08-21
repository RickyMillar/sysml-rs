# Visualization Pipeline Architecture

> Last updated: 2026-08-13, reconciled against the source tree on that date.
>
> This guide previously described a Sprotty + WASM-edge-router stack that no
> longer exists. That description is archived at
> read them for history only.
>
> **Design authority for the frontend half** is
> this guide and those disagree, they win.

## The pipeline at a glance

```
Rust — sysml-diagram (pure), cached in sysml-ide-db (salsa)     Frontend — editors/simulation-app
─────────────────────────────────────────────────────────────   ─────────────────────────────────
ModelGraph ──► ViewGenerator ──► DiagramIR ──► ViewModel ──JSON──► React SVG renderer ──► SVG
(sysml-core)     (8, per view)     (scene)     + design tokens      elkjs (layout + orthogonal
                                               + text map          edge routing) in a Web Worker
                                               + interactions      d3-zoom for pan/zoom
                                               + view frame
                                               + non-graph model
```

Rust owns semantics and scene structure. The frontend owns geometry (it runs the
layout), pixels, and event plumbing. Only the generator stage is view-specific on
the Rust side; everything downstream of `DiagramIR` is universal.

There is **one generate path**. `view_model::build_scene()`
(`crates/lang/sysml-diagram/src/view_model.rs:244`) runs the generator once, and
both outputs are derived from that single scene:

- **`ViewModel`** — the going-forward wire format, consumed by the app's renderer.
- **`SGraph`** — the legacy Sprotty-shaped JSON, still emitted by
  `smodel::to_smodel_with` for the LSP `sysml/diagram/setModel` notification, the
  CLI `export smodel` command, and the graph branch of `sysml.views.render`. It
  is an *adapter over the same scene*, not a second pipeline. No frontend in this
  repo renders it today — the VS Code extension is LSP-only (its diagram webview
  was removed) and the simulation app consumes the ViewModel.

---

## Stage 1: ModelGraph (input)

The semantic model — elements, relationships, properties — with no visualization
knowledge. Every view starts from the same graph.

**Crate**: `sysml-core` (`crates/lang/sysml-core/`).

---

## Stage 2: View generation (Rust, per view)

A **view generator** walks the `ModelGraph` and produces a `DiagramIR`. This is
the only view-specific stage in the pipeline.

Every generator implements `ViewGenerator`
(`crates/lang/sysml-diagram/src/ir/generator.rs:222`):

```rust
pub trait ViewGenerator: Send + Sync {
    fn view_type(&self) -> ViewType;
    fn elk_algorithm(&self) -> &str;
    fn elk_direction(&self) -> Option<&str> { Some("DOWN") }
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR;
    /// Subtree for embedding in another view; `None` = not embeddable.
    fn generate_for_owner(&self, ctx: &GeneratorContext, owner_id: &str) -> Option<DiagramIR>;
}
```

`ir::get_generator(ViewType)` dispatches exhaustively over the 8-variant
`ViewType` enum, so adding a variant without a generator is a compile error.

> `elk_algorithm()` / `elk_direction()` are **vestigial**: they were the Rust
> half of the old per-view ELK configuration, and outside the generators' own
> unit tests nothing reads them any more. The live renderer picks its own ELK
> options (see Stage 5). Treat them as legacy until they are removed.

`GeneratorContext` carries the graph, the expanded-node set, the spec
`ElementFilterMembership` filter, `Expose` targets, and a precompiled-filter
cache (the cache exists because recompiling a view condition per candidate
element made filtered views over the stdlib-merged graph effectively hang).

| Generator | File (`src/ir/generators/`) | Notable behaviour |
|---|---|---|
| `GeneralViewGenerator` | `general.rs` | BDD/packages, sub-diagram islands, edge rerouting, requirement notation (reqId / subject / assume-require compartments, «satisfy»/«verify»/«derive»/«trace» edge labels) |
| `InterconnectionViewGenerator` | `interconnection.rs` | IBD context frame, proxy ports, port-to-port routing, constraint notation (`{expr}` header, parameter-port squares, binding-connector edges) |
| `StateTransitionViewGenerator` | `state.rs` | Recursive state nesting (`MAX_STATE_DEPTH = 20`), hidden cardinal ports for edge anchoring |
| `ActionFlowViewGenerator` | `action.rs` | Control flow: fork / join / decision / merge nodes |
| `SequenceViewGenerator` | `sequence.rs` | Lifelines, message ordering, precomputed routes |
| `BrowserViewGenerator` | `browser.rs` | Pure ownership tree, no edges |
| `GridViewGenerator` | `grid.rs` | Fixed-position cells |
| `GeometryViewGenerator` | `geometry.rs` | Like General, but positions read from element properties |

Shared container logic (expand/collapse, compartment text, requirement
compartments) lives in `ir/generators/container.rs`.

**Retired view kinds.** `Requirements` and `Parametric` are *not* view kinds —
they were retired in 2026-06 because the spec has no such kinds. A requirement
view is a `General` view plus a filter; constraint/binding notation renders in
`Interconnection`. Do not reintroduce them as peer generators.

### The 8 spec view kinds

Verified against the graphical BNF
(`references/sysmlv2/SysML-v2-Pilot-Implementation/tool-support/bnf_grammar_tools/tests/KerML_and_SysML_grammars/SysML-graphical-bnf-corrected.kgbnf`,
rule `frameless-view`) — the five frameless kinds are exactly the ones the BNF
lists as embeddable without a diagram frame:

| View kind | Short name | Frameless (embeddable)? |
|---|---|---|
| GeneralView | `<gv>` | Yes |
| InterconnectionView | `<iv>` | Yes |
| ActionFlowView | `<afv>` | Yes |
| StateTransitionView | `<stv>` | Yes |
| SequenceView | `<sv>` | Yes |
| GeometryView | `<gev>` | No — framed only |
| GridView | `<grv>` | No — framed only |
| BrowserView | `<bv>` | No — framed only |

The spec does **not** define layout algorithms, sizes, spacing, colours, the
expand/collapse interaction model, or edge routing. Those are implementation
choices and live where this guide says they live.

---

## Stage 3: DiagramIR (universal scene)

`DiagramIR` (`src/ir/types.rs`) is the renderer-agnostic scene: nodes, edges,
compartments, ports, and islands, each tagged with its `ElementId` where one
exists. It carries **typed semantic fields only** — no CSS class strings and no
ELK option strings. Generators never construct renderer types.

Two structural patterns matter when reading generator code:

- **Islands.** An embedded sub-diagram (state, action, sequence, IBD) is a
  `DiagramChild::Island` inside its parent node, carrying its own subtree and
  expansion state, rather than being flattened into the parent's children.
- **Edge rerouting.** When a relationship endpoint sits inside a collapsed
  container, `find_rendered_ancestor()` walks up the ownership chain to the
  nearest visible ancestor; self-loops produced by that rerouting are suppressed.

`VisualKind` (38 variants) and `CompartmentKind` (69 variants) in
`src/visual_kind.rs` map the 182-variant generated `ElementKind` onto visual
categories. All matches on them are exhaustive on purpose — adding a variant
forces every site to be updated. `build.rs` additionally compares
`src/pipeline_coverage.toml` node and edge names against hardcoded lists inside
`build.rs` itself — NOT against the enum variant sets — and only counts
compartment entries without checking their names; it prints coverage warnings
(it warns; it does not fail the build). The tracker has drifted from the enums
and is not authoritative: read the `VisualKind` / `CompartmentKind` enums in
`crates/lang/sysml-diagram/src/visual_kind.rs` and `RelationshipKind` in
`crates/lang/sysml-core/src/relationship.rs` for the real coverage.

---

## Stage 4: ViewModel (the wire artifact)

`ViewModel` (`src/view_model.rs`) composes the scene with the renderer-agnostic
addenda a frontend needs:

| Field | What it carries |
|---|---|
| `scene` | `Arc<DiagramIR>` — the scene structure; no computed geometry (the renderer lays it out) |
| `tokens` | `Arc<DesignTokens>` — the OKLCH palette plus the `VisualKind → palette-category` map, the single Rust source of truth for diagram colour (`src/design_tokens.rs`; the frontend reads `tokens.categories` in `diagram-svg/palette.ts` rather than re-deriving it). Node geometry and typography are deliberately **not** carried here yet |
| `text_map` | `ElementId ↔ Span`, both directions — powers the bidirectional text↔diagram link (`src/text_map.rs`) |
| `interactions` | Semantic affordances per region, joined by `ElementId` (`src/interaction.rs`) |
| `frame` | Framed-view metadata (`«view» Name : kind` plus info compartments); `Some` only for a **declared** `View` |
| `non_graph` | `TableModel` / `TreeModel` / `GeometryModel` for the Grid / Browser / Geometry families, so one command serves every view family |

Cloning is cheap — the scene and the shared addenda are `Arc`s.

**Salsa caching** lives one layer up, in `sysml-ide-db`, keeping `sysml-diagram`
a pure function of `(ModelGraph, ViewRequest)`:

- `view_model.rs` — `file_view_model` / … / `workspace_view_model_best`
- `diagram.rs` — the parallel legacy `SGraph` triplet ending in
  `workspace_diagram_best`
- `text_map.rs`, `interaction.rs` — the cheaper standalone queries whose results
  the ViewModel query attaches

Same elaborated graph + same request key ⇒ same `Arc` back (identity equality),
so an unrelated edit does not re-render.

**Service surface** (`crates/tooling/sysml-service/src/lib.rs`):

| Command | Returns |
|---|---|
| `sysml.diagram.viewmodel` | the `ViewModel` for a declared view |
| `sysml.diagram.sim_overlay` | per-tick simulation overlay for a live session |
| `sysml.diagram.verdict_overlay` | per-run constraint verdicts + solved values |
| `sysml.diagram.diagnostic_overlay` | diagnostics joined to scene elements |
| `sysml.views.render` | the legacy tagged `DiagramPayload` (`graph`/`table`/`geometry`/`tree`) |

The three overlays are **sidecars**: pass the same `view_usage_id` as the
`viewmodel` call and they join the same scene by `ElementId`. Overlay state is
never baked into the scene.

---

## Stage 5: The renderer (React SVG)

`editors/simulation-app/src/diagram-svg/` is the graph renderer.
`components/diagram/DiagramHost.tsx` dispatches on payload shape —
`tableModel` → `TableView`, `geometryModel` → `GeometryView`, `treeModel` →
`BrowserView`, otherwise `SvgCanvas`. Those non-graph payloads come from the
same ViewModel fetch: `features/views/SelectedViewRenderer.tsx` reads
`vm.non_graph` and pushes it into the workspace store.

| File | Responsibility |
|---|---|
| `SvgCanvas.tsx` | The renderer. Fetches `sysml.diagram.viewmodel` + the overlay sidecars, renders SVG, owns selection/hover/drag, pans and zooms with `d3-zoom` |
| `layout.ts` | elkjs layout **and** edge routing (`elk.layered` + `elk.edgeRouting: ORTHOGONAL`) in one pass |
| `shapes.ts`, `palette.ts`, `edges.ts` | Shape geometry per `VisualKind`, colours from the Rust design tokens, edge decoration |
| `label-layout.ts`, `lod.ts` | Label wrapping/elision and level-of-detail |
| `overlay.ts` | Sim / verdict / diagnostic overlay rendering |
| `frame.tsx` | The framed-view chrome from `ViewModel.frame` |
| `manual-layout.ts` | User drag deltas layered over the computed layout |

**elkjs runs in a real Web Worker** in the browser (`new ELK({ workerUrl })`),
with an in-process fallback where `Worker` is undefined (vitest / jsdom / node).
The renderer sets the root options itself (`elk.algorithm: layered`,
`elk.direction: DOWN`, with `rectpacking` for particular containers) — this is
where per-view layout choice actually lives now.

The text↔diagram link is bidirectional through one Zustand selection store:
diagram click → `ElementId` → span via the text map → reveal in the editor;
editor cursor → `ElementId` → highlight the node.

---

## What is gone (so stale references do not mislead)

| Removed | Where its job went |
|---|---|
| Sprotty (`editors/diagram`, `sysml-module.ts`, `configureModelElement` registrations) | React components in `diagram-svg/` |
| `shape-catalog.json` | `diagram-svg/shapes.ts` + Rust `DesignTokens` |
| TS ELK configurator (`elk-config.ts`, `sysml-layout-configurator.ts`) | `diagram-svg/layout.ts` |
| WASM edge router / routing levels | elkjs `ORTHOGONAL` routing in the same pass |
| In-browser `DiagramEngine` WASM build of `sysml-diagram` | Server-rendered; the crate is rlib-only and cannot target `wasm32` (it depends on `sysml-runtime`/diffsol) |

`crates/tooling/sysml-layout` (the Rust orthogonal router, formerly compiled to
WASM for the browser) was **deleted on 2026-08-13** (OS-D2 decision 4). It never
had a consumer outside its own benches and tests; elkjs now does placement and
orthogonal routing in a single pass. Recoverable from git history.

---

## Adding a new view type

1. **Rust**: add the `ViewType` variant in `src/smodel/mod.rs`.
2. **Rust**: create `src/ir/generators/<name>.rs` implementing `ViewGenerator`.
3. **Rust**: add the `get_generator()` match arm in `src/ir/generator.rs` (the
   compiler will demand it).
4. **Rust**: if new `VisualKind` / `CompartmentKind` variants are needed, add
   them and update every match site plus `src/pipeline_coverage.toml`.
5. **Frontend**: add shape/colour handling in `diagram-svg/shapes.ts` /
   `palette.ts` if the view introduces unfamiliar visual kinds; the renderer
   dispatches on `VisualKind`, so most views need nothing here.
6. **LSP** (only if the legacy `SGraph` path must serve it): add the case in
   `parse_view_type()` in `sysml-lsp-server/src/diagram.rs`.

## Testing

Per-view integration tests live in `crates/lang/sysml-diagram/tests/smodel_*.rs`
(they still assert on the `SGraph` serialization). Renderer tests are vitest
suites under `editors/simulation-app/src/diagram-svg/__tests__/`, including
real-elkjs island and layout-timeout tests.

```bash
cargo test -p sysml-diagram
cd editors/simulation-app && npm run test
```
