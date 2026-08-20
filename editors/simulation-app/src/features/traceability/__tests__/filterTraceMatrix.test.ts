/**
 * Pure-function tests for `filterTraceMatrix` and its helpers (R6.2).
 *
 * Covers the acceptance criteria: empty rows/columns, only-unsatisfied,
 * only-no-coverage, substring search, and every intersection thereof.
 * The helpers (`buildTraceMatrix`, `isRowSatisfied`, `rowLinkCount`) are
 * exercised here too so the viewer + panel share one severity policy.
 */

import { describe, expect, it } from 'vitest';
import {
  buildTraceMatrix,
  filterTraceMatrix,
  isRowSatisfied,
  rowLinkCount,
} from '../filterTraceMatrix';
import {
  DEFAULT_TRACE_FILTER,
  type TraceFilter,
  type TraceMatrix,
  type TraceMatrixRow,
} from '../types';
import type { VerdictKind } from '@/engine/types';

function mkEdge(
  overrides: Partial<TraceMatrixRow> & Pick<TraceMatrixRow, 'source' | 'target'>,
): TraceMatrixRow {
  return {
    source: overrides.source,
    source_name: overrides.source_name ?? null,
    target: overrides.target,
    target_name: overrides.target_name ?? null,
    relationship: overrides.relationship ?? `${overrides.source}->${overrides.target}`,
  };
}

function mkMatrix(
  rows: Array<{ id: string; label?: string }>,
  columns: Array<{ id: string; label?: string }>,
  links: Array<{ row: string; column: string; verdict?: VerdictKind; relationship?: string }>,
): TraceMatrix {
  return {
    rows: rows.map((r) => ({ id: r.id, label: r.label ?? r.id })),
    columns: columns.map((c) => ({ id: c.id, label: c.label ?? c.id })),
    links: links.map((l) => ({
      row: l.row,
      column: l.column,
      relationship: l.relationship ?? `${l.row}->${l.column}`,
      verdict: l.verdict ?? 'inconclusive',
    })),
  };
}

function withFilter(partial: Partial<TraceFilter>): TraceFilter {
  return { ...DEFAULT_TRACE_FILTER, ...partial };
}

// ── buildTraceMatrix ─────────────────────────────────────────────────

describe('buildTraceMatrix', () => {
  it('returns an empty matrix for an empty edge list', () => {
    const out = buildTraceMatrix([]);
    expect(out.rows).toEqual([]);
    expect(out.columns).toEqual([]);
    expect(out.links).toEqual([]);
  });

  // Edge direction on the wire: source = satisfier (part / case),
  // target = requirement. Rows key on the requirement (target) side.
  it('groups edges into rows (target requirements) and columns (source satisfiers)', () => {
    const out = buildTraceMatrix([
      mkEdge({ source: 'P1', source_name: 'Part1', target: 'R1', target_name: 'Req1' }),
      mkEdge({ source: 'P2', source_name: 'Part2', target: 'R1', target_name: 'Req1' }),
      mkEdge({ source: 'P1', source_name: 'Part1', target: 'R2', target_name: 'Req2' }),
    ]);
    expect(out.rows.map((r) => r.id)).toEqual(['R1', 'R2']);
    expect(out.columns.map((c) => c.id)).toEqual(['P1', 'P2']);
    expect(out.links).toHaveLength(3);
  });

  it('keys links on (row=target requirement, column=source satisfier)', () => {
    const out = buildTraceMatrix([
      mkEdge({ source: 'P1', target: 'R1' }),
    ]);
    expect(out.links[0].row).toBe('R1');
    expect(out.links[0].column).toBe('P1');
  });

  it('uses target_name / source_name as row / column labels when present', () => {
    const out = buildTraceMatrix([
      mkEdge({ source: 'P1', source_name: 'Part1', target: 'R1', target_name: 'Req1' }),
    ]);
    expect(out.rows[0].label).toBe('Req1');
    expect(out.columns[0].label).toBe('Part1');
  });

  it('falls back to the element id when names are null', () => {
    const out = buildTraceMatrix([
      mkEdge({ source: 'P-id-only', target: 'R-id-only' }),
    ]);
    expect(out.rows[0].label).toBe('R-id-only');
    expect(out.columns[0].label).toBe('P-id-only');
  });

  it('defaults every link verdict to "inconclusive"', () => {
    const out = buildTraceMatrix([mkEdge({ source: 'P1', target: 'R1' })]);
    expect(out.links[0].verdict).toBe('inconclusive');
  });

  it('preserves the relationship id on each link', () => {
    const out = buildTraceMatrix([
      mkEdge({ source: 'P1', target: 'R1', relationship: 'rel-xyz' }),
    ]);
    expect(out.links[0].relationship).toBe('rel-xyz');
  });

  it('preserves zero-link rows when a rowUniverse is supplied', () => {
    const out = buildTraceMatrix(
      [mkEdge({ source: 'P1', target: 'R1' })],
      [
        { id: 'R1', label: 'Req1' },
        { id: 'R2', label: 'Req2' }, // no links → still preserved
      ],
    );
    expect(out.rows.map((r) => r.id).sort()).toEqual(['R1', 'R2']);
    expect(out.links).toHaveLength(1);
  });
});

