/**
 * ModeStrips — the Phase 6 mode bodies (ensemble / golden /
 * two-design) as CONTEXTUAL STRIPS above the diff canvas, per the
 * plan's layout decision: "modes as a mode switch, not extra panels".
 * All heavy math is reused from `../modes/*` (one home); these
 * components only feed it matrix data and render the results in
 * ninebar tokens.
 *
 * Token discipline: ensemble stats + two-design deltas are DESCRIPTIVE
 * reads (neutral text; outliers in the warning family). Golden
 * verdicts are REAL verdicts (a tolerance check against a locked
 * reference) — they use `--verdict-*`, which is exactly why the diff
 * layer never does.
 */

import { useMemo } from 'react';
import type { SessionSummary } from '@/features/sessions/types';
import { useArchiveList } from '@/features/archive/useArchiveList';
import { useModalStore } from '@/shared/overlays/modalStore';
import { HISTORY_BROWSER_MODAL_ID } from '@/features/archive/HistoryBrowserModal';
import type { SamplesBySession } from '../selectors';
import { compareDesignDelta } from '../modes/twoDesign';
import { computeGoldenVerdict, type Tolerance } from '../modes/golden';
import type { VerdictKind } from '@/engine/types';
import { useCompareStore } from '../useCompareStore';
import { useGoldenReference } from './compareData';
import { ensembleAtTick, rowToTimePoints } from './modeMath';
import { sessionStroke } from './channels';

const VERDICT_COLOR: Record<VerdictKind, string> = {
  pass: 'var(--verdict-pass)',
  fail: 'var(--verdict-fail)',
  inconclusive: 'var(--verdict-inconclusive)',
  error: 'var(--verdict-error)',
};

function stripFrame(testid: string, children: React.ReactNode) {
  return (
    <div
      data-testid={testid}
      className="shrink-0 overflow-y-auto"
      style={{
        maxHeight: 180,
        borderBottom: '1px solid var(--border-hairline)',
        padding: '6px 12px',
        fontSize: 'var(--text-xs)',
        color: 'var(--text-secondary)',
      }}
    >
      {children}
    </div>
  );
}

function label(s: SessionSummary): string {
  return s.label ?? (s.id.length > 8 ? s.id.slice(0, 8) : s.id);
}

const fmt = (v: number): string => {
  const a = Math.abs(v);
  if (a !== 0 && (a >= 1e6 || a < 1e-3)) return v.toExponential(2);
  return v.toFixed(3);
};

// ── Ensemble ────────────────────────────────────────────────────────

export function EnsembleStrip({
  variables,
  samplesByVar,
  summaries,
  sharedTick,
}: {
  variables: string[];
  samplesByVar: Record<string, SamplesBySession>;
  summaries: SessionSummary[];
  sharedTick: number;
}) {
  const rows = useMemo(
    () =>
      variables.map((name) => ({
        name,
        at: ensembleAtTick(samplesByVar[name] ?? [], sharedTick),
      })),
    [variables, samplesByVar, sharedTick],
  );

  return stripFrame(
    'compare-ensemble-strip',
    <table style={{ borderCollapse: 'collapse', fontFamily: 'var(--font-mono)' }}>
      <thead>
        <tr style={{ color: 'var(--text-muted)', textAlign: 'left' }}>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>variable</th>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>n</th>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>mean</th>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>σ</th>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>p5–p95</th>
          <th style={{ fontWeight: 400 }}>outliers @ tick {sharedTick}</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(({ name, at }) => (
          <tr key={name} data-testid={`compare-ensemble-row-${name}`}>
            <td style={{ paddingRight: 12, color: 'var(--text-primary)' }}>{name}</td>
            <td style={{ paddingRight: 12 }}>{at.stats.n}</td>
            <td style={{ paddingRight: 12 }}>{fmt(at.stats.mean)}</td>
            <td style={{ paddingRight: 12 }}>{fmt(at.stats.sigma)}</td>
            <td style={{ paddingRight: 12 }}>
              {at.stats.p5 !== null && at.stats.p95 !== null
                ? `${fmt(at.stats.p5)} … ${fmt(at.stats.p95)}`
                : '—'}
            </td>
            <td style={{ color: 'var(--severity-warning)' }}>
              {at.outlierIndices.length === 0
                ? '·'
                : at.outlierIndices
                    .map((i) => (summaries[i] ? label(summaries[i]) : `#${i}`))
                    .join(', ')}
            </td>
          </tr>
        ))}
      </tbody>
    </table>,
  );
}

