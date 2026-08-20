/**
 * Node color resolution for the SvgCanvas spike.
 *
 * The `VisualKind → palette-category` mapping is **emitted from Rust** (Bucket 2
 * F3: `DesignTokens::category_key`, serialized as `tokens.categories`), so the
 * frontend no longer re-implements `DesignTokens::category_colors`. We just look
 * up `categories[visual_kind]` and index the palette.
 */

import type { CategoryColors, Palette } from './viewmodel-types';

/** Resolve the {fill, stroke, header} colors for a node's `visual_kind`, using
 *  the Rust-emitted category map. Falls back to the generic node colors. */
export function colorsForVisualKind(
  palette: Palette,
  categories: Record<string, string>,
  visualKind: string,
): CategoryColors {
  const key = categories[visualKind] ?? 'node_fallback';
  return (palette[key] as CategoryColors | undefined) ?? palette.node_fallback;
}

/**
 * Wrap every colour in the Rust-emitted palette in a CSS custom-property
 * indirection so the canvas can follow the app theme.
 *
 * ## Why this lives here and not in Rust
 *
 * `DesignTokens::canonical()` is deliberately a *single* dark table with no
 * theme parameter: the palette is serialized into `ViewModel::tokens` and the
 * ViewModel is salsa-cached, so a theme parameter would leak into that cache
 * key and make a theme switch a re-elaboration. That doc comment names the
 * intended escape hatch — "a future light canvas belongs in a frontend
 * CSS-variable indirection layer, not in this Rust table". This is that layer.
 *
 * ## How it works
 *
 * Each colour becomes `var(--canvas-<path>, <emitted value>)`, e.g.
 * `block.fill` → `var(--canvas-block-fill, oklch(29% 0.02 250))`. `var()`
 * resolves inside SVG *presentation attributes* (verified in Chrome), which is
 * how the existing `fill={...}` / `stroke={...}` call sites keep working
 * untouched.
 *
 * The emitted value is the fallback, so this is **strictly additive**: with no
 * `--canvas-*` properties defined the canvas renders exactly the dark palette
 * it renders today. `tokens.css` defines the light ramp under
 * `:root[data-theme='light']` only, so the dark ground cannot regress, and a
 * theme flip is pure CSS — no re-render, no re-elaboration, cache key untouched.
 */
export function themeableCanvasPalette<T>(palette: T, prefix = '--canvas'): T {
  const walk = (node: unknown, path: string): unknown => {
    if (typeof node === 'string') {
      return `var(${path}, ${node})`;
    }
    if (node === null || typeof node !== 'object' || Array.isArray(node)) {
      // numbers (e.g. sim.inactive_opacity), booleans, null — pass through
      return node;
    }
    const out: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
      // `grid_minor` → `grid-minor`; `in_` → `in`
      const slug = key.replace(/_+$/, '').replace(/_/g, '-');
      out[key] = walk(value, `${path}-${slug}`);
    }
    return out;
  };
  return walk(palette, prefix) as T;
}
