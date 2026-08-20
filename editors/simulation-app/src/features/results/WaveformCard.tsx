/**
 * WaveformCard — the Run bottom-strip waveform (ninebar Phase 3, "Run,
 * re-composed").
 *
 * Per the plan (§1 row 4a / §3): the waveform is "the one always-useful
 * live result" and the strip's spatial budget
 * (`--strip-min-height`/`--strip-max-height`, ~210px in the demo
 * reference, `demo-crib-sheet.md` §4) only ever fits ONE time-series
 * chart — not `PlotsTab`'s multi-plot / XY / scatter / 3D / override
 * workspace. So this shell shares `PlotsTab`'s chart internals rather
 * than forking them:
 *   - the LTTB-decimated read path (`useTimeSeriesStore.
 *     getDecimatedTimeSeries(PerfBudget.MAX_RENDER_POINTS)` — ADR-008).
 *     This NEVER reads the raw ring buffer (`getTimeSeries()`), which is
 *     reserved for KPI aggregates / CSV export.
 *   - the same `timeSeriesViewer` uPlot wrapper
 *     (`shared/viewers/TimeSeriesViewer`).
 *   - the same session-keyed `usePlotSelectionStore` selection + the
 *     same `PlotVariablePicker` dialog PlotsTab uses (so a selection
 *     made in one shell is visible in the other — one store, two UIs).
 *   - the same `metricRegistry` / `classifyVariableDomain` labelling and
 *     `guessColor`/`DOMAIN_COLORS` palette (exported from `PlotsTab.tsx`
 *     for this purpose).
 *
 * The variable picker is a compact overflow button (opens
 * `PlotVariablePicker`), not a tab bar — the plan's strip budget
 * explicitly rejects a second tabbed results surface ("no tabbed results
 * workbench remains", plan §3 DoD).
 *
 * Carries the tick/time readout `SessionStatusBar` used to render
 * (`t=X.Xs step NNNN`) — ninebar Run does not mount `SessionStatusBar`
 * (see `RunWorkflow.tsx`), so this is its new home, in the card footer.
 * Throttled to 500 ms per ADR-008, mirroring `SessionStatusBar`'s own
 * throttle so a running session doesn't repaint the readout every tick.
 *
 * Header row also carries `TransportBar` (ninebar screenshot-comparison
 * ruling A, 2026-07-14 — "Transport moves to the strip"): the play/
 * pause/resume/step cluster the frame used to own. It sits first in
 * the row, ahead of the running-state pill/time readout that were
 * already here, matching the demo's transport-row layout (crib sheet
 * §4). `TransportBar` reads the single frame-owned
 * `useSessionController` via `sessionControllerBridge` rather than
 * mounting its own — see `src/app/frame/RunControls.tsx`'s doc comment
 * for why there must be exactly one mount.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Ninebar } from '@/components/Ninebar';
import { TransportBar } from '@/app/frame/TransportBar';
import { useRightRailStore } from '@/app/rail/railStore';
import { ConstraintChip, KpiMeterRow } from '@/features/results/KpiMeterRow';
import { StateTimelineStrip } from '@/features/results/StateTimelineStrip';
import { useStateTimelineIngest, useStateTimelineStore } from '@/features/results/selectors';
import { useSessionDetail } from '@/features/sessions/queries';
import { useSessionStore } from '@/features/sessions/store';
import { classifyVariableDomain, usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';
import { DOMAIN_COLORS, guessColor } from '@/features/results/plots/PlotsTab';
import { PlotVariablePicker } from '@/features/results/plots/PlotVariablePicker';
import { computeStableSortedKeys } from '@/shared/data/useStableSortedKeys';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { metricRegistry, syncVariableMetrics } from '@/shared/metrics/registry';
import type { MetricDescriptor } from '@/shared/metrics/types';
import { PerfBudget } from '@/shared/perf';
import { httpPost } from '@/shared/api/http';
import { timeSeriesViewer } from '@/shared/viewers/TimeSeriesViewer';
import type { TimeSeriesData } from '@/shared/viewers/types';
import type { TimePoint } from '@/features/sessions/types';

const CHART_HEIGHT_PX = 118;

function labelFor(id: string): string {
  return metricRegistry.get(id)?.name ?? id;
}

function formatTime(ms: number): string {
  return `${(ms / 1000).toFixed(4)} s`;
}

export function WaveformCard() {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const phase = useSessionStore((s) => s.phase);
  const running = phase === 'running';
  const { data: sessionDetail } = useSessionDetail(activeSessionId);

  // ── Decimated series read (shared with PlotsTab's Plots-tab path;
  //    see ResultsWorkbench's `needsTimeSeries` branch) ────────────────
  const tsRevision = useTimeSeriesStore((s) => s.revision);
  const timeSeries = useMemo(() => {
    return useTimeSeriesStore.getState().getDecimatedTimeSeries(PerfBudget.MAX_RENDER_POINTS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tsRevision]);

  const stableKeysRef = useRef<string[] | null>(null);
  const variableNames = useMemo(() => {
    stableKeysRef.current = computeStableSortedKeys(timeSeries, stableKeysRef.current);
    return stableKeysRef.current;
  }, [timeSeries]);

  const [registryVersion, setRegistryVersion] = useState(0);
  useEffect(() => {
    syncVariableMetrics(metricRegistry, variableNames, classifyVariableDomain);
    setRegistryVersion((v) => v + 1);
  }, [variableNames]);

  const allVars = useMemo<string[]>(() => {
    const metrics: MetricDescriptor[] = metricRegistry.filter((m) => m.source === 'variable');
    return metrics.map((m) => m.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [registryVersion]);

  // ── Selection — same store PlotsTab reads/writes ────────────────────
  const selectionsBySession = usePlotSelectionStore((s) => s.selectionsBySession);
  const setSelected = usePlotSelectionStore((s) => s.setSelected);
  const selected = useMemo(
    () => (activeSessionId ? selectionsBySession[activeSessionId] ?? [] : []),
    [activeSessionId, selectionsBySession],
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  // W3-D fix: the timeline STORE is fed by this ingest hook, which used
  // to mount only in the legacy ResultsWorkbench — flag-on, nothing fed
  // the store, so the timeline view never offered itself. The strip owns
  // the ingest now (workbench and card never render together, so no
  // double-mount; append dedupes by tick regardless).
  useStateTimelineIngest();
  // W3-D: waveform | state-timeline strip views. The timeline offers
  // itself only when SM activity exists (cheap length-selector — never
  // the entries array itself, F15).
  const [stripView, setStripView] = useState<'waveform' | 'timeline'>('waveform');
  const hasTimeline = useStateTimelineStore(
    (s) => (activeSessionId ? (s.bySession[activeSessionId]?.length ?? 0) : 0) > 0,
  );

  const visibleVars = selected.filter((v) => (timeSeries[v] ?? []).length > 0);
  const canRender = visibleVars.length > 0;

  // ── Tick/time readout — carried over from `SessionStatusBar` (not
  //    rendered under the ninebar shell; see RunWorkflow.tsx). Same
  //    500ms throttle (ADR-008). ──────────────────────────────────────
  const rawTick = sessionDetail?.summary?.tick ?? 0;
  const rawTimeMs = sessionDetail?.summary?.time_ms ?? 0;
  const [displayTick, setDisplayTick] = useState(0);
  const [displayTimeMs, setDisplayTimeMs] = useState(0);
  const lastUpdateRef = useRef(0);
  useEffect(() => {
    const now = Date.now();
    if (!running || now - lastUpdateRef.current >= 500) {
      setDisplayTick(rawTick);
      setDisplayTimeMs(rawTimeMs);
      lastUpdateRef.current = now;
    }
  }, [rawTick, rawTimeMs, running]);

  return (
    <div data-testid="waveform-card" className="flex flex-col h-full w-full overflow-hidden">
      {/* Header — transport + playback status + tick/time readout + legend + picker */}
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        style={{ height: 34, borderBottom: '1px solid var(--border-default)' }}
      >
        <TransportBar />

        {running ? (
          <span
            data-testid="waveform-running-pill"
            className="inline-flex items-center gap-1.5 mono-text"
            style={{
              border: '1px solid var(--accent)',
              borderRadius: 999,
              padding: '2px 10px',
              fontSize: 'var(--text-xs)',
              color: 'var(--accent-fg)',
            }}
          >
            <Ninebar compact size={9} label="waveform live" />
            running
          </span>
        ) : (
          <span
            className="mono-text"
            style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}
          >
            {phase}
          </span>
        )}

        {displayTick > 0 && (
          <span
            data-testid="waveform-time-readout"
            className="mono-text"
            style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}
          >
            t = {formatTime(displayTimeMs)} · step {displayTick}
          </span>
        )}

        <div className="flex-1" />

        {/* Legend — plotted variables, muted mono per the crib sheet */}
        {visibleVars.length > 0 && (
          <div className="flex items-center gap-2 min-w-0 overflow-hidden">
            {visibleVars.map((v) => (
              <span
                key={v}
                className="mono-text truncate"
                style={{ fontSize: 10, color: guessColor(v) ?? 'var(--text-muted)' }}
                title={labelFor(v)}
              >
                {labelFor(v)}
              </span>
            ))}
          </div>
        )}

        {/* Compact overflow control — NOT a tab bar (plan §3 DoD). */}
        <button
          type="button"
          data-testid="waveform-variable-picker-open"
          onClick={() => setPickerOpen(true)}
          className="inline-flex items-center gap-1 px-2 py-1 rounded mono-text shrink-0"
          style={{
            background: 'var(--surface-panel)',
            color: 'var(--text-secondary)',
            border: '1px solid var(--border-default)',
            borderRadius: 4,
            fontSize: 10,
            cursor: 'pointer',
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 13 }}>tune</span>
          Variables ({selected.length})
        </button>

        {/* Right-rail context toggles — the VISIBLE affordance for the
            variables/breakpoints rails (user feedback 2026-07-14: Cmd-K
            alone wasn't discoverable). The demo's Run screen shows
            exactly this pinnable pair in the right rail. */}
        <RailContextToggle contextId="variables" icon="data_table" label="Variables rail" />
        <RailContextToggle contextId="breakpoints" icon="radio_button_checked" label="Breakpoints rail" />
      </div>

      {/* Chart body — omitted entirely while no session exists (ghost
          mode, plan §0): the strip collapses to the transport row above;
          Play lazily creates the session and the chart area returns. */}
      {activeSessionId !== null && (
      <>
      {/* Meta row (W3-A/B/D): strip-view switcher + compact ninebar KPI
          chips + the constraint summary chip — plan §0's composition. */}
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        style={{ height: 30, borderBottom: '1px solid var(--border-default)' }}
      >
        {/* Waveform | state-timeline switcher — timeline only offers
            itself when SM activity exists (capability-gated, W3-D). */}
        {hasTimeline && (
          <div className="flex items-center gap-1 shrink-0" role="group" aria-label="Strip view">
            <StripViewButton
              icon="show_chart"
              label="Waveform"
              active={stripView === 'waveform'}
              onClick={() => setStripView('waveform')}
              testId="strip-view-waveform"
            />
            <StripViewButton
              icon="view_timeline"
              label="State timeline"
              active={stripView === 'timeline'}
              onClick={() => setStripView('timeline')}
              testId="strip-view-timeline"
            />
          </div>
        )}
        <KpiMeterRow />
        <div className="flex-1" />
        <ConstraintChip />
      </div>
      {stripView === 'timeline' && hasTimeline ? (
        <div className="flex-1 min-h-0 overflow-hidden px-3 py-2">
          <StateTimelineStrip />
        </div>
      ) : (
      <div className="flex-1 min-h-0 overflow-hidden px-3 py-2">
        {selected.length === 0 ? (
          <WaveformEmpty running={running} allVars={allVars} onPick={() => setPickerOpen(true)} />
        ) : !canRender ? (
          <div
            className="flex items-center justify-center h-full"
            style={{ color: 'var(--text-muted)', fontSize: 11 }}
          >
            {running ? 'Waiting for selected variables to produce data…' : 'Step the simulation to see traces.'}
          </div>
        ) : (
          <WaveformChart visibleVars={visibleVars} timeSeries={timeSeries} sessionId={activeSessionId} />
        )}
      </div>
      )}
      </>
      )}

      {pickerOpen && (
        <PlotVariablePicker
          allVariables={allVars}
          selected={selected}
          onChange={(next) => {
            if (activeSessionId) setSelected(activeSessionId, next);
          }}
          onClose={() => setPickerOpen(false)}
        />
      )}
    </div>
  );
}

