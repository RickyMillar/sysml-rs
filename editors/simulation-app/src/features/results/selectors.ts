/**
 * Derived-data selectors for the results surface.
 *
 * Pure functions that extract view-ready data from normalized session
 * payloads. Most exports are pure — the exception is the state-timeline
 * history store + its ingest hook, which accumulate subsystem state
 * transitions across snapshots since the backend's `latest_snapshot`
 * never carries a `timeline_entries` field of its own.
 *
 * Per ADR-008: LTTB decimation caps chart data at 1,500 points per
 * series to keep uPlot / SVG renders under budget.
 */

import { useEffect, useRef } from 'react';
import { create } from 'zustand';
import type { TimePoint, TimelineEntry, ConstraintRow } from '../sessions/types';
import type { SessionDetail } from '../sessions/types';
import type { VerdictKind } from '@/components/VerdictBadge';
import { useSessionStore } from '../sessions/store';
import { useSessionDetail } from '../sessions/queries';

// ── Constants ─────────────────────────────────────────────────────────

/** Maximum rendered points per waveform series (ADR-008). */
const MAX_RENDER_POINTS = 1_500;

/**
 * Upper bound on retained state-timeline entries per session.
 *
 * Cheap ring-buffer cap: a 10 Hz Run session produces entries only on
 * state transitions (consecutive identical states are deduped), so 1024
 * is plenty for interactive runs. Prevents unbounded growth on sims with
 * oscillating state machines.
 */
const MAX_TIMELINE_ENTRIES = 1024;

// ── LTTB decimation ───────────────────────────────────────────────────

/**
 * Largest-Triangle-Three-Buckets downsampling.
 *
 * Preserves visual extrema better than naive Nth-point sampling.
 * Returns the input unchanged when `points.length <= threshold`.
 */
export function lttbDecimate(
  points: TimePoint[],
  threshold: number = MAX_RENDER_POINTS,
): TimePoint[] {
  const len = points.length;
  if (len <= threshold || threshold < 3) return points;

  const out: TimePoint[] = [points[0]]; // always keep first
  const bucketSize = (len - 2) / (threshold - 2);

  let prevIndex = 0;

  for (let i = 0; i < threshold - 2; i++) {
    // Bucket boundaries
    const bucketStart = Math.floor((i + 1) * bucketSize) + 1;
    const bucketEnd = Math.min(Math.floor((i + 2) * bucketSize) + 1, len - 1);

    // Average of next bucket (for triangle area computation)
    let avgT = 0;
    let avgV = 0;
    const nextStart = Math.floor((i + 2) * bucketSize) + 1;
    const nextEnd = Math.min(Math.floor((i + 3) * bucketSize) + 1, len - 1);
    const nextLen = nextEnd - nextStart;
    if (nextLen > 0) {
      for (let j = nextStart; j < nextEnd; j++) {
        avgT += points[j].t;
        avgV += points[j].v;
      }
      avgT /= nextLen;
      avgV /= nextLen;
    } else {
      // Last bucket — use last point
      avgT = points[len - 1].t;
      avgV = points[len - 1].v;
    }

    // Find point in current bucket with largest triangle area
    let maxArea = -1;
    let bestIdx = bucketStart;
    const prevT = points[prevIndex].t;
    const prevV = points[prevIndex].v;

    for (let j = bucketStart; j < bucketEnd; j++) {
      const area = Math.abs(
        (prevT - avgT) * (points[j].v - prevV) -
        (prevT - points[j].t) * (avgV - prevV),
      );
      if (area > maxArea) {
        maxArea = area;
        bestIdx = j;
      }
    }

    out.push(points[bestIdx]);
    prevIndex = bestIdx;
  }

  out.push(points[len - 1]); // always keep last
  return out;
}

// ── Time-series extraction ────────────────────────────────────────────

/**
 * Extract decimated time-series data from accumulated session snapshots.
 *
 * Accepts the raw time-series map (variable name -> points) and returns
 * a render-ready copy with LTTB applied per series.
 */
export function selectDecimatedTimeSeries(
  timeSeries: Record<string, TimePoint[]>,
): Record<string, TimePoint[]> {
  const result: Record<string, TimePoint[]> = {};
  for (const [name, points] of Object.entries(timeSeries)) {
    result[name] = lttbDecimate(points, MAX_RENDER_POINTS);
  }
  return result;
}

