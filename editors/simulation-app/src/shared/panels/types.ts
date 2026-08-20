/**
 * PanelDescriptor — Layer 1 primitive (extensibility plan EP1).
 *
 * A PanelDescriptor describes one result-surface panel in the
 * ResultsWorkbench in purely declarative terms. The registry drives
 * the workbench's panel order and capability filtering, replacing the
 * hard-coded `CardId` switch statement that lived in the old card strip
 * before R1.6.
 *
 * § Layer 1 EP1.
 */
import type { ReactNode } from 'react';
import type { ExpressionAstResult } from '@sysml-rs/expression-view';
import type {
  SessionPhase,
  TimePoint,
  TimelineEntry,
} from '../../features/sessions/types';
import type { ModelCapabilities } from '../../hooks/useModelCapabilities';
import type { StreamActionEntry } from '../../features/results/selectors';

/**
 * Minimal projection of session-level state available to `applicableWhen`.
 *
 * Kept narrow on purpose — the registry decides applicability based on
 * model capabilities + a handful of session flags. Adding more fields here
 * should be a deliberate extensibility decision, not a convenience.
 */
export interface PanelSessionState {
  phase: SessionPhase;
  activeSessionId: string | null;
  /**
   * True when the backend snapshot carries streaming-action data. Today the
   * backend doesn't emit it yet, so the Streams panel stays inactive until
   * it does (see selectors.selectStreamingActions).
   */
  hasStreamingData: boolean;
}

/**
 * View-model surface passed to every panel's `render`. Each panel pulls
 * only the fields it needs; panels that don't care about a given slice can
 * ignore it. This keeps ResultsWorkbench's data plumbing in one place.
 */
export interface PanelProps {
  expanded: boolean;
  onHeaderClick?: () => void;
  running: boolean;

  // Model/session context
  uri?: string | null;

  // Time / tick context (derived from SessionDetail.summary)
  tick: number;
  clockTime: number;

  // Live data channels
  timeSeries: Record<string, TimePoint[]>;
  /**
   * F2b: lazy full-fidelity accessor — call only from click handlers
   * (CSV export etc), never from render. `timeSeries` above may be a
   * source-decimated sample (Plots tab hot path); this always walks
   * every stored point, so it costs O(total stored samples) per call.
   */
  getFullTimeSeries: () => Record<string, TimePoint[]>;
  timelineEntries: TimelineEntry[];
  constraintResults: Array<{
    name: string;
    expression: string;
    pass: boolean;
    actualValue?: string;
  }>;
  streamingActions: StreamActionEntry[];
  expressionResults: ExpressionAstResult[];
  selectedExpressionId?: string | null;
}

export type PanelPosition = 'workbench' | 'detail' | 'utility';

/**
 * Declarative description of a result panel. The panel decides whether it
 * is applicable to the current model/session via `applicableWhen`; the
 * registry is responsible only for iteration order and rendering.
 */
export interface PanelDescriptor {
  /** Stable identity — used for keys, expand/collapse state, and analytics. */
  id: string;
  /** Card header title. */
  title: string;
  /** Material symbol name rendered in the header. */
  icon: string;
  /** Accent / header indicator color. */
  accentColor: string;
  /**
   * Capability / session-aware gate. Panels that return `false` show as
   * inactive state in the workbench (contextual empty-state hint), panels
   * that return `true` render normally.
   */
  applicableWhen: (caps: ModelCapabilities, session: PanelSessionState) => boolean;
  /** Intended placement surface. Utility panels are hosted by the shell drawer. */
  defaultPosition: PanelPosition;
  /** Hint copy rendered in inactive mode when the panel isn't applicable. */
  inactiveHint?: string;
  /** Learn-more URL surfaced in contextual inactive states. */
  learnUrl?: string;
  /** Render in strip mode (default when unexpanded). */
  render: (props: PanelProps) => ReactNode;
  /**
   * Render when expanded in the detail pane. Defaults to `render` with
   * `expanded: true` — override when the expanded layout diverges.
   */
  expandedRender?: (props: PanelProps) => ReactNode;
}
