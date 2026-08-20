/**
 * Tests for useRightRailStore (ninebar Phase 1, task #9).
 *
 * Covers the rules documented on the store: empty by default, opening a
 * transient context replaces the previous one, pinning promotes the
 * current transient (freeing the transient slot), a previously-pinned
 * context is replaced on re-pin, and the pinned/transient slots are
 * independent (close/unpin each touch only their own slot) — which is
 * also how "max two stacked" is enforced: there are only ever two slots.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { useRightRailStore } from '../railStore';

function reset() {
  useRightRailStore.setState({ pinned: null, transient: null });
}

afterEach(reset);

describe('useRightRailStore', () => {
  it('is empty by default (fresh load)', () => {
    const state = useRightRailStore.getState();
    expect(state.pinned).toBeNull();
    expect(state.transient).toBeNull();
  });

  it('open() sets the transient slot and leaves pinned untouched', () => {
    useRightRailStore.getState().open('stream-status');
    const state = useRightRailStore.getState();
    expect(state.transient).toBe('stream-status');
    expect(state.pinned).toBeNull();
  });

  it('opening a new context replaces the previous transient', () => {
    const { open } = useRightRailStore.getState();
    open('a');
    open('b');
    const state = useRightRailStore.getState();
    expect(state.transient).toBe('b');
  });

  it('opening the already-pinned context is a no-op', () => {
    const { open, pin } = useRightRailStore.getState();
    open('a');
    pin('a');
    open('a');
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('a');
    expect(state.transient).toBeNull();
  });

  it('pin() promotes the current transient into the pinned slot and clears transient', () => {
    const { open, pin } = useRightRailStore.getState();
    open('a');
    pin('a');
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('a');
    expect(state.transient).toBeNull();
  });

  it('pinning replaces any previously pinned context', () => {
    const { open, pin } = useRightRailStore.getState();
    open('a');
    pin('a');
    open('b');
    pin('b');
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('b');
    expect(state.transient).toBeNull();
  });

  it('max two stacked: a pinned context and a transient context coexist, never more', () => {
    const { open, pin } = useRightRailStore.getState();
    open('a');
    pin('a');
    open('b');
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('a');
    expect(state.transient).toBe('b');
    // Structurally impossible to exceed two — only two fields exist.
    expect(Object.keys(state).filter((k) => k === 'pinned' || k === 'transient')).toHaveLength(2);
  });

  it('close() clears only the transient slot; a pinned context stays open', () => {
    const { open, pin, close } = useRightRailStore.getState();
    open('a');
    pin('a');
    open('b');
    close();
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('a');
    expect(state.transient).toBeNull();
  });

  it('unpin() clears only the pinned slot; a transient context stays open', () => {
    const { open, pin, unpin } = useRightRailStore.getState();
    open('a');
    pin('a');
    open('b');
    unpin();
    const state = useRightRailStore.getState();
    expect(state.pinned).toBeNull();
    expect(state.transient).toBe('b');
  });

  it('pinning a context that is not the current transient leaves transient untouched', () => {
    const { open, pin } = useRightRailStore.getState();
    open('b');
    pin('a'); // not the current transient
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('a');
    expect(state.transient).toBe('b');
  });
});
