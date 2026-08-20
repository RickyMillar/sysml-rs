/**
 * tableDensity — shared dense-row + sticky-header/-column primitives for
 * the result-viewer tables (ninebar Phase 1, plan §3 Phase 1: "Apply
 * density tiers to the shared table / tree-row / matrix primitives...
 * sticky header + first column").
 *
 * `SweepTableViewer`, `TradeStudyTableViewer`, and `PassFailGridViewer`
 * each built their own near-identical inline `tableStyle` /
 * `headerCellStyle` / `bodyCellStyle` objects independently — there was
 * no shared table primitive to consume. This module is that primitive:
 * a small set of composable style helpers the three viewers spread into
 * their existing style objects. It does NOT replace their per-viewer
 * padding/hover/sort styling or touch their DOM structure/testids — it
 * only adds the row-height token + sticky positioning, per the
 * discipline of consuming a primitive rather than restyling the tables.
 */
import type { CSSProperties } from 'react';

/**
 * Row height for data-dense matrix/table rows — the `--row-dense`
 * token (16px per crib-sheet reconciliation #1: the scale is
 * 16/24/32/40, snap to it). Always referenced via the CSS var, never
 * hardcoded, so a future token change propagates automatically.
 *
 * Applied as `height` (a floor on table cells per CSS table layout —
 * content taller than the token still renders in full) rather than
 * `minHeight`, since `<td>`/`<th>` don't reliably honour `min-height`.
 */
export const DENSE_ROW_HEIGHT = 'var(--row-dense)';

/** Base `<table>` style shared by the result-viewer tables. Spread this
 *  first, then let the viewer override `width` and any per-viewer
 *  concern. */
export const denseTableStyle: CSSProperties = {
  borderCollapse: 'separate',
  borderSpacing: 0,
  fontSize: 12,
  color: 'var(--on-surface)',
};

/**
 * Makes a header cell sticky to the top of its scroll container and
 * snaps it to the dense row-height token. Spread a viewer's own
 * `headerCellStyle` object in as `base` — this composes on top of it,
 * it doesn't replace the viewer's padding/cursor/hover styling.
 */
export function stickyHeaderCellStyle(base: CSSProperties): CSSProperties {
  return {
    ...base,
    height: DENSE_ROW_HEIGHT,
    position: 'sticky',
    top: 0,
    zIndex: 2,
  };
}

/**
 * Makes a first-column cell (header or body) sticky to the left edge of
 * its scroll container, so the row label stays visible while scrolling
 * a wide matrix horizontally. Pass `isHeader: true` for the corner cell
 * (first column AND header row) so it stacks above both the sticky
 * header row and the sticky column (`zIndex: 3` vs `1`).
 */
export function stickyFirstColumnCellStyle(
  base: CSSProperties,
  opts?: { isHeader?: boolean },
): CSSProperties {
  return {
    ...base,
    position: 'sticky',
    left: 0,
    zIndex: opts?.isHeader ? 3 : 1,
    background: base.background ?? 'var(--surface-panel)',
  };
}

/** Dense body-cell height. Spread into a viewer's own `bodyCellStyle`. */
export const denseBodyRowStyle: CSSProperties = {
  height: DENSE_ROW_HEIGHT,
};