// ── Constraint extraction ─────────────────────────────────────────────

/**
 * Extract constraint results from a session detail's latest snapshot.
 */
export function selectConstraintResults(
  detail: SessionDetail | undefined,
): Array<{
  name: string;
  expression: string;
  pass: boolean;
  verdict?: VerdictKind;
  actualValue?: string;
}> {
  if (!detail?.latest_snapshot) return [];
  const snap = detail.latest_snapshot as Record<string, unknown>;
  const raw = snap.constraint_results;
  if (!Array.isArray(raw)) return [];

  return raw.map((r: unknown) => {
    const row = r as Record<string, unknown>;
    return {
      name: String(row.name ?? ''),
      expression: String(row.expression ?? ''),
      // Only a decided `pass` is a pass; `inconclusive` is not a silent pass.
      // `pass` alone cannot say which, though — carry the four-valued verdict
      // alongside it so an undecided constraint is not rendered as a
      // violation. Consumers that only understand the bool keep working.
      pass: row.verdict === 'Pass',
      verdict: toVerdictKind(row.verdict),
      actualValue: row.actual != null ? String(row.actual) : undefined,
    };
  });
}

/**
 * Fold the wire's PascalCase `VerdictKind` into this layer's lowercase
 * vocabulary. Returns `undefined` for a missing or unrecognised value so the
 * consumer falls back to its `pass` bool rather than throwing — a row without
 * a verdict is backend/frontend version skew, not a data condition.
 */
function toVerdictKind(raw: unknown): VerdictKind | undefined {
  if (typeof raw !== 'string') return undefined;
  const v = raw.toLowerCase();
  return v === 'pass' || v === 'fail' || v === 'inconclusive' || v === 'error'
    ? (v as VerdictKind)
    : undefined;
}

// ── KPI summary computation ───────────────────────────────────────────

export interface KPISummary {
  label: string;
  value: string;
  unit: string;
}

/**
 * Compute KPI summaries from time-series data.
 * Pure function, no hooks.
 */
export function selectKPISummaries(
  timeSeries: Record<string, TimePoint[]>,
  clockTime: number,
): KPISummary[] {
  const kpis: KPISummary[] = [];
  for (const [name, points] of Object.entries(timeSeries)) {
    if (points.length === 0) continue;
    const lower = name.toLowerCase();

    if (lower.includes('current') || lower.startsWith('i_') || lower === 'totalcurrent') {
      const peak = Math.max(...points.map((p) => Math.abs(p.v)));
      kpis.push({ label: `Peak ${name}`, value: peak.toFixed(1), unit: 'A' });
    }
    if (lower.startsWith('t_') || lower.includes('temp')) {
      const max = Math.max(...points.map((p) => p.v));
      kpis.push({ label: `Max ${name}`, value: max.toFixed(1), unit: 'K' });
    }
  }
  if (clockTime > 0) {
    kpis.push({ label: 'Sim Time', value: (clockTime / 1000).toFixed(2), unit: 's' });
  }
  return kpis.slice(0, 8);
}

// ── Timeline history store ────────────────────────────────────────────

/**
 * Per-session history of subsystem state observations.
 *
 * The backend's `latest_snapshot` contract does not include a pre-built
 * `timeline_entries` array, so we reconstruct one client-side by sampling
 * `SessionDetail.subsystems[*].current_state` each time a new snapshot
 * arrives. Each entry is a single tick observation; consecutive identical
 * states are deduplicated to keep memory bounded for slow-moving SMs.
 */
interface StateTimelineStoreState {
  /** Map of session id -> ordered timeline entries observed so far. */
  bySession: Record<string, TimelineEntry[]>;
  /**
   * Append an entry for `sessionId`. No-ops when the entry duplicates the
   * previous tick (same tick number) or carries an identical subsystem
   * state map to the most recent observation.
   */
  append: (sessionId: string, entry: TimelineEntry) => void;
  /** Drop history for a single session (e.g., on session switch). */
  reset: (sessionId: string) => void;
  /**
   * Prune history for every session except `currentId`. Used on
   * session switch so abandoned sessions from earlier in the tab's
   * lifetime don't accumulate indefinitely.
   */
  clearAllExcept: (currentId: string | null) => void;
}

