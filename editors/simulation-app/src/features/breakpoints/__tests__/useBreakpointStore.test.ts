/**
 * useBreakpointStore — add / clear round-trip + extension-field persistence.
 *
 * The store is the shared source of truth for the panel AND the diagram
 * overlay, so its semantics need to be pinned tight. Tests focus on:
 *   - optimistic add → backend reconcile flow
 *   - remove / clearAll behaviour
 *   - hit tracking (lastHit) dropping a reference when the row is removed
 *   - R4 extension fields (condition / hitCount / logMessage / enabled)
 *     surviving the round trip untouched
 *   - selector correctness (armed count, target nodes for overlay)
 *   - pure helpers (breakpointLabel, breakpointIcon, breakpointTargetNode)
 */

import { beforeEach, describe, expect, it } from 'vitest';
import {
  useBreakpointStore,
  breakpointLabel,
  breakpointIcon,
  breakpointTargetNode,
  selectArmedCount,
  selectTargetNodes,
  makeLocalId,
} from '../useBreakpointStore';
import type { Breakpoint } from '@/engine/types';

// Reset between tests so Zustand's module-scope state doesn't leak.
beforeEach(() => {
  useBreakpointStore.getState().clearAll();
  useBreakpointStore.setState({ isAdding: false, lastHit: null });
});

describe('makeLocalId', () => {
  it('prefixes with `local-` so backend ids can never collide', () => {
    const id = makeLocalId();
    expect(id.startsWith('local-')).toBe(true);
  });

  it('emits unique ids across multiple calls', () => {
    const ids = new Set<string>();
    for (let i = 0; i < 25; i++) ids.add(makeLocalId());
    expect(ids.size).toBe(25);
  });
});

describe('breakpointLabel', () => {
  it('uses explicit label when set', () => {
    const bp: Breakpoint = {
      kind: 'state-entry',
      target: 'Engine.Running',
      label: 'Engine is running',
    };
    expect(breakpointLabel(bp)).toBe('Engine is running');
  });

  it('falls back to target for element-kinds', () => {
    expect(breakpointLabel({ kind: 'state-entry', target: 'Engine.Off' })).toBe('Engine.Off');
    expect(breakpointLabel({ kind: 'transition-fire', target: 'T1' })).toBe('T1');
    expect(breakpointLabel({ kind: 'action-invoke', target: 'Boot' })).toBe('Boot');
    expect(breakpointLabel({ kind: 'constraint-violation', target: 'C1' })).toBe('C1');
  });

  it('formats threshold-crossing with glyph per direction', () => {
    expect(
      breakpointLabel({
        kind: 'threshold-crossing',
        target: 'v',
        variable: 'v',
        threshold: 5,
        direction: 'rising',
      }),
    ).toBe('when `v` crosses ↑ 5');
    expect(
      breakpointLabel({
        kind: 'threshold-crossing',
        target: 'v',
        variable: 'v',
        threshold: 2,
        direction: 'falling',
      }),
    ).toBe('when `v` crosses ↓ 2');
    expect(
      breakpointLabel({
        kind: 'threshold-crossing',
        target: 'v',
        variable: 'v',
        threshold: 0,
      }),
    ).toBe('when `v` crosses ⇅ 0');
  });

  it('formats threshold-crossing with debounce suffix when set', () => {
    expect(
      breakpointLabel({
        kind: 'threshold-crossing',
        target: 'breaker.current',
        variable: 'breaker.current',
        threshold: 100,
        direction: 'rising',
        debounce_ticks: 3,
      }),
    ).toBe('when `breaker.current` crosses ↑ 100 (debounce 3t)');
  });

  it('formats conditional with scoped `target.variable op value`', () => {
    expect(
      breakpointLabel({
        kind: 'conditional',
        target: 'circuit',
        variable: 'voltage',
        op: 'gt',
        value: 12,
      }),
    ).toBe('when `circuit.voltage` > 12');
    expect(
      breakpointLabel({
        kind: 'conditional',
        target: 'p',
        variable: 'x',
        op: 'le',
        value: 0,
      }),
    ).toBe('when `p.x` ≤ 0');
  });
});

describe('breakpointIcon', () => {
  it('maps every kind to a material-symbol name', () => {
    expect(breakpointIcon('state-entry')).toBe('circle');
    expect(breakpointIcon('transition-fire')).toBe('arrow_forward');
    expect(breakpointIcon('action-invoke')).toBe('play_arrow');
    expect(breakpointIcon('constraint-violation')).toBe('rule');
    expect(breakpointIcon('threshold-crossing')).toBe('show_chart');
    expect(breakpointIcon('conditional')).toBe('function');
  });
});

