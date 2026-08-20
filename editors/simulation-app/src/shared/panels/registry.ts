/**
 * panelRegistry — Layer 1 primitive (extensibility plan EP1).
 *
 * The canonical list of result-surface panels, in workbench order. Each
 * descriptor co-located here so the card files stay render-only; new
 * panels (Variables pane, Monte Carlo histogram, Verdict matrix, ...)
 * slot in by pushing to this array and need no ResultsWorkbench edit.
 */
import { createElement } from 'react';
import { PlotsTab } from '../../features/results/plots/PlotsTab';
import { StateTimelineCard } from '../../components/cards/StateTimelineCard';
import { ConstraintCard } from '../../components/cards/ConstraintCard';
import { KpisTab } from '../../features/results/kpis/KpisTab';
import { EquationsTab } from '../../features/results/equations/EquationsTab';
import { StreamCard } from '../../components/cards/StreamCard';
import { breakpointsPanel } from './breakpoints';
import { variablesPanel } from './variables';
import { archivePanel } from './archive';
import { diagnosticsPanel } from './diagnostics';
import { sourcePanel } from './source';
import { viewsPanel } from './views';
import { integrationsPanel } from './integrations';
import { debugPanel } from './debug';
import { TraceabilityMatrixPanel } from '../../features/traceability/TraceabilityMatrixPanel';
import { CausalTracePanel } from '../../features/causal-trace/CausalTracePanel';
import type { PanelDescriptor } from './types';

const BOOK_BASE = 'https://www.omg.org/spec/SysML/';

/**
 * Plots panel — live ODE/variable traces. Active iff the model has any
 * ODE dynamics. Empty-state copy inside the tab handles the "no
 * variables picked" case.
 */
const plotsPanel: PanelDescriptor = {
  id: 'plots',
  title: 'Plots',
  icon: 'show_chart',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'workbench',
  inactiveHint:
    'Add `calc def :> GetDerivative` or run a model with numeric variables to build custom time-series plots.',
  learnUrl: `${BOOK_BASE}/calculations`,
  applicableWhen: (caps) => caps.hasODEs,
  render: (props) =>
    createElement(PlotsTab, {
      timeSeries: props.timeSeries,
      getFullTimeSeries: props.getFullTimeSeries,
      running: props.running,
      expanded: props.expanded,
      onHeaderClick: props.onHeaderClick,
    }),
};

/** State timeline — swimlane view. Active iff the model has state machines. */
const stateTimelinePanel: PanelDescriptor = {
  id: 'stateTimeline',
  title: 'State Timeline',
  icon: 'timeline',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'workbench',
  inactiveHint:
    'Add `state def` with transitions to see state machine timelines as horizontal swim-lane bars with event markers.',
  learnUrl: `${BOOK_BASE}/state-machines`,
  applicableWhen: (caps) => caps.hasStateMachines,
  render: (props) =>
    createElement(StateTimelineCard, {
      entries: props.timelineEntries,
      currentStep: props.tick,
      running: props.running,
      expanded: props.expanded,
      onHeaderClick: props.onHeaderClick,
    }),
};

/** Constraint pass/fail pills — live during eval. */
const constraintsPanel: PanelDescriptor = {
  id: 'constraints',
  title: 'Constraints',
  icon: 'rule',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'workbench',
  inactiveHint:
    'Add `constraint def` or `assert constraint` to track pass/fail status with rendered math and actual values.',
  learnUrl: `${BOOK_BASE}/constraints`,
  applicableWhen: (caps) => caps.hasConstraints,
  render: (props) =>
    createElement(ConstraintCard, {
      constraints: props.constraintResults,
      running: props.running,
      expanded: props.expanded,
      onHeaderClick: props.onHeaderClick,
    }),
};

/**
 * Equations card — rendered math for every expression-bearing element.
 * Always applicable (mirrors pre-registry behaviour); gated inside the
 * component by the `EXPRESSION_VIEW_ENABLED` feature flag.
 */
