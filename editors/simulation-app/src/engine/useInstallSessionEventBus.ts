/**
 * Boot-time wiring for the `SessionEventBus` (R1.5 event stream, BP5).
 *
 * `SessionEventBus` (see `./SessionEvents.ts`) is a plain, non-React
 * class. It needs a `SnapshotSource` — something that can hand back the
 * latest `SessionDetail` + `SessionPhase` for a session key — and an
 * explicit `installSessionEventBus(...)` call before any
 * `useSessionEvents(...)` subscriber (e.g. `BreakpointsPanel`'s
 * breakpoint-hit toast/row-flash) does anything.
 *
 * GAP FOUND DURING BP5: nothing in the app called
 * `installSessionEventBus` before this file existed — only
 * `SessionEvents.test.ts` constructed a bus, with a hand-rolled mock
 * source. `useSessionEvents` therefore silently no-op'd in production
 * (`getSessionEventBus()` returned `null`), so `breakpoint-hit` could
 * never fire regardless of whether `paused_at_breakpoint` was wired
 * correctly. This hook closes that gap by installing one bus, once,
 * backed by the same react-query cache `useSessionDetail` already
 * populates, plus the session-phase Zustand store.
 *
 * Cache-key note: `useSessionDetail(id)` (no `includeVariables` option
 * — the shape `SessionHeader` and most Run-workflow surfaces use) reads
 * `[...sessionKeys.detail(id), 'meta']`. This hook reads that same key
 * so the bus sees whatever `SessionHeader`'s own poll last refreshed
 * (worst case ~1-2s latency while `phase === 'running'`, matching that
 * poll's documented cadence). Note that a handful of OTHER call sites
 * (`useStepSession`/`useResumeSession`/`useResetSession`/`useInjectEvent`
 * in `features/sessions/mutations.ts`, and `VariableInspection.ts`)
 * write/read the bare `sessionKeys.detail(id)` key (no `'meta'`/`'full'`
 * suffix) — a PRE-EXISTING mismatch with `useSessionDetail`'s actual key
 * that predates BP5 and is out of scope here (their direct-cache-write
 * optimization is currently inert; they still work correctly via the
 * next poll). Flagged for a follow-up, not fixed by this hook.
 *
 * Call once near the app root — see `AppLayout` in `App.tsx`.
 */
import { useEffect } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { SessionEventBus, installSessionEventBus } from './SessionEvents';
import { sessionKeys } from '../features/sessions/queries';
import { useSessionStore } from '../features/sessions/store';
import type { SessionDetail, SessionPhase } from '../features/sessions/types';
import type { SessionId } from './types';

export function useInstallSessionEventBus(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    const bus = new SessionEventBus({
      source: {
        getDetail: (session: SessionId): SessionDetail | null =>
          queryClient.getQueryData<SessionDetail>([
            ...sessionKeys.detail(session),
            'meta',
          ]) ?? null,
        getPhase: (session: SessionId): SessionPhase | null => {
          const { activeSessionId, phase } = useSessionStore.getState();
          return activeSessionId === session ? phase : null;
        },
      },
    });
    installSessionEventBus(bus);
    return () => bus.disposeAll();
  }, [queryClient]);
}