describe('breakpointTargetNode', () => {
  it('returns target for element-kinds', () => {
    expect(breakpointTargetNode({ kind: 'state-entry', target: 'Engine.On' })).toBe('Engine.On');
  });
  it('returns null for threshold-crossing (variable-based)', () => {
    expect(
      breakpointTargetNode({
        kind: 'threshold-crossing',
        target: 'v',
        variable: 'v',
        threshold: 1,
      }),
    ).toBe(null);
  });
  it('returns target for conditional breakpoints (element-scoped)', () => {
    expect(
      breakpointTargetNode({
        kind: 'conditional',
        target: 'circuit1',
        variable: 'voltage',
        op: 'gt',
        value: 12,
      }),
    ).toBe('circuit1');
  });
});

describe('useBreakpointStore — conditional breakpoints (R4.4)', () => {
  it('accepts a conditional shape and carries all required fields', () => {
    const { addLocal } = useBreakpointStore.getState();
    const id = addLocal({
      breakpoint: {
        kind: 'conditional',
        target: 'circuit1',
        variable: 'voltage',
        op: 'gt',
        value: 12.0,
      },
    });
    const { breakpoints } = useBreakpointStore.getState();
    expect(breakpoints).toHaveLength(1);
    const entry = breakpoints[0]!;
    expect(entry.id).toBe(id);
    expect(entry.enabled).toBe(true);
    expect(entry.breakpoint).toEqual({
      kind: 'conditional',
      target: 'circuit1',
      variable: 'voltage',
      op: 'gt',
      value: 12.0,
    });
  });

  it('removes a conditional row via the standard store API', () => {
    const { addLocal, remove } = useBreakpointStore.getState();
    const id = addLocal({
      breakpoint: {
        kind: 'conditional',
        target: 'p',
        variable: 'x',
        op: 'lt',
        value: 0,
      },
    });
    remove(id);
    expect(useBreakpointStore.getState().breakpoints).toHaveLength(0);
  });

  it('selectTargetNodes surfaces the owning element for conditional breakpoints', () => {
    const { addLocal } = useBreakpointStore.getState();
    addLocal({
      breakpoint: {
        kind: 'conditional',
        target: 'Engine.main',
        variable: 'rpm',
        op: 'ge',
        value: 5000,
      },
    });
    expect(selectTargetNodes(useBreakpointStore.getState())).toEqual(['Engine.main']);
  });
});

describe('useBreakpointStore — add / remove / reconcile', () => {
  it('addLocal inserts a new entry with a fresh id and defaults enabled=true', () => {
    const { addLocal } = useBreakpointStore.getState();
    const id = addLocal({
      breakpoint: { kind: 'state-entry', target: 'Engine.Off' },
    });
    const { breakpoints } = useBreakpointStore.getState();
    expect(breakpoints).toHaveLength(1);
    expect(breakpoints[0]!.id).toBe(id);
    expect(breakpoints[0]!.enabled).toBe(true);
  });

  it('reconcileId replaces the local id with the backend one on both wrappers', () => {
    const { addLocal, reconcileId } = useBreakpointStore.getState();
    const localId = addLocal({
      breakpoint: { kind: 'state-entry', target: 'Engine.On' },
    });
    reconcileId(localId, 'backend-42');
    const [entry] = useBreakpointStore.getState().breakpoints;
    expect(entry!.id).toBe('backend-42');
    expect(entry!.breakpoint.id).toBe('backend-42');
  });

  it('reconcileId leaves other entries alone', () => {
    const { addLocal, reconcileId } = useBreakpointStore.getState();
    const a = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    const b = addLocal({ breakpoint: { kind: 'state-entry', target: 'B' } });
    reconcileId(a, 'bk-A');
    const { breakpoints } = useBreakpointStore.getState();
    expect(breakpoints.find((e) => e.id === 'bk-A')).toBeDefined();
    expect(breakpoints.find((e) => e.id === b)).toBeDefined();
  });

  it('remove drops the entry (and lastHit if it matched)', () => {
    const { addLocal, remove, recordHit } = useBreakpointStore.getState();
    const id = addLocal({
      breakpoint: { kind: 'state-entry', target: 'A' },
    });
    recordHit({ id, hitAtMs: 1 });
    remove(id);
    const state = useBreakpointStore.getState();
    expect(state.breakpoints).toHaveLength(0);
    expect(state.lastHit).toBe(null);
  });

  it('remove leaves lastHit alone when a non-hit row is removed', () => {
    const { addLocal, remove, recordHit } = useBreakpointStore.getState();
    const hitId = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    const otherId = addLocal({ breakpoint: { kind: 'state-entry', target: 'B' } });
    recordHit({ id: hitId, hitAtMs: 1 });
    remove(otherId);
    expect(useBreakpointStore.getState().lastHit?.id).toBe(hitId);
  });

  it('clearAll drops every entry and the lastHit', () => {
    const { addLocal, clearAll, recordHit } = useBreakpointStore.getState();
    const id = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    recordHit({ id, hitAtMs: 1 });
    clearAll();
    const state = useBreakpointStore.getState();
    expect(state.breakpoints).toHaveLength(0);
    expect(state.lastHit).toBe(null);
  });

  it('replaceAll swaps the whole list (used by listBreakpoints sync)', () => {
    const { addLocal, replaceAll } = useBreakpointStore.getState();
    addLocal({ breakpoint: { kind: 'state-entry', target: 'Old' } });
    replaceAll([
      {
        id: 'bk-1',
        breakpoint: { kind: 'state-entry', target: 'New1' },
      },
      {
        id: 'bk-2',
        breakpoint: { kind: 'action-invoke', target: 'New2' },
      },
    ]);
    const list = useBreakpointStore.getState().breakpoints;
    expect(list.map((e) => e.id)).toEqual(['bk-1', 'bk-2']);
  });

  it('patch updates R4 extension fields without touching the core breakpoint', () => {
    const { addLocal, patch } = useBreakpointStore.getState();
    const id = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    patch(id, { condition: 'x > 5', hitCount: 3, logMessage: 'Hello' });
    const [entry] = useBreakpointStore.getState().breakpoints;
    expect(entry!.condition).toBe('x > 5');
    expect(entry!.hitCount).toBe(3);
    expect(entry!.logMessage).toBe('Hello');
    expect(entry!.breakpoint).toEqual({ kind: 'state-entry', target: 'A' });
  });
});

