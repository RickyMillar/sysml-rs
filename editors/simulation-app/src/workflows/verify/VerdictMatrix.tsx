/**
 * VerdictMatrix — the ninebar Verify hero (Phase 4).
 *
 * The verdict matrix as the whole primary surface: rows = verification
 * cases, columns = requirement checks, cells = one of the seven ninebar
 * cell states (demo crib sheet §6). Sticky header + sticky first column
 * via the shared `tableDensity` primitive at the `--row-dense` (16px)
 * tier; a filter-tab bar (all / failing / not run) + a right-aligned
 * rollup sit above it.
 *
 * This is a NEW component (not a fork of the shared `PassFailGridViewer`,
 * which only knows the four *completed* verdict kinds): the ninebar
 * treatment adds the three run-lifecycle states the demo requires —
 * **not-run** (a selected case with no verdict yet), **running** (the
 * `<Ninebar/>` 3-bar mark while a case is being evaluated), and
 * **selected + failing** (the amber selection echo on a failing cell).
 * Verdict colour comes only from `--verdict-*`; amber (`--accent`) is
 * selection-only, never a verdict signal.
 *
 * Pure renderer: it takes verdicts + the pending/running lifecycle hints
 * and reports selection out via `onSelect`. The parent owns the run, the
 * selection store, and the right-rail detail context.
 */

import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import { Ninebar } from '@/components/Ninebar';
import {
  EvaluationModeBadge,
  normalizeEvaluationMode,
  type EvaluationMode,
} from '@/components/EvaluationModeBadge';
import type { Verdict, VerdictKind } from '@/engine/types';
import {
  denseTableStyle,
  stickyFirstColumnCellStyle,
  stickyHeaderCellStyle,
} from '@/shared/viewers/tableDensity';

// ── Metadata helpers (case / requirement taxonomy) ──────────────────

const CASE_COLUMN = '∙'; // '∙' — the synthetic case-level column key.

function metaString(meta: Record<string, unknown> | undefined, key: string): string | undefined {
  if (!meta) return undefined;
  const raw = meta[key];
  if (typeof raw === 'string' && raw.length > 0) return raw;
  if (typeof raw === 'number' || typeof raw === 'boolean') return String(raw);
  return undefined;
}

/**
 * Row key: the verification case (or constraint element) a verdict
 * belongs to. Verification-case results carry `case_name`; constraint
 * results carry only `element_id`/`display`; fall through so each path
 * still groups into meaningful rows rather than collapsing to one.
 */
export function caseNameOf(v: Verdict): string {
  return (
    metaString(v.metadata, 'case_name') ??
    metaString(v.metadata, 'case_id') ??
    v.label ??
    v.id ??
    metaString(v.metadata, 'display') ??
    metaString(v.metadata, 'element_id') ??
    'Unnamed case'
  );
}

/** Column key for a verdict — requirement id, or the case-level column. */
function reqKeyOf(v: Verdict): string {
  return metaString(v.metadata, 'requirement_id') ?? CASE_COLUMN;
}

/** Human column label for a verdict. */
function reqLabelOf(v: Verdict): string {
  return (
    metaString(v.metadata, 'requirement_name') ??
    metaString(v.metadata, 'requirement_id') ??
    'verdict'
  );
}

// ── Cell state model ────────────────────────────────────────────────

/** The ninebar cell states (crib §6) plus the bare-objective teaching
 *  state: a case whose objective binds NO checks mints no verdict, so its
 *  cell renders the teaching absence rather than the wire's Inconclusive. */
type CellState =
  | { kind: 'pass' | 'fail' | 'inconclusive' | 'error'; verdict: Verdict }
  | { kind: 'not-run' }
  | { kind: 'running' }
  | { kind: 'bare-objective' }
  | { kind: 'absent' };

/**
 * A case with `total_requirements === 0` binds no checks. The wire
 * honestly returns Inconclusive, but the FE mints no verdict for it (1e):
 * "no verdict is minted for a case with no checks — not even inconclusive".
 */
function isBareObjective(v: Verdict): boolean {
  const total = v.metadata?.total_requirements;
  return typeof total === 'number' && total === 0;
}

export type MatrixFilter = 'all' | 'failing' | 'not-run';

