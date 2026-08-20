/**
 * compareData — the Phase 6 Compare data layer (react-query).
 *
 *
 *   - `sysml.sessions.list` — the pickable population + per-session
 *     `fork_point_tick` / `forkable_ticks` (via the shared
 *     `useSessionList`, not re-fetched here).
 *   - `sysml.sessions.timeseries_names` / `timeseries_decimated` —
 *     per-(session × variable) curves, keyed by sessionId. This is the
 *     deliberate answer to the Phase 2 deferral ("re-keying by
 *     sessionId is deferred to Phase 6 — the first surface that needs
 *     two live mirrors"): Compare reads RECORDED series through plain
 *     react-query keyed by sessionId, so no second live-socket mirror
 *     and no `sessionLiveStore` re-keying is needed. Recorded reads
 *     poll gently while any picked session is still advancing.
 *   - `sysml.sessions.diff_timeline` — PAIR mode's exact divergence:
 *     `first_divergence_tick`, sparse `tick_diffs`, and the
 *     `history_truncated` honesty flag the banner renders.
 *
 * Query keys live under `compareKeys` so mutations elsewhere can
 * invalidate the whole compare slice with one prefix.
 */

import { useMemo } from 'react';
import { useQueries, useQuery } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import { useSessionList } from '@/features/sessions/queries';
import type {
  SessionSummary,
  SessionTimelineDivergence,
} from '@/features/sessions/types';
import type { TimeseriesNamesResult, TimeseriesResult } from '@/features/sessions/queries';
import type { SamplesBySession } from '../selectors';
import { buildSampleMatrix, pointsToTickSeries, sessionDtMs } from './seriesMath';

function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Query keys ───────────────────────────────────────────────────────

export const compareKeys = {
  all: ['compare'] as const,
  timeline: (a: string, b: string) =>
    [...compareKeys.all, 'diff_timeline', a, b] as const,
  names: (id: string) => [...compareKeys.all, 'names', id] as const,
  series: (id: string, name: string, targetPoints: number) =>
    [...compareKeys.all, 'series', id, name, targetPoints] as const,
  golden: (id: string) => [...compareKeys.all, 'golden', id] as const,
};

/** Points per curve requested from the LTTB decimator. ~2× a typical
 *  chart column width; the backend caps at the real buffer length. */
export const COMPARE_TARGET_POINTS = 600;

// ── Picked summaries ─────────────────────────────────────────────────

/**
 * Resolve picked ids against the live session list. Ids that no longer
 * resolve (reaped / server restart) are reported so the shell can show
 * an honest "no longer available" row instead of silently dropping.
 */
export function usePickedSummaries(pickedIds: string[]): {
  summaries: SessionSummary[];
  missingIds: string[];
  isLoading: boolean;
} {
  const { data, isLoading } = useSessionList();
  return useMemo(() => {
    const byId = new Map((data ?? []).map((s) => [s.id, s]));
    const summaries: SessionSummary[] = [];
    const missingIds: string[] = [];
    for (const id of pickedIds) {
      const s = byId.get(id);
      if (s) summaries.push(s);
      else missingIds.push(id);
    }
    return { summaries, missingIds, isLoading };
  }, [data, pickedIds, isLoading]);
}

// ── Pair timeline diff ───────────────────────────────────────────────

/**
 * Backend timeline divergence for a picked PAIR. Disabled outside
 * 2-pick mode. Polls at 2 s while either side is still advancing so a
 * live fork comparison trails the runs; settles once both complete.
 */
export function useTimelineDiff(
  a: SessionSummary | null,
  b: SessionSummary | null,
) {
  const bothDone = !!a?.completed && !!b?.completed;
  return useQuery({
    queryKey: compareKeys.timeline(a?.id ?? '', b?.id ?? ''),
    queryFn: () =>
      cmd<SessionTimelineDivergence>('sysml.sessions.diff_timeline', {
        a_id: a?.id,
        b_id: b?.id,
      }),
    enabled: !!a && !!b,
    staleTime: 5_000,
    refetchInterval: bothDone ? false : 2_000,
  });
}

// ── Variable discovery ───────────────────────────────────────────────

/**
 * Union of recorded variable names across the picked sessions, sorted.
 * Also reports per-session name sets so the canvas can dim variables a
 * session never recorded (missing ≠ zero).
 */