const equationsPanel: PanelDescriptor = {
  id: 'equations',
  title: 'Equations',
  icon: 'function',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'workbench',
  inactiveHint:
    'Add `calc def` or `constraint def` with expressions to inspect rendered math, symbols, and latest values.',
  learnUrl: `${BOOK_BASE}/expressions`,
  applicableWhen: () => true,
  render: (props) =>
    createElement(EquationsTab, {
      results: props.expressionResults,
      timeSeries: props.timeSeries,
      uri: props.uri,
      loading: false,
      selectedElementId: props.selectedExpressionId,
      expanded: props.expanded,
      onHeaderClick: props.onHeaderClick,
    }),
};

/**
 * Streams — only active when the backend snapshot actually carries
 * streaming-action data. Stays inactive today; auto-promotes to active
 * when the backend lands `streaming_actions` on ExecutionSnapshot.
 */
const streamsPanel: PanelDescriptor = {
  id: 'streams',
  title: 'Streams',
  icon: 'stream',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'workbench',
  inactiveHint:
    'Stream monitoring is in progress. Backend support is queued \u2014 ExecutionSnapshot will surface streaming_actions in a future release.',
  learnUrl: `${BOOK_BASE}/actions`,
  applicableWhen: (_caps, session) => session.hasStreamingData,
  render: (props) =>
    createElement(StreamCard, {
      streams: props.streamingActions,
      clockTimeMs: props.clockTime,
      running: props.running,
      expanded: props.expanded,
      onHeaderClick: props.onHeaderClick,
    }),
};

/**
 * KPIs — always applicable. The card decides internally whether it has
 * enough numeric data to compute any KPIs (empty state is shown when it
 * doesn't).
 */
const kpiPanel: PanelDescriptor = {
  id: 'kpi',
  title: 'KPIs',
  icon: 'speed',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'workbench',
  inactiveHint:
    'Add numeric `attribute` values or ODE state variables to define KPIs with thresholds.',
  learnUrl: `${BOOK_BASE}/attributes`,
  applicableWhen: () => true,
  render: (props) =>
    createElement(KpisTab, {
      timeSeries: props.timeSeries,
      clockTime: props.clockTime,
      expanded: props.expanded,
      onHeaderClick: props.onHeaderClick,
    }),
};

/**
 * R6.2 — Traceability matrix panel. Position `'detail'` because the
 * matrix reads wider than the sidebar column can host (rows ×
 * potentially many columns). Always applicable — the viewer's own
 * empty-state handles the "no trace edges" case.
 *
 * Accent: was rose, historically distinct from Archive's pink and the
 * workbench accents, picked so the detail-pane tabs stay visually
 * separable once the R6 panels start sharing a surface. ninebar sweep:
 * accentColor now resolves to --text-secondary; the hex this comment
 * once cited is historical and no longer a live value.
 */
const traceabilityMatrixPanel: PanelDescriptor = {
  id: 'traceabilityMatrix',
  title: 'Traceability',
  icon: 'account_tree',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'detail',
  inactiveHint:
    'Add `satisfy` / `verify` / `derive` relationships between requirements and model elements to populate the matrix.',
  learnUrl: `${BOOK_BASE}/requirements`,
  applicableWhen: () => true,
  render: () => createElement(TraceabilityMatrixPanel),
  expandedRender: () => createElement(TraceabilityMatrixPanel),
};

/**
 * R7.1 — Causal trace panel. Position `'detail'` because the vertical
 * timeline reads deep and wants the full pane height. Always applicable
 * — the panel's own empty state handles the "no root selected" case.
 *
 * Accent: was violet, historically distinct from Variables' violet,
 * Breakpoints' red, Archive's rose, Diagnostics' amber, and Traceability's
 * rose. Exposed as the CSS custom property `--sim-causal-trace` so the
 * row component + header share it. ninebar sweep: accentColor now
 * resolves to --text-secondary; the hex this comment once cited are
 * historical and no longer live values.
 */
