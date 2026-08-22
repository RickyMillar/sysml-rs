# Embed font subset

`MaterialSymbolsOutlined-embed.woff2` is a subset of the vendored
`public/fonts/MaterialSymbolsOutlined.woff2` (Material Symbols Outlined
variable font, Apache-2.0 — license text and attribution in
`public/fonts/`). The full font is ~4 MB; the embed needs two icon ligatures
(BrowserView's disclosure chevrons), so this cut is ~30 KB:

1. `pyftsubset` keeps the ligatures reachable from the two icon names. The
   icon substitutions live in the font's **`rlig`/`rclt`** features (NOT
   `liga` — subsetting with only `liga` yields a font that renders icon names
   as literal text), and they are contextual lookups whose closure retains
   every icon spellable from the names' letters (~550 glyphs).
2. `varLib.instancer` then pins the four variation axes to the defaults the
   embed renders at (`FILL=0 GRAD=0 opsz=24 wght=400`), dropping the variable
   outline data — the bulk of the weight.

Regenerate after adding any icon to a component the embed bundles — a missing
ligature renders as its literal name text:

```sh
pyftsubset public/fonts/MaterialSymbolsOutlined.woff2 \
  --output-file=src/embed/fonts/MaterialSymbolsOutlined-embed.woff2 \
  --flavor=woff2 --layout-features='rlig,rclt,liga,calt' \
  --text='expand_morechevron_right'   # concatenate new icon names here

python3 - <<'EOF'
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
f = TTFont('src/embed/fonts/MaterialSymbolsOutlined-embed.woff2')
instantiateVariableFont(f, {'FILL': 0, 'GRAD': 0, 'opsz': 24, 'wght': 400}, inplace=True)
f.flavor = 'woff2'
f.save('src/embed/fonts/MaterialSymbolsOutlined-embed.woff2')
EOF
```
