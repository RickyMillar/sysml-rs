/**
 * SweepTableViewer — R5.2 streaming sweep results table.
 *
 * Rows: one per `ChildDescriptor`. Columns: param1, param2, …, rollup
 * verdict, status, plus a right-edge numeric column for the currently-
 * selected metric. Columns are sortable by header click. Clicking a row
 * fires `onChildSelect(child)` which DD wires to drill into the Run
 * workflow at that sweep point.
 *
 * Streaming-friendly:
 *   - `pending` rows render a pulsing skeleton bar in the metric column.
 *     The pulse is disabled when the OS reports `prefers-reduced-motion`.
 *   - `running` rows render a slower pulse + a tiny spinner glyph.
 *   - `failed` rows render red with `child.reason` inline.
 *   - Empty input renders the same dashed empty-state as PassFailGrid so
 *     the feel is consistent across the kit.
 *
 * The viewer is registered as `{ kind: 'sweep-table' }`. Sweep-specific
 * data shape lives on the local `SweepTableData` interface — it is NOT
 * exported via `types.ts` because BB's shell always hands us a
 * `ChildDescriptor[]` directly (the kind discriminator is still useful
 * for programmatic viewer dispatch).
 */
import { useMemo, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import type { AxesConfig, ResultData, ResultViewer } from './types';
import {
  type ChildDescriptor,
  type SweepMetricId,
  collectOutcomeNames,
  collectParamNames,
  extractorFor,
  outcomeReading,
  outcomeUnit,
  rollupVerdict,
  toNumber,
} from './sweepViewerHelpers';
import { OutcomeSparkline, describeSeries } from './OutcomeSparkline';
import {
  denseBodyRowStyle,
  denseTableStyle,
  stickyFirstColumnCellStyle,
  stickyHeaderCellStyle,
} from './tableDensity';

// ── Payload ────────────────────────────────────────────────────────

export interface SweepTableConfig {
  /** Which metric to show in the right-edge column. */
  metric?: SweepMetricId;
  /** Invoked when the user clicks / keys-into a row. */
  onChildSelect?: (child: ChildDescriptor) => void;
  /** Override metric column label — default is derived from `metric`. */
  metricLabel?: string;
}

export interface SweepTableData extends ResultData {
  kind: 'sweep-table';
  children: ChildDescriptor[];
  config?: SweepTableConfig;
}

// ── Sort model ─────────────────────────────────────────────────────

type SortKey =
  | { kind: 'index' }
  | { kind: 'param'; name: string }
  | { kind: 'outcome'; name: string }
  | { kind: 'status' }
  | { kind: 'verdict' }
  | { kind: 'metric' };

interface SortState {
  key: SortKey;
  dir: 'asc' | 'desc';
}

function sameKey(a: SortKey, b: SortKey): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'param' && b.kind === 'param') return a.name === b.name;
  return true;
}

// ── Colour tokens ──────────────────────────────────────────────────

const STATUS_COLOUR: Record<ChildDescriptor['status'], string> = {
  // "not-yet-evaluated" doubles onto --verdict-inconclusive per tokens.css.
  pending: 'var(--verdict-inconclusive)',
  running: 'var(--sim-state-active)',
  complete: 'var(--sim-state-completed)',
  failed: 'var(--sim-state-blocked)',
};

const STATUS_LABEL: Record<ChildDescriptor['status'], string> = {
  pending: 'Pending',
  running: 'Running',
  complete: 'Complete',
  failed: 'Failed',
};

// ── Skeleton + spinner pulses (reduced-motion-aware) ───────────────

/**
 * CSS keyframes are injected lazily so the module stays tree-shakeable.
 * Vitest/jsdom tolerates the `<style>` tag; Vite hoists it once.
 */