const causalTracePanel: PanelDescriptor = {
  id: 'causalTrace',
  title: 'Causal trace',
  icon: 'device_hub',
  accentColor: 'var(--text-secondary)',
  defaultPosition: 'detail',
  inactiveHint:
    'Click a failing verdict or a triggered breakpoint to walk backward through the causation graph and see what led to it.',
  learnUrl: `${BOOK_BASE}/constraints`,
  applicableWhen: () => true,
  render: () => createElement(CausalTracePanel),
  expandedRender: () => createElement(CausalTracePanel),
};

/**
 * The canonical workbench order. ResultsWorkbench groups these panels into
 * task-oriented tabs and uses `applicableWhen` for contextual empty states.
 */
export const panelRegistry: PanelDescriptor[] = [
  plotsPanel,
  stateTimelinePanel,
  constraintsPanel,
  equationsPanel,
  streamsPanel,
  kpiPanel,
  breakpointsPanel,
  // R2.2 (Agent I) — legacy Variables pane. Position 'utility' and
  // hidden by applicableWhen; filtered out of workbench consumers by defaultPosition.
  variablesPanel,
  // R4.1 — Session archive panel. Position 'utility'; lists / searches /
  // filters / restores archived sessions from the shell drawer.
  archivePanel,
  // R6.1 — Diagnostics panel. Position 'utility'; groups parse +
  // semantic diagnostics by file with severity / search / scope filters
  // and click-to-navigate from the shell drawer.
  diagnosticsPanel,
  // R6.2 — Traceability matrix panel. Position 'detail'; shows
  // requirement → part satisfaction edges with 4-valued verdict cells
  // and sticky row / column headers. Appended (not reordered) to stay
  // cooperative with parallel registry edits.
  traceabilityMatrixPanel,
  // R7.1 — Causal trace panel. Position 'detail'; walks backward through
  // the causation graph from a failing verdict or breakpoint hit.
  // Appended (not reordered) to stay cooperative with parallel registry
  // edits.
  causalTracePanel,
  // S4.T4 — Source panel. Position 'utility'; mounts MonacoSysmlEditor
  // against the currently-selected element via sysml.get_source. The
  // mount also registers the `sysml` language id + view snippets so the
  // same Monaco instances can be reused by sneak-peeks (T5) and the
  // eventual live editor (post-ADR-013).
  sourcePanel,
  // Phase 5 — Views panel. Position 'utility'; lists user-authored
  // ViewUsage / ViewDefinition declarations and routes the diagram
  // pane to render the picked one via sysml.views.render.
  //
  // Bucket 5-followup-2 (2026-05-05): Outline panel was removed —
  // the Run page's SessionTree already shows the containment
  // hierarchy, so a separate outline drawer was redundant. The
  // per-element "views exposing this element" affordance now lives
  // on each SessionTree row.
  viewsPanel,
  // Phase 7 — Integrations panel. Position 'utility'; renders MCP /
  // REST / LSP connection details + copy-snippets so users can wire
  // external agents (Claude Desktop / Claude Code) and scripts to
  // this backend. Always-on product feature (no env gate).
  integrationsPanel,
  // Phase 8 — Debug drawer. Position 'utility'; surfaced as a toolbar
  // affordance only when `VITE_DEBUG_DRAWER=1` (see
  // `isDebugDrawerEnabled()` and `UtilityDrawer`). The descriptor is
  // always present so other consumers can `findPanel('debug')`, but
  // the toolbar gate keeps it out of production / default-dev surface.
  debugPanel,
];

/** Lookup helper — returns `undefined` when no panel matches. */
export function findPanel(id: string): PanelDescriptor | undefined {
  return panelRegistry.find((p) => p.id === id);
}
