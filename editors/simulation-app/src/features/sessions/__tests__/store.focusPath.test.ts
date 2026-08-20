/**
 * Focused unit tests for the `focusPath` slice of the session store.
 *
 * Phase A2 scaffolding: no UI reads `focusPath` yet, but the slice is
 * the single source of truth the new tree will consume in Phase B. The
 * non-obvious semantics — REPLACE on click (not push), reset on session
 * switch — are worth pinning down now so Phase B can't regress them by
 * accident.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useSessionStore } from '../store';
import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';

function reset() {
  useSessionStore.getState().resetViewState();
  useSessionStore.getState().setActiveSession(null);
}

beforeEach(() => {
  reset();
});

describe('focusPath — defaults', () => {
  it('starts empty (focused on the session root)', () => {
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });
});

describe('setFocusPath — replace semantics', () => {
  it('replaces the path entirely (never appends)', () => {
    const { setFocusPath } = useSessionStore.getState();
    setFocusPath(['production cell']);
    setFocusPath(['production cell', 'station1']);
    setFocusPath(['production cell', 'station1', 'groupHead']);
    expect(useSessionStore.getState().focusPath).toEqual([
      'production cell',
      'station1',
      'groupHead',
    ]);
    // Replacing with a shorter path truncates — no stack-like growth.
    setFocusPath(['production cell']);
    expect(useSessionStore.getState().focusPath).toEqual(['production cell']);
  });

  it('copies the input so later mutation of the caller array does not leak in', () => {
    const input = ['a', 'b', 'c'];
    useSessionStore.getState().setFocusPath(input);
    input.push('d');
    expect(useSessionStore.getState().focusPath).toEqual(['a', 'b', 'c']);
  });

  it('accepts [] to focus the session root', () => {
    const { setFocusPath } = useSessionStore.getState();
    setFocusPath(['a', 'b']);
    setFocusPath([]);
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });
});

describe('navigateFocusToDepth — breadcrumb navigation', () => {
  it('truncates to the requested depth, leaving a strict prefix', () => {
    useSessionStore.getState().setFocusPath(['a', 'b', 'c', 'd']);
    useSessionStore.getState().navigateFocusToDepth(2);
    expect(useSessionStore.getState().focusPath).toEqual(['a', 'b']);
  });

  it('depth 0 returns to the root', () => {
    useSessionStore.getState().setFocusPath(['a', 'b']);
    useSessionStore.getState().navigateFocusToDepth(0);
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });

  it('depth beyond the current path leaves the path unchanged (never extends)', () => {
    useSessionStore.getState().setFocusPath(['a', 'b']);
    useSessionStore.getState().navigateFocusToDepth(99);
    expect(useSessionStore.getState().focusPath).toEqual(['a', 'b']);
  });

  it('negative depth clamps to root instead of throwing', () => {
    useSessionStore.getState().setFocusPath(['a', 'b']);
    useSessionStore.getState().navigateFocusToDepth(-3);
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });
});

describe('clearFocusPath', () => {
  it('resets focusPath to []', () => {
    useSessionStore.getState().setFocusPath(['x', 'y']);
    useSessionStore.getState().clearFocusPath();
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });
});

describe('session switching', () => {
  it('clears focusPath when the active session changes', () => {
    const store = useSessionStore.getState();
    store.setActiveSession('session-a');
    useSessionStore.getState().setFocusPath(['production cell', 'station1']);
    expect(useSessionStore.getState().focusPath).toEqual([
      'production cell',
      'station1',
    ]);
    useSessionStore.getState().setActiveSession('session-b');
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });

  it('preserves focusPath when setActiveSession is called with the same id (idempotent)', () => {
    useSessionStore.getState().setActiveSession('session-a');
    useSessionStore.getState().setFocusPath(['a', 'b']);
    useSessionStore.getState().setActiveSession('session-a');
    expect(useSessionStore.getState().focusPath).toEqual(['a', 'b']);
  });

  it('resetViewState clears focusPath', () => {
    useSessionStore.getState().setFocusPath(['a', 'b']);
    useSessionStore.getState().resetViewState();
    expect(useSessionStore.getState().focusPath).toEqual([]);
  });
});

describe('investigation trail — cleared on session switch (ninebar Phase 1, F15)', () => {
  afterEach(() => {
    useInvestigationTrail.getState().clear();
  });

  it('clears useInvestigationTrail when the active session changes', () => {
    useSessionStore.getState().setActiveSession('session-a');
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'session-a',
      tick: 5,
      label: 'Verify · tick 5',
    });
    expect(useInvestigationTrail.getState().hops).toHaveLength(1);

    useSessionStore.getState().setActiveSession('session-b');
    expect(useInvestigationTrail.getState().hops).toEqual([]);
    expect(useInvestigationTrail.getState().cursor).toBe(-1);
  });

  it('preserves the trail when setActiveSession is called with the same id (idempotent)', () => {
    useSessionStore.getState().setActiveSession('session-a');
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'session-a',
      tick: 5,
      label: 'Verify · tick 5',
    });
    useSessionStore.getState().setActiveSession('session-a');
    expect(useInvestigationTrail.getState().hops).toHaveLength(1);
  });

  it('resetViewState also clears the trail', () => {
    useSessionStore.getState().setActiveSession('session-a');
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'session-a',
      tick: 5,
      label: 'Verify · tick 5',
    });
    useSessionStore.getState().resetViewState();
    expect(useInvestigationTrail.getState().hops).toEqual([]);
  });
});

describe('isolation from existing slices', () => {
  it('mutating focusPath does not touch focusedActionPath or selectedScope', () => {
    useSessionStore.getState().pushFocusedAction('doThing');
    useSessionStore.getState().setSelectedScope(['mod', 'sub']);
    useSessionStore.getState().setFocusPath(['x', 'y']);
    const s = useSessionStore.getState();
    expect(s.focusedActionPath).toEqual(['doThing']);
    expect(s.selectedScope).toEqual(['mod', 'sub']);
    expect(s.focusPath).toEqual(['x', 'y']);
  });
});
