/**
 * TradeStudyTableViewer — R5.11 Layer 1 EP6 primitive.
 *
 * Renders a trade-study table over a `ChildDescriptor[]` (alternatives)
 * plus a `TradeStudyConfig` (objectives + weights + criteria). Each row
 * is one alternative; columns are:
 *
 *     [✓] | Label | <criterion metric>… | Weighted score | Rank | Pareto
 *
 * Behaviours:
 *   - Column sorting (click header). Default sort is rank ascending.
 *   - Pareto front overlay: alternatives not dominated by any other
 *     (better on ALL criteria) get a highlighted background + "★ Pareto"
 *     chip in the Pareto column.
 *   - Best-weighted-score row (rank 1) gets a gold left border.
 *   - Streaming: rows with `status === 'pending'` show a skeleton in
 *     each metric cell; their weighted score / rank / Pareto are
 *     blanked (they cannot be dominated or score yet).
 *   - Local multi-select via row checkboxes (2..6 enabled window); the
 *     selection state is forwarded to `<PromoteToCompareButton>` in the
 *     toolbar.
 *
 * The viewer is pure render — all scoring lives in `./tradeHelpers`.
 *
 * Boundaries:
 *   - GG owns the config UI (objectives picker, weight sliders). Our
 *     consumer surface is the plain `TradeStudyConfig` object.
 *   - CompareWorkflow (R4.2) is consumed read-only: we push
 *     `session_id`s into `useCompareStore` and `navigate('/run/compare')`.
 *     No Compare internals touched here.
 */
