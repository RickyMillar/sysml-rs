/**
 * label-layout — deterministic FE edge-label de-confliction
 * (diagram-layout-quality brief §1).
 *
 * Greedy slot search over an occupancy index seeded with every obstacle rect
 * (node bodies, container header bands, port glyphs, port labels, frame
 * heading/corner boxes — §1 step 1); labels are then placed in a deterministic
 * order (center labels by ascending edge path length — shortest edge has the
 * fewest candidate slots) and stamped into the index as they land.
 *
 * Implementation note: the brief sketches an 8px-cell grid; this uses EXACT
 * rect intersection instead — cell quantization inflates every chip by up to a
 * cell in each direction, which falsely rejects the brief's own ±12px
 * perpendicular slots for adjacent parallel edges. The candidate scheme,
 * ordering, and degraded-fallback contract are exactly the brief's; only the
 * occupancy test is exact. O(labels × candidates × rects) is well within
 * budget under the 250-node/600-edge render cap.
 *
 * A center label slides ALONG its routed polyline (path parameter
 * t ∈ {0.5, 0.5±0.1, 0.5±0.2, 0.5±0.3}) with a perpendicular offset
 * ∈ {0, +12, −12}px — preferring on-line (inline chip) first; it never detaches
 * from its path. When no clean slot exists the minimum-overlap candidate wins
 * and the placement is tagged `degraded: true` — the geometry dump counts
 * these, so an unfixable overlap is still measurable (§1 step 4).
 *
 * Secondary labels (trigger-source annotations — the IR's only sub-label kind
 * today; true end adornments like rolenames are a future composer emission)
 * stack below their center label when the stacked rect is clear, otherwise
 * they run their own slot search (§1 step 5).
 *
 * Long labels wrap at ~20ch into a multi-line chip; the layout always reserves
 * the FULL chip size so LOD-band elision (≤12ch + …) never reflows (§1
 * "Measurement"). This module is pure geometry — trivially unit-testable and
 * shared by the elk and fixed-layout paths.
 */

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Pt {
  x: number;
  y: number;
}

/** Chip metrics — must mirror the `EdgeLabel` renderer (10px font). */
export const LABEL_CHAR_W = 6.4;
export const LABEL_LINE_H = 14;
export const LABEL_PAD_X = 4;
/** Wrap column for long labels (guard expressions), in characters.
 *
 *  Tightened 28 -> 20: with `edgeLabels.inline` the chip becomes a dummy node
 *  the edge must route THROUGH, so a wide one-line chip drags its edge sideways
 *  to reach it. On runtimeStateMachines the `when B >= 0.999 * Bs` transition
 *  detoured far right and back to meet its label — a 4.29x route that the G8
 *  gate flagged. Wrapping the same text onto two ~20ch lines halves the chip
 *  width and the detour disappears (G8 clean, and the G1/G2 label-overlap gates
 *  stay green on every tile, so the C9b pile-up fix is preserved). */
export const WRAP_CH = 20;
/** Elision budget at reduced/glyph LOD, in characters. */
export const ELIDE_CH = 12;

/** Path-parameter slots for a center label, in preference order. */
const T_SLOTS = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8];
/** Perpendicular offsets (px) per t-slot — on-line (inline chip) first. */
const PERP_OFFSETS = [0, 12, -12];

export interface EdgeLabelInput {
  edgeId: string;
  /** Routed polyline (scene coords). */
  points: Pt[];
  /** Composed center label text (edgeDecor.label) — '' for none. */
  centerText: string;
  /** Secondary label texts, in stack order. */
  secondaryTexts: string[];
  /** Preferred center-label anchor (elk-native label position), if any. */
  preferredAt?: Pt | null;
}

export interface ResolvedLabel {
  edgeId: string;
  kind: 'center' | 'secondary';
  /** Full (unwrapped) text. */
  text: string;
  /** Wrapped display lines (full text; LOD elision is render-side only). */
  lines: string[];
  /** Reserved chip rect (always full size, so LOD toggling never reflows). */
  rect: Rect;
  /** Chip center. */
  anchor: Pt;
  /** No clean slot existed — placed at minimum overlap, chip bg kept. */
  degraded: boolean;
}

/** Word-wrap at `width` chars (long unbreakable tokens split hard). */
export function wrapLabel(text: string, width: number = WRAP_CH): string[] {
  if (text.length <= width) return [text];
  const words = text.split(' ');
  const lines: string[] = [];
  let cur = '';
  for (const w of words) {
    if (cur && cur.length + 1 + w.length <= width) {
      cur = `${cur} ${w}`;
    } else {
      if (cur) lines.push(cur);
      let rest = w;
      while (rest.length > width) {
        lines.push(rest.slice(0, width));
        rest = rest.slice(width);
      }
      cur = rest;
    }
  }
  if (cur) lines.push(cur);
  return lines;
}

/** Chip box for a set of wrapped lines (mirrors the EdgeLabel renderer). */
export function chipSize(lines: string[]): { width: number; height: number } {
  const chars = lines.reduce((m, l) => Math.max(m, l.length), 1);
  return { width: chars * LABEL_CHAR_W + 2 * LABEL_PAD_X, height: lines.length * LABEL_LINE_H };
}

function pathLength(points: Pt[]): number {
  let len = 0;
  for (let i = 1; i < points.length; i++) {
    len += Math.hypot(points[i].x - points[i - 1].x, points[i].y - points[i - 1].y);
  }
  return len;
}