// ── Two-design ──────────────────────────────────────────────────────

export function TwoDesignStrip({
  variables,
  samplesByVar,
  summaries,
  onScrubTo,
}: {
  variables: string[];
  samplesByVar: Record<string, SamplesBySession>;
  summaries: SessionSummary[];
  onScrubTo: (tick: number) => void;
}) {
  if (summaries.length !== 2) {
    return stripFrame(
      'compare-twodesign-strip',
      <span data-testid="compare-twodesign-needs-two">
        two-design compares exactly two picked sessions — currently {summaries.length}.
      </span>,
    );
  }

  return stripFrame(
    'compare-twodesign-strip',
    <TwoDesignTable
      variables={variables}
      samplesByVar={samplesByVar}
      aLabel={label(summaries[0])}
      bLabel={label(summaries[1])}
      onScrubTo={onScrubTo}
    />,
  );
}

function TwoDesignTable({
  variables,
  samplesByVar,
  aLabel,
  bLabel,
  onScrubTo,
}: {
  variables: string[];
  samplesByVar: Record<string, SamplesBySession>;
  aLabel: string;
  bLabel: string;
  onScrubTo: (tick: number) => void;
}) {
  const rows = useMemo(
    () =>
      variables.map((name) => {
        const m = samplesByVar[name] ?? [];
        return {
          name,
          delta: compareDesignDelta(
            rowToTimePoints(m[0] ?? []),
            rowToTimePoints(m[1] ?? []),
          ),
        };
      }),
    [variables, samplesByVar],
  );

  return (
    <table style={{ borderCollapse: 'collapse', fontFamily: 'var(--font-mono)' }}>
      <thead>
        <tr style={{ color: 'var(--text-muted)', textAlign: 'left' }}>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>variable</th>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>
            ∫|{aLabel} − {bLabel}|
          </th>
          <th style={{ paddingRight: 12, fontWeight: 400 }}>peak Δ</th>
          <th style={{ fontWeight: 400 }}>at tick</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(({ name, delta }) => (
          <tr key={name} data-testid={`compare-twodesign-row-${name}`}>
            <td style={{ paddingRight: 12, color: 'var(--text-primary)' }}>{name}</td>
            <td style={{ paddingRight: 12 }}>{fmt(delta.integral)}</td>
            <td style={{ paddingRight: 12 }}>{fmt(delta.peakDelta)}</td>
            <td>
              {delta.peakTick === null ? (
                '—'
              ) : (
                <button
                  type="button"
                  data-testid={`compare-twodesign-peak-${name}`}
                  onClick={() => onScrubTo(delta.peakTick ?? 0)}
                  title="Jump the playhead to the peak delta"
                  style={{
                    border: 'none',
                    background: 'transparent',
                    color: 'var(--accent-fg)',
                    cursor: 'pointer',
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--text-xs)',
                    padding: 0,
                    textDecoration: 'underline',
                  }}
                >
                  {delta.peakTick}
                </button>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── Golden ──────────────────────────────────────────────────────────

export function GoldenStrip({
  variables,
  samplesByVar,
  summaries,
}: {
  variables: string[];
  samplesByVar: Record<string, SamplesBySession>;
  summaries: SessionSummary[];
}) {
  const goldenArchiveId = useCompareStore((s) => s.goldenArchiveId);
  const setGoldenArchiveId = useCompareStore((s) => s.setGoldenArchiveId);
  const tolRel = useCompareStore((s) => s.goldenToleranceRel);
  const setTolRel = useCompareStore((s) => s.setGoldenToleranceRel);
  const openModal = useModalStore((s) => s.openModal);

  // The golden PICKER (plan row 24): golden-pinned archived runs only.
  const goldenList = useArchiveList(
    { search: '', origin: 'all', since: 'all', onlyGolden: true },
    { workspaceUri: null },
  );
  const goldens = goldenList.data ?? [];
  const { data: reference } = useGoldenReference(goldenArchiveId);

  const tolerance: Tolerance = { kind: 'relative', value: tolRel };

  const verdictRows = useMemo(() => {
    if (!reference) return [];
    return variables.map((name) => {
      const goldenSeries = reference.series[name] ?? [];
      const m = samplesByVar[name] ?? [];
      return {
        name,
        cells: summaries.map((s, i) => ({
          session: s,
          outcome: computeGoldenVerdict(goldenSeries, rowToTimePoints(m[i] ?? []), tolerance),
          index: i,
        })),
      };
    });
  }, [reference, variables, samplesByVar, summaries, tolerance]);

  return stripFrame(
    'compare-golden-strip',
    <div className="flex flex-col" style={{ gap: 6 }}>
      <div className="flex items-center" style={{ gap: 8, flexWrap: 'wrap' }}>
        <span style={{ textTransform: 'uppercase', letterSpacing: '0.05em', color: 'var(--text-muted)' }}>
          golden
        </span>
        <select
          data-testid="compare-golden-picker"
          value={goldenArchiveId ?? ''}
          onChange={(e) => setGoldenArchiveId(e.target.value || null)}
          style={{
            fontSize: 'var(--text-xs)',
            background: 'var(--surface-panel)',
            color: 'var(--text-primary)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            padding: '1px 4px',
            maxWidth: 260,
          }}
        >
          <option value="">pick a golden run…</option>
          {goldens.map((g) => (
            <option key={g.id} value={g.id}>
              {g.golden_label ? `${g.label} — ${g.golden_label}` : g.label}
            </option>
          ))}
        </select>
        <label className="flex items-center" style={{ gap: 4 }}>
          tolerance ±
          <input
            data-testid="compare-golden-tolerance"
            type="number"
            min={0}
            step={0.5}
            value={(tolRel * 100).toString()}
            onChange={(e) => setTolRel(Number(e.target.value) / 100)}
            style={{
              width: 52,
              fontSize: 'var(--text-xs)',
              fontFamily: 'var(--font-mono)',
              background: 'var(--surface-panel)',
              color: 'var(--text-primary)',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-sm)',
              padding: '1px 4px',
            }}
          />
          %
        </label>
        <button
          type="button"
          data-testid="compare-golden-manage"
          onClick={() => openModal(HISTORY_BROWSER_MODAL_ID)}
          title="Open the history browser to mark/unmark golden runs"
          style={{
            fontSize: 'var(--text-xs)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            background: 'transparent',
            color: 'var(--text-secondary)',
            cursor: 'pointer',
            padding: '1px 6px',
          }}
        >
          manage archive…
        </button>
      </div>

      {!goldenArchiveId && goldens.length === 0 && (
        <div data-testid="compare-golden-empty">
          No golden runs pinned yet — open the archive and “Mark Golden” a
          reference run.
        </div>
      )}
      {goldenArchiveId && reference && reference.snapshotCount === 0 && (
        <div data-testid="compare-golden-no-snapshots">
          This golden record archived no snapshot history — no series to
          compare against.
        </div>
      )}

      {reference && verdictRows.length > 0 && (
        <table style={{ borderCollapse: 'collapse', fontFamily: 'var(--font-mono)' }}>
          <thead>
            <tr style={{ color: 'var(--text-muted)', textAlign: 'left' }}>
              <th style={{ paddingRight: 12, fontWeight: 400 }}>variable</th>
              {summaries.map((s, i) => (
                <th key={s.id} style={{ paddingRight: 12, fontWeight: 400 }}>
                  <span
                    aria-hidden
                    style={{
                      display: 'inline-block',
                      width: 7,
                      height: 7,
                      borderRadius: 2,
                      background: sessionStroke(i),
                      marginRight: 4,
                    }}
                  />
                  {label(s)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {verdictRows.map((row) => (
              <tr key={row.name} data-testid={`compare-golden-row-${row.name}`}>
                <td style={{ paddingRight: 12, color: 'var(--text-primary)' }}>{row.name}</td>
                {row.cells.map((cell) => (
                  <td key={cell.session.id} style={{ paddingRight: 12 }}>
                    <span
                      data-testid={`compare-golden-verdict-${row.name}-${cell.session.id}`}
                      title={
                        cell.outcome.verdict === 'fail'
                          ? `max Δ ${fmt(cell.outcome.maxDelta)} · first fail @ tick ${cell.outcome.firstFailTick}`
                          : cell.outcome.verdict === 'error'
                            ? cell.outcome.errorReason
                            : `${cell.outcome.evaluatedTicks} ticks evaluated`
                      }
                      style={{
                        color: VERDICT_COLOR[cell.outcome.verdict],
                        border: `1px solid ${VERDICT_COLOR[cell.outcome.verdict]}`,
                        borderRadius: 'var(--radius-sm)',
                        padding: '0 6px',
                      }}
                    >
                      {cell.outcome.verdict}
                    </span>
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>,
  );
}
