/**
 * useDrillReceiver — RunWorkflow side of the drill-from-verdict handshake (R3.5).
 *
 * VerifyWorkflow's `useDrillFromVerdict` hook (R3.5 frontend) builds a URL
 * of shape `/run?session=<id>&tick=<n>&element=<qname>` and navigates. This
 * hook is what picks those params up on the RunWorkflow side:
 *
 *   1. Attach the named session (`setActiveSession`). This also clears any
 *      stale investigation trail from a previous session — see
 *      `useSessionStore.setActiveSession`.
 *   2. Pause the session so it doesn't keep advancing past the evidence tick.
 *   3. Select the offending element via `useSelectionStore.select`.
 *   4. Push a hop onto `useInvestigationTrail` (ninebar Phase 1, audit F15
 *      — replaces the old single-hop `useSessionStore.drilledFrom`) so
 *      waveform/timeline components can highlight the tick and the banner
 *      can show "you landed here from Verify", with a "back" affordance
 *      when this isn't the trail's first hop.
 *   5. Clear the query string from the URL so a browser reload doesn't
 *      re-trigger the handshake.
 *
 * Mounts once. No-ops when the query string doesn't carry drill params —
 * calling this in other contexts is safe.
 */

import { useEffect, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useSessionStore } from '@/features/sessions/store';
import { useSelectionStore } from '@/features/selection/store';
import {
  useInvestigationTrail,
  type InvestigationOrigin,
} from '@/features/investigation/useInvestigationTrail';

/**
 * Human form of each drill origin for the breadcrumb label. Keys are the
 * `InvestigationOrigin` union — a sender names itself via the optional
 * `origin` query param (Phase 5: Analyze drills were previously
 * mislabeled "Verify" because the receiver hardcoded it).
 */
const ORIGIN_LABELS: Record<InvestigationOrigin, string> = {
  verify: 'Verify',
  run: 'Run',
  analyze: 'Analyze',
};

/** Parse + validate the `origin` param; absent/unknown → 'verify' (the
 *  original handshake's only sender, so old URLs keep their meaning). */
export function parseDrillOrigin(raw: string | null): InvestigationOrigin {
  return raw === 'analyze' || raw === 'run' || raw === 'verify' ? raw : 'verify';
}

/**
 * Build the breadcrumb label shown in `DrilledFromBanner` for a hop
 * received off the `/run?session=&tick=&element=[&origin=]` handshake.
 * Exported for tests. Tick is preferred over the session id because it's
 * the more useful "where in time" anchor; falls back to a truncated
 * session id when no tick was supplied (session-only drill).
 */
export function drillLabel(
  tick: number | null,
  session: string | null,
  origin: InvestigationOrigin = 'verify',
): string {
  const name = ORIGIN_LABELS[origin];
  if (tick != null) return `${name} · tick ${tick}`;
  if (session) return `${name} · session ${session.slice(0, 8)}`;
  return name;
}

/**
 * Pure helper — parses `?session=&tick=&element=&origin=` out of a URL
 * search string. Exported for testing.
 */
export function parseDrillParams(search: string): {
  session: string | null;
  tick: number | null;
  element: string | null;
  origin: InvestigationOrigin;
} | null {
  const params = new URLSearchParams(search);
  const session = params.get('session');
  const tickStr = params.get('tick');
  const element = params.get('element');
  if (!session && !tickStr && !element) return null;
  const tick = tickStr !== null ? Number.parseInt(tickStr, 10) : null;
  return {
    session,
    tick: Number.isFinite(tick as number) ? (tick as number) : null,
    element,
    origin: parseDrillOrigin(params.get('origin')),
  };
}

/**
 * Strip the drill-handshake keys from a search string without touching
 * unrelated params. Returns the serialised tail that should replace the
 * URL's search portion (may be the empty string).
 */
export function stripDrillParams(search: string): string {
  const params = new URLSearchParams(search);
  params.delete('session');
  params.delete('tick');
  params.delete('element');
  params.delete('origin');
  const remaining = params.toString();
  return remaining.length > 0 ? `?${remaining}` : '';
}

export function useDrillReceiver() {
  const { search, pathname } = useLocation();
  const navigate = useNavigate();
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const setPhase = useSessionStore((s) => s.setPhase);
  const pushHop = useInvestigationTrail((s) => s.push);
  const select = useSelectionStore((s) => s.select);

  // Idempotency: the effect must fire at most once per URL visit. Storing
  // the consumed search string guards against React strict-mode double
  // invocation and against re-runs caused by unrelated rerenders.
  const consumedRef = useRef<string | null>(null);

  useEffect(() => {
    if (consumedRef.current === search) return;
    const parsed = parseDrillParams(search);
    if (!parsed || (!parsed.session && parsed.tick == null && !parsed.element)) {
      return;
    }
    consumedRef.current = search;

    if (parsed.session) {
      setActiveSession(parsed.session);
      // Pause so the playhead doesn't race past the evidence tick. We
      // can't assume the session is in 'running' phase, but setting
      // 'paused' is safe regardless — the controller ignores no-op
      // transitions.
      setPhase('paused');
    }

    if (parsed.element) {
      // Selection with `uri = null` still highlights on the diagram;
      // element detail fetch is skipped until the URI is known.
      select(null, parsed.element);
    }

    pushHop({
      origin: parsed.origin,
      fromSessionId: parsed.session ?? '',
      tick: parsed.tick ?? undefined,
      elementId: parsed.element ?? undefined,
      label: drillLabel(parsed.tick, parsed.session, parsed.origin),
    });

    // Clear the drill keys from the URL so a refresh doesn't retrigger.
    // `replace` avoids polluting browser history with a duplicate entry.
    const nextSearch = stripDrillParams(search);
    if (nextSearch !== search) {
      navigate({ pathname, search: nextSearch }, { replace: true });
    }
  }, [search, pathname, navigate, setActiveSession, setPhase, pushHop, select]);
}