// ── Row helpers ──────────────────────────────────────────────────────

describe('isRowSatisfied', () => {
  const matrix = mkMatrix(
    [{ id: 'R1' }, { id: 'R2' }, { id: 'R3' }, { id: 'R4' }],
    [{ id: 'P1' }, { id: 'P2' }],
    [
      { row: 'R1', column: 'P1', verdict: 'pass' },
      { row: 'R1', column: 'P2', verdict: 'pass' },
      { row: 'R2', column: 'P1', verdict: 'pass' },
      { row: 'R2', column: 'P2', verdict: 'fail' },
      { row: 'R3', column: 'P1', verdict: 'inconclusive' },
      // R4 has no links at all
    ],
  );

  it('returns true when every link passes', () => {
    expect(isRowSatisfied(matrix, 'R1')).toBe(true);
  });

  it('returns false when any link is non-pass', () => {
    expect(isRowSatisfied(matrix, 'R2')).toBe(false);
    expect(isRowSatisfied(matrix, 'R3')).toBe(false);
  });

  it('returns false for a row with zero links (no coverage ≠ satisfied)', () => {
    expect(isRowSatisfied(matrix, 'R4')).toBe(false);
  });
});

describe('rowLinkCount', () => {
  const matrix = mkMatrix(
    [{ id: 'R1' }, { id: 'R2' }],
    [{ id: 'P1' }, { id: 'P2' }],
    [
      { row: 'R1', column: 'P1' },
      { row: 'R1', column: 'P2' },
    ],
  );

  it('counts the links attached to a row', () => {
    expect(rowLinkCount(matrix, 'R1')).toBe(2);
  });

  it('returns 0 for a row with no links', () => {
    expect(rowLinkCount(matrix, 'R2')).toBe(0);
  });

  it('returns 0 for an unknown row id', () => {
    expect(rowLinkCount(matrix, 'nonexistent')).toBe(0);
  });
});

// ── filterTraceMatrix ────────────────────────────────────────────────