export const useStateTimelineStore = create<StateTimelineStoreState>((set, get) => ({
  bySession: {},
  append: (sessionId, entry) => {
    const list = get().bySession[sessionId] ?? [];
    const last = list[list.length - 1];
    if (last) {
      if (last.tick === entry.tick) return; // dedup re-poll of same tick
      // Dedup unchanged states (memory bound for long idle runs).
      const sameStates = (() => {
        const a = last.subsystems;
        const b = entry.subsystems;
        const aKeys = Object.keys(a);
        const bKeys = Object.keys(b);
        if (aKeys.length !== bKeys.length) return false;
        for (const k of aKeys) if (a[k] !== b[k]) return false;
        return true;
      })();
      if (sameStates) return;
    }
    // Ring-buffer cap: evict the oldest entries on overflow.
    const nextList = list.length >= MAX_TIMELINE_ENTRIES
      ? [...list.slice(list.length - MAX_TIMELINE_ENTRIES + 1), entry]
      : [...list, entry];
    set({
      bySession: {
        ...get().bySession,
        [sessionId]: nextList,
      },
    });
  },
  reset: (sessionId) => {
    const next = { ...get().bySession };
    delete next[sessionId];
    set({ bySession: next });
  },
  clearAllExcept: (currentId) => {
    const current = get().bySession;
    if (currentId == null) {
      if (Object.keys(current).length === 0) return;
      set({ bySession: {} });
      return;
    }
    const retained = current[currentId];
    if (retained === undefined && Object.keys(current).length === 0) return;
    set({
      bySession: retained === undefined ? {} : { [currentId]: retained },
    });
  },
}));

/**
 * Effect-only hook: watches the active session's detail query and feeds
 * each new snapshot's subsystem-state map into `useStateTimelineStore`.
 * Sibling pattern to `useTimeSeriesIngest`.
 */
export function useStateTimelineIngest(): void {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const { data: detail } = useSessionDetail(activeSessionId);
  const lastTickRef = useRef<number | null>(null);

  // Reset dedupe + prune history for every abandoned session when the
  // active session changes. Keeps the incoming session's entries (if any)
  // so a reattach doesn't wipe in-flight observations.
  useEffect(() => {
    lastTickRef.current = null;
    useStateTimelineStore.getState().clearAllExcept(activeSessionId);
  }, [activeSessionId]);

  useEffect(() => {
    if (!activeSessionId || !detail) return;
    const tick = detail.summary?.tick;
    const timeMs = detail.summary?.time_ms;
    if (typeof tick !== 'number' || typeof timeMs !== 'number') return;
    if (lastTickRef.current !== null && tick <= lastTickRef.current) return;

    const subsystems: Record<string, string> = {};
    for (const s of detail.subsystems ?? []) {
      if (s?.name && typeof s.current_state === 'string') {
        subsystems[s.name] = s.current_state;
      }
    }
    if (Object.keys(subsystems).length === 0) return;

    useStateTimelineStore.getState().append(activeSessionId, { tick, timeMs, subsystems });
    lastTickRef.current = tick;
  }, [activeSessionId, detail]);
}

// ── Timeline extraction ───────────────────────────────────────────────

/**
 * Read the accumulated state-timeline history for a session.
 *
 * Returns entries in tick order. When `sessionId` is null/unknown returns
 * an empty array. The history is populated by `useStateTimelineIngest`,
 * which must be mounted somewhere up the tree (currently `ResultsWorkbench`).
 */
export function selectTimelineEntries(sessionId: string | null): TimelineEntry[] {
  if (!sessionId) return [];
  return useStateTimelineStore.getState().bySession[sessionId] ?? [];
}

// ── Streaming action extraction ──────────────────────────────────────

export interface StreamActionEntry {
  name: string;
  emissions: number;
  lastValue: string | null;
}

/**
 * Streaming-action emissions snapshot.
 *
 * The backend `ExecutionSnapshot` does not yet expose a `streaming_actions`
 * field (see crates/tooling/sysml-service/README.md). Until it does this
 * always returns an empty array, which keeps the workbench timeline tab in
 * its empty-state path. The unused `_detail` parameter is retained so callers
 * can pass session detail forward without a breaking signature change once
 * the backend lands the field.
 */
export function selectStreamingActions(
  _detail: SessionDetail | undefined,
): StreamActionEntry[] {
  return [];
}