export interface VerdictMatrixProps {
  /** Displayed verdicts (case- or requirement-level; keyed by metadata). */
  verdicts: Verdict[];
  /** Selected case names that have produced no verdict yet (→ not-run rows). */
  pendingCaseNames?: string[];
  /** The case currently being evaluated, when a run is in flight. */
  runningCaseName?: string | null;
  /** True while a verify run is in flight — pending rows read as running. */
  isRunning?: boolean;
  /** The verdict whose cell shows the amber selection echo. */
  selectedVerdict?: Verdict | null;
  /** Fired when a cell with a verdict is activated (click / Enter / Space). */
  onSelect?: (verdict: Verdict) => void;
  /**
   * Fired on row DOUBLE-click — opens the case document (design footnote:
   * "chip click selects · double-click opens the case"). The id is the
   * case's `case_id` (or name) so the case view can look the row up.
   */
  /** Opens the case document. The row's computed `mode ƒ` travels with it so
   *  the case view can lead with the evidence the reader actually clicked —
   *  a "trajectory" row must not open onto a static desk check presented as
   *  its verdict (J5). Null when the row has no computed verdict. */
  onOpenCase?: (caseId: string, mode: EvaluationMode | null) => void;
}

interface RowModel {
  caseName: string;
  /** The id the case view opens on — `case_id` when the wire carries it,
   *  else the case name. */
  caseId: string;
  /** Per-column verdicts for this row. */
  cells: Map<string, Verdict>;
  /** True when this row is a selected-but-unevaluated case. */
  pending: boolean;
  /** Aggregate: does any cell fail/error? */
  hasFailure: boolean;
  /**
   * How THIS row's shown verdict was computed (B10 layer 2) — the per-row
   * `mode ƒ` column. Null when the row has no computed verdict (pending, or
   * a bare objective): a mode is never fabricated for a verdict that wasn't
   * minted (1e "never run" → no badge).
   */
  mode: EvaluationMode | null;
}

/** The row's single computed mode: distinct evaluation modes across its
 *  real verdicts (bare-objective cells excluded — they mint no verdict).
 *  A run's row is single-mode; if somehow mixed, computed modes win over
 *  external (which never enters this rollup anyway). */
function rowModeOf(cells: Map<string, Verdict>): EvaluationMode | null {
  const seen = new Set<EvaluationMode>();
  for (const v of cells.values()) {
    if (isBareObjective(v)) continue;
    const m = normalizeEvaluationMode(metaString(v.metadata, 'evaluation_mode'));
    if (m) seen.add(m);
  }
  for (const m of ['trajectory', 'static', 'external'] as const) {
    if (seen.has(m)) return m;
  }
  return null;
}