describe('extensibility fields persist through the add round-trip', () => {
  it('addLocal records condition / hitCount / logMessage untouched', () => {
    const { addLocal } = useBreakpointStore.getState();
    addLocal({
      breakpoint: { kind: 'state-entry', target: 'A' },
      condition: 'I > 32',
      hitCount: 5,
      logMessage: 'trip',
    });
    const [entry] = useBreakpointStore.getState().breakpoints;
    expect(entry!.condition).toBe('I > 32');
    expect(entry!.hitCount).toBe(5);
    expect(entry!.logMessage).toBe('trip');
  });

  it('soft-disable via patch updates enabled without removing the row', () => {
    const { addLocal, patch } = useBreakpointStore.getState();
    const id = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    patch(id, { enabled: false });
    const state = useBreakpointStore.getState();
    expect(state.breakpoints).toHaveLength(1);
    expect(state.breakpoints[0]!.enabled).toBe(false);
  });
});

describe('selectors', () => {
  it('selectArmedCount excludes soft-disabled rows', () => {
    const { addLocal, patch } = useBreakpointStore.getState();
    const a = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    addLocal({ breakpoint: { kind: 'state-entry', target: 'B' } });
    patch(a, { enabled: false });
    expect(selectArmedCount(useBreakpointStore.getState())).toBe(1);
  });

  it('selectTargetNodes returns unique element targets and skips thresholds', () => {
    const { addLocal } = useBreakpointStore.getState();
    addLocal({ breakpoint: { kind: 'state-entry', target: 'Engine.On' } });
    // Dup should collapse.
    addLocal({ breakpoint: { kind: 'action-invoke', target: 'Engine.On' } });
    addLocal({ breakpoint: { kind: 'transition-fire', target: 'T1' } });
    addLocal({
      breakpoint: {
        kind: 'threshold-crossing',
        target: 'v',
        variable: 'v',
        threshold: 3,
      },
    });
    const nodes = selectTargetNodes(useBreakpointStore.getState());
    expect(new Set(nodes)).toEqual(new Set(['Engine.On', 'T1']));
  });

  it('selectTargetNodes drops soft-disabled rows', () => {
    const { addLocal, patch } = useBreakpointStore.getState();
    const id = addLocal({ breakpoint: { kind: 'state-entry', target: 'A' } });
    patch(id, { enabled: false });
    expect(selectTargetNodes(useBreakpointStore.getState())).toEqual([]);
  });
});

describe('recordHit', () => {
  it('captures hit id + context', () => {
    const { recordHit } = useBreakpointStore.getState();
    recordHit({ id: 'bk-1', hitAtMs: 1700_000_000_000, context: { target: 'Engine.On' } });
    const { lastHit } = useBreakpointStore.getState();
    expect(lastHit?.id).toBe('bk-1');
    expect(lastHit?.context?.target).toBe('Engine.On');
  });
});
