/**
 * filterTraceMatrix — pure client-side narrowing helper for the trace
 * matrix viewer (R6.2).
 *
 * Keeping the helper pure makes it trivial to unit-test every filter
 * combination: only-unsatisfied, only-no-coverage, substring search,
 * and every union thereof.
 *
 * Also exports:
 *   - `buildTraceMatrix` — derives the grouped `TraceMatrix` shape
 *     (rows / columns / links) from the flat `TraceMatrixRow[]`
 *     returned by `sysml.trace_matrix`. Kept separate from filtering
 *     so the hook can build once and the viewer can filter without
 *     re-grouping. Satisfy/Verify edges run source=satisfier/case →
 *     target=requirement, so matrix ROWS key on the TARGET endpoint —
 *     everything downstream (filters, viewer) treats a row as a
 *     requirement.
 *   - `isRowSatisfied` — worst-wins helper: a row is satisfied when
 *     it has at least one link and *every* link passes. Exposed so
 *     the viewer and tests share one implementation.
 */

import type { VerdictKind } from '@/engine/types';
import type { TraceFilter, TraceMatrix, TraceMatrixRow } from './types';

// ── Building the grouped matrix ──────────────────────────────────────

/**
 * Group the flat backend edge list into the `TraceMatrix` shape
 * consumed by `<TraceabilityMatrixViewer />`. Rows / columns are
 * inserted in first-seen order so the output is deterministic.
 *
 * The requirement sits on each edge's TARGET endpoint (Satisfy/Verify
 * mint satisfier/case → requirement), so rows key on `target` and
 * columns on `source`.
 *
 * `rowUniverse` (optional) — when supplied, the helper preserves every
 * row id even if it has zero links. This lets the "no coverage"
 * filter surface requirements that exist in the model but have no
 * `Satisfy` edges. Pass an empty array (or omit) to derive the row set
 * from the links themselves.
 */
export function buildTraceMatrix(
  rows: TraceMatrixRow[],
  rowUniverse?: Array<{ id: string; label?: string | null }>,
): TraceMatrix {
  const rowMap = new Map<string, string>(); // id → label
  const colMap = new Map<string, string>(); // id → label
  const links: TraceMatrix['links'] = [];

  // Seed with the universe first so zero-link rows are preserved.
  if (rowUniverse) {
    for (const r of rowUniverse) {
      if (!rowMap.has(r.id)) {
        rowMap.set(r.id, r.label ?? r.id);
      }
    }
  }

  for (const row of rows) {
    if (!rowMap.has(row.target)) {
      rowMap.set(row.target, row.target_name ?? row.target);
    }
    if (!colMap.has(row.source)) {
      colMap.set(row.source, row.source_name ?? row.source);
    }
    links.push({
      row: row.target,
      column: row.source,
      relationship: row.relationship,
      // `sysml.trace_matrix` returns the structural link only — the
      // verdict is populated by an overlay that joins verification
      // results with trace rows. Until that overlay runs, every link
      // is "not yet evaluated" (= inconclusive).
      verdict: 'inconclusive' satisfies VerdictKind,
      reason: 'Not yet evaluated',
    });
  }

  return {
    rows: Array.from(rowMap, ([id, label]) => ({ id, label })),
    columns: Array.from(colMap, ([id, label]) => ({ id, label })),
    links,
  };
}

// ── Row-level verdict helpers ────────────────────────────────────────

/**
 * A row is "satisfied" when it has at least one link *and* every link
 * passes. Zero-link rows are explicitly not satisfied (they're "no
 * coverage" rather than "done"). Non-pass verdicts (fail / error /
 * inconclusive) always knock a row out of satisfied.
 *
 * Exported so the panel and its tests share one severity policy —
 * there are no inline checks sprinkled across components.
 */
export function isRowSatisfied(matrix: TraceMatrix, rowId: string): boolean {
  const rowLinks = matrix.links.filter((l) => l.row === rowId);
  if (rowLinks.length === 0) return false;
  return rowLinks.every((l) => l.verdict === 'pass');
}

/**
 * Count the links attached to a given row. O(links) per call — fine
 * for UIs because the matrix is typically tens / hundreds of edges,
 * not millions. Exposed so the "no coverage" filter can call a
 * single helper rather than inlining `filter` + `.length`.
 */
export function rowLinkCount(matrix: TraceMatrix, rowId: string): number {
  let n = 0;
  for (const l of matrix.links) {
    if (l.row === rowId) n += 1;
  }
  return n;
}

// ── Filtering ────────────────────────────────────────────────────────

/**
 * Narrow the matrix client-side. The server emits every (source,
 * target) edge — the UI decides what subset to render. Idempotent
 * and allocation-frugal: the input `TraceMatrix` is never mutated.
 *
 * Filter semantics:
 *   - `search`          — case-insensitive substring on row label.
 *   - `onlyUnsatisfied` — keep rows where `isRowSatisfied` is false.
 *   - `onlyNoCoverage`  — keep rows where `rowLinkCount` is zero.
 *
 * When both `onlyUnsatisfied` and `onlyNoCoverage` are set, the
 * filter returns their intersection (zero-coverage rows are always
 * unsatisfied, so the result is just the no-coverage subset).
 *
 * Columns are filtered to only those reachable from the surviving
 * rows — the viewer doesn't show empty columns. This matches the
 * Archive panel's "no-op drop" pattern: filtered-to-zero columns
 * simply never render.
 */
export function filterTraceMatrix(
  matrix: TraceMatrix,
  filter: TraceFilter,
): TraceMatrix {
  const needle = filter.search.trim().toLowerCase();

  const keepRow = (row: (typeof matrix.rows)[number]): boolean => {
    if (needle.length > 0) {
      const hay = row.label.toLowerCase();
      if (!hay.includes(needle)) return false;
    }
    if (filter.onlyNoCoverage && rowLinkCount(matrix, row.id) !== 0) {
      return false;
    }
    if (filter.onlyUnsatisfied && isRowSatisfied(matrix, row.id)) {
      return false;
    }
    return true;
  };

  const rows = matrix.rows.filter(keepRow);
  const rowIds = new Set(rows.map((r) => r.id));
  const links = matrix.links.filter((l) => rowIds.has(l.row));
  const reachedCols = new Set(links.map((l) => l.column));
  const columns = matrix.columns.filter((c) => reachedCols.has(c.id));

  return { rows, columns, links };
}