/** Point + unit direction at arc-length parameter t ∈ [0,1] on a polyline. */
export function pointAt(points: Pt[], t: number): { point: Pt; dir: Pt } {
  const total = pathLength(points);
  if (total === 0 || points.length < 2) {
    return { point: points[0] ?? { x: 0, y: 0 }, dir: { x: 1, y: 0 } };
  }
  let remaining = Math.min(Math.max(t, 0), 1) * total;
  for (let i = 1; i < points.length; i++) {
    const seg = Math.hypot(points[i].x - points[i - 1].x, points[i].y - points[i - 1].y);
    if (seg >= remaining || i === points.length - 1) {
      const f = seg === 0 ? 0 : remaining / seg;
      const dx = points[i].x - points[i - 1].x;
      const dy = points[i].y - points[i - 1].y;
      const inv = seg === 0 ? 0 : 1 / seg;
      return {
        point: { x: points[i - 1].x + dx * f, y: points[i - 1].y + dy * f },
        dir: { x: dx * inv, y: dy * inv },
      };
    }
    remaining -= seg;
  }
  return { point: points[points.length - 1], dir: { x: 1, y: 0 } };
}

/** Occupancy index: exact-rect intersection over everything stamped so far. */
class Grid {
  private rects: Rect[] = [];

  stamp(rect: Rect): void {
    this.rects.push(rect);
  }

  /** Total intersection AREA the rect incurs (0 = clean slot). */
  overlap(rect: Rect): number {
    let area = 0;
    for (const r of this.rects) {
      const w = Math.min(rect.x + rect.width, r.x + r.width) - Math.max(rect.x, r.x);
      if (w <= 0) continue;
      const h = Math.min(rect.y + rect.height, r.y + r.height) - Math.max(rect.y, r.y);
      if (h > 0) area += w * h;
    }
    return area;
  }
}

function rectAt(center: Pt, size: { width: number; height: number }): Rect {
  return { x: center.x - size.width / 2, y: center.y - size.height / 2, width: size.width, height: size.height };
}

/** Place one chip: first clean candidate wins; else minimum-overlap, degraded. */
function placeChip(
  grid: Grid,
  candidates: Pt[],
  size: { width: number; height: number },
): { rect: Rect; anchor: Pt; degraded: boolean } {
  let best: { rect: Rect; anchor: Pt } | null = null;
  let bestOverlap = Infinity;
  for (const c of candidates) {
    const rect = rectAt(c, size);
    const ov = grid.overlap(rect);
    if (ov === 0) {
      grid.stamp(rect);
      return { rect, anchor: c, degraded: false };
    }
    if (ov < bestOverlap) {
      bestOverlap = ov;
      best = { rect, anchor: c };
    }
  }
  // No clean slot — legibility over purity: keep the chip, tag it (§1 step 4).
  const fallback = best ?? { rect: rectAt(candidates[0], size), anchor: candidates[0] };
  grid.stamp(fallback.rect);
  return { ...fallback, degraded: true };
}

/** Candidate anchors for a center label on its path. */
function centerCandidates(points: Pt[], preferredAt: Pt | null | undefined): Pt[] {
  const out: Pt[] = [];
  if (preferredAt) out.push(preferredAt);
  for (const t of T_SLOTS) {
    const { point, dir } = pointAt(points, t);
    const perp = { x: -dir.y, y: dir.x };
    for (const off of PERP_OFFSETS) {
      out.push({ x: point.x + perp.x * off, y: point.y + perp.y * off });
    }
  }
  return out;
}

/**
 * Resolve every edge label against the final placed geometry.
 *
 * Deterministic: obstacles are stamped first, then center labels in ascending
 * path-length order (input index breaks ties), each stamped as placed.
 */
export function resolveEdgeLabels(input: {
  obstacles: Rect[];
  edges: EdgeLabelInput[];
  /** Layout bbox — reserved for a future spatial index; placement is exact. */
  bounds: Rect;
}): ResolvedLabel[] {
  const grid = new Grid();
  for (const o of input.obstacles) grid.stamp(o);

  const ordered = input.edges
    .map((e, i) => ({ e, i, len: pathLength(e.points) }))
    .sort((a, b) => a.len - b.len || a.i - b.i);

  const out: ResolvedLabel[] = [];
  for (const { e } of ordered) {
    if (e.points.length < 2) continue;
    let centerRect: Rect | null = null;
    if (e.centerText) {
      const lines = wrapLabel(e.centerText);
      const size = chipSize(lines);
      const placed = placeChip(grid, centerCandidates(e.points, e.preferredAt), size);
      centerRect = placed.rect;
      out.push({ edgeId: e.edgeId, kind: 'center', text: e.centerText, lines, ...placed });
    }
    for (const text of e.secondaryTexts) {
      const lines = wrapLabel(text);
      const size = chipSize(lines);
      // Stack below the previous chip of this edge when that slot is clear…
      if (centerRect) {
        const stacked = rectAt(
          { x: centerRect.x + centerRect.width / 2, y: centerRect.y + centerRect.height + size.height / 2 + 2 },
          size,
        );
        if (grid.overlap(stacked) === 0) {
          grid.stamp(stacked);
          const anchor = { x: stacked.x + stacked.width / 2, y: stacked.y + stacked.height / 2 };
          out.push({ edgeId: e.edgeId, kind: 'secondary', text, lines, rect: stacked, anchor, degraded: false });
          centerRect = stacked;
          continue;
        }
      }
      // …otherwise it gets its own slot search (§1 step 5).
      const placed = placeChip(grid, centerCandidates(e.points, null), size);
      out.push({ edgeId: e.edgeId, kind: 'secondary', text, lines, ...placed });
      centerRect = placed.rect;
    }
  }
  return out;
}

/** Render-side elision for reduced/glyph LOD — reservation is unaffected. */
export function elideForLod(text: string, full: boolean): string {
  if (full || text.length <= ELIDE_CH) return text;
  return `${text.slice(0, ELIDE_CH)}…`;
}
