/**
 * Unit tests for the pure helpers extracted from `useSessionController`
 * (closeout-plan.md item 1 — "backend advances server-side, GUI
 * follows"). The hook itself pulls in react-query mutations + several
 * Zustand stores + workspace queries, so it isn't mounted directly
 * here; instead these tests pin the two pieces of logic that are
 * shared between the throttled play loop (`tick`) and the unthrottled
 * bulk-step loops (`fastForward` / `runToBreakpoint`), mirroring the
 * `dispatchFrame` pattern in `useSessionStream.test.ts`.
 */
import { describe, it, expect } from 'vitest';
import { computePlayChunkTicks, interpretChunkResult } from './useSessionController';

// ── computePlayChunkTicks ────────────────────────────────────────────

describe('computePlayChunkTicks', () => {
  it('scales chunk size with stepsPerSecond at the default 250ms interval', () => {
    // 10 sps (the store default / 1x) * 250ms -> 2.5 ticks, rounds to 3.
    expect(computePlayChunkTicks(10)).toBe(3);
    // 100 sps (10x) * 250ms -> 25 ticks.
    expect(computePlayChunkTicks(100)).toBe(25);
    // 5 sps (0.5x) * 250ms -> 1.25 ticks, rounds to 1.
    expect(computePlayChunkTicks(5)).toBe(1);
  });

  it('floors at 1 tick even for a very low speed', () => {
    expect(computePlayChunkTicks(0.1)).toBe(1);
  });

  it('respects a custom interval', () => {
    // 10 sps * 1000ms -> 10 ticks/chunk if the caller widens the interval.
    expect(computePlayChunkTicks(10, 1000)).toBe(10);
  });

  it('caps at MAX_BULK_STEP_TICKS for a pathological speed value', () => {
    expect(computePlayChunkTicks(1_000_000)).toBe(20_000);
  });
});

// ── interpretChunkResult ─────────────────────────────────────────────
//
// BP5: this now reads the EXPLICIT `paused` / `paused_at_breakpoint`
// flags BP1 put on `SessionSummary`, never a tick-unchanged inference.

describe('interpretChunkResult', () => {
  it('reports completed when the session finished', () => {
    const outcome = interpretChunkResult({
      completed: true,
      paused: false,
      paused_at_breakpoint: null,
    });
    expect(outcome).toEqual({
      completed: true,
      halted: true,
      pausedAtBreakpoint: null,
    });
  });

  it('is not halted on a plain in-progress chunk', () => {
    const outcome = interpretChunkResult({
      completed: false,
      paused: false,
      paused_at_breakpoint: null,
    });
    expect(outcome).toEqual({
      completed: false,
      halted: false,
      pausedAtBreakpoint: null,
    });
  });

  it('flags halted when the backend reports paused, regardless of prior chunks', () => {
    const outcome = interpretChunkResult({
      completed: false,
      paused: true,
      paused_at_breakpoint: 'bp-42',
    });
    expect(outcome).toEqual({
      completed: false,
      halted: true,
      pausedAtBreakpoint: 'bp-42',
    });
  });

  it('normalizes a missing paused_at_breakpoint to null', () => {
    const outcome = interpretChunkResult({
      completed: false,
      paused: true,
      paused_at_breakpoint: undefined,
    });
    expect(outcome.pausedAtBreakpoint).toBeNull();
  });
});
