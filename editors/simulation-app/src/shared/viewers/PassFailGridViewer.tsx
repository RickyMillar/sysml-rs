/**
 * PassFailGridViewer — Layer 1 EP6 primitive (R3.3).
 *
 * A verdict matrix: rows = verification cases, columns = individual
 * requirement checks, cells = `<VerdictBadge size="compact" />`.
 *
 * The viewer consumes a flat `Verdict[]` (the universal shape from
 * `engine/types.ts` — locked at R1.3). It derives the row/column taxonomy
 * from `verdict.metadata.case_name` and `verdict.metadata.requirement_id`.
 * Producers that don't populate these fall back to synthetic keys so the
 * matrix still renders something meaningful.
 *
 * Drill-from-verdict (R3.5) is *not* implemented here — Agent Q wires it
 * on top of our `onVerdictSelect` callback. Clicking a cell fires the
 * callback with the underlying verdict; wiring is the parent's problem.
 *
 * Shared with: VerifyWorkflow and any future aggregating workflow that
 * emits `Verdict[]`.
 */
import { useMemo, useState } from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent } from 'react';
import { VerdictBadge } from '../../components/VerdictBadge';
import type { Value, Verdict, VerdictKind } from '../../engine/types';
import type {
  AxesConfig,
  PassFailGridData,
  ResultData,
  ResultViewer,
} from './types';
import {
  denseBodyRowStyle,
  denseTableStyle,
  stickyFirstColumnCellStyle,
  stickyHeaderCellStyle,
} from './tableDensity';

// ── Metadata helpers ────────────────────────────────────────────────

const UNKNOWN_CASE = 'Unnamed case';
const UNKNOWN_REQ = 'check';

/** Pull a stringy metadata value, tolerating the permissive `unknown` union. */
function metaString(meta: Record<string, unknown> | undefined, key: string): string | undefined {
  if (!meta) return undefined;
  const raw = meta[key];
  if (raw == null) return undefined;
  if (typeof raw === 'string') return raw;
  if (typeof raw === 'number' || typeof raw === 'boolean') return String(raw);
  return undefined;
}

/** Row key for a verdict (verification case name, with a stable fallback). */
function caseNameOf(v: Verdict): string {
  return metaString(v.metadata, 'case_name') ?? UNKNOWN_CASE;
}

/** Column key for a verdict (requirement id, with a stable fallback). */
function requirementOf(v: Verdict): string {
  return metaString(v.metadata, 'requirement_id') ?? UNKNOWN_REQ;
}

// ── Summary tiles ───────────────────────────────────────────────────

type Totals = Record<VerdictKind, number>;

function emptyTotals(): Totals {
  return { pass: 0, fail: 0, inconclusive: 0, error: 0 };
}

function computeTotals(verdicts: Verdict[]): Totals {
  const totals = emptyTotals();
  for (const v of verdicts) totals[v.verdict] += 1;
  return totals;
}

const TILE_META: Record<VerdictKind, { label: string; color: string }> = {
  pass: { label: 'Pass', color: 'var(--verdict-pass)' },
  fail: { label: 'Fail', color: 'var(--verdict-fail)' },
  inconclusive: { label: 'Inconclusive', color: 'var(--verdict-inconclusive)' },
  error: { label: 'Error', color: 'var(--verdict-error)' },
};

interface SummaryBarProps {
  totals: Totals;
}

