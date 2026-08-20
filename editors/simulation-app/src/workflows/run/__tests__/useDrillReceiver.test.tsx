/**
 * Tests for the drill-from-verdict RunWorkflow receiver (R3.5; migrated to
 * the multi-hop `useInvestigationTrail` store in ninebar Phase 1, audit
 * F15 — the receiver used to write `useSessionStore.drilledFrom`).
 *
 * Covers the parsing helpers, the hook's URL → store handshake, and the
 * "clear query string after consumption" behaviour. Rendering of the
 * banner lives in DrilledFromBanner.test.tsx.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { render, act, cleanup } from '@testing-library/react';
import {
  drillLabel,
  parseDrillParams,
  stripDrillParams,
  useDrillReceiver,
} from '../useDrillReceiver';
import { useSessionStore } from '@/features/sessions/store';
import { useSelectionStore } from '@/features/selection/store';
import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';

afterEach(() => {
  cleanup();
  useInvestigationTrail.getState().clear();
  useSessionStore.getState().setActiveSession(null);
  useSessionStore.getState().setPhase('idle');
  useSelectionStore.getState().clear();
  vi.restoreAllMocks();
});

function ReceiverProbe() {
  useDrillReceiver();
  return null;
}

function mountWithSearch(initialSearch: string) {
  const router = createMemoryRouter(
    [{ path: '/run', element: <ReceiverProbe /> }],
    { initialEntries: [{ pathname: '/run', search: initialSearch }] },
  );
  const utils = render(<RouterProvider router={router} />);
  return { router, ...utils };
}

describe('parseDrillParams', () => {
  it('returns null when no drill keys are present', () => {
    expect(parseDrillParams('')).toBeNull();
    expect(parseDrillParams('?workspace=foo')).toBeNull();
  });

  it('parses all three keys when present (origin defaults to verify)', () => {
    const out = parseDrillParams('?session=abc&tick=42&element=pkg.x');
    expect(out).toEqual({ session: 'abc', tick: 42, element: 'pkg.x', origin: 'verify' });
  });

  it('tolerates missing tick (leaves tick as null)', () => {
    const out = parseDrillParams('?session=abc&element=pkg.x');
    expect(out).toEqual({ session: 'abc', tick: null, element: 'pkg.x', origin: 'verify' });
  });

  it('reads a valid origin param; unknown values fall back to verify', () => {
    expect(parseDrillParams('?session=abc&origin=analyze')?.origin).toBe('analyze');
    expect(parseDrillParams('?session=abc&origin=run')?.origin).toBe('run');
    expect(parseDrillParams('?session=abc&origin=bogus')?.origin).toBe('verify');
  });

  it('rejects non-integer tick values by returning null tick', () => {
    const out = parseDrillParams('?session=abc&tick=not-a-number&element=pkg.x');
    expect(out?.tick).toBeNull();
  });
});

describe('stripDrillParams', () => {
  it('removes drill keys but preserves unrelated ones', () => {
    expect(stripDrillParams('?session=a&tick=1&element=x&origin=analyze&workspace=foo')).toBe(
      '?workspace=foo',
    );
  });

  it('returns an empty string when no other params remain', () => {
    expect(stripDrillParams('?session=a&tick=1&element=x')).toBe('');
  });
});

describe('drillLabel', () => {
  it('prefers the tick when present', () => {
    expect(drillLabel(482, 'sess-xyz')).toBe('Verify · tick 482');
  });

  it('falls back to a truncated session id when tick is absent', () => {
    expect(drillLabel(null, 'sess-only-12345')).toBe('Verify · session sess-onl');
  });

  it('falls back to a bare label when neither is present', () => {
    expect(drillLabel(null, null)).toBe('Verify');
  });

  it('names the sender when an origin is supplied (Phase 5 Analyze drills)', () => {
    expect(drillLabel(42, 'sess-xyz', 'analyze')).toBe('Analyze · tick 42');
    expect(drillLabel(null, null, 'run')).toBe('Run');
  });
});

describe('useDrillReceiver — URL → trail handshake', () => {
  it('is a no-op when the URL carries no drill keys', () => {
    mountWithSearch('?workspace=foo');
    expect(useInvestigationTrail.getState().hops).toEqual([]);
    expect(useSessionStore.getState().activeSessionId).toBeNull();
  });

  it('attaches the named session, pauses, and pushes the drill hop', () => {
    mountWithSearch('?session=sess-xyz&tick=482&element=groupHead.trip_time');
    const s = useSessionStore.getState();
    expect(s.activeSessionId).toBe('sess-xyz');
    expect(s.phase).toBe('paused');
    const trail = useInvestigationTrail.getState();
    expect(trail.hops).toEqual([
      {
        origin: 'verify',
        fromSessionId: 'sess-xyz',
        tick: 482,
        elementId: 'groupHead.trip_time',
        label: 'Verify · tick 482',
      },
    ]);
    expect(trail.cursor).toBe(0);
  });

  it('selects the offending element in the selection store', () => {
    mountWithSearch('?session=sess-xyz&tick=100&element=station.valve');
    expect(useSelectionStore.getState().selectedElementId).toBe('station.valve');
  });

  it('pushes an analyze-labeled hop when the sender names itself', () => {
    mountWithSearch('?session=sess-abc&tick=7&origin=analyze');
    const trail = useInvestigationTrail.getState();
    expect(trail.hops).toEqual([
      {
        origin: 'analyze',
        fromSessionId: 'sess-abc',
        tick: 7,
        elementId: undefined,
        label: 'Analyze · tick 7',
      },
    ]);
  });

  it('clears the drill keys from the URL after consumption', () => {
    const { router } = mountWithSearch(
      '?session=a&tick=1&element=x&workspace=golden',
    );
    // React Router's memory history strips the consumed keys; unrelated
    // `workspace` param is preserved.
    const current = router.state.location.search;
    expect(current).toBe('?workspace=golden');
  });

  it('does not double-trigger under StrictMode re-invocation', () => {
    mountWithSearch('?session=sess-xyz&tick=10&element=x');
    const push = vi.spyOn(useInvestigationTrail.getState(), 'push');
    // Force a rerender of the probe. The ref guard should prevent a
    // second application of the handshake against the same URL.
    act(() => {
      useSessionStore.setState({ phase: 'running' });
    });
    expect(push).not.toHaveBeenCalled();
  });

  it('handles a session-only drill (no element, no tick)', () => {
    mountWithSearch('?session=sess-only');
    const s = useSessionStore.getState();
    expect(s.activeSessionId).toBe('sess-only');
    const trail = useInvestigationTrail.getState();
    expect(trail.hops).toEqual([
      {
        origin: 'verify',
        fromSessionId: 'sess-only',
        tick: undefined,
        elementId: undefined,
        label: 'Verify · session sess-onl',
      },
    ]);
    expect(useSelectionStore.getState().selectedElementId).toBeNull();
  });

  it('pushing a second drill onto an existing session extends the trail (does not replace it)', () => {
    mountWithSearch('?session=sess-xyz&tick=10&element=a');
    // setActiveSession('sess-xyz') a second time (same id) must not clear
    // the trail — mirrors the sessions/store.ts idempotency guarantee.
    useSessionStore.getState().setActiveSession('sess-xyz');
    useInvestigationTrail.getState().push({
      origin: 'run',
      fromSessionId: 'sess-xyz',
      tick: 20,
      label: 'Run · tick 20',
    });
    expect(useInvestigationTrail.getState().hops).toHaveLength(2);
    expect(useInvestigationTrail.getState().cursor).toBe(1);
  });
});