interface ZoomWindow {
  fromMs: number;
  toMs: number;
  /** Backend-decimated points per variable for the window. */
  series: Record<string, TimePoint[]>;
}

function WaveformChart({
  visibleVars,
  timeSeries,
  sessionId,
}: {
  visibleVars: string[];
  timeSeries: Record<string, TimePoint[]>;
  sessionId: string | null;
}) {
  // W3-E zoom re-query: a drag-select fetches the backend's
  // LTTB-decimated window (`sessions.timeseries_decimated`) at
  // display-budget resolution INSIDE the window — full fidelity where
  // the user is looking, instead of zooming into the ring buffer's
  // already-decimated display data. While zoomed, the chart is a static
  // window (live updates continue in the stores; Reset returns to the
  // live view).
  const [zoom, setZoom] = useState<ZoomWindow | null>(null);
  const zoomBusyRef = useRef(false);

  const handleSelectRange = (fromMs: number, toMs: number) => {
    if (!sessionId || zoomBusyRef.current || toMs <= fromMs) return;
    zoomBusyRef.current = true;
    void Promise.all(
      visibleVars.map((name) =>
        httpPost<{ var: string; points: Array<{ time_ms: number; value: number }> }>('/api/command', {
          command: 'sysml.sessions.timeseries_decimated',
          params: {
            session_id: sessionId,
            var: name,
            target_points: PerfBudget.MAX_RENDER_POINTS,
            start_ms: fromMs,
            end_ms: toMs,
          },
        }).then((r) => [name, (r.points ?? []).map((p) => ({ t: p.time_ms, v: p.value }))] as const),
      ),
    )
      .then((pairs) => setZoom({ fromMs, toMs, series: Object.fromEntries(pairs) }))
      .catch((err) => {
        // Fail loud but non-fatally: the un-zoomed live view stays up.
        console.error('[WaveformChart] decimated window query failed:', err);
      })
      .finally(() => {
        zoomBusyRef.current = false;
      });
  };

  const source = zoom?.series ?? timeSeries;
  const series = visibleVars.map((name) => ({
    name: labelFor(name),
    points: source[name] ?? [],
    color: guessColor(name) ?? DOMAIN_COLORS.electrical,
  }));
  const viewerData: TimeSeriesData = { kind: 'time-series', series };
  return (
    <div className="relative h-full">
      {timeSeriesViewer.render(viewerData, {
        height: CHART_HEIGHT_PX,
        onSelectRange: handleSelectRange,
      })}
      {zoom && (
        <button
          type="button"
          data-testid="waveform-zoom-reset"
          onClick={() => setZoom(null)}
          title={`Zoomed ${formatTime(zoom.fromMs)}–${formatTime(zoom.toMs)} (backend-decimated window) — click to return to live`}
          className="mono-text absolute"
          style={{
            top: 2,
            right: 2,
            fontSize: 'var(--text-xs)',
            color: 'var(--accent-fg)',
            background: 'var(--accent-tint)',
            border: '1px solid var(--accent)',
            borderRadius: 'var(--radius-sm)',
            padding: '2px 8px',
            cursor: 'pointer',
            zIndex: 5,
          }}
        >
          zoomed · reset
        </button>
      )}
    </div>
  );
}

