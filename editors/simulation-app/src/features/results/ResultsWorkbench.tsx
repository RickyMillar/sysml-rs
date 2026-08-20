/**
 * ResultsWorkbench -- tabbed bottom workbench for Run.
 *
 * Replaces the old horizontal result-card strip. The workbench keeps the
 * same panel registry/data plumbing, but groups capability panels by user
 * task: plots, KPIs, equations, constraints, timelines, and raw/advanced
 * data. Inactive capabilities now appear as contextual empty
 * states inside the relevant tab instead of crowding the bottom bar.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';
import { useModelCapabilities } from '@/hooks/useModelCapabilities';
import { useSessionStore } from '@/features/sessions/store';
import { useSessionDetail } from '@/features/sessions/queries';
import { useSessionLiveStore } from '@/features/sessions/sessionLiveStore';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { PerfBudget } from '@/shared/perf';
import {
  selectConstraintResults,
  selectStreamingActions,
  useStateTimelineIngest,
  useStateTimelineStore,
} from './selectors';
import { useExpressionAst } from './useExpressionAst';
import { findPanel, panelRegistry } from '@/shared/panels/registry';
import type { PanelDescriptor, PanelProps, PanelSessionState } from '@/shared/panels/types';
import type { TimePoint } from '@/features/sessions/types';
import type { VerdictKind } from '@/components/VerdictBadge';

type WorkbenchTabId =
  | 'plots'
  | 'kpis'
  | 'equations'
  | 'constraints'
  | 'timeline';

interface WorkbenchTab {
  id: WorkbenchTabId;
  label: string;
  icon: string;
  panelIds: string[];
  emptyTitle: string;
  emptyHint: string;
}

const WORKBENCH_TABS: WorkbenchTab[] = [
  {
    id: 'plots',
    label: 'Plots',
    icon: 'show_chart',
    panelIds: ['plots'],
    emptyTitle: 'No plottable signals yet',
    emptyHint:
      'Run a model with ODE state, numeric attributes, or flow results to plot time-series data here.',
  },
  {
    id: 'kpis',
    label: 'KPIs',
    icon: 'speed',
    panelIds: ['kpi'],
    emptyTitle: 'No KPI data yet',
    emptyHint:
      'Run a model with numeric time-series data to compute and inspect key metrics.',
  },
  {
    id: 'equations',
    label: 'Equations',
    icon: 'function',
    panelIds: ['equations'],
    emptyTitle: 'No equations discovered',
    emptyHint:
      'Add calc or constraint expressions to inspect rendered equations and their symbols.',
  },
  {
    id: 'constraints',
    label: 'Constraints',
    icon: 'rule',
    panelIds: ['constraints'],
    emptyTitle: 'No constraint results yet',
    emptyHint:
      'Add constraints or run verification to inspect pass/fail status and evaluated values.',
  },
  {
    id: 'timeline',
    label: 'Timeline',
    icon: 'timeline',
    panelIds: ['stateTimeline', 'streams'],
    emptyTitle: 'No timeline events yet',
    emptyHint:
      'Run a model with state machines or streaming actions to see event timelines here.',
  },
  // NOTE: the "Raw Data" tab was removed (R56) — it had no panels and only
  // ever showed a "lives here next" placeholder, which read as a broken peer
  // tab. Re-add it here once a structured run-data inspector exists; until then
  // session Export covers raw access.
];

/** Stable empty time-series map — reused instead of allocating a fresh
 *  `{}` every render for tabs that don't need it (UX closeout #4). */
const EMPTY_TIME_SERIES: Record<string, TimePoint[]> = {};

function defaultTabFor(activeIds: Set<string>): WorkbenchTabId {
  if (activeIds.has('plots')) return 'plots';
  if (activeIds.has('constraints')) return 'constraints';
  if (activeIds.has('stateTimeline') || activeIds.has('streams')) return 'timeline';
  if (activeIds.has('kpi')) return 'kpis';
  return 'equations';
}

function tabState(
  tab: WorkbenchTab,
  panelStates: Map<string, { panel: PanelDescriptor; applicable: boolean }>,
) {
  const panels = tab.panelIds
    .map((id) => panelStates.get(id))
    .filter((state): state is { panel: PanelDescriptor; applicable: boolean } => !!state);
  return {
    panels,
    activeCount: panels.filter((p) => p.applicable).length,
  };
}

