/**
 * Tests for the R5.12 PromoteToCompareButton — trade-study → compare
 * handoff.
 *
 * Covers:
 *   - Enabled/disabled gating (0, 1, 2, 6, 7 selections).
 *   - Dropping rows without a `session_id` BEFORE the 2..6 check.
 *   - Click path: calls `useCompareStore.setPickedSessionIds` with the
 *     promotable ids AND `navigate(navigateTo)`.
 *   - Pure helper `toPromotedSessionIds` deduplicates and filters.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import {
  PROMOTE_MAX_SELECTED,
  PROMOTE_MIN_SELECTED,
  PromoteToCompareButton,
  promoteDisabledReason,
  toPromotedSessionIds,
} from '../PromoteToCompareButton';
import { useCompareStore } from '@/workflows/compare/useCompareStore';
import type { ChildDescriptorLike } from '../tradeHelpers';

// ── Store reset (isolation between tests) ────────────────────────────

afterEach(() => {
  cleanup();
  useCompareStore.getState().setPickedSessionIds([]);
  vi.restoreAllMocks();
});

// ── Harness ─────────────────────────────────────────────────────────

/** A probe element that picks up the current pathname for asserts. */
function PathnameProbe() {
  const router = useRouterLocation();
  return <span data-testid="pathname">{router}</span>;
}
// Avoid pulling in useLocation twice — the router itself exposes location.
import { useLocation } from 'react-router-dom';
function useRouterLocation(): string {
  const loc = useLocation();
  return loc.pathname;
}

function renderWithRouter(
  selectedChildren: ChildDescriptorLike[],
  navigateTo = '/run/compare',
) {
  const router = createMemoryRouter(
    [
      {
        path: '/trade',
        element: (
          <>
            <PromoteToCompareButton
              selectedChildren={selectedChildren}
              navigateTo={navigateTo}
            />
            <PathnameProbe />
          </>
        ),
      },
      { path: navigateTo, element: <PathnameProbe /> },
    ],
    { initialEntries: ['/trade'] },
  );
  return { router, ...render(<RouterProvider router={router} />) };
}

function mkChild(
  id: string,
  session_id: string | null,
  status: ChildDescriptorLike['status'] = 'completed',
): ChildDescriptorLike {
  return { id, session_id, status };
}

// ── toPromotedSessionIds ────────────────────────────────────────────

describe('toPromotedSessionIds', () => {
  it('drops null and empty-string session_ids', () => {
    expect(
      toPromotedSessionIds([
        mkChild('a', null),
        mkChild('b', ''),
        mkChild('c', 'sid-c'),
      ]),
    ).toEqual(['sid-c']);
  });

  it('deduplicates repeated session_ids (same parent → multiple rows)', () => {
    expect(
      toPromotedSessionIds([
        mkChild('a', 'sid-x'),
        mkChild('b', 'sid-x'),
        mkChild('c', 'sid-y'),
      ]),
    ).toEqual(['sid-x', 'sid-y']);
  });

  it('preserves caller-supplied order', () => {
    expect(
      toPromotedSessionIds([
        mkChild('a', 'sid-3'),
        mkChild('b', 'sid-1'),
        mkChild('c', 'sid-2'),
      ]),
    ).toEqual(['sid-3', 'sid-1', 'sid-2']);
  });
});

// ── promoteDisabledReason ────────────────────────────────────────────

describe('promoteDisabledReason', () => {
  it('returns a minimum-selection string when below the floor', () => {
    const msg = promoteDisabledReason(0, 0);
    expect(msg).toMatch(/at least/i);
  });

  it('returns a maximum-selection string when above the ceiling', () => {
    const msg = promoteDisabledReason(PROMOTE_MAX_SELECTED + 1, 0);
    expect(msg).toMatch(/at most/i);
  });

  it('returns the "no completed runs" message when promotable < 2', () => {
    // 3 selected, but only 1 has a session_id.
    const msg = promoteDisabledReason(3, 1);
    expect(msg).toMatch(/no completed runs/i);
  });

  it('returns null when 2..6 selected and enough are promotable', () => {
    expect(promoteDisabledReason(2, 2)).toBeNull();
    expect(promoteDisabledReason(PROMOTE_MAX_SELECTED, PROMOTE_MAX_SELECTED)).toBeNull();
  });
});