import { useMemo, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import type { AxesConfig, ResultData, ResultViewer } from './types';
import {
  denseBodyRowStyle,
  denseTableStyle,
  stickyFirstColumnCellStyle,
  stickyHeaderCellStyle,
} from './tableDensity';
import {
  buildAlternativeScores,
  computeParetoFront,
  computeWeightedScore,
  objectivesOf,
  rankAlternatives,
  type AlternativeScore,
  type ChildDescriptorLike,
  type TradeCriterion,
} from './tradeHelpers';
import { PromoteToCompareButton } from './PromoteToCompareButton';

// ── Data contract ───────────────────────────────────────────────────

/**
 * Payload accepted by `tradeStudyTableViewer`.
 *
 * The viewer is display-only; it does not know how rows were generated
 * (R5.10 workflow uses a sweep; future workflows may populate the same
 * shape from a stored table).
 */
export interface TradeStudyTableData extends ResultData {
  kind: 'trade-table';
  /** One alternative per row — ordering reproduces in the UI. */
  alternatives: ChildDescriptorLike[];
  /** Scoring criteria (with objectives + weights). */
  criteria: TradeCriterion[];
  /**
   * Optional override for the navigate-on-promote URL. Useful in
   * embeds (Storybook / test harnesses) that don't want the real
   * route. Forwarded to `<PromoteToCompareButton>`.
   */
  promoteNavigateTo?: string;
  /** Show the promote toolbar (default true). */
  showPromote?: boolean;
}

// ── Sorting model ───────────────────────────────────────────────────

type ColumnSortKey =
  | 'label'
  | 'score'
  | 'rank'
  | 'pareto'
  | `criterion:${string}`;
type SortDir = 'asc' | 'desc';

interface SortState {
  key: ColumnSortKey;
  dir: SortDir;
}

const DEFAULT_SORT: SortState = { key: 'rank', dir: 'asc' };

// ── Component ───────────────────────────────────────────────────────

interface TradeStudyTableProps {
  data: TradeStudyTableData;
  axes: AxesConfig;
}

function labelFor(row: ChildDescriptorLike): string {
  return row.label ?? row.id;
}

function isPending(row: ChildDescriptorLike): boolean {
  const s = row.status;
  return s === 'pending' || s === 'running';
}

function TradeStudyTable({ data, axes }: TradeStudyTableProps) {
  const { alternatives, criteria, showPromote = true, promoteNavigateTo } = data;
  const [sort, setSort] = useState<SortState>(DEFAULT_SORT);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());

  // Derive all scoring in one memo — these all depend on the same
  // (alternatives, criteria) pair and share intermediate results.
  const scoring = useMemo(() => {
    const objectives = objectivesOf(criteria);
    // Only materialise scores for rows that have a `completed` status
    // (or no status — treated as "ready"). Pending/running/failed rows
    // participate in the table but not in the Pareto math.
    const alts: AlternativeScore[] = buildAlternativeScores(
      alternatives,
      criteria,
    );

    const readyIdx: number[] = [];
    const readyAlts: AlternativeScore[] = [];
    for (let i = 0; i < alternatives.length; i++) {
      const s = alternatives[i].status;
      // Default (no status field) is treated as "ready" — keeps the
      // viewer usable on hand-crafted fixtures without status plumbing.
      const ready = s === undefined || s === 'completed';
      if (ready) {
        readyIdx.push(i);
        readyAlts.push(alts[i]);
      }
    }

    // Pareto indices are into `readyAlts`; map back to original idx.
    const paretoInReady = new Set(
      computeParetoFront(readyAlts, objectives),
    );
    const paretoIdx = new Set<number>();
    for (let i = 0; i < readyIdx.length; i++) {
      if (paretoInReady.has(i)) paretoIdx.add(readyIdx[i]);
    }

    // Ranking: rank only ready rows; pending rows receive rank = null.
    const rankOrder = rankAlternatives(readyAlts, criteria, objectives);
    const rankByIdx = new Map<number, number>();
    rankOrder.forEach((localIdx, rankZeroBased) => {
      rankByIdx.set(readyIdx[localIdx], rankZeroBased + 1);
    });

    const scores = new Map<number, number>();
    for (let i = 0; i < readyIdx.length; i++) {
      scores.set(
        readyIdx[i],
        computeWeightedScore(readyAlts[i], criteria, objectives),
      );
    }

    // Best-of-ready index = the first entry in rankOrder (mapped back).
    const bestIdx = rankOrder.length > 0 ? readyIdx[rankOrder[0]] : -1;

    return { alts, objectives, paretoIdx, rankByIdx, scores, bestIdx };
  }, [alternatives, criteria]);

  // Build the displayed row order per sort state.
  const sortedIndices = useMemo<number[]>(() => {
    const base = alternatives.map((_, i) => i);
    const { key, dir } = sort;
    const sign = dir === 'asc' ? 1 : -1;

    const cmp = (a: number, b: number): number => {
      let primary = 0;
      switch (key) {
        case 'label':
          primary = labelFor(alternatives[a]).localeCompare(
            labelFor(alternatives[b]),
          );
          break;
        case 'score': {
          const sa = scoring.scores.get(a);
          const sb = scoring.scores.get(b);
          if (sa === undefined && sb === undefined) primary = 0;
          else if (sa === undefined) primary = 1; // pending → push to bottom in asc, top in desc... but asc=1 pushes to end
          else if (sb === undefined) primary = -1;
          else primary = sa - sb;
          break;
        }
        case 'rank': {
          const ra = scoring.rankByIdx.get(a);
          const rb = scoring.rankByIdx.get(b);
          if (ra === undefined && rb === undefined) primary = 0;
          else if (ra === undefined) primary = 1;
          else if (rb === undefined) primary = -1;
          else primary = ra - rb;
          break;
        }
        case 'pareto': {
          const pa = scoring.paretoIdx.has(a) ? 1 : 0;
          const pb = scoring.paretoIdx.has(b) ? 1 : 0;
          // Pareto=true should sort to top in desc; use -1 sign convention
          primary = pa - pb;
          break;
        }
        default: {
          if (key.startsWith('criterion:')) {
            const critKey = key.slice('criterion:'.length);
            const idx = criteria.findIndex((c) => c.key === critKey);
            if (idx === -1) {
              primary = 0;
            } else {
              const va = scoring.alts[a].values[idx];
              const vb = scoring.alts[b].values[idx];
              const aFin = Number.isFinite(va);
              const bFin = Number.isFinite(vb);
              if (!aFin && !bFin) primary = 0;
              else if (!aFin) primary = 1;
              else if (!bFin) primary = -1;
              else primary = va - vb;
            }
          }
        }
      }
      if (primary !== 0) return sign * primary;
      // Stable tie-break by original index.
      return a - b;
    };

    base.sort(cmp);
    return base;
  }, [alternatives, criteria, scoring, sort]);

  const toggleSort = (key: ColumnSortKey) => {
    setSort((prev) =>
      prev.key === key
        ? { key, dir: prev.dir === 'asc' ? 'desc' : 'asc' }
        : { key, dir: key === 'rank' || key === 'label' ? 'asc' : 'desc' },
    );
  };

  const sortIndicator = (key: ColumnSortKey): string => {
    if (sort.key !== key) return '';
    return sort.dir === 'asc' ? ' \u25B2' : ' \u25BC';
  };

  const toggleSelect = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const selectedChildren = useMemo(
    () => alternatives.filter((r) => selectedIds.has(r.id)),
    [alternatives, selectedIds],
  );

  if (alternatives.length === 0) {
    return <EmptyState axes={axes} />;
  }

  const tableStyle: CSSProperties = {
    ...denseTableStyle,
    width: axes.width ?? '100%',
  };

  const headerCellStyle: CSSProperties = stickyHeaderCellStyle({
    padding: '8px 12px',
    textAlign: 'left',
    background: 'color-mix(in srgb, var(--outline-variant) 10%, transparent)',
    borderBottom:
      '1px solid color-mix(in srgb, var(--outline-variant) 30%, transparent)',
    fontWeight: 600,
    cursor: 'pointer',
    userSelect: 'none',
    whiteSpace: 'nowrap',
    transition: 'none',
  });

  // Corner cell: the select-all header is both the sticky header row AND
  // the sticky first column.
  const cornerHeaderStyle: CSSProperties = stickyFirstColumnCellStyle(headerCellStyle, {
    isHeader: true,
  });

  const bodyCellStyle: CSSProperties = {
    ...denseBodyRowStyle,
    padding: '6px 12px',
    borderBottom:
      '1px solid color-mix(in srgb, var(--outline-variant) 15%, transparent)',
    textAlign: 'left',
    verticalAlign: 'middle',
  };

  // First body column (row-select checkbox) stays visible while
  // scrolling horizontally.
  const firstColumnCellStyle: CSSProperties = stickyFirstColumnCellStyle(bodyCellStyle);

  return (
    <div
      data-testid="trade-study-table"
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: axes.height,
        gap: 8,
      }}
    >
      {showPromote ? (
        <div
          data-testid="trade-study-toolbar"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            flexWrap: 'wrap',
            padding: '4px 0',
          }}
        >
          <span
            data-testid="trade-study-selected-count"
            style={{ fontSize: 11, opacity: 0.8 }}
          >
            {selectedChildren.length} selected
          </span>
          <PromoteToCompareButton
            selectedChildren={selectedChildren}
            navigateTo={promoteNavigateTo}
          />
        </div>
      ) : null}

      <div style={{ overflow: 'auto' }}>
        <table style={tableStyle} data-testid="trade-study-table-grid">
          <thead>
            <tr>
              <th
                scope="col"
                style={{ ...cornerHeaderStyle, cursor: 'default', width: 32 }}
                data-testid="header-select"
                aria-label="Select alternative"
              >
                {''}
              </th>
              <HeaderCell
                label="Alternative"
                sortKey="label"
                sort={sort}
                onToggle={toggleSort}
                headerStyle={headerCellStyle}
                indicator={sortIndicator('label')}
              />
              {criteria.map((c) => (
                <HeaderCell
                  key={c.key}
                  label={`${c.label ?? c.key}${c.unit ? ` (${c.unit})` : ''} ${c.objective === 'min' ? '↓' : '↑'}`}
                  sortKey={`criterion:${c.key}`}
                  sort={sort}
                  onToggle={toggleSort}
                  headerStyle={headerCellStyle}
                  indicator={sortIndicator(`criterion:${c.key}`)}
                  testId={`header-criterion-${c.key}`}
                />
              ))}
              <HeaderCell
                label="Score"
                sortKey="score"
                sort={sort}
                onToggle={toggleSort}
                headerStyle={headerCellStyle}
                indicator={sortIndicator('score')}
                testId="header-score"
              />
              <HeaderCell
                label="Rank"
                sortKey="rank"
                sort={sort}
                onToggle={toggleSort}
                headerStyle={headerCellStyle}
                indicator={sortIndicator('rank')}
                testId="header-rank"
              />
              <HeaderCell
                label="Pareto"
                sortKey="pareto"
                sort={sort}
                onToggle={toggleSort}
                headerStyle={headerCellStyle}
                indicator={sortIndicator('pareto')}
                testId="header-pareto"
              />
            </tr>
          </thead>
          <tbody>
            {sortedIndices.map((i) => {
              const row = alternatives[i];
              const pending = isPending(row);
              const isBest = i === scoring.bestIdx;
              const isParetoRow = scoring.paretoIdx.has(i);
              const score = scoring.scores.get(i);
              const rank = scoring.rankByIdx.get(i);
              const checked = selectedIds.has(row.id);

              const rowStyle: CSSProperties = {
                background: isParetoRow
                  ? 'color-mix(in srgb, var(--primary) 9%, transparent)'
                  : undefined,
                // Gold left border on best row — genuine primacy marker.
                boxShadow: isBest
                  ? 'inset 3px 0 0 0 var(--accent)'
                  : undefined,
              };

              return (
                <tr
                  key={row.id}
                  data-testid={`row-${row.id}`}
                  data-pareto={isParetoRow ? 'true' : 'false'}
                  data-best={isBest ? 'true' : 'false'}
                  data-pending={pending ? 'true' : 'false'}
                  style={rowStyle}
                >
                  <td style={firstColumnCellStyle}>
                    <input
                      type="checkbox"
                      data-testid={`select-${row.id}`}
                      aria-label={`Select ${labelFor(row)}`}
                      checked={checked}
                      onChange={() => toggleSelect(row.id)}
                    />
                  </td>
                  <td
                    style={{ ...bodyCellStyle, fontWeight: 500 }}
                    data-testid={`label-${row.id}`}
                  >
                    {labelFor(row)}
                  </td>
                  {criteria.map((c, colIdx) => {
                    const v = scoring.alts[i].values[colIdx];
                    return (
                      <td
                        key={c.key}
                        style={bodyCellStyle}
                        data-testid={`cell-${row.id}-${c.key}`}
                      >
                        {pending ? (
                          <Skeleton />
                        ) : Number.isFinite(v) ? (
                          formatNumber(v)
                        ) : (
                          <span style={{ opacity: 0.4 }}>{'\u2014'}</span>
                        )}
                      </td>
                    );
                  })}
                  <td
                    style={bodyCellStyle}
                    data-testid={`score-${row.id}`}
                  >
                    {pending ? (
                      <Skeleton />
                    ) : score !== undefined ? (
                      formatNumber(score)
                    ) : (
                      <span style={{ opacity: 0.4 }}>{'\u2014'}</span>
                    )}
                  </td>
                  <td
                    style={bodyCellStyle}
                    data-testid={`rank-${row.id}`}
                  >
                    {pending ? (
                      <Skeleton />
                    ) : rank !== undefined ? (
                      rank
                    ) : (
                      <span style={{ opacity: 0.4 }}>{'\u2014'}</span>
                    )}
                  </td>
                  <td style={bodyCellStyle} data-testid={`pareto-${row.id}`}>
                    {pending ? (
                      <Skeleton />
                    ) : isParetoRow ? (
                      <ParetoChip />
                    ) : (
                      <span style={{ opacity: 0.4 }}>{'\u2014'}</span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ── Sub-components ─────────────────────────────────────────────────

interface HeaderCellProps {
  label: string;
  sortKey: ColumnSortKey;
  sort: SortState;
  onToggle: (key: ColumnSortKey) => void;
  headerStyle: CSSProperties;
  indicator: string;
  testId?: string;
}

function HeaderCell({
  label,
  sortKey,
  sort,
  onToggle,
  headerStyle,
  indicator,
  testId,
}: HeaderCellProps): ReactNode {
  const isSorted = sort.key === sortKey;
  return (
    <th
      scope="col"
      style={headerStyle}
      onClick={() => onToggle(sortKey)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onToggle(sortKey);
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
      data-testid={testId ?? `header-${sortKey}`}
    >
      {label}
      {indicator}
    </th>
  );
}

function ParetoChip(): ReactNode {
  return (
    <span
      data-testid="pareto-chip"
      aria-label="Pareto-optimal"
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        padding: '1px 6px',
        borderRadius: 10,
        background: 'color-mix(in srgb, var(--primary) 18%, transparent)',
        color: 'var(--primary)',
        fontSize: 10,
        fontWeight: 600,
        letterSpacing: 0.2,
      }}
    >
      {'\u2605'} Pareto
    </span>
  );
}

function Skeleton(): ReactNode {
  return (
    <span
      data-testid="skeleton-cell"
      aria-label="Pending"
      style={{
        display: 'inline-block',
        width: 40,
        height: 10,
        borderRadius: 3,
        background: 'color-mix(in srgb, var(--outline-variant) 25%, transparent)',
        opacity: 0.7,
      }}
    />
  );
}

function EmptyState({ axes }: { axes: AxesConfig }): ReactNode {
  return (
    <div
      data-testid="trade-study-table-empty"
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
        border:
          '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
        borderRadius: 8,
        padding: 16,
      }}
    >
      Generate alternatives to populate the trade study
    </div>
  );
}

function formatNumber(v: number): string {
  // Compact format: 3 sig-figs, no trailing zeros from toFixed.
  if (!Number.isFinite(v)) return '—';
  const abs = Math.abs(v);
  if (abs === 0) return '0';
  if (abs >= 1000 || abs < 0.01) return v.toExponential(2);
  // 3 significant digits via toPrecision, strip trailing zeros.
  const s = v.toPrecision(4);
  return s.includes('.') ? s.replace(/\.?0+$/, '') : s;
}

// ── Canonical viewer export ────────────────────────────────────────

export const tradeStudyTableViewer: ResultViewer<TradeStudyTableData> = {
  id: 'trade-table-default',
  kind: 'trade-table',
  accepts: (data): data is TradeStudyTableData => data.kind === 'trade-table',
  render: (data, axes) => <TradeStudyTable data={data} axes={axes} />,
};