function SummaryBar({ totals }: SummaryBarProps) {
  const order: VerdictKind[] = ['pass', 'fail', 'inconclusive', 'error'];
  return (
    <div
      role="group"
      aria-label="Verdict totals"
      data-testid="pass-fail-grid-summary"
      style={{
        display: 'flex',
        gap: 8,
        marginBottom: 12,
        flexWrap: 'wrap',
      }}
    >
      {order.map((k) => {
        const meta = TILE_META[k];
        const count = totals[k];
        return (
          <div
            key={k}
            data-testid={`summary-tile-${k}`}
            aria-label={`${meta.label}: ${count}`}
            style={{
              minWidth: 96,
              padding: '10px 14px',
              borderRadius: 8,
              border: `1px solid color-mix(in srgb, ${meta.color} 35%, transparent)`,
              background: `color-mix(in srgb, ${meta.color} 10%, transparent)`,
              color: meta.color,
              display: 'flex',
              flexDirection: 'column',
              gap: 2,
            }}
          >
            <span style={{ fontSize: 22, fontWeight: 600, lineHeight: 1 }}>{count}</span>
            <span style={{ fontSize: 11, color: 'var(--on-surface)', opacity: 0.8 }}>
              {meta.label}
            </span>
          </div>
        );
      })}
    </div>
  );
}

// ── Sort model ──────────────────────────────────────────────────────

type SortKey = 'case' | `col:${string}`;
type SortDir = 'asc' | 'desc';

interface SortState {
  key: SortKey;
  dir: SortDir;
}

/**
 * Score a row by a single column — count of pass verdicts in that column
 * for the row. Inconclusive/error/fail all score 0 so pass-heavy rows
 * sort to the top in `desc`, and fail-heavy rows sort to the top in `asc`.
 */
function columnScore(rowVerdicts: Verdict[], column: string): number {
  let n = 0;
  for (const v of rowVerdicts) {
    if (requirementOf(v) === column && v.verdict === 'pass') n += 1;
  }
  return n;
}

// ── Main component ─────────────────────────────────────────────────

interface PassFailGridProps {
  data: PassFailGridData;
  axes: AxesConfig;
}

