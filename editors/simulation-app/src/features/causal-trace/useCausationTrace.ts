/**
 * useCausationTrace — react-query hook for the CausalTracePanel (R7.1).
 *
 * Thin wrapper around `sysml.causation.trace`. Given a root selection
 * ({ sessionId, eventId } or { sessionId, tick, target }), the hook fans
 * out a single command request and returns the backend's chain.
 *
 * Wire shape (see `crates/tooling/sysml-service/src/lib.rs`):
 *   request : { command: "sysml.causation.trace", params: { session_id,
 *               root_event_id?, root_tick?, root_target?, max_depth? } }
 *   response: CausationTraceResult
 */

import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import type { CausationTraceResult } from '@/engine/types';

/**
 * The discriminated "root pointer" consumed by the hook. Either carries
 * an opaque `eventId` (preferred — the panel stores them when the user
 * clicks a verdict / breakpoint hit) or a `(tick, target)` pair.
 */
export type CausalTraceRoot =
  | {
      kind: 'by-id';
      sessionId: string;
      eventId: string;
      maxDepth?: number;
    }
  | {
      kind: 'by-tick';
      sessionId: string;
      tick: number;
      target: string;
      maxDepth?: number;
    };

export const causationTraceKeys = {
  all: ['causationTrace'] as const,
  byRoot: (root: CausalTraceRoot | null) =>
    root === null
      ? ([...causationTraceKeys.all, 'none'] as const)
      : root.kind === 'by-id'
        ? ([
            ...causationTraceKeys.all,
            'by-id',
            root.sessionId,
            root.eventId,
            root.maxDepth ?? 5,
          ] as const)
        : ([
            ...causationTraceKeys.all,
            'by-tick',
            root.sessionId,
            root.tick,
            root.target,
            root.maxDepth ?? 5,
          ] as const),
};

async function dispatchCausationTrace(
  root: CausalTraceRoot,
): Promise<CausationTraceResult> {
  const params: Record<string, unknown> = {
    session_id: root.sessionId,
    max_depth: root.maxDepth ?? 5,
  };
  if (root.kind === 'by-id') {
    params['root_event_id'] = root.eventId;
  } else {
    params['root_tick'] = root.tick;
    params['root_target'] = root.target;
  }
  return httpPost<CausationTraceResult>('/api/command', {
    command: 'sysml.causation.trace',
    params,
  });
}

/**
 * React-query wrapper. Parked (no fetch) when `root` is null.
 */
export function useCausationTrace(
  root: CausalTraceRoot | null,
): UseQueryResult<CausationTraceResult, Error> {
  return useQuery<CausationTraceResult, Error>({
    queryKey: causationTraceKeys.byRoot(root),
    queryFn: () => {
      if (root === null) {
        return Promise.resolve<CausationTraceResult>({
          root: null,
          chain: [],
          max_depth_used: 0,
        });
      }
      return dispatchCausationTrace(root);
    },
    enabled: root !== null,
    // The recorder is a bounded ring buffer — the result changes rarely
    // but when it does, rapid clicks across chain events shouldn't
    // refetch. Cache for a short window; the panel refetches manually
    // when the user picks a new root.
    staleTime: 5_000,
  });
}

// Exported for tests / non-hook callers (e.g. a command palette action).
export const __internals = { dispatchCausationTrace };
