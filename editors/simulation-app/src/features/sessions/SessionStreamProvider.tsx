/**
 * SessionStreamProvider — the app-level (and ONLY) owner of the live
 * session stream (ninebar Phase 1, audit F15).
 *
 * Mounts `useSessionStream(activeSessionId)` + `useLiveTimeSeriesBridge`
 * exactly once, above the workflow routes, so the live frame (phase pill,
 * session chips, pinned rail contexts) stays fed when the user navigates
 * off `/run`. Previously both hooks mounted inside `RunWorkflow`, which
 * meant (a) the stream died on route change and (b) any second consumer
 * would have raced two sockets against one `sessionLiveStore`.
 *
 * The single-owner rule is enforced by a dev assertion inside
 * `useSessionStream` itself — mounting this provider twice (or mounting
 * the hook anywhere else) logs loudly in dev. See ADR-004.
 */
import { useSessionStore } from './store';
import { useSessionStream } from './useSessionStream';
import { useLiveTimeSeriesBridge } from './useLiveTimeSeriesBridge';

export function SessionStreamProvider() {
  const sessionId = useSessionStore((s) => s.activeSessionId);
  useSessionStream(sessionId);
  useLiveTimeSeriesBridge();
  return null;
}
