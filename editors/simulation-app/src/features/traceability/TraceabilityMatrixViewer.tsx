/**
 * TraceabilityMatrixViewer — requirement×target matrix with verdict
 * cells (R6.2).
 *
 * Rows    = requirements (the TARGET endpoint of each Satisfy/Verify
 *           edge — the mint runs satisfier/case → requirement).
 * Columns = linked satisfiers (parts / constraints / verification cases,
 *           the source endpoint).
 * Cells   = 4-valued verdict badge (`<VerdictBadge size="compact">`).
 *
 * Viewer responsibilities:
 *   - Sticky first column (row label) and first row (column headers)
 *     via `position: sticky` so large matrices scroll without losing
 *     context.
 *   - Sortable column headers (asc ↔ desc, tie-broken by label).
 *   - Compact / roomy density toggle — compact is the default.
 *   - Top-bar filters: only-unsatisfied, only-no-coverage, search.
 *   - Row click → `useSelectionStore.select(uri, reqElementId)` so the
 *     inspector pane focuses on the requirement.
 *
 * Not wired here:
 *   - Column click → target reveal (columns can be constraint /
 *     verification case ids which don't always map to a focusable
 *     element; Agent Q / future work). The row-click path covers
 *     the primary "reveal requirement" UX.
 *   - Verdict overlay — `sysml.trace_matrix` returns the structural
 *     edges only; every cell is `inconclusive` ("not evaluated") until
 *     a verification run annotates the links. That overlay is a
 *     separate feature; this viewer renders whatever verdict the
 *     caller supplies on each `TraceLink`.
 *
 * `prefers-reduced-motion` compliance: hover transitions are
 * `transition: none` so reduced-motion users see the same static
 * badges as everyone else.
 */

import { useCallback, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ChangeEvent, KeyboardEvent as ReactKeyboardEvent } from 'react';
import { VerdictBadge } from '@/components/VerdictBadge';
import { SourcePreviewPopover } from '@/features/editor/SourcePreviewPopover';
import { useSelectionStore } from '@/features/selection/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import type { VerdictKind } from '@/engine/types';
import { filterTraceMatrix, isRowSatisfied, rowLinkCount } from './filterTraceMatrix';
import {
  DEFAULT_TRACE_FILTER,
  type TraceColumn,
  type TraceDensity,
  type TraceFilter,
  type TraceLink,
  type TraceMatrix,
  type TraceRow,
} from './types';

// ── Props ────────────────────────────────────────────────────────────

export interface TraceabilityMatrixViewerProps {
  /** The full matrix — typically derived via `useTraceMatrix` then filtered. */
  data: TraceMatrix;
  /**
   * Workspace URI forwarded to the selection store when a row is
   * clicked. When absent, row clicks are inert (useful for storybook /
   * preview contexts).
   */
  workspaceUri?: string | null;
  /** Initial filter state. Defaults to `DEFAULT_TRACE_FILTER`. */
  initialFilter?: TraceFilter;
  /** Initial density. Defaults to `'compact'`. */
  initialDensity?: TraceDensity;
  /**
   * When `false`, hides the top filter bar. The viewer then runs with
   * whatever filter state was supplied at mount — useful for embeds
   * that want to drive the filter from a parent UI.
   */
  showFilterBar?: boolean;
  /**
   * Injectable selection override for tests. Omit in production — the
   * viewer wires to `useSelectionStore` automatically.
   */
  testHooks?: {
    select?: (uri: string | null, id: string | null) => void;
  };
}

// ── Sort model ───────────────────────────────────────────────────────

type SortKey = 'row' | `col:${string}`;
type SortDir = 'asc' | 'desc';

interface SortState {
  key: SortKey;
  dir: SortDir;
}

/**
 * Severity weight used for sorting — lower is "better".
 *   pass=0, inconclusive=1, error=2, fail=3
 * Keeps `desc` putting failing rows at the top (loud-first).
 */
const SEVERITY: Record<VerdictKind, number> = {
  pass: 0,
  inconclusive: 1,
  error: 2,
  fail: 3,
};

/**
 * Worst-wins verdict for a (row, column) cell, or `null` if no link
 * exists. Exposed so the sort comparator and the cell renderer share
 * one implementation.
 */
function cellVerdict(matrix: TraceMatrix, row: string, col: string): TraceLink | null {
  for (const link of matrix.links) {
    if (link.row === row && link.column === col) return link;
  }
  return null;
}

/** Numeric score used to sort rows by a single column. */
function cellScore(matrix: TraceMatrix, row: string, col: string): number {
  const link = cellVerdict(matrix, row, col);
  if (!link) return -1; // unlinked cells sort below any verdict
  return SEVERITY[link.verdict];
}

