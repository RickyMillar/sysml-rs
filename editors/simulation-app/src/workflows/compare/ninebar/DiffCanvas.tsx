/**
 * DiffCanvas — the Phase 6 Compare hero: per-variable diff rows under
 * the one shared playhead.
 *
 * Layout per row (plan: "diff = its own semantic layer, never
 * borrowing verdict tokens"; DoD: "two side-by-side recoloured Run
 * views fail"):
 *
 *   - overlay chart: session curves in DESATURATED channel strokes
 *     (`channels.ts`), the cross-session min/max envelope FILLED in
 *     `--diff-modified` — the fill IS the diff signal. NaN gaps break
 *     lines (missing ≠ zero) and dim the value chip in
 *     `--diff-missing`.
 *   - divergence gutter: PAIR mode paints the backend's exact
 *     `diff_timeline` tick mask; N-way paints the FE spread score over
 *     decimated series (a navigation aid — max-pooled into buckets so
 *     a single divergent tick never disappears).
 *   - value chips at the playhead per session; PAIR mode adds the
 *     added/removed/modified classification manufactured from
 *     null-on-one-side.
 *
 * Pair-mode subsystem STATE divergence renders as a compact strip
 * above the variable rows (states aren't curves; a chip row is their
 * honest shape).
 */

import { useMemo } from 'react';
import type {
  SessionSummary,
  SessionTimelineDivergence,
} from '@/features/sessions/types';
import { computeDivergence, type SamplesBySession } from '../selectors';
import { sessionStroke } from './channels';
import {
  bucketScores,
  bucketStartTick,
  classifyVariableDiff,
  diffEntryAtOrBefore,
  pairDivergenceMask,
  type PairDiffKind,
} from './seriesMath';
import { envelopePath, linePath, valueDomain, type Scale } from './svgPaths';

const CHART_W = 600;
const CHART_H = 72;
const GUTTER_BUCKETS = 240;

const DIFF_COLOR: Record<PairDiffKind, string> = {
  added: 'var(--diff-added)',
  removed: 'var(--diff-removed)',
  modified: 'var(--diff-modified)',
};

export function DiffCanvas({
  summaries,
  samplesByVar,
  variables,
  maxTick,
  sharedTick,
  onScrubTo,
  pairDiff,
  namesBySession,
}: {
  summaries: SessionSummary[];
  samplesByVar: Record<string, SamplesBySession>;
  variables: string[];
  maxTick: number;
  sharedTick: number;
  onScrubTo: (tick: number) => void;
  /** Backend timeline diff — PAIR mode only, else null. */
  pairDiff: SessionTimelineDivergence | null;
  namesBySession: Map<string, Set<string>>;
}) {
  const pairEntryAtPlayhead = useMemo(
    () => (pairDiff ? diffEntryAtOrBefore(pairDiff.tick_diffs, sharedTick) : null),
    [pairDiff, sharedTick],
  );

  if (variables.length === 0) {
    return (
      <div
        data-testid="compare-canvas-empty"
        className="flex items-center justify-center h-full"
        style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}
      >
        No recorded variables across the picked sessions.
      </div>
    );
  }

  return (
    <div
      data-testid="compare-diff-canvas"
      className="flex-1 overflow-y-auto"
      style={{ padding: '8px 12px' }}
    >
      {pairDiff && (
        <StateDiffStrip
          entry={pairEntryAtPlayhead}
          aLabel={sessionLabel(summaries[0])}
          bLabel={sessionLabel(summaries[1])}
        />
      )}
      <div className="flex flex-col" style={{ gap: 10 }}>
        {variables.map((name) => (
          <VariableDiffRow
            key={name}
            name={name}
            samples={samplesByVar[name] ?? []}
            summaries={summaries}
            maxTick={maxTick}
            sharedTick={sharedTick}
            onScrubTo={onScrubTo}
            pairDiff={pairDiff}
            pairEntryAtPlayhead={pairEntryAtPlayhead}
            namesBySession={namesBySession}
          />
        ))}
      </div>
    </div>
  );
}

function sessionLabel(s: SessionSummary | undefined): string {
  if (!s) return '?';
  return s.label ?? (s.id.length > 8 ? s.id.slice(0, 8) : s.id);
}

// ── Subsystem state divergence (pair mode) ──────────────────────────

