/**
 * Zustand store tests for `useCausalTraceStore` (R7.1).
 */

import { beforeEach, describe, expect, it } from 'vitest';
import {
  setCausalTraceRoot,
  useCausalTraceStore,
} from '../useCausalTraceStore';

beforeEach(() => {
  useCausalTraceStore.setState({ root: null, refocusTick: 0 });
});

describe('useCausalTraceStore', () => {
  it('starts with no root and refocusTick=0', () => {
    const state = useCausalTraceStore.getState();
    expect(state.root).toBeNull();
    expect(state.refocusTick).toBe(0);
  });

  it('setRoot stores the root and bumps refocusTick', () => {
    useCausalTraceStore.getState().setRoot({
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    });
    const after = useCausalTraceStore.getState();
    expect(after.root).toEqual({
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    });
    expect(after.refocusTick).toBe(1);
  });

  it('setRoot twice bumps the refocus tick twice even with identical root', () => {
    const root = {
      kind: 'by-tick' as const,
      sessionId: 's1',
      tick: 5,
      target: 'speed',
    };
    useCausalTraceStore.getState().setRoot(root);
    useCausalTraceStore.getState().setRoot(root);
    expect(useCausalTraceStore.getState().refocusTick).toBe(2);
  });

  it('clear resets root to null (refocusTick left alone)', () => {
    useCausalTraceStore.getState().setRoot({
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    });
    useCausalTraceStore.getState().clear();
    const after = useCausalTraceStore.getState();
    expect(after.root).toBeNull();
    // refocusTick was bumped by setRoot; clear doesn't bump it.
    expect(after.refocusTick).toBe(1);
  });

  it('setCausalTraceRoot helper works outside React', () => {
    setCausalTraceRoot({
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    });
    expect(useCausalTraceStore.getState().root).not.toBeNull();
  });
});