export function ResultsWorkbench() {
  const caps = useModelCapabilities();
  const location = useLocation();
  const deepLink = useMemo(() => {
    const params = new URLSearchParams(location.search);
    const resultTab = params.get('result_tab') as WorkbenchTabId | null;
    return {
      tab: resultTab && WORKBENCH_TABS.some((tab) => tab.id === resultTab) ? resultTab : null,
      equation: params.get('equation'),
    };
  }, [location.search]);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const phase = useSessionStore((s) => s.phase);
  const { data: sessionDetail } = useSessionDetail(activeSessionId);
  const liveSnapshot = useSessionLiveStore(
    (s) => (s.sessionId === activeSessionId ? s.snapshot : null),
  );

  // Keep state-machine history ingest mounted while Run is visible.
  useStateTimelineIngest();
  const timelineBySession = useStateTimelineStore((s) => s.bySession);

  const running = phase === 'running';

  const constraintResults = useMemo(() => {
    if (liveSnapshot) {
      return liveSnapshot.constraint_results.map((c) => ({
        name: c.name,
        expression: c.expression ?? '',
        pass: c.verdict === 'Pass',
        // Carry the four-valued verdict, not just the bool: ConstraintCard
        // groups by it, and without it every non-pass row falls back to
        // `pass ? 'pass' : 'fail'` — which renders the run's *undecided*
        // constraints as violations, the exact conflation the backend
        // change removed from the wire.
        verdict: c.verdict === undefined
          ? undefined
          : (c.verdict.toLowerCase() as VerdictKind),
      }));
    }
    return selectConstraintResults(sessionDetail);
  }, [liveSnapshot, sessionDetail]);

  const timelineEntries = useMemo(
    () => (activeSessionId ? timelineBySession[activeSessionId] ?? [] : []),
    [activeSessionId, timelineBySession],
  );
  const streamingActions = useMemo(
    () => selectStreamingActions(sessionDetail),
    [sessionDetail],
  );

  const hasStreamingData = streamingActions.length > 0;
  const tick = liveSnapshot?.tick ?? sessionDetail?.summary?.tick ?? 0;
  const clockTime = liveSnapshot?.time_ms ?? sessionDetail?.summary?.time_ms ?? 0;

  const tsRevision = useTimeSeriesStore((s) => s.revision);

  const sessionUri = sessionDetail?.summary?.uri ?? null;
  const { data: expressionResults = [] } = useExpressionAst(sessionUri);

  const sessionState: PanelSessionState = useMemo(
    () => ({ phase, activeSessionId, hasStreamingData }),
    [phase, activeSessionId, hasStreamingData],
  );

  const statesById = useMemo(() => {
    const entries = panelRegistry
      .filter((panel) => panel.defaultPosition === 'workbench')
      .map((panel) => [
        panel.id,
        { panel, applicable: panel.applicableWhen(caps, sessionState) },
      ] as const);
    return new Map(entries);
  }, [caps, sessionState]);

  const activeIds = useMemo(() => {
    const ids = new Set<string>();
    for (const [id, state] of statesById) {
      if (state.applicable) ids.add(id);
    }
    return ids;
  }, [statesById]);

  const [selectedTab, setSelectedTab] = useState<WorkbenchTabId>(() => deepLink.tab ?? defaultTabFor(activeIds));
  const userSelectedTab = useRef(false);

  useEffect(() => {
    if (deepLink.tab) {
      setSelectedTab(deepLink.tab);
      return;
    }
    if (!userSelectedTab.current) {
      setSelectedTab(defaultTabFor(activeIds));
    }
  }, [activeIds, deepLink.tab]);

  const activeTab = WORKBENCH_TABS.find((tab) => tab.id === selectedTab) ?? WORKBENCH_TABS[0]!;
  const selectedState = tabState(activeTab, statesById);
  const activePanels = selectedState.panels.filter((state) => state.applicable);

  // UX closeout #4 / #17: rebuilding the full `Record<string,
  // TimePoint[]>` from the ring buffer is O(total stored samples) — on
  // a big model (espresso-production-cell's ~14k variables) that's real work
  // to pay on *every* tick (tsRevision bumps once per tick) even when
  // the active tab can't render it (Constraints/Equations/Timeline
  // don't consume `timeSeries`). Only pay it for the tabs that do.
  const needsTimeSeries = activeTab.id === 'plots' || activeTab.id === 'kpis';
  // F2b: even gated to the Plots tab, `getTimeSeries()` still walks/copies
  // every ring from scratch — over a long run (hybrid run-to-trip ~132k
  // ticks x ~60 vars) that per-flush O(total) copy is what saturates the
  // main thread near the tail, independent of F1's 60Hz cap. The Plots
  // chart only ever displays a decimated view anyway (F2, TimeSeriesViewer
  // LTTB), so feed it a source-side decimated read instead — O(budget)
  // regardless of run length. KPIs still needs every sample (min/max/mean/
  // rms over the whole run), so that path keeps the full-fidelity read.
  const timeSeries = useMemo(() => {
    if (!needsTimeSeries) return EMPTY_TIME_SERIES;
    const store = useTimeSeriesStore.getState();
    return activeTab.id === 'plots'
      ? store.getDecimatedTimeSeries(PerfBudget.MAX_RENDER_POINTS)
      : store.getTimeSeries();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tsRevision, needsTimeSeries, activeTab.id]);

  // F2b: CSV export / other exact-fidelity consumers must never read the
  // decimated series above. Handed down as a lazy accessor (not a value)
  // so exporting stays O(total) only on click, not on every ~60Hz flush.
  const getFullTimeSeries = useMemo(
    () => () => useTimeSeriesStore.getState().getTimeSeries(),
    [],
  );

  const baseProps: Omit<PanelProps, 'expanded' | 'onHeaderClick'> = {
    uri: sessionUri,
    running,
    tick,
    clockTime,
    timeSeries,
    getFullTimeSeries,
    timelineEntries,
    constraintResults,
    streamingActions,
    expressionResults,
    selectedExpressionId: deepLink.equation,
  };

  return (
    <section
      data-testid="results-workbench"
      className="shrink-0 flex flex-col overflow-hidden"
      style={{
        minHeight: 220,
        maxHeight: '42vh',
        borderTop: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-low)',
      }}
    >
      <div
        role="tablist"
        aria-label="Run results workbench"
        className="flex items-center gap-1 px-2 py-1.5 shrink-0 overflow-x-auto"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        {WORKBENCH_TABS.map((tab) => {
          const state = tabState(tab, statesById);
          const selected = tab.id === activeTab.id;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={selected}
              data-testid={`results-workbench-tab-${tab.id}`}
              onClick={() => {
                userSelectedTab.current = true;
                setSelectedTab(tab.id);
              }}
              className="inline-flex items-center gap-1.5 px-2.5 py-1 transition-all"
              style={{
                border: '1px solid var(--outline-variant)',
                borderRadius: 6,
                background: selected ? 'var(--primary-container)' : 'var(--surface-container)',
                color: selected ? 'var(--on-primary-container)' : 'var(--on-surface-variant)',
                fontSize: 11,
                fontWeight: selected ? 700 : 500,
                cursor: 'pointer',
                whiteSpace: 'nowrap',
              }}
            >
              <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
                {tab.icon}
              </span>
              {tab.label}
              {state.activeCount > 0 && (
                <span
                  className="mono-text"
                  style={{
                    fontSize: 10,
                    color: selected ? 'inherit' : 'var(--outline)',
                    opacity: 0.85,
                  }}
                >
                  {state.activeCount}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <div
        data-testid={`results-workbench-panel-${activeTab.id}`}
        role="tabpanel"
        className="flex-1 min-h-0 overflow-auto p-2"
      >
        {activePanels.length > 0 ? (
          <div className="grid gap-2" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(320px, 1fr))' }}>
            {activePanels.map(({ panel }) => {
              const def = findPanel(panel.id) ?? panel;
              const props: PanelProps = {
                ...baseProps,
                expanded: true,
                onHeaderClick: undefined,
              };
              return (
                <div key={panel.id} data-testid={`results-workbench-card-${panel.id}`}>
                  {(def.expandedRender ?? def.render)(props)}
                </div>
              );
            })}
          </div>
        ) : (
          <EmptyWorkbenchTab tab={activeTab} inactivePanels={selectedState.panels.map((s) => s.panel)} />
        )}
      </div>
    </section>
  );
}

function EmptyWorkbenchTab({
  tab,
  inactivePanels,
}: {
  tab: WorkbenchTab;
  inactivePanels: PanelDescriptor[];
}) {
  return (
    <div
      data-testid={`results-workbench-empty-${tab.id}`}
      className="flex flex-col items-center justify-center h-full gap-2 px-6 py-8"
      style={{ color: 'var(--outline)', textAlign: 'center' }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 32, opacity: 0.75 }}>
        {tab.icon}
      </span>
      <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--on-surface-variant)' }}>
        {tab.emptyTitle}
      </div>
      <div style={{ fontSize: 11, maxWidth: 520, lineHeight: 1.5 }}>{tab.emptyHint}</div>
      {inactivePanels.length > 0 && (
        <div
          className="flex flex-wrap justify-center gap-1.5"
          style={{ marginTop: 8, maxWidth: 640 }}
        >
          {inactivePanels.map((panel) => (
            <span
              key={panel.id}
              title={panel.inactiveHint}
              className="inline-flex items-center gap-1 px-2 py-0.5"
              style={{
                border: '1px dashed var(--outline-variant)',
                borderRadius: 999,
                fontSize: 10,
                color: 'var(--outline)',
              }}
            >
              <span className="material-symbols-outlined" style={{ fontSize: 12 }}>
                {panel.icon}
              </span>
              {panel.title}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