// ── Main component ───────────────────────────────────────────────────

export function TraceabilityMatrixViewer(props: TraceabilityMatrixViewerProps) {
  const {
    data,
    workspaceUri = null,
    initialFilter = DEFAULT_TRACE_FILTER,
    initialDensity = 'compact',
    showFilterBar = true,
    testHooks,
  } = props;

  const [filter, setFilter] = useState<TraceFilter>(initialFilter);
  const [density, setDensity] = useState<TraceDensity>(initialDensity);
  const [sort, setSort] = useState<SortState>({ key: 'row', dir: 'asc' });

  // Selection wiring — injectable for tests.
  const storeSelect = useSelectionStore((s) => s.select);
  const select = testHooks?.select ?? storeSelect;
  const setFocusedUri = useWorkspaceStore((s) => s.setFocusedUri);
  const setActiveUtility = useWorkspaceUIStore((s) => s.setActiveUtility);

  /**
   * Phase 3 — promote a hover-preview into the Source utility drawer.
   * Workspace URI is the trace context, not a file URI, so we skip
   * the focusedUri push when it's the workspace placeholder.
   */
  const handleRowPromote = useCallback(
    (rowId: string) => {
      const target = workspaceUri && workspaceUri !== '__workspace__'
        ? workspaceUri
        : null;
      select(target, rowId);
      if (target) setFocusedUri(target);
      setActiveUtility('source');
    },
    [select, setActiveUtility, setFocusedUri, workspaceUri],
  );

  const filtered = useMemo(() => filterTraceMatrix(data, filter), [data, filter]);

  const sortedRows = useMemo(() => {
    const next = [...filtered.rows];
    const { key, dir } = sort;
    const sign = dir === 'asc' ? 1 : -1;
    if (key === 'row') {
      next.sort((a, b) => sign * a.label.localeCompare(b.label));
    } else {
      const column = key.slice('col:'.length);
      next.sort((a, b) => {
        const diff = cellScore(filtered, a.id, column) - cellScore(filtered, b.id, column);
        if (diff !== 0) return sign * diff;
        return a.label.localeCompare(b.label);
      });
    }
    return next;
  }, [filtered, sort]);

  const toggleSort = useCallback((key: SortKey) => {
    setSort((prev) =>
      prev.key === key
        ? { key, dir: prev.dir === 'asc' ? 'desc' : 'asc' }
        : { key, dir: 'asc' },
    );
  }, []);

  const handleRowClick = useCallback(
    (row: TraceRow) => {
      select(workspaceUri, row.id);
    },
    [select, workspaceUri],
  );

  /**
   * URI passed to the per-row preview popover. Workspace-scoped
   * trace matrices use `__workspace__`; `sysml.get_source` resolves
   * element ids workspace-wide there, so we forward as-is.
   */
  const rowPreviewUri = workspaceUri ?? null;

  const handleSearch = useCallback((ev: ChangeEvent<HTMLInputElement>) => {
    const next = ev.target.value;
    setFilter((prev) => ({ ...prev, search: next }));
  }, []);

  const toggleOnlyUnsatisfied = useCallback(() => {
    setFilter((prev) => ({ ...prev, onlyUnsatisfied: !prev.onlyUnsatisfied }));
  }, []);

  const toggleOnlyNoCoverage = useCallback(() => {
    setFilter((prev) => ({ ...prev, onlyNoCoverage: !prev.onlyNoCoverage }));
  }, []);

  const clearFilters = useCallback(() => {
    setFilter(DEFAULT_TRACE_FILTER);
  }, []);

  // Density-driven spacing. Kept together so tweaks stay co-located.
  const spacing = density === 'compact'
    ? { rowPadV: 3, rowPadH: 8, fontSize: 11, cellMinWidth: 96 }
    : { rowPadV: 8, rowPadH: 12, fontSize: 12, cellMinWidth: 128 };

  // ── Render ─────────────────────────────────────────────────────────

  return (
    <div
      data-testid="trace-matrix-viewer"
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        background: 'var(--surface-sunken)',
        color: 'var(--text-primary)',
        overflow: 'hidden',
      }}
    >
      {showFilterBar ? (
        <FilterBar
          filter={filter}
          density={density}
          totalRows={data.rows.length}
          visibleRows={filtered.rows.length}
          onSearchChange={handleSearch}
          onToggleUnsatisfied={toggleOnlyUnsatisfied}
          onToggleNoCoverage={toggleOnlyNoCoverage}
          onDensityChange={setDensity}
          onClear={clearFilters}
        />
      ) : null}

      <div
        data-testid="trace-matrix-scroller"
        style={{
          flex: 1,
          minHeight: 0,
          overflow: 'auto',
          position: 'relative',
        }}
      >
        {data.rows.length === 0 ? (
          <EmptyState
            testId="trace-matrix-empty-total"
            message="No trace edges in this workspace."
            hint="Add `satisfy` / `verify` / `derive` relationships between requirements and model elements to populate the matrix."
          />
        ) : filtered.rows.length === 0 ? (
          <EmptyState
            testId="trace-matrix-empty-filtered"
            message="No requirements match the current filters."
            hint="Try clearing the filter bar."
            action={{ label: 'Clear filters', onClick: clearFilters, testId: 'trace-matrix-clear-filters' }}
          />
        ) : (
          <MatrixTable
            rows={sortedRows}
            columns={filtered.columns}
            matrix={filtered}
            sort={sort}
            onSort={toggleSort}
            onRowClick={handleRowClick}
            onRowPromote={handleRowPromote}
            previewUri={rowPreviewUri}
            spacing={spacing}
            density={density}
          />
        )}
      </div>
    </div>
  );
}

