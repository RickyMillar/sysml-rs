---
title: Views and diagrams
description: Define views in your model, then render or export them as diagrams with the ViewModel pipeline.
scope:
  - SysML v2 / KerML
  - sysml-rs tooling
status: pre-alpha
last_verified_against: fcd1305
source_of_truth:
  - crates/tooling/sysml-cli/src/export.rs
  - editors/simulation-app/src/embed/README.md
  - editors/simulation-app/vite.embed.config.ts
---

You have a model and you want pictures of it: a structure diagram for a
review, a state chart in a document, an interactive diagram in a web page.
In SysML v2 the *what to show* is part of the model itself — you declare
**views** in your source — and sysml-rs turns a declared view into a
rendered diagram. The pipeline is:

```
source model  →  view definitions & views  →  ViewModel JSON  →  a renderer
   (.sysml)         (also .sysml)              (sysml export,      (embed viewer,
                                                API, MCP)           desktop app)
```

## Declaring views in the model

**SysML v2 / KerML** — this part is standard language, not a sysml-rs
convention. A *view definition* is the reusable recipe (what kinds of
elements to include, how to render them); a *view* is a concrete selection
that `expose`s model elements. The standard library ships view definitions
such as `StandardViewDefinitions::GeneralView`, `InterconnectionView`,
`ActionFlowView`, `StateTransitionView`, and `BrowserView`, so most views
just pick one and expose a package or element:

```sysml
package MyViews {
    // Everything in the Definitions package, as a general diagram
    view definitionsOverview : StandardViewDefinitions::GeneralView {
        expose Definitions::*;
    }

    // One state machine, as a state-transition diagram
    view lifecycleStates : StandardViewDefinitions::StateTransitionView {
        expose States::CoffeeMachineLifecycle;
    }
}
```

This example is taken from the model behind the
[SysML v2 Book](https://rickymillar.github.io/sysmlv2-book/)'s embedded
diagrams and exports successfully with the command below. The Book teaches
the view/viewpoint/rendering language itself — start there for `view def`,
`filter`, and `render`.

## Exporting a view: `sysml export viewmodel`

**sysml-rs tooling** — a declared view is exported to **ViewModel JSON**,
the renderer-neutral diagram contract used by every sysml-rs surface
(CLI, REST API, MCP, and the app renderers all speak the same shape):

```bash
sysml export viewmodel \
  --workspace path/to/model-directory \
  --view MyViews::definitionsOverview \
  -o definitions.json
```

- `--workspace` is the model *directory*: declared views render against the
  whole workspace, not a single file.
- `--view` takes the qualified name; a unique bare name also resolves.
- `--expand-all`, or repeated `--expand <element-id>`, controls which
  expandable nodes render expanded.
- Without `-o` the JSON goes to stdout.

The payload carries the scene graph plus everything an interactive renderer
needs — styling tokens, a text map, interaction metadata, the diagram frame,
and a non-graph payload for view kinds that are not node-and-edge diagrams
(browsers, tables). Treat the structure as pre-alpha: it can change without
a deprecation period.

Earlier sysml-rs versions had a different, Sprotty-based diagram payload;
that contract is fully retired, and ViewModel is the only one.

## Rendering: static vs interactive

**Static exports.** For a picture in a document or a quick look at a single
file, export PlantUML text instead — no view declarations needed:

```bash
sysml export plantuml model.sysml --view state   # general | state | action | sequence
sysml export json model.sysml --pretty           # canonical model JSON, not a diagram
```

Both operate on one file and print to stdout; render the PlantUML with any
PlantUML toolchain.

**Interactive rendering.** ViewModel JSON is rendered by the **embeddable
viewer**, a self-contained static build of the diagram renderer (pan, zoom,
node expansion — no backend, no app shell). This is exactly how the
interactive diagrams in the SysML v2 Book work: each one is the viewer in an
iframe pointed at a pre-exported ViewModel file. To use it on your own
site:

```bash
cd editors/simulation-app
npx vite build --config vite.embed.config.ts   # → dist-embed/
```

Copy `dist-embed/` anywhere static, drop your exported `.json` next to it,
and load `embed.html?src=your-view.json`. The build uses only relative
asset URLs and fetches nothing from the network beyond its own files and
the `src` fixture, so documentation built this way never depends on a live
server.

**Experimental / partial support** — the desktop workbench
(`editors/simulation-app`) renders the same ViewModels live against a
running API server, including simulation and verification overlays, but the
workbench and its diagram surfaces are in active rework; treat them as a
preview. The VS Code extension currently ships **no** diagram webview — see
[Editor setup](/sysml-rs/use/editors/).

## The same views over the API and MCP

A running [API server](/sysml-rs/use/integrations/) lists and renders the
declared views of a loaded model (`GET /models/:uri/views`,
`GET /models/:uri/views/:view_id/render`), and MCP agents get the same
operations as tools (`sysml_views_list`, `sysml_views_render`,
`sysml_diagram_viewmodel`). All of them return the same ViewModel contract
as the CLI export.
