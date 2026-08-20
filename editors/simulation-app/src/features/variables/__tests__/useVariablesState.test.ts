/**
 * useVariablesState — pin / collapse / filter / search / sparkline toggle.
 *
 * Tests the plain-function store API (createVariablesStore), not the React
 * hook, so the suite stays DOM-free. The hook is a thin
 * useSyncExternalStore wrapper and is covered by higher-level integration
 * tests in Playwright.
 */

import { describe, it, expect } from 'vitest';
import {
  createVariablesStore,
  createMemoryStorage,
} from '../useVariablesState';

describe('createVariablesStore — defaults', () => {
  it('starts with an empty pinned set, sparklines on, filter "all"', () => {
    const store = createVariablesStore(createMemoryStorage());
    const s = store.getState();
    expect(Array.from(s.pinned)).toEqual([]);
    expect(Array.from(s.collapsed)).toEqual([]);
    expect(s.showSparklines).toBe(true);
    expect(s.filter).toBe('all');
    expect(s.search).toBe('');
  });

  it('hydrates from provided storage', () => {
    const storage = createMemoryStorage({
      pinned: ['a', 'b'],
      collapsed: ['pkg'],
      showSparklines: false,
      filter: 'failing',
    });
    const store = createVariablesStore(storage);
    const s = store.getState();
    expect(Array.from(s.pinned).sort()).toEqual(['a', 'b']);
    expect(Array.from(s.collapsed)).toEqual(['pkg']);
    expect(s.showSparklines).toBe(false);
    expect(s.filter).toBe('failing');
  });
});

describe('pin state', () => {
  it('togglePinned adds and removes names idempotently', () => {
    const store = createVariablesStore(createMemoryStorage());
    store.actions.togglePinned('foo');
    expect(store.getState().pinned.has('foo')).toBe(true);
    store.actions.togglePinned('foo');
    expect(store.getState().pinned.has('foo')).toBe(false);
  });

  it('persists pin changes through the storage contract', () => {
    const storage = createMemoryStorage();
    const store = createVariablesStore(storage);
    store.actions.togglePinned('x');
    store.actions.togglePinned('y');
    // A fresh store using the same storage should rehydrate the pins.
    const rehydrated = createVariablesStore(storage);
    expect(Array.from(rehydrated.getState().pinned).sort()).toEqual(['x', 'y']);
  });

  it('setPinned replaces the entire set atomically', () => {
    const store = createVariablesStore(createMemoryStorage());
    store.actions.togglePinned('foo');
    store.actions.setPinned(['a', 'b', 'c']);
    expect(Array.from(store.getState().pinned).sort()).toEqual(['a', 'b', 'c']);
  });
});

describe('collapse state', () => {
  it('toggleCollapsed flips path membership', () => {
    const store = createVariablesStore(createMemoryStorage());
    store.actions.toggleCollapsed('circuit1.breaker');
    expect(store.getState().collapsed.has('circuit1.breaker')).toBe(true);
    store.actions.toggleCollapsed('circuit1.breaker');
    expect(store.getState().collapsed.has('circuit1.breaker')).toBe(false);
  });

  it('expandAll clears the set', () => {
    const store = createVariablesStore(createMemoryStorage());
    store.actions.toggleCollapsed('a');
    store.actions.toggleCollapsed('b');
    store.actions.expandAll();
    expect(store.getState().collapsed.size).toBe(0);
  });
});

describe('search / filter / sparkline toggle', () => {
  it('search is transient: does NOT round-trip via storage', () => {
    const storage = createMemoryStorage();
    const store = createVariablesStore(storage);
    store.actions.setSearch('temperature');
    expect(store.getState().search).toBe('temperature');

    const rehydrated = createVariablesStore(storage);
    expect(rehydrated.getState().search).toBe('');
  });

  it('filter and showSparklines ARE persisted', () => {
    const storage = createMemoryStorage();
    const store = createVariablesStore(storage);
    store.actions.setFilter('failing');
    store.actions.setShowSparklines(false);

    const rehydrated = createVariablesStore(storage);
    expect(rehydrated.getState().filter).toBe('failing');
    expect(rehydrated.getState().showSparklines).toBe(false);
  });
});

describe('selectedPath', () => {
  it('setSelectedPath updates without persisting', () => {
    const storage = createMemoryStorage();
    const store = createVariablesStore(storage);
    store.actions.setSelectedPath('circuit1.breaker.v');
    expect(store.getState().selectedPath).toBe('circuit1.breaker.v');
    const rehydrated = createVariablesStore(storage);
    expect(rehydrated.getState().selectedPath).toBeNull();
  });
});

describe('reset()', () => {
  it('clears every piece of transient + persistent state', () => {
    const storage = createMemoryStorage();
    const store = createVariablesStore(storage);
    store.actions.togglePinned('a');
    store.actions.toggleCollapsed('b');
    store.actions.setFilter('failing');
    store.actions.setSearch('x');
    store.actions.setShowSparklines(false);
    store.actions.setSelectedPath('a');

    store.actions.reset();

    const s = store.getState();
    expect(s.pinned.size).toBe(0);
    expect(s.collapsed.size).toBe(0);
    expect(s.filter).toBe('all');
    expect(s.search).toBe('');
    expect(s.showSparklines).toBe(true);
    expect(s.selectedPath).toBeNull();
  });
});

describe('subscribe contract', () => {
  it('notifies subscribers on every mutation and unsubscribes cleanly', () => {
    const store = createVariablesStore(createMemoryStorage());
    const calls: number[] = [];
    const unsub = store.subscribe(() => calls.push(store.getState().pinned.size));
    store.actions.togglePinned('a');
    store.actions.togglePinned('b');
    store.actions.togglePinned('a');
    expect(calls).toEqual([1, 2, 1]);
    unsub();
    store.actions.togglePinned('c');
    expect(calls).toEqual([1, 2, 1]); // no more notifications
  });
});