export function VerdictMatrix({
  verdicts,
  pendingCaseNames = [],
  runningCaseName = null,
  isRunning = false,
  selectedVerdict = null,
  onSelect,
  onOpenCase,
}: VerdictMatrixProps) {
  const [filter, setFilter] = useState<MatrixFilter>('all');

  const { rows, columns } = useMemo(() => {
    const rowMap = new Map<string, RowModel>();
    const columnKeys = new Set<string>();
    const columnLabels = new Map<string, string>();

    for (const v of verdicts) {
      const caseName = caseNameOf(v);
      const col = reqKeyOf(v);
      columnKeys.add(col);
      if (!columnLabels.has(col)) columnLabels.set(col, reqLabelOf(v));
      let row = rowMap.get(caseName);
      if (!row) {
        const caseId =
          metaString(v.metadata, 'case_id') ??
          metaString(v.metadata, 'case_name') ??
          caseName;
        row = { caseName, caseId, cells: new Map(), pending: false, hasFailure: false, mode: null };
        rowMap.set(caseName, row);
      }
      if (!row.cells.has(col)) row.cells.set(col, v);
      if (v.verdict === 'fail' || v.verdict === 'error') row.hasFailure = true;
    }

    // Pending (selected-but-not-run) cases become their own rows.
    for (const name of pendingCaseNames) {
      if (!rowMap.has(name)) {
        rowMap.set(name, { caseName: name, caseId: name, cells: new Map(), pending: true, hasFailure: false, mode: null });
      }
    }

    // The per-row computed mode (`mode ƒ` column). Never for pending rows.
    for (const row of rowMap.values()) {
      row.mode = row.pending ? null : rowModeOf(row.cells);
    }

    // Deterministic column order: case-level column first, then requirements
    // in first-seen order.
    const cols = [...columnKeys].sort((a, b) => {
      if (a === CASE_COLUMN) return -1;
      if (b === CASE_COLUMN) return 1;
      return 0;
    });
    if (cols.length === 0) cols.push(CASE_COLUMN);
    if (!columnLabels.has(CASE_COLUMN)) columnLabels.set(CASE_COLUMN, 'verdict');

    return {
      rows: [...rowMap.values()],
      columns: cols.map((key) => ({ key, label: columnLabels.get(key) ?? key })),
    };
  }, [verdicts, pendingCaseNames]);

  const totals = useMemo(() => computeTotals(verdicts, rows, isRunning, runningCaseName), [verdicts, rows, isRunning, runningCaseName]);

  const visibleRows = useMemo(() => {
    switch (filter) {
      case 'failing':
        return rows.filter((r) => r.hasFailure);
      case 'not-run':
        return rows.filter((r) => r.pending);
      default:
        return rows;
    }
  }, [rows, filter]);

  const cellStateFor = (row: RowModel, col: string): CellState => {
    const verdict = row.cells.get(col);
    if (verdict) {
      // A bare objective mints no verdict — render the teaching absence,
      // never the wire's Inconclusive (1e).
      if (isBareObjective(verdict)) return { kind: 'bare-objective' };
      return { kind: verdict.verdict, verdict };
    }
    if (row.pending) {
      const running = isRunning && (runningCaseName === row.caseName || runningCaseName == null);
      return running ? { kind: 'running' } : { kind: 'not-run' };
    }
    return { kind: 'absent' };
  };

  const headerCellStyle = stickyHeaderCellStyle({
    padding: '0 10px',
    textAlign: 'center',
    background: 'var(--surface-panel)',
    borderBottom: '1px solid var(--border-hairline)',
    fontWeight: 700,
    fontSize: 10,
    textTransform: 'uppercase',
    letterSpacing: '0.04em',
    color: 'var(--text-muted)',
    whiteSpace: 'nowrap',
  });
  const cornerHeaderStyle = stickyFirstColumnCellStyle(
    { ...headerCellStyle, textAlign: 'left', minWidth: 220 },
    { isHeader: true },
  );
  const rowHeaderStyle = stickyFirstColumnCellStyle({
    padding: '0 10px',
    textAlign: 'left',
    fontSize: 11,
    fontWeight: 500,
    whiteSpace: 'nowrap',
    color: 'var(--text-primary)',
    background: 'var(--surface-sunken)',
    borderBottom: '1px solid var(--border-hairline)',
  });

  const modeHeaderStyle = stickyHeaderCellStyle({
    padding: '0 12px 0 10px',
    textAlign: 'right',
    background: 'var(--surface-panel)',
    borderBottom: '1px solid var(--border-hairline)',
    fontWeight: 700,
    fontSize: 10,
    textTransform: 'uppercase',
    letterSpacing: '0.04em',
    color: 'var(--text-muted)',
    whiteSpace: 'nowrap',
  });

  return (
    <div data-testid="verdict-matrix" className="flex flex-col h-full w-full min-h-0">
      <MatrixToolbar filter={filter} onFilter={setFilter} totals={totals} />
      {rows.length === 0 ? (
        <div
          data-testid="verdict-matrix-empty"
          className="flex flex-1 items-center justify-center"
          style={{ color: 'var(--text-muted)', fontSize: 12 }}
        >
          Select cases and run verification — the matrix appears here.
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-auto">
          <table style={denseTableStyle} data-testid="verdict-matrix-table">
            <thead>
              <tr>
                <th scope="col" style={cornerHeaderStyle} data-testid="verdict-matrix-corner">
                  Case
                </th>
                {columns.map((col) => (
                  <th key={col.key} scope="col" style={headerCellStyle} title={col.label}>
                    {col.label}
                  </th>
                ))}
                <th
                  scope="col"
                  style={modeHeaderStyle}
                  data-testid="verdict-matrix-mode-header"
                  title="How the shown verdict was computed (B10 layer 2) — not what the case declares"
                >
                  mode ƒ
                </th>
              </tr>
            </thead>
            <tbody>
              {visibleRows.map((row) => (
                <tr
                  key={row.caseName}
                  data-testid={`verdict-matrix-row-${row.caseName}`}
                  data-case-id={row.caseId}
                  onDoubleClick={onOpenCase ? () => onOpenCase(row.caseId, row.mode) : undefined}
                  style={onOpenCase ? { cursor: 'pointer' } : undefined}
                >
                  <th
                    scope="row"
                    style={rowHeaderStyle}
                    title={onOpenCase ? `${row.caseName} — double-click to open the case` : row.caseName}
                  >
                    {row.caseName}
                  </th>
                  {columns.map((col) => {
                    const state = cellStateFor(row, col.key);
                    return (
                      <td
                        key={col.key}
                        style={cellTdStyle}
                        data-testid={`verdict-matrix-cell-${row.caseName}-${col.key}`}
                      >
                        <MatrixCell
                          state={state}
                          caseName={row.caseName}
                          column={col.label}
                          selected={
                            state.kind !== 'not-run' &&
                            state.kind !== 'running' &&
                            state.kind !== 'absent' &&
                            state.kind !== 'bare-objective' &&
                            selectedVerdict != null &&
                            state.verdict === selectedVerdict
                          }
                          onSelect={onSelect}
                        />
                      </td>
                    );
                  })}
                  <td
                    style={modeCellStyle}
                    data-testid={`verdict-matrix-rowmode-${row.caseName}`}
                  >
                    {row.mode ? (
                      <EvaluationModeBadge
                        mode={row.mode}
                        size="compact"
                        testId={`verdict-matrix-rowmode-badge-${row.caseName}`}
                      />
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <div
        data-testid="verdict-matrix-mode-footnote"
        className="shrink-0"
        style={{
          padding: '4px 12px',
          fontSize: 10,
          lineHeight: 1.4,
          color: 'var(--text-muted)',
          borderTop: '1px solid var(--border-hairline)',
        }}
      >
        row mode ƒ = how the shown verdict was computed, not what the case
        declares · external verdicts never enter this rollup
        {onOpenCase ? ' · chip click selects · double-click a row to open the case' : ''}
      </div>
    </div>
  );
}

// ── Toolbar (filter tabs + rollup) ──────────────────────────────────

interface Totals {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
  notRun: number;
  running: number;
}

function computeTotals(
  verdicts: Verdict[],
  rows: RowModel[],
  isRunning: boolean,
  runningCaseName: string | null,
): Totals {
  const totals: Totals = { pass: 0, fail: 0, inconclusive: 0, error: 0, notRun: 0, running: 0 };
  for (const v of verdicts) totals[v.verdict] += 1;
  for (const row of rows) {
    if (!row.pending) continue;
    if (isRunning && (runningCaseName === row.caseName || runningCaseName == null)) totals.running += 1;
    else totals.notRun += 1;
  }
  return totals;
}

function MatrixToolbar({
  filter,
  onFilter,
  totals,
}: {
  filter: MatrixFilter;
  onFilter: (f: MatrixFilter) => void;
  totals: Totals;
}) {
  const tabs: Array<{ id: MatrixFilter; label: string }> = [
    { id: 'all', label: 'all' },
    { id: 'failing', label: 'failing' },
    { id: 'not-run', label: 'not run' },
  ];
  return (
    <div
      data-testid="verdict-matrix-toolbar"
      className="flex items-center gap-2 px-3 shrink-0"
      style={{ height: 34, borderBottom: '1px solid var(--border-hairline)' }}
    >
      <div role="tablist" aria-label="Verdict filter" className="flex items-center gap-1">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={filter === tab.id}
            data-testid={`verdict-matrix-filter-${tab.id}`}
            data-active={filter === tab.id}
            onClick={() => onFilter(tab.id)}
            style={filterTabStyle(filter === tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </div>
      <div
        data-testid="verdict-matrix-rollup"
        className="mono-text flex items-center gap-2"
        style={{ marginLeft: 'auto', fontSize: 11 }}
      >
        <RollupCount glyph="✓" label="pass" count={totals.pass} color="var(--verdict-pass)" />
        <RollupCount glyph="✗" label="fail" count={totals.fail} color="var(--verdict-fail)" />
        <RollupCount glyph="?" label="inconclusive" count={totals.inconclusive} color="var(--verdict-inconclusive)" />
        <RollupCount glyph="⨯" label="error" count={totals.error} color="var(--verdict-error)" />
        <span style={{ color: 'var(--text-muted)' }}>·</span>
        <span aria-label={`not run: ${totals.notRun}`} style={{ color: 'var(--text-muted)' }}>
          {totals.notRun} not run
        </span>
        {totals.running > 0 && (
          <span className="flex items-center gap-1" style={{ color: 'var(--accent-fg)' }}>
            <Ninebar compact size={11} label="running" />
            {totals.running} running
          </span>
        )}
      </div>
    </div>
  );
}

function RollupCount({ glyph, label, count, color }: { glyph?: string; label: string; count: number; color: string }) {
  return (
    <span aria-label={`${label}: ${count}`} style={{ color }}>
      {glyph ? <span aria-hidden="true">{glyph} </span> : null}
      {count}
    </span>
  );
}

// ── Cell renderer ───────────────────────────────────────────────────

const CELL_GLYPH: Record<'pass' | 'fail' | 'inconclusive' | 'error', string> = {
  pass: '✓', // ✓
  fail: '✗', // ✗
  inconclusive: '?',
  error: '⨯', // ⨯
};

function MatrixCell({
  state,
  caseName,
  column,
  selected,
  onSelect,
}: {
  state: CellState;
  caseName: string;
  column: string;
  selected: boolean;
  onSelect?: (v: Verdict) => void;
}) {
  if (state.kind === 'absent') {
    return <span aria-label={`No verdict for ${caseName} · ${column}`} style={{ opacity: 0.35 }}>{'—'}</span>;
  }
  if (state.kind === 'bare-objective') {
    // A bare objective mints no verdict — not even inconclusive (1e).
    return (
      <span
        data-cell-state="bare-objective"
        aria-label={`${caseName}: a bare objective verifies nothing — add verify <req>; (no verdict is minted for a case with no checks)`}
        title="A bare objective verifies nothing — add `verify <req>;`. No verdict is minted for a case with no checks."
        style={{ color: 'var(--text-muted)', fontSize: 10 }}
      >
        no checks
      </span>
    );
  }
  if (state.kind === 'not-run') {
    return (
      <span
        data-cell-state="not-run"
        aria-label={`Not run: ${caseName} · ${column}`}
        style={{ ...pillBase, border: '1px solid var(--border-hairline)', opacity: 0.5 }}
      />
    );
  }
  if (state.kind === 'running') {
    return (
      <span
        data-cell-state="running"
        aria-label={`Running: ${caseName} · ${column}`}
        style={{ ...pillBase, border: '1px solid var(--accent)', display: 'inline-flex', alignItems: 'center', justifyContent: 'center' }}
      >
        <Ninebar compact size={9} label={`running ${caseName}`} />
      </span>
    );
  }

  const kind = state.kind;
  const verdict = state.verdict;
  const label = `${caseName} · ${column}: ${kind}`;
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-cell-state={kind}
      data-verdict-kind={kind}
      data-selected={selected || undefined}
      onClick={() => onSelect?.(verdict)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onSelect?.(verdict);
        }
      }}
      disabled={!onSelect}
      style={cellPillStyle(kind, selected, !!onSelect)}
    >
      <span aria-hidden="true">{CELL_GLYPH[kind]}</span>
    </button>
  );
}

// ── Styles ──────────────────────────────────────────────────────────

const cellTdStyle: CSSProperties = {
  padding: '2px 6px',
  textAlign: 'center',
  verticalAlign: 'middle',
  borderBottom: '1px solid var(--border-hairline)',
};

// The trailing `mode ƒ` column — right-aligned, dense (compact badge).
const modeCellStyle: CSSProperties = {
  padding: '2px 12px 2px 10px',
  textAlign: 'right',
  verticalAlign: 'middle',
  whiteSpace: 'nowrap',
  borderBottom: '1px solid var(--border-hairline)',
};

const pillBase: CSSProperties = {
  display: 'inline-block',
  width: 26,
  height: 14,
  borderRadius: 999,
  boxSizing: 'border-box',
};

function cellPillStyle(
  kind: 'pass' | 'fail' | 'inconclusive' | 'error',
  selected: boolean,
  interactive: boolean,
): CSSProperties {
  const base: CSSProperties = {
    ...pillBase,
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    fontSize: 9,
    fontWeight: 700,
    lineHeight: 1,
    padding: 0,
    cursor: interactive ? 'pointer' : 'default',
    // Amber selection echo — selection-only, never a verdict signal.
    boxShadow: selected ? '0 0 0 2.5px var(--accent-tint)' : 'none',
  };
  switch (kind) {
    case 'pass':
      return { ...base, background: 'var(--verdict-pass)', color: 'var(--text-primary)', border: '1px solid var(--verdict-pass)' };
    case 'fail':
      return { ...base, background: 'var(--verdict-fail)', color: 'var(--text-primary)', border: '1px solid var(--verdict-fail)' };
    case 'inconclusive':
      // hollow + dashed, muted — never filled.
      return { ...base, background: 'transparent', color: 'var(--verdict-inconclusive)', border: '1px dashed var(--verdict-inconclusive)' };
    case 'error':
      // hatched, ink border — the distinct "couldn't evaluate" channel.
      return {
        ...base,
        color: 'var(--text-primary)',
        border: '1px solid var(--text-primary)',
        backgroundImage:
          'repeating-linear-gradient(45deg, transparent 0 3px, color-mix(in srgb, var(--text-primary) 22%, transparent) 3px 5px)',
      };
  }
}

function filterTabStyle(active: boolean): CSSProperties {
  return {
    padding: '3px 10px',
    borderRadius: 4,
    fontSize: 12,
    cursor: 'pointer',
    background: active ? 'var(--surface-raised)' : 'transparent',
    color: active ? 'var(--text-primary)' : 'var(--text-muted)',
    border: active ? '1px solid var(--border-hairline)' : '1px solid transparent',
  };
}
