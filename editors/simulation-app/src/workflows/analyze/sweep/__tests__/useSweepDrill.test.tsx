/**
 * Tests for useSweepDrill (R5.5).
 *
 * Covers:
 *   - `buildSweepDrillUrl` — all URL shapes (with / without element,
 *     percent-encoding, null session).
 *   - `canDrillChild` — predicate true/false for various descriptor shapes.
 *   - Hook integration: navigate called with the exact URL when session_id
 *     is present; no-op + warn when session_id is null.
 *   - URL contract matches R3.5 drill-receiver (`/run?session=…&tick=…&element=…`).
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook } from '@testing-library/react';
import { createElement, type ReactNode } from 'react';
import { MemoryRouter, Routes, Route, useLocation } from 'react-router-dom';
import type { ChildDescriptor } from '@/engine/types';
import {
  buildSweepDrillUrl,
  canDrillChild,
  useSweepDrill,
} from '../useSweepDrill';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function descriptor(
  overrides: Partial<ChildDescriptor> = {},
): ChildDescriptor {
  return {
    id: 'c0',
    index: 0,
    session_id: 'sess-1',
    status: 'complete',
    params: { voltage: 12 },
    last_tick: 42,
    first_failing_element: null,
    ...overrides,
  };
}

describe('buildSweepDrillUrl', () => {
  // `origin=analyze` names the sender so the receiver's breadcrumb hop
  // reads "Analyze · tick N" instead of the default "Verify" (Phase 5).
  it('emits /run?session=…&tick=…&origin=analyze with no element when first_failing_element is null', () => {
    expect(buildSweepDrillUrl(descriptor())).toBe(
      '/run?session=sess-1&tick=42&origin=analyze',
    );
  });

  it('includes the element param when first_failing_element is present', () => {
    expect(
      buildSweepDrillUrl(
        descriptor({ first_failing_element: 'Req.voltage' }),
      ),
    ).toBe('/run?session=sess-1&tick=42&element=Req.voltage&origin=analyze');
  });

  it('falls back to tick=0 when last_tick is null or missing', () => {
    expect(buildSweepDrillUrl(descriptor({ last_tick: null }))).toBe(
      '/run?session=sess-1&tick=0&origin=analyze',
    );
    expect(buildSweepDrillUrl(descriptor({ last_tick: undefined }))).toBe(
      '/run?session=sess-1&tick=0&origin=analyze',
    );
  });

  it('returns null when session_id is null', () => {
    expect(buildSweepDrillUrl(descriptor({ session_id: null }))).toBeNull();
  });

  it('returns null when session_id is the empty string', () => {
    expect(buildSweepDrillUrl(descriptor({ session_id: '' }))).toBeNull();
  });

  it('percent-encodes special chars in session_id and element', () => {
    const url = buildSweepDrillUrl(
      descriptor({
        session_id: 'file://foo bar.sysml:MySm',
        first_failing_element: 'Pkg::Req 1',
      }),
    )!;
    expect(url).toContain('session=file%3A%2F%2Ffoo+bar.sysml%3AMySm');
    expect(url).toContain('element=Pkg%3A%3AReq+1');
    expect(url).toContain('tick=42');
  });
});

describe('canDrillChild', () => {
  it('returns true when session_id is a non-empty string', () => {
    expect(canDrillChild(descriptor())).toBe(true);
  });
  it('returns false when session_id is null', () => {
    expect(canDrillChild(descriptor({ session_id: null }))).toBe(false);
  });
  it('returns false when session_id is empty', () => {
    expect(canDrillChild(descriptor({ session_id: '' }))).toBe(false);
  });
});

// ── Hook integration ─────────────────────────────────────────────────

/**
 * Memory-router wrapper + a small route that records the current
 * location. Lets us assert on the navigation target without stubbing
 * `useNavigate`.
 */
function withRouter(capture: { location: { pathname: string; search: string } | null }) {
  function Sniffer() {
    const loc = useLocation();
    capture.location = { pathname: loc.pathname, search: loc.search };
    return null;
  }
  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      MemoryRouter,
      { initialEntries: ['/analyze/sweep'] },
      createElement(
        Routes,
        null,
        createElement(Route, {
          path: '*',
          element: createElement(
            'div',
            null,
            children,
            createElement(Sniffer, null),
          ),
        }),
      ),
    );
  };
}

describe('useSweepDrill — drill()', () => {
  it('navigates to /run with session, tick, element when session_id present', () => {
    const capture: { location: { pathname: string; search: string } | null } = {
      location: null,
    };
    const { result } = renderHook(() => useSweepDrill(), {
      wrapper: withRouter(capture),
    });
    act(() => {
      result.current.drill(
        descriptor({
          session_id: 'sess-A',
          last_tick: 7,
          first_failing_element: 'Req.1',
        }),
      );
    });
    expect(capture.location?.pathname).toBe('/run');
    expect(capture.location?.search).toBe(
      '?session=sess-A&tick=7&element=Req.1&origin=analyze',
    );
  });

  it('navigates without element param when first_failing_element is null', () => {
    const capture: { location: { pathname: string; search: string } | null } = {
      location: null,
    };
    const { result } = renderHook(() => useSweepDrill(), {
      wrapper: withRouter(capture),
    });
    act(() => {
      result.current.drill(
        descriptor({
          session_id: 'sess-B',
          last_tick: 3,
          first_failing_element: null,
        }),
      );
    });
    expect(capture.location?.pathname).toBe('/run');
    expect(capture.location?.search).toBe('?session=sess-B&tick=3&origin=analyze');
  });

  it('is a no-op (does not navigate) + warns when session_id is null', () => {
    const warn = vi.fn();
    const capture: { location: { pathname: string; search: string } | null } = {
      location: null,
    };
    const { result } = renderHook(() => useSweepDrill({ warn }), {
      wrapper: withRouter(capture),
    });
    act(() => {
      result.current.drill(descriptor({ session_id: null }));
    });

    // MemoryRouter starts at /analyze/sweep — location should be unchanged.
    expect(capture.location?.pathname).toBe('/analyze/sweep');
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toMatch(/no session_id/);
  });
});

describe('useSweepDrill — URL contract (R3.5 receiver compatibility)', () => {
  it('emits the exact parameter keys the R3.5 drill-receiver reads', () => {
    // The R3.5 `parseDrillParams` helper splits on `session`, `tick`, `element`.
    const url = buildSweepDrillUrl(
      descriptor({
        session_id: 'sess',
        last_tick: 1,
        first_failing_element: 'E',
      }),
    )!;
    const search = url.slice(url.indexOf('?'));
    const params = new URLSearchParams(search);
    expect(params.get('session')).toBe('sess');
    expect(params.get('tick')).toBe('1');
    expect(params.get('element')).toBe('E');
    // Phase 5: the receiver also reads `origin` so the breadcrumb hop is
    // labeled "Analyze" (absent → 'verify' for old URLs).
    expect(params.get('origin')).toBe('analyze');
  });
});
