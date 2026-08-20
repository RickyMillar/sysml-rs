/**
 * Tests for `clearWorkspace()` (Bucket 5-followup, 2026-05-05).
 *
 * Pins the per-slot wipe contract so a future refactor that adds a new
 * slot to `useWorkspaceStore` doesn't silently leave stale data
 * dangling across workspace switches.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { useWorkspaceStore, type Capabilities } from '@/store/workspace';

const ORIGINAL_STATE = useWorkspaceStore.getState();

function seedStaleState() {
  useWorkspaceStore.setState({
    workspaceRoot: '/old/root',
    loadedFiles: new Map([
      [
        'file:///a.sysml',
        { uri: 'file:///a.sysml', source: 'x', dirty: false, tree: [] },
      ],
    ]),
    focusedUri: 'file:///a.sysml',
    smodel: { kind: 'graph' } as Record<string, unknown>,
    tableModel: { columns: [], rows: [] } as never,
    geometryModel: { primitives: [] } as never,
    treeModel: { roots: [] } as never,
    selectedViewId: 'view-old',
    workspaceTree: [{ uri: 'file:///a.sysml', name: 'a.sysml', children: [] } as never],
    capabilities: {
      ...ORIGINAL_STATE.capabilities,
      hasStateMachines: true,
      stateMachineNames: ['Engine'],
    } as Capabilities,
    statsCache: new Map([
      ['file:///a.sysml', { PartUsage: 1 } as Record<string, number>],
    ]),
  });
}

describe('useWorkspaceStore.clearWorkspace', () => {
  beforeEach(() => seedStaleState());
  afterEach(() => useWorkspaceStore.setState(ORIGINAL_STATE, true));

  it('clears loadedFiles', () => {
    useWorkspaceStore.getState().clearWorkspace();
    expect(useWorkspaceStore.getState().loadedFiles.size).toBe(0);
  });

  it('clears statsCache', () => {
    useWorkspaceStore.getState().clearWorkspace();
    expect(useWorkspaceStore.getState().statsCache.size).toBe(0);
  });

  it('clears focusedUri', () => {
    useWorkspaceStore.getState().clearWorkspace();
    expect(useWorkspaceStore.getState().focusedUri).toBeNull();
  });

  it('clears selectedViewId', () => {
    useWorkspaceStore.getState().clearWorkspace();
    expect(useWorkspaceStore.getState().selectedViewId).toBeNull();
  });

  it('clears the four diagram-model slots', () => {
    useWorkspaceStore.getState().clearWorkspace();
    const s = useWorkspaceStore.getState();
    expect(s.smodel).toBeNull();
    expect(s.tableModel).toBeNull();
    expect(s.geometryModel).toBeNull();
    expect(s.treeModel).toBeNull();
  });

  it('resets workspaceTree', () => {
    useWorkspaceStore.getState().clearWorkspace();
    expect(useWorkspaceStore.getState().workspaceTree).toEqual([]);
  });

  it('resets capabilities to the empty default', () => {
    useWorkspaceStore.getState().clearWorkspace();
    const caps = useWorkspaceStore.getState().capabilities;
    expect(caps.hasStateMachines).toBe(false);
    expect(caps.stateMachineNames).toEqual([]);
  });

  it('preserves workspaceRoot — set by the caller after clearing', () => {
    useWorkspaceStore.getState().clearWorkspace();
    expect(useWorkspaceStore.getState().workspaceRoot).toBe('/old/root');
  });
});