// ── Filter bar ───────────────────────────────────────────────────────

interface FilterBarProps {
  filter: TraceFilter;
  density: TraceDensity;
  totalRows: number;
  visibleRows: number;
  onSearchChange: (ev: ChangeEvent<HTMLInputElement>) => void;
  onToggleUnsatisfied: () => void;
  onToggleNoCoverage: () => void;
  onDensityChange: (d: TraceDensity) => void;
  onClear: () => void;
}

function FilterBar(props: FilterBarProps) {
  const {
    filter,
    density,
    totalRows,
    visibleRows,
    onSearchChange,
    onToggleUnsatisfied,
    onToggleNoCoverage,
    onDensityChange,
    onClear,
  } = props;

  const rowStyle: CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    gap: 8,
    padding: '8px 12px',
    borderBottom: '1px solid var(--border-default)',
    background: 'var(--surface-panel)',
    flexWrap: 'wrap',
  };

  return (
    <div data-testid="trace-matrix-filter-bar" style={rowStyle}>
      <input
        type="search"
        value={filter.search}
        onChange={onSearchChange}
        placeholder="Search requirement label…"
        data-testid="trace-matrix-search"
        aria-label="Search requirements"
        style={{
          padding: '5px 8px',
          fontSize: 'var(--text-xs)',
          background: 'var(--surface-raised)',
          border: '1px solid var(--border-default)',
          borderRadius: 3,
          color: 'var(--text-primary)',
          minWidth: 180,
          flex: 1,
        }}
      />
      <ToggleChip
        label="Only unsatisfied"
        active={filter.onlyUnsatisfied}
        onClick={onToggleUnsatisfied}
        testId="trace-matrix-toggle-unsatisfied"
      />
      <ToggleChip
        label="Only no coverage"
        active={filter.onlyNoCoverage}
        onClick={onToggleNoCoverage}
        testId="trace-matrix-toggle-no-coverage"
      />
      <div role="radiogroup" aria-label="Density" style={{ display: 'inline-flex', gap: 4 }}>
        <ToggleChip
          label="Compact"
          active={density === 'compact'}
          onClick={() => onDensityChange('compact')}
          testId="trace-matrix-density-compact"
        />
        <ToggleChip
          label="Roomy"
          active={density === 'roomy'}
          onClick={() => onDensityChange('roomy')}
          testId="trace-matrix-density-roomy"
        />
      </div>
      <div style={{ flex: 1 }} />
      <span
        data-testid="trace-matrix-count"
        aria-live="polite"
        style={{ fontSize: 11, color: 'var(--text-muted)' }}
      >
        {visibleRows === totalRows
          ? `${totalRows} requirement${totalRows === 1 ? '' : 's'}`
          : `${visibleRows} / ${totalRows}`}
      </span>
      {(filter.search.length > 0 || filter.onlyUnsatisfied || filter.onlyNoCoverage) ? (
        <button
          type="button"
          onClick={onClear}
          data-testid="trace-matrix-clear-filters-inline"
          style={{
            padding: '3px 8px',
            fontSize: 11,
            background: 'transparent',
            color: 'var(--accent-fg)',
            border: '1px solid var(--border-default)',
            borderRadius: 3,
            cursor: 'pointer',
          }}
        >
          Clear
        </button>
      ) : null}
    </div>
  );
}

// ── Toggle chip (shared by filter + density) ─────────────────────────

interface ToggleChipProps {
  label: string;
  active: boolean;
  onClick: () => void;
  testId?: string;
}