function StateDiffStrip({
  entry,
  aLabel,
  bLabel,
}: {
  entry: { tick: number; subsystem_diffs: Array<{ name: string; a_state: string | null; b_state: string | null }> } | null;
  aLabel: string;
  bLabel: string;
}) {
  const diffs = entry?.subsystem_diffs ?? [];
  return (
    <div
      data-testid="compare-state-strip"
      className="flex items-center"
      style={{
        gap: 8,
        minHeight: 26,
        marginBottom: 8,
        fontSize: 'var(--text-xs)',
        color: 'var(--text-muted)',
        flexWrap: 'wrap',
      }}
    >
      <span style={{ textTransform: 'uppercase', letterSpacing: '0.05em' }}>states</span>
      {diffs.length === 0 && (
        <span data-testid="compare-state-strip-agree">
          {entry ? 'no subsystem-state differences at this tick' : 'in agreement up to this tick'}
        </span>
      )}
      {diffs.map((d) => {
        const kind: PairDiffKind =
          d.a_state === null ? 'added' : d.b_state === null ? 'removed' : 'modified';
        return (
          <span
            key={d.name}
            data-testid={`compare-state-diff-${d.name}`}
            title={`${aLabel}: ${d.a_state ?? '—'} · ${bLabel}: ${d.b_state ?? '—'} (as of tick ${entry?.tick})`}
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              padding: '1px 6px',
              border: `1px solid ${DIFF_COLOR[kind]}`,
              borderRadius: 'var(--radius-sm)',
              color: 'var(--text-primary)',
              fontFamily: 'var(--font-mono)',
            }}
          >
            {d.name}: {d.a_state ?? '—'} ⇥ {d.b_state ?? '—'}
          </span>
        );
      })}
    </div>
  );
}

// ── One variable row ────────────────────────────────────────────────

