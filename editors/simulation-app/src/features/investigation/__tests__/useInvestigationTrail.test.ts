/**
 * Unit tests for `useInvestigationTrail` (ninebar Phase 1, audit F15) —
 * the multi-hop breadcrumb store that replaced `useSessionStore.drilledFrom`.
 *
 * Covers push/popTo/clear and the "pushing while behind the end
 * truncates forward hops" browser-history semantic that is the whole
 * reason this store exists instead of a plain single-slot field.
 */

import { beforeEach, describe, expect, it } from 'vitest';
import {
  selectCurrentHop,
  useInvestigationTrail,
  type InvestigationHop,
} from '../useInvestigationTrail';

function hop(overrides: Partial<InvestigationHop> = {}): InvestigationHop {
  return {
    origin: 'verify',
    fromSessionId: 'sess-a',
    tick: 1,
    label: 'Verify · tick 1',
    ...overrides,
  };
}

beforeEach(() => {
  useInvestigationTrail.getState().clear();
});

describe('defaults', () => {
  it('starts empty with cursor -1', () => {
    const s = useInvestigationTrail.getState();
    expect(s.hops).toEqual([]);
    expect(s.cursor).toBe(-1);
  });
});

describe('push', () => {
  it('appends a hop and moves the cursor to it', () => {
    useInvestigationTrail.getState().push(hop({ label: 'a' }));
    let s = useInvestigationTrail.getState();
    expect(s.hops.map((h) => h.label)).toEqual(['a']);
    expect(s.cursor).toBe(0);

    useInvestigationTrail.getState().push(hop({ label: 'b' }));
    s = useInvestigationTrail.getState();
    expect(s.hops.map((h) => h.label)).toEqual(['a', 'b']);
    expect(s.cursor).toBe(1);
  });

  it('truncates forward hops when pushed from a popped-back cursor (browser-history semantics)', () => {
    const trail = useInvestigationTrail.getState();
    trail.push(hop({ label: 'a' }));
    trail.push(hop({ label: 'b' }));
    trail.push(hop({ label: 'c' }));
    expect(useInvestigationTrail.getState().hops.map((h) => h.label)).toEqual([
      'a',
      'b',
      'c',
    ]);

    // Pop back to 'a' — 'b' and 'c' are still in `hops`, just past cursor.
    useInvestigationTrail.getState().popTo(0);
    expect(useInvestigationTrail.getState().hops.map((h) => h.label)).toEqual([
      'a',
      'b',
      'c',
    ]);
    expect(useInvestigationTrail.getState().cursor).toBe(0);

    // A fresh push from here discards 'b' and 'c' and appends 'd'.
    useInvestigationTrail.getState().push(hop({ label: 'd' }));
    const s = useInvestigationTrail.getState();
    expect(s.hops.map((h) => h.label)).toEqual(['a', 'd']);
    expect(s.cursor).toBe(1);
  });

  it('pushing from an empty trail after popTo(-1) starts a fresh single-hop trail', () => {
    const trail = useInvestigationTrail.getState();
    trail.push(hop({ label: 'a' }));
    trail.popTo(-1);
    trail.push(hop({ label: 'b' }));
    const s = useInvestigationTrail.getState();
    expect(s.hops.map((h) => h.label)).toEqual(['b']);
    expect(s.cursor).toBe(0);
  });
});

describe('popTo', () => {
  it('moves the cursor without discarding hops', () => {
    const trail = useInvestigationTrail.getState();
    trail.push(hop({ label: 'a' }));
    trail.push(hop({ label: 'b' }));
    trail.push(hop({ label: 'c' }));
    trail.popTo(1);
    const s = useInvestigationTrail.getState();
    expect(s.cursor).toBe(1);
    expect(s.hops).toHaveLength(3);
  });

  it('clamps below -1 to -1', () => {
    useInvestigationTrail.getState().push(hop());
    useInvestigationTrail.getState().popTo(-99);
    expect(useInvestigationTrail.getState().cursor).toBe(-1);
  });

  it('clamps above the last index to the last index', () => {
    useInvestigationTrail.getState().push(hop());
    useInvestigationTrail.getState().popTo(99);
    expect(useInvestigationTrail.getState().cursor).toBe(0);
  });

  it('is a no-op on an empty trail (clamps to -1)', () => {
    useInvestigationTrail.getState().popTo(3);
    expect(useInvestigationTrail.getState().cursor).toBe(-1);
    expect(useInvestigationTrail.getState().hops).toEqual([]);
  });
});

describe('clear', () => {
  it('drops all hops and resets the cursor', () => {
    const trail = useInvestigationTrail.getState();
    trail.push(hop({ label: 'a' }));
    trail.push(hop({ label: 'b' }));
    trail.clear();
    const s = useInvestigationTrail.getState();
    expect(s.hops).toEqual([]);
    expect(s.cursor).toBe(-1);
  });
});

describe('selectCurrentHop', () => {
  it('returns null on an empty trail', () => {
    expect(selectCurrentHop(useInvestigationTrail.getState())).toBeNull();
  });

  it('returns the hop at the cursor', () => {
    useInvestigationTrail.getState().push(hop({ label: 'a' }));
    useInvestigationTrail.getState().push(hop({ label: 'b' }));
    expect(selectCurrentHop(useInvestigationTrail.getState())?.label).toBe('b');
    useInvestigationTrail.getState().popTo(0);
    expect(selectCurrentHop(useInvestigationTrail.getState())?.label).toBe('a');
  });

  it('returns null after popTo(-1) even though hops remain in the array', () => {
    useInvestigationTrail.getState().push(hop());
    useInvestigationTrail.getState().popTo(-1);
    expect(selectCurrentHop(useInvestigationTrail.getState())).toBeNull();
    expect(useInvestigationTrail.getState().hops).toHaveLength(1);
  });
});
