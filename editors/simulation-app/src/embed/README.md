# Standalone diagram embed

A self-contained build of the app's diagram pipeline (SelectedViewRenderer →
EmbedDiagramHost → SvgCanvas / BrowserView / TableView / GeometryView) that
renders a pre-exported ViewModel JSON with no backend and no app shell.
Made to be iframed from a static site (the mdBook).

## Build

```sh
npx vite build --config vite.embed.config.ts   # → dist-embed/
```

`dist-embed/` uses only relative asset URLs — copy the whole directory to any
mount path (e.g. the book's `src/viewer/`) and serve it statically. Fonts
(IBM Plex Sans/Mono and a subset of Material Symbols — see `fonts/README.md`)
are bundled as hashed assets; nothing is fetched from the network except the
page's own files and the `?src=` fixture.

## URL parameters

| Param | Meaning |
| --- | --- |
| `?src=<url>` | Fetch the ViewModel JSON from `<url>`, resolved relative to `embed.html`. **The primary path for hosts**: export a ViewModel with `sysml-cli`, drop the `.json` next to the viewer, point `src` at it. |
| `?fixture=<name>` | Render one of the demo fixtures bundled from `src/embed/fixtures/` (basename, no extension — e.g. `coffee-machine-structuralOverview`). Unknown names fail with the list of known ones. |
| *(neither)* | The first bundled fixture, alphabetically. |
| `?theme=light` | Sets `data-theme="light"` on `<html>`; the light-canvas ramp in `tokens.css` takes over. Default is the dark canvas. |

Example iframe:

```html
<iframe src="viewer/embed.html?src=fixtures/pump-structure.json&theme=light"
        style="width:100%; height:480px; border:none"></iframe>
```

## Interactions

Pan/zoom (wheel + drag on empty canvas), node drag, hover, and expand/collapse
are fully live. Element click and double-click (go-to-definition) only move the
**visual** selection highlight — there is no inspector or editor in the embed,
and the fixture transport answers the selection store's detail fetches with an
empty stub, so clicks never error or fetch off-page.

## CSS hooks

Every canvas colour the Rust ViewModel emits is wrapped as
`var(--canvas-<slot>, <emitted>)` (see `src/diagram-svg/palette.ts`), so a
same-origin host page can restyle the canvas by defining `--canvas-*` custom
properties on the iframe's document element:

```js
const doc = iframe.contentDocument;
doc.documentElement.style.setProperty('--canvas-bg', 'oklch(0.98 0 0)');
doc.documentElement.style.setProperty('--canvas-grid-minor', 'transparent');
```

(or by injecting a `<style>` into `iframe.contentDocument.head`). The slot
names are the kebab-cased palette paths — `--canvas-bg`, `--canvas-text`,
`--canvas-grid-minor`, `--canvas-block-fill`, `--canvas-block-stroke`, … — the
full set is visible in `src/styles/tokens.css` under
`:root[data-theme='light']`. Undefined slots fall back to the built-in palette
for the active theme, so overrides are strictly additive.

## Fixture format

The JSON is the `sysml.diagram.viewmodel` command's response payload (the
renderer-agnostic ViewModel): a graph ViewModel renders on SvgCanvas; a
payload whose `non_graph` field carries a table / tree / geometry model
renders on the matching dedicated view.