function VariableDiffRow({
  name,
  samples,
  summaries,
  maxTick,
  sharedTick,
  onScrubTo,
  pairDiff,
  pairEntryAtPlayhead,
  namesBySession,
}: {
  name: string;
  samples: SamplesBySession;
  summaries: SessionSummary[];
  maxTick: number;
  sharedTick: number;
  onScrubTo: (tick: number) => void;
  pairDiff: SessionTimelineDivergence | null;
  pairEntryAtPlayhead: { tick: number; variable_diffs: Array<{ name: string; a_value: number | null; b_value: number | null }> } | null;
  namesBySession: Map<string, Set<string>>;
}) {
  const domain = useMemo(() => valueDomain(samples), [samples]);
  const scale: Scale = useMemo(
    () => ({
      width: CHART_W,
      height: CHART_H,
      maxTick: Math.max(1, maxTick),
      yMin: domain?.yMin ?? 0,
      yMax: domain?.yMax ?? 1,
    }),
    [domain, maxTick],
  );

  const envelope = useMemo(() => envelopePath(samples, scale), [samples, scale]);
  const lines = useMemo(
    () => samples.map((row) => linePath(row, scale)),
    [samples, scale],
  );

  // Gutter: exact backend mask for pairs, FE spread otherwise.
  const gutter = useMemo(() => {
    const perTick = pairDiff
      ? pairDivergenceMask(
          pairDiff.tick_diffs.filter((d) =>
            d.variable_diffs.some((vd) => vd.name === name),
          ),
          maxTick,
        )
      : computeDivergence(samples);
    return bucketScores(perTick, GUTTER_BUCKETS);
  }, [pairDiff, samples, name, maxTick]);

  const tickCount = maxTick + 1;

  // The pair classification for THIS variable at the playhead, if the
  // last known difference mentions it.
  const pairKind: PairDiffKind | null = useMemo(() => {
    const vd = pairEntryAtPlayhead?.variable_diffs.find((d) => d.name === name);
    return vd ? classifyVariableDiff(vd) : null;
  }, [pairEntryAtPlayhead, name]);

  const scrubFromEvent = (e: React.MouseEvent<SVGSVGElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const frac = rect.width > 0 ? (e.clientX - rect.left) / rect.width : 0;
    onScrubTo(Math.round(Math.max(0, Math.min(1, frac)) * maxTick));
  };

  return (
    <div
      data-testid={`compare-variable-row-${name}`}
      style={{
        border: '1px solid var(--border-hairline)',
        borderRadius: 'var(--radius-sm)',
        background: 'var(--surface-panel)',
        padding: 8,
        display: 'flex',
        gap: 10,
      }}
    >
      <div className="flex-1 min-w-0 flex flex-col" style={{ gap: 4 }}>
        <div className="flex items-center justify-between" style={{ gap: 8 }}>
          <span
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--text-sm)',
              color: 'var(--text-primary)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {name}
          </span>
          {pairKind && (
            <span
              data-testid={`compare-variable-pairkind-${name}`}
              style={{
                fontSize: 'var(--text-xs)',
                padding: '0 6px',
                border: `1px solid ${DIFF_COLOR[pairKind]}`,
                borderRadius: 'var(--radius-sm)',
                color: DIFF_COLOR[pairKind],
              }}
            >
              {pairKind}
            </span>
          )}
        </div>

        <svg
          data-testid={`compare-variable-chart-${name}`}
          viewBox={`0 0 ${CHART_W} ${CHART_H}`}
          preserveAspectRatio="none"
          style={{ width: '100%', height: CHART_H, display: 'block', cursor: 'crosshair' }}
          onClick={scrubFromEvent}
        >
          {/* diff signal: the envelope fill */}
          {envelope && (
            <path
              data-testid={`compare-envelope-${name}`}
              d={envelope}
              fill="var(--diff-modified)"
              fillOpacity={0.16}
              stroke="none"
            />
          )}
          {/* reclaimed channels: desaturated session strokes */}
          {lines.map((d, i) =>
            d ? (
              <path
                key={summaries[i]?.id ?? i}
                d={d}
                fill="none"
                stroke={sessionStroke(i)}
                strokeWidth={1.2}
                vectorEffect="non-scaling-stroke"
              />
            ) : null,
          )}
          {/* the one playhead */}
          <line
            x1={(sharedTick / Math.max(1, maxTick)) * CHART_W}
            x2={(sharedTick / Math.max(1, maxTick)) * CHART_W}
            y1={0}
            y2={CHART_H}
            stroke="var(--accent)"
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
          />
        </svg>

        {/* divergence gutter */}
        <div
          data-testid={`compare-gutter-${name}`}
          className="flex"
          style={{ height: 8, borderRadius: 2, overflow: 'hidden' }}
        >
          {gutter.map((score, b) => (
            <div
              key={b}
              onClick={() => onScrubTo(bucketStartTick(b, GUTTER_BUCKETS, tickCount))}
              title={
                score > 0
                  ? `divergence ${(score * 100).toFixed(0)}% · jump to tick ${bucketStartTick(b, GUTTER_BUCKETS, tickCount)}`
                  : undefined
              }
              style={{
                flex: 1,
                cursor: score > 0 ? 'pointer' : 'default',
                background:
                  score > 0
                    ? `color-mix(in oklch, var(--diff-modified) ${Math.round(
                        20 + 80 * Math.min(1, score),
                      )}%, transparent)`
                    : 'var(--surface-sunken)',
              }}
            />
          ))}
        </div>
      </div>

      {/* value chips at the playhead */}
      <div
        className="flex flex-col shrink-0"
        style={{ gap: 3, width: 148, justifyContent: 'center' }}
      >
        {summaries.map((s, i) => {
          const row = samples[i] ?? [];
          const v = row[Math.min(sharedTick, row.length - 1)] ?? NaN;
          const recorded = namesBySession.get(s.id)?.has(name) ?? false;
          const missing = !Number.isFinite(v);
          return (
            <div
              key={s.id}
              data-testid={`compare-value-${name}-${s.id}`}
              className="flex items-center"
              style={{ gap: 6, fontSize: 'var(--text-xs)' }}
              title={
                missing
                  ? recorded
                    ? `${sessionLabel(s)} — no sample at this tick`
                    : `${sessionLabel(s)} never recorded ${name}`
                  : `${sessionLabel(s)} @ tick ${sharedTick}`
              }
            >
              <span
                aria-hidden
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: 2,
                  background: sessionStroke(i),
                  flexShrink: 0,
                }}
              />
              <span
                style={{
                  color: 'var(--text-muted)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  flex: 1,
                }}
              >
                {sessionLabel(s)}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  color: missing ? 'var(--diff-missing)' : 'var(--text-primary)',
                }}
              >
                {missing ? '—' : formatValue(v)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function formatValue(v: number): string {
  const a = Math.abs(v);
  if (a !== 0 && (a >= 1e6 || a < 1e-3)) return v.toExponential(3);
  return v.toFixed(3);
}