function ToggleChip({ label, active, onClick, testId }: ToggleChipProps) {
  const style: CSSProperties = {
    padding: '3px 8px',
    fontSize: 11,
    border: `1px solid ${active ? 'var(--accent-fg)' : 'var(--border-default)'}`,
    background: active ? 'color-mix(in srgb, var(--accent-fg) 18%, transparent)' : 'transparent',
    color: active ? 'var(--accent-fg)' : 'var(--text-muted)',
    borderRadius: 3,
    cursor: 'pointer',
    transition: 'none',
  };
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      data-active={active ? 'true' : 'false'}
      data-testid={testId}
      onClick={onClick}
      style={style}
    >
      {label}
    </button>
  );
}

// ── Matrix table ─────────────────────────────────────────────────────

interface MatrixTableProps {
  rows: TraceRow[];
  columns: TraceColumn[];
  matrix: TraceMatrix;
  sort: SortState;
  onSort: (key: SortKey) => void;
  onRowClick: (row: TraceRow) => void;
  onRowPromote?: (rowId: string) => void;
  previewUri: string | null;
  spacing: { rowPadV: number; rowPadH: number; fontSize: number; cellMinWidth: number };
  density: TraceDensity;
}

function MatrixTable(props: MatrixTableProps) {
  const {
    rows,
    columns,
    matrix,
    sort,
    onSort,
    onRowClick,
    onRowPromote,
    previewUri,
    spacing,
    density,
  } = props;

  const sortIndicator = (key: SortKey): string => {
    if (sort.key !== key) return '';
    return sort.dir === 'asc' ? ' \u25B2' : ' \u25BC';
  };

  // Sticky positioning uses `position: sticky` with `z-index`
  // layering. The top-left header sits above both the header row and
  // the left column so it never gets hidden under them when scrolled.
  const tableStyle: CSSProperties = {
    borderCollapse: 'separate',
    borderSpacing: 0,
    width: '100%',
    fontSize: spacing.fontSize,
    color: 'var(--text-primary)',
    tableLayout: 'auto',
  };

  const stickyHeaderCell: CSSProperties = {
    padding: `${spacing.rowPadV + 2}px ${spacing.rowPadH}px`,
    textAlign: 'left',
    background: 'var(--surface-panel)',
    borderBottom: '1px solid var(--border-default)',
    fontWeight: 600,
    cursor: 'pointer',
    userSelect: 'none',
    whiteSpace: 'nowrap',
    position: 'sticky',
    top: 0,
    zIndex: 2,
    minWidth: spacing.cellMinWidth,
    transition: 'none',
  };

  const stickyCorner: CSSProperties = {
    ...stickyHeaderCell,
    left: 0,
    zIndex: 3,
    // Slightly stronger background so the corner reads as "above" the
    // other stickies.
    background: 'var(--surface-raised)',
    minWidth: 220,
  };

  const bodyCellStyle: CSSProperties = {
    padding: `${spacing.rowPadV}px ${spacing.rowPadH}px`,
    borderBottom: '1px solid color-mix(in srgb, var(--border-default) 15%, transparent)',
    textAlign: 'center',
    verticalAlign: 'middle',
    transition: 'none',
  };

  const rowHeaderStyle: CSSProperties = {
    ...bodyCellStyle,
    textAlign: 'left',
    fontWeight: 500,
    whiteSpace: 'nowrap',
    position: 'sticky',
    left: 0,
    zIndex: 1,
    background: 'var(--surface-sunken)',
    cursor: 'pointer',
    color: 'var(--text-primary)',
  };

  return (
    <table style={tableStyle} data-testid="trace-matrix-table" data-density={density}>
      <thead>
        <tr>
          <th
            scope="col"
            style={stickyCorner}
            onClick={() => onSort('row')}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onSort('row');
              }
            }}
            tabIndex={0}
            role="columnheader"
            aria-sort={
              sort.key === 'row'
                ? sort.dir === 'asc'
                  ? 'ascending'
                  : 'descending'
                : 'none'
            }
            data-testid="trace-matrix-header-row"
          >
            Requirement{sortIndicator('row')}
          </th>
          {columns.map((col) => {
            const key: SortKey = `col:${col.id}`;
            const isSorted = sort.key === key;
            return (
              <th
                key={col.id}
                scope="col"
                style={stickyHeaderCell}
                onClick={() => onSort(key)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    onSort(key);
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
                data-testid={`trace-matrix-header-col-${col.id}`}
              >
                {col.label}
                {sortIndicator(key)}
              </th>
            );
          })}
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => (
          <MatrixRow
            key={row.id}
            row={row}
            columns={columns}
            matrix={matrix}
            onRowClick={onRowClick}
            onRowPromote={onRowPromote}
            previewUri={previewUri}
            bodyCellStyle={bodyCellStyle}
            rowHeaderStyle={rowHeaderStyle}
          />
        ))}
      </tbody>
    </table>
  );
}

