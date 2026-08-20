/**
 * useSweepDrill — drill-from-sweep-row-to-RunWorkflow hook (R5.5).
 *
 * Reuses the R3.5 drill-receiver URL contract from `useDrillReceiver.ts`:
 *
 *     /run?session=<session_id>&tick=<last_tick>&element=<firstFailingElement|∅>
 *
 * We do NOT reinvent the handshake — the same `useDrillReceiver` that
 * picks up drills from Verify picks up drills from Sweep. This is
 * deliberate: drilling a child row exercises exactly the same code path
 * as drilling a verdict.
 *
 * Boundaries:
 *   - Pure URL builder (`buildSweepDrillUrl`) exported for testing.
 *   - The hook exposes a `{ drill, canDrill }` API that callers forward
 *     to `<SweepTableViewer onChildSelect={drill} />`. No coupling to
 *     the viewer component itself.
 *   - When `session_id` is null (the child has not been materialised
 *     yet), `drill()` is a no-op and emits a warning. This is the
 *     expected path when a user clicks a "pending" row.
 */

import { useCallback, useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import type { ChildDescriptor } from '@/engine/types';

// ── Pure helpers ────────────────────────────────────────────────────

/**
 * Build the `/run?session=…&tick=…&element=…` URL for a sweep child.
 * Returns `null` when the child has no session id yet (pending row).
 *
 * Mirrors `buildDrillUrl` from `useDrillFromVerdict.ts` in both shape
 * and percent-encoding. Both drill origins converge on the same URL
 * contract so `useDrillReceiver` stays origin-agnostic.
 */
export function buildSweepDrillUrl(child: ChildDescriptor): string | null {
  if (!child.session_id || child.session_id.length === 0) return null;

  const params = new URLSearchParams();
  params.set('session', child.session_id);

  // `last_tick` defaults to 0 when the child has not yet streamed a
  // snapshot — the receiver pauses the session at whatever tick it is
  // on, so this is a safe fallback.
  const tick =
    typeof child.last_tick === 'number' && Number.isFinite(child.last_tick)
      ? child.last_tick
      : 0;
  params.set('tick', String(tick));

  if (child.first_failing_element) {
    params.set('element', child.first_failing_element);
  }

  // Name the drill origin so the receiver's breadcrumb hop reads
  // "Analyze · tick N" (absent origin defaults to 'verify' — the
  // handshake's original sender).
  params.set('origin', 'analyze');

  return `/run?${params.toString()}`;
}

/**
 * Pure predicate — `true` iff `drill()` would actually navigate. Lets
 * the viewer disable the drill affordance for pending rows.
 */
export function canDrillChild(child: ChildDescriptor): boolean {
  return (
    typeof child.session_id === 'string' && child.session_id.length > 0
  );
}

// ── Hook ────────────────────────────────────────────────────────────

export interface SweepDrillApi {
  /**
   * Navigate to the Run workflow with the child's session deep-linked.
   * No-ops (and warns) if the child has no session id.
   */
  drill: (child: ChildDescriptor) => void;
  /** Pure predicate — same as the module-level {@link canDrillChild}. */
  canDrill: (child: ChildDescriptor) => boolean;
}

/** Sink for warnings. Exported so tests can inject a silencer. */
export interface UseSweepDrillOpts {
  warn?: (msg: string) => void;
}

/**
 * Drill hook. Must be called inside a `<BrowserRouter>` (or any Router
 * that exposes `useNavigate`). Returns a stable `{ drill, canDrill }`
 * object.
 *
 * Usage:
 *
 *   const { drill } = useSweepDrill();
 *   return <SweepTableViewer … onChildSelect={drill} />;
 */
export function useSweepDrill(opts: UseSweepDrillOpts = {}): SweepDrillApi {
  const navigate = useNavigate();
  const warn = opts.warn ?? ((m) => console.warn(m));

  const drill = useCallback(
    (child: ChildDescriptor) => {
      const url = buildSweepDrillUrl(child);
      if (url === null) {
        warn(
          `sweep drill: child "${child.id}" has no session_id yet — ` +
            `cannot navigate (status="${child.status}")`,
        );
        return;
      }
      navigate(url);
    },
    [navigate, warn],
  );

  return useMemo<SweepDrillApi>(
    () => ({ drill, canDrill: canDrillChild }),
    [drill],
  );
}
