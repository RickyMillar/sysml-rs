/**
 * Tests for R3.5 `useDrillFromVerdict`.
 *
 * Covers:
 *  - Pure helpers (`hasEvidence`, `buildDrillUrl`).
 *  - Hook behaviour with present evidence → navigate called with exact URL.
 *  - Hook behaviour with absent evidence → showToast invoked, navigate NOT called.
 *  - `hasEvidence` predicate returns correct booleans in both cases.
 *
 * Tests inject a fake `DrillContextValue` via `<DrillProvider value={…}>`
 * so the router is not required — the integration that mounts inside a
 * `<BrowserRouter>` is exercised by Playwright (not in scope here).
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { cleanup, render, renderHook } from '@testing-library/react';
import { createElement } from 'react';
import type { NavigateFunction } from 'react-router-dom';
import type { ReactNode } from 'react';
import type { Verdict } from '@/engine/types';
import {
  DrillProvider,
  useDrillFromVerdict,
  hasEvidence,
  buildDrillUrl,
  DRILL_TOAST_DURATION_MS,
  type DrillContextValue,
} from '../useDrillFromVerdict';
import { DRILL_NO_EVIDENCE_MESSAGE } from '../DrillStatusToast';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

// ── Pure helpers ─────────────────────────────────────────────────────

describe('hasEvidence', () => {
  it('returns true when evidence has a non-empty session_id', () => {
    const v: Verdict = {
      verdict: 'fail',
      evidence: { session_id: 'sess-1', tick: 42 },
    };
    expect(hasEvidence(v)).toBe(true);
  });

  it('returns true when evidence carries session_id + tick + element_id', () => {
    const v: Verdict = {
      verdict: 'fail',
      evidence: { session_id: 'sess-1', tick: 42, element_id: 'Req.1' },
    };
    expect(hasEvidence(v)).toBe(true);
  });

  it('returns false when evidence is undefined', () => {
    const v: Verdict = { verdict: 'fail' };
    expect(hasEvidence(v)).toBe(false);
  });

  it('returns false when evidence is null', () => {
    const v: Verdict = { verdict: 'fail', evidence: null };
    expect(hasEvidence(v)).toBe(false);
  });

  it('returns false for malformed evidence (empty session_id)', () => {
    const v: Verdict = {
      verdict: 'fail',
      evidence: { session_id: '', tick: 1 },
    };
    expect(hasEvidence(v)).toBe(false);
  });
});

describe('buildDrillUrl', () => {
  it('emits /run?session=…&tick=…&element=… when element_id present', () => {
    const v: Verdict = {
      verdict: 'fail',
      evidence: { session_id: 'sess-1', tick: 42, element_id: 'Req.1' },
    };
    expect(buildDrillUrl(v)).toBe(
      '/run?session=sess-1&tick=42&element=Req.1',
    );
  });

  it('omits element param when element_id is null/undefined', () => {
    const v: Verdict = {
      verdict: 'fail',
      evidence: { session_id: 'sess-1', tick: 7 },
    };
    expect(buildDrillUrl(v)).toBe('/run?session=sess-1&tick=7');
  });

  it('percent-encodes special characters in session_id and element_id', () => {
    const v: Verdict = {
      verdict: 'fail',
      evidence: {
        session_id: 'file://foo bar.sysml:MySm',
        tick: 3,
        element_id: 'Pkg::Req 1',
      },
    };
    const url = buildDrillUrl(v)!;
    expect(url).toContain('session=file%3A%2F%2Ffoo+bar.sysml%3AMySm');
    expect(url).toContain('element=Pkg%3A%3AReq+1');
    expect(url).toContain('tick=3');
  });

  it('returns null when evidence is absent', () => {
    expect(buildDrillUrl({ verdict: 'pass' })).toBe(null);
    expect(buildDrillUrl({ verdict: 'pass', evidence: null })).toBe(null);
  });
});

// ── Hook ─────────────────────────────────────────────────────────────

function makeCtx(): DrillContextValue & {
  navigate: ReturnType<typeof vi.fn>;
  showToast: ReturnType<typeof vi.fn>;
} {
  const navigate = vi.fn() as unknown as NavigateFunction &
    ReturnType<typeof vi.fn>;
  const showToast = vi.fn();
  return { navigate, showToast };
}

function withProvider(ctx: DrillContextValue) {
  return ({ children }: { children: ReactNode }) =>
    createElement(DrillProvider, { value: ctx, children });
}

describe('useDrillFromVerdict — drill()', () => {
  it('navigates to /run with session, tick, element when evidence is present', () => {
    const ctx = makeCtx();
    const { result } = renderHook(() => useDrillFromVerdict(), {
      wrapper: withProvider(ctx),
    });

    const verdict: Verdict = {
      verdict: 'fail',
      evidence: {
        session_id: 'sess-1',
        tick: 42,
        element_id: 'Req.voltage',
      },
    };
    result.current.drill(verdict);

    expect(ctx.navigate).toHaveBeenCalledTimes(1);
    expect(ctx.navigate).toHaveBeenCalledWith(
      '/run?session=sess-1&tick=42&element=Req.voltage',
    );
    expect(ctx.showToast).not.toHaveBeenCalled();
  });

  it('navigates without element param when element_id is omitted', () => {
    const ctx = makeCtx();
    const { result } = renderHook(() => useDrillFromVerdict(), {
      wrapper: withProvider(ctx),
    });

    result.current.drill({
      verdict: 'fail',
      evidence: { session_id: 'sess-2', tick: 7 },
    });

    expect(ctx.navigate).toHaveBeenCalledWith('/run?session=sess-2&tick=7');
  });

  it('invokes toast and does NOT navigate when evidence is absent', () => {
    const ctx = makeCtx();
    const { result } = renderHook(() => useDrillFromVerdict(), {
      wrapper: withProvider(ctx),
    });

    result.current.drill({ verdict: 'fail' });

    expect(ctx.navigate).not.toHaveBeenCalled();
    expect(ctx.showToast).toHaveBeenCalledTimes(1);
    expect(ctx.showToast).toHaveBeenCalledWith(DRILL_NO_EVIDENCE_MESSAGE);
  });

  it('invokes toast when evidence is explicitly null', () => {
    const ctx = makeCtx();
    const { result } = renderHook(() => useDrillFromVerdict(), {
      wrapper: withProvider(ctx),
    });

    result.current.drill({ verdict: 'fail', evidence: null });

    expect(ctx.showToast).toHaveBeenCalledWith(DRILL_NO_EVIDENCE_MESSAGE);
    expect(ctx.navigate).not.toHaveBeenCalled();
  });
});

describe('useDrillFromVerdict — hasEvidence predicate', () => {
  it('returns true for verdicts with populated evidence', () => {
    const ctx = makeCtx();
    const { result } = renderHook(() => useDrillFromVerdict(), {
      wrapper: withProvider(ctx),
    });

    const withEv: Verdict = {
      verdict: 'fail',
      evidence: { session_id: 'sess', tick: 1 },
    };
    expect(result.current.hasEvidence(withEv)).toBe(true);
  });

  it('returns false for verdicts without evidence', () => {
    const ctx = makeCtx();
    const { result } = renderHook(() => useDrillFromVerdict(), {
      wrapper: withProvider(ctx),
    });

    expect(result.current.hasEvidence({ verdict: 'pass' })).toBe(false);
    expect(
      result.current.hasEvidence({ verdict: 'pass', evidence: null }),
    ).toBe(false);
  });
});

// ── Provider guard ───────────────────────────────────────────────────

describe('useDrillFromVerdict — provider guard', () => {
  it('throws a helpful error when called outside <DrillProvider>', () => {
    // Silence React's error-boundary log spam.
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    expect(() => renderHook(() => useDrillFromVerdict())).toThrow(
      /must be called inside a <DrillProvider>/,
    );
    err.mockRestore();
  });
});

// ── Constants sanity check ───────────────────────────────────────────

describe('exported constants', () => {
  it('DRILL_TOAST_DURATION_MS is a finite positive number', () => {
    expect(Number.isFinite(DRILL_TOAST_DURATION_MS)).toBe(true);
    expect(DRILL_TOAST_DURATION_MS).toBeGreaterThan(0);
  });

  it('DRILL_NO_EVIDENCE_MESSAGE names the backend task and is friendly', () => {
    expect(DRILL_NO_EVIDENCE_MESSAGE).toMatch(/No evidence/);
    expect(DRILL_NO_EVIDENCE_MESSAGE).toMatch(/R3\.5 backend/);
    // not accusatory
    expect(DRILL_NO_EVIDENCE_MESSAGE).not.toMatch(/error|failed/i);
  });
});

// ── Provider integration (injected value short-circuit) ──────────────

describe('DrillProvider (injected value mode)', () => {
  it('renders children and does NOT render the toast UI when value is injected', () => {
    const ctx = makeCtx();
    const { queryByTestId, getByTestId } = render(
      createElement(DrillProvider, {
        value: ctx,
        children: createElement('div', { 'data-testid': 'child' }, 'ok'),
      }),
    );

    expect(getByTestId('child')).toBeTruthy();
    // No toast rendered even if showToast is invoked — injected-mode
    // provider leaves visibility to the test.
    ctx.showToast(DRILL_NO_EVIDENCE_MESSAGE);
    expect(queryByTestId('drill-status-toast')).toBeNull();
  });
});