function PassFailGrid({ data, axes }: PassFailGridProps) {
  const { verdicts, onVerdictSelect } = data;
  const [sort, setSort] = useState<SortState>({ key: 'case', dir: 'asc' });

  const { rows, columns, cellIndex } = useMemo(() => {
    // Build deterministic row/column sets first (insertion-ordered).
    const rowSet = new Map<string, Verdict[]>();
    const columnSet = new Set<string>();
    for (const v of verdicts) {
      const row = caseNameOf(v);
      const col = requirementOf(v);
      columnSet.add(col);
      if (!rowSet.has(row)) rowSet.set(row, []);
      rowSet.get(row)!.push(v);
    }

    // Cell lookup: (row, column) → verdict (first match wins; producers are
    // expected to emit one verdict per (case, requirement)).
    const cellIndex = new Map<string, Verdict>();
    for (const v of verdicts) {
      const key = `${caseNameOf(v)}\u0000${requirementOf(v)}`;
      if (!cellIndex.has(key)) cellIndex.set(key, v);
    }

    return {
      rows: [...rowSet.keys()],
      columns: [...columnSet],
      cellIndex,
    };
  }, [verdicts]);

  const sortedRows = useMemo(() => {
    const next = [...rows];
    const { key, dir } = sort;
    const sign = dir === 'asc' ? 1 : -1;
    if (key === 'case') {
      next.sort((a, b) => sign * a.localeCompare(b));
    } else {
      const column = key.slice('col:'.length);
      next.sort((a, b) => {
        const aRow = verdicts.filter((v) => caseNameOf(v) === a);
        const bRow = verdicts.filter((v) => caseNameOf(v) === b);
        const diff = columnScore(aRow, column) - columnScore(bRow, column);
        // Tie-break by case name for deterministic ordering.
        if (diff !== 0) return sign * diff;
        return a.localeCompare(b);
      });
    }
    return next;
  }, [rows, sort, verdicts]);

  const totals = useMemo(() => computeTotals(verdicts), [verdicts]);

  if (verdicts.length === 0) {
    return <EmptyState axes={axes} />;
  }

  const toggleSort = (key: SortKey) => {
    setSort((prev) =>
      prev.key === key
        ? { key, dir: prev.dir === 'asc' ? 'desc' : 'asc' }
        : { key, dir: 'asc' },
    );
  };

  const sortIndicator = (key: SortKey): string => {
    if (sort.key !== key) return '';
    return sort.dir === 'asc' ? ' \u25B2' : ' \u25BC';
  };

  const tableStyle: CSSProperties = {
    ...denseTableStyle,
    width: axes.width ?? '100%',
  };

  const headerCellStyle: CSSProperties = stickyHeaderCellStyle({
    padding: '8px 12px',
    textAlign: 'left',
    background: 'color-mix(in srgb, var(--outline-variant) 10%, transparent)',
    borderBottom: '1px solid color-mix(in srgb, var(--outline-variant) 30%, transparent)',
    fontWeight: 600,
    cursor: 'pointer',
    userSelect: 'none',
    whiteSpace: 'nowrap',
  });

  const bodyCellStyle: CSSProperties = {
    ...denseBodyRowStyle,
    padding: '4px 8px',
    borderBottom: '1px solid color-mix(in srgb, var(--outline-variant) 15%, transparent)',
    textAlign: 'center',
    verticalAlign: 'middle',
  };

  const rowHeaderStyle: CSSProperties = stickyFirstColumnCellStyle({
    ...bodyCellStyle,
    textAlign: 'left',
    fontWeight: 500,
    whiteSpace: 'nowrap',
    color: 'var(--on-surface)',
  });

  // Corner cell: the "Case" header is both the sticky header row AND
  // the sticky first column.
  const cornerHeaderStyle: CSSProperties = stickyFirstColumnCellStyle(headerCellStyle, {
    isHeader: true,
  });

  return (
    <div
      data-testid="pass-fail-grid"
      style={{ display: 'flex', flexDirection: 'column', height: axes.height }}
    >
      <SummaryBar totals={totals} />
      <div style={{ overflow: 'auto' }}>
        <table style={tableStyle} data-testid="pass-fail-grid-table">
          <thead>
            <tr>
              <th
                scope="col"
                style={cornerHeaderStyle}
                onClick={() => toggleSort('case')}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    toggleSort('case');
                  }
                }}
                tabIndex={0}
                role="columnheader"
                aria-sort={
                  sort.key === 'case'
                    ? sort.dir === 'asc'
                      ? 'ascending'
                      : 'descending'
                    : 'none'
                }
                data-testid="header-case"
              >
                Case{sortIndicator('case')}
              </th>
              {columns.map((col) => {
                const key: SortKey = `col:${col}`;
                const isSorted = sort.key === key;
                return (
                  <th
                    key={col}
                    scope="col"
                    style={headerCellStyle}
                    onClick={() => toggleSort(key)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        toggleSort(key);
                      }
                    }}
                    tabIndex={0}
                    role="columnheader"
                    aria-sort={
                      isSorted
                        ? sort.dir === 'asc'
                          ? 'ascending'
                          : 'descending'
                        : 'none'
                    }
                    data-testid={`header-col-${col}`}
                  >
                    {col}
                    {sortIndicator(key)}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {sortedRows.map((row) => (
              <tr key={row} data-testid={`row-${row}`}>
                <th scope="row" style={rowHeaderStyle}>
                  {row}
                </th>
                {columns.map((col) => {
                  const verdict = cellIndex.get(`${row}\u0000${col}`);
                  return (
                    <td
                      key={col}
                      style={bodyCellStyle}
                      data-testid={`cell-${row}-${col}`}
                    >
                      {verdict ? (
                        <VerdictCell
                          verdict={verdict}
                          caseName={row}
                          requirement={col}
                          onSelect={onVerdictSelect}
                        />
                      ) : (
                        <span
                          aria-label={`No verdict for ${row}.${col}`}
                          style={{ opacity: 0.4 }}
                        >
                          {'\u2014'}
                        </span>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ── Empty state ─────────────────────────────────────────────────────

function EmptyState({ axes }: { axes: AxesConfig }) {
  return (
    <div
      data-testid="pass-fail-grid-empty"
      role="status"
      style={{
        height: axes.height ?? 200,
        width: axes.width ?? '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: 'var(--on-surface)',
        opacity: 0.6,
        fontSize: 13,
        fontStyle: 'italic',
        border: '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
        borderRadius: 8,
        padding: 16,
      }}
    >
      Run verification to populate matrix
    </div>
  );
}

// ── Single cell ─────────────────────────────────────────────────────

interface VerdictCellProps {
  verdict: Verdict;
  caseName: string;
  requirement: string;
  onSelect?: (verdict: Verdict) => void;
}

const LABEL_BY_KIND: Record<VerdictKind, string> = {
  pass: 'Pass',
  fail: 'Fail',
  inconclusive: 'Inconclusive',
  error: 'Error',
};

function VerdictCell({ verdict, caseName, requirement, onSelect }: VerdictCellProps) {
  const label = LABEL_BY_KIND[verdict.verdict];
  const ariaLabel = `Verdict for ${caseName}.${requirement}: ${label}`;

  const errorStr =
    typeof verdict.error === 'string' && verdict.error.length > 0 ? verdict.error : null;
  // When the backend reports an evaluation error, surface `error: <msg>`
  // in the actual slot — distinct from the inconclusive case where both
  // `actual` and `error` are null.
  const actualStr = errorStr
    ? `error: ${errorStr}`
    : verdict.actual != null && (typeof verdict.actual === 'string' || typeof verdict.actual === 'number' || typeof verdict.actual === 'boolean')
      ? String(verdict.actual)
      : undefined;
  const expectedStr =
    verdict.expected != null && (typeof verdict.expected === 'string' || typeof verdict.expected === 'number' || typeof verdict.expected === 'boolean')
      ? String(verdict.expected)
      : undefined;
  const reason =
    errorStr
    ?? metaString(verdict.metadata, 'error_reason')
    ?? metaString(verdict.metadata, 'message');

  const buttonStyle: CSSProperties = {
    background: 'transparent',
    border: 'none',
    padding: 0,
    cursor: onSelect ? 'pointer' : 'default',
    display: 'inline-flex',
    // Disable hover transitions when the user prefers reduced motion.
    transition: 'none',
  };

  const handleClick = () => {
    if (onSelect) onSelect(verdict);
  };

  const handleKeyDown = (e: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (!onSelect) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onSelect(verdict);
    }
  };

  return (
    <button
      type="button"
      aria-label={ariaLabel}
      data-testid={`verdict-cell-${caseName}-${requirement}`}
      data-verdict-kind={verdict.verdict}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      disabled={!onSelect}
      style={buttonStyle}
    >
      <VerdictBadge
        verdict={verdict.verdict}
        name={`${caseName}.${requirement}`}
        actual={actualStr ?? null}
        expected={expectedStr ?? null}
        reason={reason ?? null}
        error={errorStr}
        size="compact"
      />
    </button>
  );
}

// ── Canonical viewer export ─────────────────────────────────────────

/**
 * The verdict-matrix viewer. Registered in the viewer kit so any workflow
 * emitting `Verdict[]` (Verify, future aggregators) can mount the same
 * matrix via `kind: 'pass-fail-grid'`.
 */
export const passFailGridViewer: ResultViewer<PassFailGridData> = {
  id: 'pass-fail-grid-default',
  kind: 'pass-fail-grid',
  accepts: (data): data is PassFailGridData => data.kind === 'pass-fail-grid',
  render: (data, axes) => <PassFailGrid data={data} axes={axes} />,
};

// Local helper re-exported so Agent Q / Agent S can build on top of the
// same metadata conventions without duplicating the fallback logic.
export const __internals = { caseNameOf, requirementOf, computeTotals };