/** Compact icon button for the strip-view switcher (W3-D). Active =
 *  accent tint (a genuine selected state). */
function StripViewButton({
  icon,
  label,
  active,
  onClick,
  testId,
}: {
  icon: string;
  label: string;
  active: boolean;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      aria-pressed={active}
      onClick={onClick}
      title={label}
      className="material-symbols-outlined shrink-0"
      style={{
        fontSize: 14,
        width: 24,
        height: 22,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: active ? 'var(--accent-fg)' : 'var(--text-secondary)',
        background: active ? 'var(--accent-tint)' : 'none',
        border: `1px solid ${active ? 'var(--accent)' : 'var(--border-default)'}`,
        borderRadius: 'var(--radius-sm)',
        cursor: 'pointer',
      }}
    >
      {icon}
    </button>
  );
}

/**
 * Icon toggle for a right-rail context (variables / breakpoints). Open
 * state gets the accent tint — this IS a selection/active state, so
 * amber is legitimate here.
 */
function RailContextToggle({
  contextId,
  icon,
  label,
}: {
  contextId: string;
  icon: string;
  label: string;
}) {
  const pinned = useRightRailStore((s) => s.pinned);
  const transient = useRightRailStore((s) => s.transient);
  const isOpen = pinned === contextId || transient === contextId;
  return (
    <button
      type="button"
      data-testid={`strip-rail-toggle-${contextId}`}
      aria-pressed={isOpen}
      onClick={() => {
        const rail = useRightRailStore.getState();
        if (isOpen) {
          if (rail.pinned === contextId) rail.unpin();
          else rail.close();
        } else {
          rail.open(contextId);
        }
      }}
      title={isOpen ? `Close ${label.toLowerCase()}` : `Open ${label.toLowerCase()}`}
      className="material-symbols-outlined shrink-0"
      style={{
        fontSize: 14,
        width: 24,
        height: 22,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: isOpen ? 'var(--accent-fg)' : 'var(--text-secondary)',
        background: isOpen ? 'var(--accent-tint)' : 'none',
        border: `1px solid ${isOpen ? 'var(--accent)' : 'var(--border-default)'}`,
        borderRadius: 'var(--radius-sm)',
        cursor: 'pointer',
      }}
    >
      {icon}
    </button>
  );
}

function WaveformEmpty({
  running,
  allVars,
  onPick,
}: {
  running: boolean;
  allVars: string[];
  onPick: () => void;
}) {
  return (
    <div
      data-testid="waveform-empty"
      className="flex flex-col items-center justify-center gap-1.5 h-full"
      style={{ color: 'var(--text-muted)', textAlign: 'center' }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 20 }}>show_chart</span>
      <div style={{ fontSize: 11 }}>
        {allVars.length > 0 ? (
          <>
            {allVars.length} variables available —{' '}
            <button
              type="button"
              onClick={onPick}
              style={{ background: 'none', border: 'none', color: 'var(--accent-fg)', cursor: 'pointer', padding: 0, font: 'inherit', textDecoration: 'underline' }}
            >
              choose what to plot
            </button>
          </>
        ) : running ? (
          'Waiting for the first simulation snapshot…'
        ) : (
          'Run or step the model to populate variables.'
        )}
      </div>
    </div>
  );
}
