/**
 * Tests for railCommands — the client-side rail open/close/pin/unpin
 * command-palette entries (ninebar Phase 1).
 *
 * Node-environment vitest tests (no DOM), matching `CommandPalette.test.tsx`'s
 * convention: these exercise the pure catalog-shape + store-mutation
 * contract, not the interactive DOM flow (that's Playwright's job).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { getRailCommands } from '../railCommands';
import { filterCommands } from '../commandCatalog';
import { useRightRailStore } from '@/app/rail/railStore';

function resetRailStore() {
  useRightRailStore.setState({ pinned: null, transient: null });
}

afterEach(resetRailStore);

describe('getRailCommands', () => {
  it('returns a CommandMeta-shaped entry per rail action, all category "Client"', () => {
    const commands = getRailCommands();
    expect(commands.length).toBeGreaterThanOrEqual(5);
    for (const cmd of commands) {
      expect(cmd.category).toBe('Client');
      // Every client entry acts via a direct action OR a router
      // navigation target (Phase 6 `open.compare`) — never neither.
      expect(
        typeof cmd.clientAction === 'function' ||
          typeof cmd.navigateTo === 'string',
      ).toBe(true);
      expect(cmd.params).toEqual([]);
    }
    expect(commands.map((c) => c.name)).toEqual(
      expect.arrayContaining([
        'rail.open.variables',
        'rail.open.breakpoints',
        'rail.open.diagnostics',
        'rail.close',
        'rail.pin',
        'rail.unpin',
      ]),
    );
  });

  it('is searchable via the palette\'s fuzzy filter like any backend command', () => {
    const results = filterCommands(getRailCommands(), 'variables');
    expect(results.map((c) => c.name)).toContain('rail.open.variables');
  });

  it('rail.open.variables opens the variables context as transient', () => {
    const cmd = getRailCommands().find((c) => c.name === 'rail.open.variables');
    cmd?.clientAction?.();
    expect(useRightRailStore.getState().transient).toBe('variables');
  });

  it('rail.open.breakpoints replaces whatever was transient', () => {
    useRightRailStore.getState().open('diagnostics');
    const cmd = getRailCommands().find((c) => c.name === 'rail.open.breakpoints');
    cmd?.clientAction?.();
    expect(useRightRailStore.getState().transient).toBe('breakpoints');
  });

  it('rail.close clears only the transient slot', () => {
    useRightRailStore.getState().open('variables');
    useRightRailStore.getState().pin('diagnostics');
    const cmd = getRailCommands().find((c) => c.name === 'rail.close');
    cmd?.clientAction?.();
    const state = useRightRailStore.getState();
    expect(state.transient).toBeNull();
    expect(state.pinned).toBe('diagnostics');
  });

  it('rail.pin promotes the current transient into the pinned slot', () => {
    useRightRailStore.getState().open('views');
    const cmd = getRailCommands().find((c) => c.name === 'rail.pin');
    cmd?.clientAction?.();
    const state = useRightRailStore.getState();
    expect(state.pinned).toBe('views');
    expect(state.transient).toBeNull();
  });

  it('rail.pin is a no-op when nothing is transient', () => {
    const cmd = getRailCommands().find((c) => c.name === 'rail.pin');
    cmd?.clientAction?.();
    expect(useRightRailStore.getState().pinned).toBeNull();
  });

  it('rail.unpin clears only the pinned slot', () => {
    useRightRailStore.getState().open('variables');
    useRightRailStore.getState().pin('variables');
    useRightRailStore.getState().open('breakpoints');
    const cmd = getRailCommands().find((c) => c.name === 'rail.unpin');
    cmd?.clientAction?.();
    const state = useRightRailStore.getState();
    expect(state.pinned).toBeNull();
    expect(state.transient).toBe('breakpoints');
  });
});