// ── Gating (disabled window) ────────────────────────────────────────

describe('<PromoteToCompareButton> — gating', () => {
  it('is disabled with 0 selected', () => {
    renderWithRouter([]);
    const btn = screen.getByTestId('promote-to-compare-button');
    expect(btn).toBeDisabled();
  });

  it('is disabled with 1 selected (below min of 2)', () => {
    renderWithRouter([mkChild('a', 'sid-a')]);
    expect(screen.getByTestId('promote-to-compare-button')).toBeDisabled();
  });

  it(`is enabled with ${PROMOTE_MIN_SELECTED} selected, both with session_ids`, () => {
    renderWithRouter([
      mkChild('a', 'sid-a'),
      mkChild('b', 'sid-b'),
    ]);
    expect(screen.getByTestId('promote-to-compare-button')).not.toBeDisabled();
  });

  it('is enabled at exactly MAX selected', () => {
    const rows = Array.from({ length: PROMOTE_MAX_SELECTED }, (_, i) =>
      mkChild(`id-${i}`, `sid-${i}`),
    );
    renderWithRouter(rows);
    expect(screen.getByTestId('promote-to-compare-button')).not.toBeDisabled();
  });

  it(`is disabled above MAX (${PROMOTE_MAX_SELECTED + 1} selected)`, () => {
    const rows = Array.from({ length: PROMOTE_MAX_SELECTED + 1 }, (_, i) =>
      mkChild(`id-${i}`, `sid-${i}`),
    );
    renderWithRouter(rows);
    expect(screen.getByTestId('promote-to-compare-button')).toBeDisabled();
  });

  it('is disabled when 2+ selected but all lack session_ids (pending)', () => {
    renderWithRouter([
      mkChild('a', null, 'pending'),
      mkChild('b', null, 'running'),
    ]);
    const btn = screen.getByTestId('promote-to-compare-button');
    expect(btn).toBeDisabled();
    expect(btn.getAttribute('title') ?? '').toMatch(/no completed runs/i);
  });
});

// ── Click path (navigate + store) ───────────────────────────────────

describe('<PromoteToCompareButton> — click path', () => {
  beforeEach(() => {
    useCompareStore.getState().setPickedSessionIds([]);
  });

  it('writes session ids to the compare store and navigates', async () => {
    const user = userEvent.setup();
    const { router } = renderWithRouter(
      [
        mkChild('a', 'sid-a'),
        mkChild('b', 'sid-b'),
        mkChild('c', 'sid-c'),
      ],
      '/run/compare',
    );

    await user.click(screen.getByTestId('promote-to-compare-button'));

    expect(useCompareStore.getState().pickedSessionIds).toEqual([
      'sid-a',
      'sid-b',
      'sid-c',
    ]);
    expect(router.state.location.pathname).toBe('/run/compare');
  });

  it('is a no-op when disabled (no store write, no navigate)', async () => {
    const user = userEvent.setup();
    const { router } = renderWithRouter([mkChild('a', 'sid-a')], '/run/compare');
    await user.click(screen.getByTestId('promote-to-compare-button'));
    expect(useCompareStore.getState().pickedSessionIds).toEqual([]);
    expect(router.state.location.pathname).toBe('/trade');
  });

  it('respects a custom navigateTo target', async () => {
    const user = userEvent.setup();
    const { router } = renderWithRouter(
      [mkChild('a', 'sid-a'), mkChild('b', 'sid-b')],
      '/run/compare',
    );
    await user.click(screen.getByTestId('promote-to-compare-button'));
    expect(router.state.location.pathname).toBe('/run/compare');
  });
});
