/**
 * useCompareStore — round-trip tests for the R4.2 compare slice.
 *
 * Covers:
 *   - picking sessions (toggle, clamp to 6, dedupe)
 *   - shared tick setter clamps to non-negative integers
 *   - layout setter (overlay / side-by-side / null auto)
 *   - active mode id
 *   - variable picker default + manual override + reset to auto
 */

import { beforeEach, describe, expect, it } from 'vitest';
import {
  clampPicks,
  resolveLayout,
  useCompareStore,
} from '../useCompareStore';

beforeEach(() => {
  useCompareStore.setState({
    pickedSessionIds: [],
    sharedTick: 0,
    isPlaying: false,
    layout: null,
    activeModeId: null,
    pickedVariables: null,
  });
});

describe('clampPicks', () => {
  it('dedupes while preserving first-seen order', () => {
    expect(clampPicks(['a', 'b', 'a', 'c', 'b'])).toEqual(['a', 'b', 'c']);
  });
  it('caps the set at 6 entries', () => {
    const picks = clampPicks(['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']);
    expect(picks).toHaveLength(6);
    expect(picks).toEqual(['a', 'b', 'c', 'd', 'e', 'f']);
  });
});

describe('resolveLayout', () => {
  it('uses overlay for ≤3 picks when no override', () => {
    expect(resolveLayout(null, 2)).toBe('overlay');
    expect(resolveLayout(null, 3)).toBe('overlay');
  });
  it('uses side-by-side for ≥4 picks when no override', () => {
    expect(resolveLayout(null, 4)).toBe('side-by-side');
    expect(resolveLayout(null, 6)).toBe('side-by-side');
  });
  it('respects the explicit user choice', () => {
    expect(resolveLayout('overlay', 5)).toBe('overlay');
    expect(resolveLayout('side-by-side', 2)).toBe('side-by-side');
  });
});

describe('useCompareStore — picks', () => {
  it('adds picks through toggle', () => {
    const { togglePickedSession } = useCompareStore.getState();
    togglePickedSession('s1');
    togglePickedSession('s2');
    expect(useCompareStore.getState().pickedSessionIds).toEqual(['s1', 's2']);
  });

  it('removes picks on re-toggle', () => {
    const { togglePickedSession } = useCompareStore.getState();
    togglePickedSession('s1');
    togglePickedSession('s2');
    togglePickedSession('s1');
    expect(useCompareStore.getState().pickedSessionIds).toEqual(['s2']);
  });

  it('setPickedSessionIds replaces the set (clamped and deduped)', () => {
    useCompareStore.getState().setPickedSessionIds([
      'a', 'b', 'c', 'd', 'e', 'f', 'g', 'a',
    ]);
    expect(useCompareStore.getState().pickedSessionIds).toEqual([
      'a', 'b', 'c', 'd', 'e', 'f',
    ]);
  });
});

describe('useCompareStore — playhead', () => {
  it('setSharedTick floors + clamps to >= 0', () => {
    const { setSharedTick } = useCompareStore.getState();
    setSharedTick(-1);
    expect(useCompareStore.getState().sharedTick).toBe(0);
    setSharedTick(3.9);
    expect(useCompareStore.getState().sharedTick).toBe(3);
  });

  it('ignores NaN / Infinity and stays at 0', () => {
    const { setSharedTick } = useCompareStore.getState();
    setSharedTick(Number.NaN);
    expect(useCompareStore.getState().sharedTick).toBe(0);
    setSharedTick(Number.POSITIVE_INFINITY);
    expect(useCompareStore.getState().sharedTick).toBe(0);
  });

  it('setIsPlaying toggles round-trip', () => {
    useCompareStore.getState().setIsPlaying(true);
    expect(useCompareStore.getState().isPlaying).toBe(true);
    useCompareStore.getState().setIsPlaying(false);
    expect(useCompareStore.getState().isPlaying).toBe(false);
  });
});

describe('useCompareStore — layout', () => {
  it('accepts null / overlay / side-by-side', () => {
    const { setLayout } = useCompareStore.getState();
    setLayout('overlay');
    expect(useCompareStore.getState().layout).toBe('overlay');
    setLayout('side-by-side');
    expect(useCompareStore.getState().layout).toBe('side-by-side');
    setLayout(null);
    expect(useCompareStore.getState().layout).toBe(null);
  });
});

describe('useCompareStore — active mode', () => {
  it('round-trips an id', () => {
    useCompareStore.getState().setActiveModeId('ensemble');
    expect(useCompareStore.getState().activeModeId).toBe('ensemble');
    useCompareStore.getState().setActiveModeId(null);
    expect(useCompareStore.getState().activeModeId).toBe(null);
  });
});

describe('useCompareStore — picked variables', () => {
  it('starts as null (auto)', () => {
    expect(useCompareStore.getState().pickedVariables).toBe(null);
  });
  it('accepts manual lists + resets to auto with null', () => {
    useCompareStore.getState().setPickedVariables(['x', 'y']);
    expect(useCompareStore.getState().pickedVariables).toEqual(['x', 'y']);
    useCompareStore.getState().setPickedVariables(null);
    expect(useCompareStore.getState().pickedVariables).toBe(null);
  });
});