describe('filterTraceMatrix', () => {
  const matrix = mkMatrix(
    [
      { id: 'R1', label: 'Brake force' },
      { id: 'R2', label: 'Trip time at 5x' },
      { id: 'R3', label: 'Temperature rise' },
      { id: 'R4', label: 'Orphan requirement' },
    ],
    [
      { id: 'P1', label: 'Main breaker' },
      { id: 'P2', label: 'Aux breaker' },
      { id: 'P3', label: 'Relay' },
    ],
    [
      { row: 'R1', column: 'P1', verdict: 'pass' },
      { row: 'R1', column: 'P2', verdict: 'pass' },
      { row: 'R2', column: 'P1', verdict: 'fail' },
      { row: 'R3', column: 'P3', verdict: 'inconclusive' },
      // R4 — no links (zero coverage)
    ],
  );

  it('passes everything through with defaults', () => {
    const out = filterTraceMatrix(matrix, DEFAULT_TRACE_FILTER);
    expect(out.rows.map((r) => r.id).sort()).toEqual(['R1', 'R2', 'R3', 'R4']);
  });

  it('handles an empty matrix without error', () => {
    const empty = mkMatrix([], [], []);
    const out = filterTraceMatrix(empty, DEFAULT_TRACE_FILTER);
    expect(out).toEqual(empty);
  });

  it('only-unsatisfied filters out fully-passing rows', () => {
    const out = filterTraceMatrix(matrix, withFilter({ onlyUnsatisfied: true }));
    // R1 is the only fully-passing row.
    expect(out.rows.map((r) => r.id).sort()).toEqual(['R2', 'R3', 'R4']);
  });

  it('only-unsatisfied keeps zero-coverage rows (unsatisfied by policy)', () => {
    const out = filterTraceMatrix(matrix, withFilter({ onlyUnsatisfied: true }));
    expect(out.rows.find((r) => r.id === 'R4')).toBeDefined();
  });

  it('only-no-coverage keeps only requirements with zero links', () => {
    const out = filterTraceMatrix(matrix, withFilter({ onlyNoCoverage: true }));
    expect(out.rows.map((r) => r.id)).toEqual(['R4']);
    // Columns collapse to empty — no links to display for R4.
    expect(out.columns).toEqual([]);
  });

  it('only-no-coverage + only-unsatisfied intersects to the no-coverage subset', () => {
    const out = filterTraceMatrix(
      matrix,
      withFilter({ onlyNoCoverage: true, onlyUnsatisfied: true }),
    );
    expect(out.rows.map((r) => r.id)).toEqual(['R4']);
  });

  it('search narrows by row label substring, case-insensitive', () => {
    const out = filterTraceMatrix(matrix, withFilter({ search: 'TRIP' }));
    expect(out.rows.map((r) => r.id)).toEqual(['R2']);
  });

  it('search trims whitespace', () => {
    const out = filterTraceMatrix(matrix, withFilter({ search: '  brake  ' }));
    expect(out.rows.map((r) => r.id)).toEqual(['R1']);
  });

  it('search + only-unsatisfied combines as AND', () => {
    const out = filterTraceMatrix(
      matrix,
      withFilter({ search: 'brake', onlyUnsatisfied: true }),
    );
    // "brake" matches R1 (fully passing) only → filtered out by onlyUnsatisfied.
    expect(out.rows).toEqual([]);
  });

  it('filters columns down to those reachable from surviving rows', () => {
    const out = filterTraceMatrix(matrix, withFilter({ search: 'brake force' }));
    // R1 links to P1, P2 — P3 is dropped.
    expect(out.columns.map((c) => c.id).sort()).toEqual(['P1', 'P2']);
  });

  it('returns an empty matrix when filters exclude everything', () => {
    const out = filterTraceMatrix(matrix, withFilter({ search: 'nonexistent' }));
    expect(out.rows).toEqual([]);
    expect(out.columns).toEqual([]);
    expect(out.links).toEqual([]);
  });

  it('does not mutate the input matrix', () => {
    const snapshot = JSON.parse(JSON.stringify(matrix));
    filterTraceMatrix(matrix, withFilter({ onlyUnsatisfied: true }));
    expect(matrix).toEqual(snapshot);
  });

  it('preserves input row ordering among matches', () => {
    const out = filterTraceMatrix(matrix, withFilter({ onlyUnsatisfied: true }));
    expect(out.rows.map((r) => r.id)).toEqual(['R2', 'R3', 'R4']);
  });

  it('handles a matrix with rows but no links', () => {
    const sparse = mkMatrix(
      [{ id: 'R1', label: 'Lonely' }, { id: 'R2', label: 'Also lonely' }],
      [],
      [],
    );
    const out = filterTraceMatrix(sparse, withFilter({ onlyNoCoverage: true }));
    expect(out.rows.map((r) => r.id)).toEqual(['R1', 'R2']);
  });

  it('handles a matrix with columns but no links', () => {
    const sparse = mkMatrix(
      [],
      [{ id: 'P1', label: 'Part' }],
      [],
    );
    const out = filterTraceMatrix(sparse, DEFAULT_TRACE_FILTER);
    expect(out.rows).toEqual([]);
    // Columns collapse because no links survive.
    expect(out.columns).toEqual([]);
  });
});