export function useUnionVariableNames(summaries: SessionSummary[]): {
  names: string[];
  namesBySession: Map<string, Set<string>>;
  isLoading: boolean;
} {
  const queries = useQueries({
    queries: summaries.map((s) => ({
      queryKey: compareKeys.names(s.id),
      queryFn: () =>
        cmd<TimeseriesNamesResult>('sysml.sessions.timeseries_names', {
          session_id: s.id,
        }),
      staleTime: 5_000,
      refetchInterval: s.completed ? false : 2_000,
    })),
  });

  const loadedKey = queries.map((q) => (q.data ? q.data.names.join(',') : '∅')).join('|');
  return useMemo(() => {
    const union = new Set<string>();
    const namesBySession = new Map<string, Set<string>>();
    queries.forEach((q, i) => {
      const id = summaries[i]?.id;
      if (!id || !q.data) return;
      const set = new Set(q.data.names);
      namesBySession.set(id, set);
      for (const n of set) union.add(n);
    });
    return {
      names: Array.from(union).sort(),
      namesBySession,
      isLoading: queries.some((q) => q.isLoading),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadedKey, summaries]);
}

// ── Golden reference (Phase 6, plan row 24) ─────────────────────────

export interface GoldenReferenceData {
  /** Per-variable tick series reconstructed from the archived record's
   *  snapshot history. Empty when the record archived no snapshots. */
  series: Record<string, Array<{ t: number; v: number }>>;
  label: string;
  snapshotCount: number;
}

/**
 * Load a golden-pinned ARCHIVED run (`sysml.sessions.archive.get`) and
 * reconstruct per-variable series from its snapshot history. Archived
 * records are immutable — cache forever.
 */
export function useGoldenReference(archiveId: string | null) {
  return useQuery({
    queryKey: compareKeys.golden(archiveId ?? ''),
    queryFn: async (): Promise<GoldenReferenceData | null> => {
      const res = await cmd<{ entry: { label?: string | null; snapshots?: unknown[] } | null }>(
        'sysml.sessions.archive.get',
        { id: archiveId },
      );
      if (!res.entry) return null;
      const snapshots = res.entry.snapshots ?? [];
      const { archivedSnapshotsToSeries } = await import('./modeMath');
      return {
        series: archivedSnapshotsToSeries(snapshots),
        label: res.entry.label ?? archiveId ?? '',
        snapshotCount: snapshots.length,
      };
    },
    enabled: !!archiveId,
    staleTime: Infinity,
  });
}

// ── Series matrices ──────────────────────────────────────────────────

export interface CompareSeries {
  /** Per-variable rectangular matrix, session order == `summaries`. */
  samplesByVar: Record<string, SamplesBySession>;
  /** Playhead domain: max tick across picked sessions' summaries. */
  maxTick: number;
  isLoading: boolean;
}

/**
 * Fetch decimated series for every (picked session × focused variable)
 * and assemble the per-variable `samples[s][t]` matrices the
 * divergence selectors + canvas consume. Sessions whose dt cannot be
 * derived yet (never stepped) contribute all-NaN rows — honest "no
 * data", never a guessed timebase.
 */
export function useCompareSeries(
  summaries: SessionSummary[],
  varNames: string[],
  targetPoints: number = COMPARE_TARGET_POINTS,
): CompareSeries {
  const pairs = useMemo(
    () =>
      summaries.flatMap((s) =>
        varNames.map((name) => ({ session: s, name })),
      ),
    [summaries, varNames],
  );

  const queries = useQueries({
    queries: pairs.map(({ session, name }) => ({
      queryKey: compareKeys.series(session.id, name, targetPoints),
      queryFn: () =>
        cmd<TimeseriesResult>('sysml.sessions.timeseries_decimated', {
          session_id: session.id,
          var: name,
          target_points: targetPoints,
          start_ms: null,
          end_ms: null,
        }),
      staleTime: 5_000,
      refetchInterval: session.completed ? false : 2_000,
    })),
  });

  const loadedKey = queries
    .map((q) => (q.data ? `${q.data.var}:${q.data.points.length}` : '∅'))
    .join('|');
  return useMemo(() => {
    const maxTick = summaries.reduce((m, s) => Math.max(m, s.tick), 0);
    const samplesByVar: Record<string, SamplesBySession> = {};
    varNames.forEach((name, vi) => {
      const perSession = summaries.map((s, si) => {
        const q = queries[si * varNames.length + vi];
        const dt = sessionDtMs(s);
        if (!q?.data || dt === null) return [];
        return pointsToTickSeries(q.data.points, dt);
      });
      samplesByVar[name] = buildSampleMatrix(perSession, maxTick);
    });
    return {
      samplesByVar,
      maxTick,
      isLoading: queries.some((q) => q.isLoading),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadedKey, summaries, varNames]);
}