// ── Matrix row (per-row state for hover preview) ────────────────────

interface MatrixRowProps {
  row: TraceRow;
  columns: TraceColumn[];
  matrix: TraceMatrix;
  onRowClick: (row: TraceRow) => void;
  onRowPromote?: (rowId: string) => void;
  previewUri: string | null;
  bodyCellStyle: CSSProperties;
  rowHeaderStyle: CSSProperties;
}

function MatrixRow({
  row,
  columns,
  matrix,
  onRowClick,
  onRowPromote,
  previewUri,
  bodyCellStyle,
  rowHeaderStyle,
}: MatrixRowProps) {
  const [hovered, setHovered] = useState(false);
  const headerRef = useRef<HTMLTableCellElement | null>(null);
  const satisfied = isRowSatisfied(matrix, row.id);
  const links = rowLinkCount(matrix, row.id);
  const noCoverage = links === 0;

  const handlePromote = useCallback(() => {
    onRowPromote?.(row.id);
  }, [onRowPromote, row.id]);

  return (
    <tr
      data-testid={`trace-matrix-row-${row.id}`}
      data-row-satisfied={satisfied ? 'true' : 'false'}
      data-row-coverage={noCoverage ? 'none' : 'linked'}
    >
      <th
        ref={headerRef}
        scope="row"
        style={rowHeaderStyle}
        onClick={() => onRowClick(row)}
        onKeyDown={(e: ReactKeyboardEvent<HTMLTableCellElement>) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onRowClick(row);
          }
        }}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onFocus={() => setHovered(true)}
        onBlur={() => setHovered(false)}
        tabIndex={0}
        role="button"
        aria-label={`Reveal requirement ${row.label}`}
        data-testid={`trace-matrix-row-header-${row.id}`}
      >
        <span>{row.label}</span>
        {noCoverage ? (
          <span
            aria-label="No coverage"
            title="No links — this requirement has no satisfy / verify edges"
            style={{
              marginLeft: 6,
              fontSize: 10,
              color: 'var(--text-muted)',
              fontStyle: 'italic',
            }}
          >
            no coverage
          </span>
        ) : null}
        <SourcePreviewPopover
          triggerRef={headerRef}
          triggerHovered={hovered}
          uri={previewUri}
          elementId={row.id}
          onPromote={onRowPromote ? handlePromote : undefined}
          testId={`trace-matrix-preview-${row.id}`}
        />
      </th>
      {columns.map((col) => {
        const link = cellVerdict(matrix, row.id, col.id);
        return (
          <td
            key={col.id}
            style={bodyCellStyle}
            data-testid={`trace-matrix-cell-${row.id}-${col.id}`}
            data-verdict-kind={link?.verdict ?? 'empty'}
          >
            {link ? (
              <VerdictBadge
                verdict={link.verdict}
                name={`${col.label} → ${row.label}`}
                reason={link.reason ?? null}
                size="compact"
              />
            ) : (
              <span
                aria-label={`No link between ${row.label} and ${col.label}`}
                style={{ opacity: 0.4 }}
              >
                {'—'}
              </span>
            )}
          </td>
        );
      })}
    </tr>
  );
}

// ── Empty state ──────────────────────────────────────────────────────

interface EmptyStateProps {
  testId: string;
  message: string;
  hint?: string;
  action?: { label: string; onClick: () => void; testId: string };
}

function EmptyState({ testId, message, hint, action }: EmptyStateProps) {
  return (
    <div
      data-testid={testId}
      role="status"
      style={{
        padding: '32px 20px',
        textAlign: 'center',
        color: 'var(--text-muted)',
        fontSize: 12,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 8,
      }}
    >
      <div style={{ fontStyle: 'italic' }}>{message}</div>
      {hint ? (
        <div style={{ fontSize: 11, opacity: 0.8, maxWidth: 480 }}>{hint}</div>
      ) : null}
      {action ? (
        <button
          type="button"
          onClick={action.onClick}
          data-testid={action.testId}
          style={{
            padding: '4px 12px',
            fontSize: 11,
            background: 'var(--accent)',
            color: 'var(--on-accent)',
            border: 'none',
            borderRadius: 3,
            cursor: 'pointer',
          }}
        >
          {action.label}
        </button>
      ) : null}
    </div>
  );
}
