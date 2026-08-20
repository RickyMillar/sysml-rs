/**
 * Level-of-detail bands for the canvas (ninebar Phase 2, Wave 4).
 *
 * The payload already carries everything the renderer needs; LOD is a
 * frontend-only decision about how MUCH of each node to draw — never a
 * re-fetch (design brief §4). Effective LOD is the **stricter** of the
 * node-count band and the zoom band, so a dense scene collapses detail even
 * when zoomed in, and a sparse scene collapses when zoomed out.
 *
 * Bands (brief §4):
 *   | Band    | Nodes    | Zoom     | Node renders                              |
 *   | full    | ≤ 50     | > 80%    | everything (header, name, compartments,   |
 *   |         |          |          |   value strip, badges)                    |
 *   | reduced | 50–150   | 40–80%   | drop value strip + meta; badge → dot      |
 *   | glyph   | 150–250  | < 40%    | shape + family hue + selection + one dot  |
 *
 * `> 250` nodes is the truncate band, handled upstream by `truncateScene`
 * (the render cap), so this function only decides full/reduced/glyph.
 */

export type Lod = 'full' | 'reduced' | 'glyph';

/** Ordered most- to least-detailed, so `strictest` can pick the min. */
const RANK: Record<Lod, number> = { full: 2, reduced: 1, glyph: 0 };

/** Node-count band (brief §4). `> 250` is truncated upstream, so counts above
 *  the reduced ceiling fall to `glyph`. */
export function countBand(nodeCount: number): Lod {
  if (nodeCount <= 50) return 'full';
  if (nodeCount <= 150) return 'reduced';
  return 'glyph';
}

/** Zoom band (brief §4). `zoom` is the d3 scale factor (1 = 100%). */
export function zoomBand(zoom: number): Lod {
  if (zoom > 0.8) return 'full';
  if (zoom >= 0.4) return 'reduced';
  return 'glyph';
}

/** Effective LOD = the stricter (least-detailed) of the two bands. */
export function effectiveLod(nodeCount: number, zoom: number): Lod {
  const c = countBand(nodeCount);
  const z = zoomBand(zoom);
  return RANK[c] <= RANK[z] ? c : z;
}

/** Device-pixel stroke clamp (brief §4: "strokes in device pixels, clamped at
 *  1px minimum at every zoom"). A stroke authored as `w` diagram-units renders
 *  at `w * zoom` device px; this returns the diagram-unit width to author so the
 *  on-screen stroke never drops below `minDevicePx`. */
export function clampStroke(widthUnits: number, zoom: number, minDevicePx = 1): number {
  if (zoom <= 0) return widthUnits;
  return Math.max(widthUnits, minDevicePx / zoom);
}

/** Human-readable LOD label for the canvas info pill (crib §3:
 *  `{zoom%} · {N} nodes · {LOD}`). */
export function lodLabel(lod: Lod): string {
  return lod === 'full' ? 'full LOD' : lod === 'reduced' ? 'reduced LOD' : 'glyph LOD';
}