const STYLE_ID = 'sweep-table-viewer-style';
function ensureStyle() {
  if (typeof document === 'undefined') return;
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  style.textContent = `
    @keyframes sweep-table-pulse {
      0% { opacity: 0.35; }
      50% { opacity: 0.9; }
      100% { opacity: 0.35; }
    }
    .sweep-table-skeleton {
      display: inline-block;
      height: 8px;
      width: 64px;
      border-radius: 4px;
      background: color-mix(in srgb, var(--outline-variant) 45%, transparent);
      animation: sweep-table-pulse 1.4s ease-in-out infinite;
    }
    .sweep-table-skeleton.running {
      animation-duration: 0.9s;
      background: color-mix(in srgb, var(--sim-state-active) 40%, transparent);
    }
    @media (prefers-reduced-motion: reduce) {
      .sweep-table-skeleton {
        animation: none !important;
        opacity: 0.6;
      }
    }
  `;
  document.head.appendChild(style);
}

// ── Component ──────────────────────────────────────────────────────

interface SweepTableProps {
  data: SweepTableData;
  axes: AxesConfig;
}

function SweepTable({ data, axes }: SweepTableProps) {
  ensureStyle();
  const { children, config } = data;
  const metric = config?.metric ?? 'fail_count';
  const metricLabel = config?.metricLabel ?? (metric === 'fail_count' ? 'Fails' : 'Margin');
  const onChildSelect = config?.onChildSelect;
  const extractor = useMemo(() => extractorFor(metric), [metric]);
  const paramNames = useMemo(() => collectParamNames(children), [children]);
  // One column per measured outcome, sitting immediately right of the inputs:
  // the table reads left-to-right as "what was varied → what came out".
  const outcomeNames = useMemo(() => collectOutcomeNames(children), [children]);

  const [sort, setSort] = useState<SortState>({ key: { kind: 'index' }, dir: 'asc' });

  const sorted = useMemo(() => {
    if (children.length === 0) return children;
    const sign = sort.dir === 'asc' ? 1 : -1;
    const next = [...children];
    next.sort((a, b) => {
      const k = sort.key;
      if (k.kind === 'index') return sign * (a.index - b.index);
      if (k.kind === 'param') {
        const av = toNumber(a.params[k.name]);
        const bv = toNumber(b.params[k.name]);
        if (Number.isNaN(av) && Number.isNaN(bv)) return a.index - b.index;
        if (Number.isNaN(av)) return 1;
        if (Number.isNaN(bv)) return -1;
        return sign * (av - bv);
      }
      if (k.kind === 'outcome') {
        const av = outcomeReading(a, k.name)?.value ?? Number.NaN;
        const bv = outcomeReading(b, k.name)?.value ?? Number.NaN;
        // Unreadable outcomes sort last in either direction — they are not
        // "small", they are absent.
        if (Number.isNaN(av) && Number.isNaN(bv)) return a.index - b.index;
        if (Number.isNaN(av)) return 1;
        if (Number.isNaN(bv)) return -1;
        return sign * (av - bv);
      }
      if (k.kind === 'status') {
        return sign * a.status.localeCompare(b.status);
      }
      if (k.kind === 'verdict') {
        const av = rollupVerdict(a) ?? '';
        const bv = rollupVerdict(b) ?? '';
        return sign * av.localeCompare(bv);
      }
      // metric
      const ma = extractor(a);
      const mb = extractor(b);
      if (Number.isNaN(ma) && Number.isNaN(mb)) return a.index - b.index;
      if (Number.isNaN(ma)) return 1;
      if (Number.isNaN(mb)) return -1;
      return sign * (ma - mb);
    });
    return next;
  }, [children, sort, extractor]);

  if (children.length === 0) {
    return <EmptyState axes={axes} />;
  }

  const toggleSort = (key: SortKey) => {
    setSort((prev) =>
      sameKey(prev.key, key)
        ? { key, dir: prev.dir === 'asc' ? 'desc' : 'asc' }
        : { key, dir: 'asc' },
    );
  };

  const ariaSortFor = (key: SortKey): 'ascending' | 'descending' | 'none' =>
    sameKey(sort.key, key) ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none';

  const tableStyle: CSSProperties = {
    ...denseTableStyle,
    width: axes.width ?? '100%',
  };

  const headerStyle: CSSProperties = stickyHeaderCellStyle({
    padding: '8px 12px',
    textAlign: 'left',
    background: 'color-mix(in srgb, var(--outline-variant) 10%, transparent)',
    borderBottom: '1px solid color-mix(in srgb, var(--outline-variant) 30%, transparent)',
    fontWeight: 600,
    cursor: 'pointer',
    userSelect: 'none',
    whiteSpace: 'nowrap',
  });

  // Corner cell: the "#" header is both the sticky header row AND the
  // sticky first column.
  const cornerHeaderStyle: CSSProperties = stickyFirstColumnCellStyle(headerStyle, {
    isHeader: true,
  });

  const cellStyle: CSSProperties = {
    ...denseBodyRowStyle,
    padding: '6px 12px',
    borderBottom: '1px solid color-mix(in srgb, var(--outline-variant) 15%, transparent)',
    whiteSpace: 'nowrap',
  };

  // First body column ("#") stays visible while scrolling horizontally.
  const firstColumnCellStyle: CSSProperties = stickyFirstColumnCellStyle(cellStyle);

  return (
    <div
      data-testid="sweep-table"
      style={{ display: 'flex', flexDirection: 'column', height: axes.height, overflow: 'auto' }}
    >
      <table style={tableStyle} data-testid="sweep-table-el">
        <thead>
          <tr>
            <HeaderCell
              label="#"
              testId="header-index"
              style={cornerHeaderStyle}
              onClick={() => toggleSort({ kind: 'index' })}
              ariaSort={ariaSortFor({ kind: 'index' })}
              active={sort.key.kind === 'index'}
              dir={sort.dir}
            />
            {paramNames.map((name) => (
              <HeaderCell
                key={name}
                label={name}
                testId={`header-param-${name}`}
                style={headerStyle}
                onClick={() => toggleSort({ kind: 'param', name })}
                ariaSort={ariaSortFor({ kind: 'param', name })}
                active={sort.key.kind === 'param' && sort.key.name === name}
                dir={sort.dir}
              />
            ))}
            {outcomeNames.map((name) => {
              const unit = outcomeUnit(children, name);
              return (
                <HeaderCell
                  key={`outcome-${name}`}
                  label={unit ? `${name} (${unit})` : name}
                  testId={`header-outcome-${name}`}
                  style={{ ...headerStyle, textAlign: 'right' }}
                  onClick={() => toggleSort({ kind: 'outcome', name })}
                  ariaSort={ariaSortFor({ kind: 'outcome', name })}
                  active={sort.key.kind === 'outcome' && sort.key.name === name}
                  dir={sort.dir}
                />
              );
            })}
            <HeaderCell
              label="Status"
              testId="header-status"
              style={headerStyle}
              onClick={() => toggleSort({ kind: 'status' })}
              ariaSort={ariaSortFor({ kind: 'status' })}
              active={sort.key.kind === 'status'}
              dir={sort.dir}
            />
            <HeaderCell
              label="Verdict"
              testId="header-verdict"
              style={headerStyle}
              onClick={() => toggleSort({ kind: 'verdict' })}
              ariaSort={ariaSortFor({ kind: 'verdict' })}
              active={sort.key.kind === 'verdict'}
              dir={sort.dir}
            />
            <HeaderCell
              label={metricLabel}
              testId="header-metric"
              style={{ ...headerStyle, textAlign: 'right' }}
              onClick={() => toggleSort({ kind: 'metric' })}
              ariaSort={ariaSortFor({ kind: 'metric' })}
              active={sort.key.kind === 'metric'}
              dir={sort.dir}
            />
          </tr>
        </thead>
        <tbody>
          {sorted.map((child) => (
            <SweepRow
              key={child.session_id}
              child={child}
              paramNames={paramNames}
              outcomeNames={outcomeNames}
              metricLabel={metricLabel}
              extractor={extractor}
              cellStyle={cellStyle}
              firstColumnCellStyle={firstColumnCellStyle}
              onClick={onChildSelect}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── Header cell ────────────────────────────────────────────────────

interface HeaderCellProps {
  label: string;
  testId: string;
  style: CSSProperties;
  onClick: () => void;
  ariaSort: 'ascending' | 'descending' | 'none';
  active: boolean;
  dir: 'asc' | 'desc';
}

function HeaderCell({ label, testId, style, onClick, ariaSort, active, dir }: HeaderCellProps) {
  const indicator = active ? (dir === 'asc' ? ' \u25B2' : ' \u25BC') : '';
  return (
    <th
      scope="col"
      style={style}
      role="columnheader"
      aria-sort={ariaSort}
      tabIndex={0}
      data-testid={testId}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onClick();
        }
      }}
    >
      {label}
      {indicator}
    </th>
  );
}

// ── Row ────────────────────────────────────────────────────────────

interface SweepRowProps {
  child: ChildDescriptor;
  paramNames: string[];
  outcomeNames: string[];
  metricLabel: string;
  extractor: (child: ChildDescriptor) => number;
  cellStyle: CSSProperties;
  firstColumnCellStyle: CSSProperties;
  onClick?: (child: ChildDescriptor) => void;
}

function SweepRow({
  child,
  paramNames,
  outcomeNames,
  extractor,
  cellStyle,
  firstColumnCellStyle,
  onClick,
}: SweepRowProps) {
  const rollup = rollupVerdict(child);
  const metricValue = extractor(child);
  const failed = child.status === 'failed';
  const isPending = child.status === 'pending' || child.status === 'running';

  const rowStyle: CSSProperties = {
    cursor: onClick ? 'pointer' : 'default',
    background: failed
      ? 'color-mix(in srgb, var(--sim-state-blocked) 8%, transparent)'
      : undefined,
  };

  const handleClick = () => {
    if (onClick) onClick(child);
  };

  return (
    <tr
      data-testid={`sweep-row-${child.index}`}
      data-status={child.status}
      onClick={handleClick}
      onKeyDown={(e) => {
        if (!onClick) return;
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onClick(child);
        }
      }}
      tabIndex={onClick ? 0 : undefined}
      style={rowStyle}
    >
      <td style={firstColumnCellStyle}>{child.index}</td>
      {paramNames.map((name) => {
        const raw = child.params[name];
        return (
          <td key={name} style={cellStyle} data-testid={`sweep-cell-${child.index}-${name}`}>
            {formatParam(raw)}
          </td>
        );
      })}
      {outcomeNames.map((name) => (
        <td
          key={`outcome-${name}`}
          style={{ ...cellStyle, textAlign: 'right' }}
          data-testid={`sweep-outcome-${child.index}-${name}`}
        >
          <OutcomeCell child={child} name={name} isPending={isPending} />
        </td>
      ))}
      <td style={cellStyle}>
        <StatusPill status={child.status} />
      </td>
      <td style={cellStyle}>
        {rollup ? (
          <span
            data-verdict-kind={rollup}
            style={{
              color: verdictColour(rollup),
              fontWeight: 600,
              textTransform: 'capitalize',
            }}
          >
            {rollup}
          </span>
        ) : (
          <span style={{ opacity: 0.5 }}>—</span>
        )}
      </td>
      <td style={{ ...cellStyle, textAlign: 'right' }}>
        {isPending ? (
          <span
            className={`sweep-table-skeleton${child.status === 'running' ? ' running' : ''}`}
            aria-label={`${STATUS_LABEL[child.status]} — metric not ready`}
            data-testid={`sweep-skeleton-${child.index}`}
          />
        ) : failed ? (
          <span
            style={{ color: 'var(--sim-state-blocked)', fontSize: 11 }}
            data-testid={`sweep-failure-${child.index}`}
          >
            {child.reason ?? 'Failed'}
          </span>
        ) : Number.isFinite(metricValue) ? (
          <span data-testid={`sweep-metric-${child.index}`}>{formatMetric(metricValue)}</span>
        ) : (
          <span style={{ opacity: 0.5 }}>—</span>
        )}
      </td>
    </tr>
  );
}

function formatParam(value: unknown): ReactNode {
  if (value == null) return <span style={{ opacity: 0.5 }}>—</span>;
  if (typeof value === 'number') return Number.isInteger(value) ? value : value.toFixed(3);
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function formatMetric(value: number): string {
  if (Number.isInteger(value)) return String(value);
  return value.toFixed(3);
}

function verdictColour(kind: NonNullable<ReturnType<typeof rollupVerdict>>): string {
  if (kind === 'pass') return 'var(--verdict-pass)';
  if (kind === 'fail') return 'var(--verdict-fail)';
  if (kind === 'inconclusive') return 'var(--verdict-inconclusive)';
  return 'var(--verdict-error)';
}

// ── Status pill ────────────────────────────────────────────────────

/**
 * One measured outcome for one child.
 *
 * Three renderings, because there are three genuinely different states and
 * collapsing any two of them would be a lie:
 *
 *   still running   → the same skeleton the metric column uses
 *   read            → the number
 *   could not read  → "unavailable", with the backend's reason on hover
 *
 * There is deliberately no fourth branch that prints `0`. The backend
 * distinguishes "no finite sample" from "settled at zero" (`OutcomeReading`
 * carries `value?` plus `error?`), and that distinction has to survive all
 * the way to the cell or the table quietly invents results.
 */
function OutcomeCell({
  child,
  name,
  isPending,
}: {
  child: ChildDescriptor;
  name: string;
  isPending: boolean;
}) {
  const reading = outcomeReading(child, name);

  if (isPending && !reading) {
    return (
      <span
        className={`sweep-table-skeleton${child.status === 'running' ? ' running' : ''}`}
        aria-hidden="true"
      />
    );
  }

  if (!reading || reading.error !== undefined || typeof reading.value !== 'number') {
    const reason =
      reading?.error ??
      (child.status === 'failed'
        ? 'the child run failed before this outcome was measured'
        : `'${name}' was not measured on this child`);
    return (
      <span
        data-testid={`sweep-outcome-unavailable-${child.index}-${name}`}
        title={reason}
        style={{ opacity: 0.6, fontStyle: 'italic', color: 'var(--sim-state-blocked, inherit)' }}
      >
        unavailable
      </span>
    );
  }

  // Number AND shape. The number says where the run ended; the shape says
  // whether it was going anywhere when it stopped — the question a column of
  // near-identical readings should always raise and never used to.
  const series = reading.series ?? [];
  const description = describeSeries(series, reading.unit);
  return (
    <span
      data-testid={`sweep-outcome-value-${child.index}-${name}`}
      title={description}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 8,
        justifyContent: 'flex-end',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {series.length >= 2 && (
        <OutcomeSparkline
          series={series}
          title={description}
          testId={`sweep-outcome-spark-${child.index}-${name}`}
        />
      )}
      {formatOutcome(reading.value)}
    </span>
  );
}

/**
 * Render an outcome value at a readable precision without pretending to a
 * significance the run does not have. Mirrors `formatParam`'s intent for the
 * input side of the same row.
 */
function formatOutcome(value: number): string {
  if (!Number.isFinite(value)) return '—';
  const magnitude = Math.abs(value);
  if (magnitude !== 0 && (magnitude < 1e-3 || magnitude >= 1e6)) {
    return value.toExponential(3);
  }
  return Number(value.toPrecision(6)).toString();
}

function StatusPill({ status }: { status: ChildDescriptor['status'] }) {
  const colour = STATUS_COLOUR[status];
  return (
    <span
      data-testid={`sweep-status-${status}`}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        padding: '2px 8px',
        borderRadius: 999,
        fontSize: 11,
        background: `color-mix(in srgb, ${colour} 15%, transparent)`,
        color: colour,
        border: `1px solid color-mix(in srgb, ${colour} 35%, transparent)`,
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: colour,
        }}
      />
      {STATUS_LABEL[status]}
    </span>
  );
}

// ── Empty state ────────────────────────────────────────────────────

function EmptyState({ axes }: { axes: AxesConfig }) {
  return (
    <div
      data-testid="sweep-table-empty"
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
      Configure a sweep and launch to populate rows
    </div>
  );
}

// ── Canonical viewer export ────────────────────────────────────────

export const sweepTableViewer: ResultViewer<SweepTableData> = {
  id: 'sweep-table-default',
  kind: 'sweep-table',
  accepts: (data): data is SweepTableData => data.kind === 'sweep-table',
  render: (data, axes) => <SweepTable data={data} axes={axes} />,
};
