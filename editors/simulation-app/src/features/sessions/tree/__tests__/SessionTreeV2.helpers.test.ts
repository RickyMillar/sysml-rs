/**
 * Pure-helper tests for SessionTreeV2's persistence + seeding logic.
 *
 * Task 1 scope — depth-2 auto-expand, Expand-all coverage, and the
 * localStorage roundtrip. Full-component tests (render + store +
 * react-query) are intentionally skipped: the helpers below are
 * where the logic lives, and they're verifiable without mocking
 * five hooks.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { TreeNode } from '@/types/element';
import { buildModelTree } from '../buildModelTree';
import {
  collectAllExpandableIds,
  loadPersistedState,
  savePersistedState,
  seedExpandedToDepth,
  storageKeyFor,
  runtimeOverrideName,
} from '../SessionTreeV2';

import { archetypeForKind } from './testHelpers';

function n(
  id: string,
  name: string | null,
  kind: string,
  children: TreeNode[] = [],
  extra: Partial<TreeNode> = {},
): TreeNode {
  return {
    id,
    name,
    kind,
    archetype: archetypeForKind(kind),
    children,
    ...extra,
  };
}

const URI = 'file:///w.sysml';

function sampleTree() {
  return buildModelTree(
    [
      n('sb', 'ProductionCell', 'PartUsage', [
        n('c1', 'Station1', 'PartUsage', [
          n('groupHead', 'GroupHead', 'PartUsage', [
            n('temp', 'bimetalTemp', 'AttributeUsage'),
          ]),
        ]),
        n('c2', 'Station2', 'PartUsage', [
          n('v', 'temperature', 'AttributeUsage'),
        ]),
      ]),
    ],
    URI,
  );
}

describe('runtimeOverrideName', () => {
  it('converts the owner-qualified tree key to the backend override key', () => {
    expect(runtimeOverrideName('PumpDynamics.ReciprocatingPump.restrictionConductance')).toBe(
      'restrictionConductance',
    );
  });

  it('keeps a bare runtime key unchanged', () => {
    expect(runtimeOverrideName('restrictionConductance')).toBe('restrictionConductance');
  });
});

describe('seedExpandedToDepth', () => {
  it('depth 0 opens only the roots that have children', () => {
    const ids = seedExpandedToDepth(sampleTree(), 0);
    expect(Array.from(ids).sort()).toEqual(['sb']);
  });

  it('depth 1 opens roots + their direct children', () => {
    const ids = seedExpandedToDepth(sampleTree(), 1);
    expect(Array.from(ids).sort()).toEqual(['c1', 'c2', 'sb']);
  });

  it('depth 2 opens 3 levels — the task acceptance condition', () => {
    const ids = seedExpandedToDepth(sampleTree(), 2);
    // sb, c1, c2 have children; groupHead also has children and is at
    // depth 2 so it joins the expanded set.
    expect(Array.from(ids).sort()).toEqual(['c1', 'c2', 'groupHead', 'sb']);
  });

  it('skips leaves (nodes without children) at any depth', () => {
    const ids = seedExpandedToDepth(sampleTree(), 5);
    // `temp`, `temperature` are leaves — no children, so not in the set.
    expect(ids.has('temp')).toBe(false);
    expect(ids.has('temperature')).toBe(false);
  });

  it('empty tree returns an empty set without crashing', () => {
    const ids = seedExpandedToDepth([], 5);
    expect(ids.size).toBe(0);
  });

  it('skips nodes flagged default_collapsed by the backend', () => {
    // Commit 2 of the model-tree rework: backend stamps
    // default_collapsed=true on Port + Connection nodes with
    // children so the FE doesn't bury structural rows under
    // typed-def-inlined fan-out. The seeder must respect that.
    const tree = buildModelTree(
      [
        n(
          'sb',
          'ProductionCell',
          'PartUsage',
          [
            n(
              'p1',
              'phaseIn',
              'PortUsage',
              [n('v', 'temperature', 'AttributeUsage')],
              { default_collapsed: true },
            ),
            n('c1', 'station1', 'PartUsage', [
              n('v2', 'temperature', 'AttributeUsage'),
            ]),
          ],
        ),
      ],
      URI,
    );
    const ids = seedExpandedToDepth(tree, 3);
    // sb + c1 expand; p1 stays collapsed despite having children
    // and being within the depth budget.
    expect(ids.has('sb')).toBe(true);
    expect(ids.has('c1')).toBe(true);
    expect(ids.has('p1')).toBe(false);
  });
});

describe('collectAllExpandableIds', () => {
  it('returns every id whose children > 0 — "Expand all" coverage', () => {
    const ids = collectAllExpandableIds(sampleTree());
    expect(Array.from(ids).sort()).toEqual(['c1', 'c2', 'groupHead', 'sb']);
  });

  it('empty tree returns an empty set', () => {
    expect(collectAllExpandableIds([]).size).toBe(0);
  });
});

describe('localStorage persistence', () => {
  beforeEach(() => {
    if (typeof window !== 'undefined') window.localStorage.clear();
  });

  afterEach(() => {
    if (typeof window !== 'undefined') window.localStorage.clear();
  });

  it('storageKeyFor namespaces by workspace root', () => {
    expect(storageKeyFor('/proj/a')).toBe('sysml.tree.v2./proj/a');
    expect(storageKeyFor(null)).toBeNull();
    expect(storageKeyFor('')).toBeNull();
  });

  it('save then load roundtrips expanded + filterMode', () => {
    savePersistedState('/proj/a', {
      expanded: ['sb', 'c1'],
      filterMode: 'live',
      definitionMode: 'usages',
    });
    const loaded = loadPersistedState('/proj/a');
    expect(loaded).toEqual({
      expanded: ['sb', 'c1'],
      filterMode: 'live',
      definitionMode: 'usages',
      detailHeightPx: undefined,
      groupByPackage: true,
    });
  });

  it('roundtrips definitionMode for any of the three values', () => {
    for (const dm of ['usages', 'definitions', 'both'] as const) {
      savePersistedState('/proj/dm', {
        expanded: [],
        filterMode: 'all',
        definitionMode: dm,
      });
      expect(loadPersistedState('/proj/dm')?.definitionMode).toBe(dm);
    }
  });

  it('load returns null when no state has been written', () => {
    expect(loadPersistedState('/proj/b')).toBeNull();
  });

  it('load returns null when workspace is null (no key)', () => {
    savePersistedState('/proj/a', {
      expanded: ['x'],
      filterMode: 'all',
    });
    expect(loadPersistedState(null)).toBeNull();
  });

  it('load recovers from a corrupt JSON payload without throwing', () => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem('sysml.tree.v2./proj/a', 'not valid json {');
    expect(loadPersistedState('/proj/a')).toBeNull();
  });

  it('load coerces an unknown filterMode to "all" to keep the rest of the state', () => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(
      'sysml.tree.v2./proj/a',
      JSON.stringify({ expanded: ['x'], filterMode: 'bogus' }),
    );
    expect(loadPersistedState('/proj/a')).toEqual({
      expanded: ['x'],
      filterMode: 'all',
      definitionMode: 'usages',
      detailHeightPx: undefined,
      groupByPackage: true,
    });
  });

  it('load coerces an unknown definitionMode to "usages"', () => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(
      'sysml.tree.v2./proj/a',
      JSON.stringify({
        expanded: ['x'],
        filterMode: 'live',
        definitionMode: 'nonsense',
      }),
    );
    expect(loadPersistedState('/proj/a')?.definitionMode).toBe('usages');
  });

  it('save to null workspace is a no-op (does not throw or write)', () => {
    savePersistedState(null, { expanded: ['x'], filterMode: 'all' });
    // No keys touched.
    if (typeof window !== 'undefined') {
      expect(window.localStorage.length).toBe(0);
    }
  });

  it('persists per-workspace — root A does not leak into root B', () => {
    savePersistedState('/proj/a', { expanded: ['a1'], filterMode: 'live' });
    savePersistedState('/proj/b', { expanded: ['b1'], filterMode: 'pinned' });
    expect(loadPersistedState('/proj/a')?.expanded).toEqual(['a1']);
    expect(loadPersistedState('/proj/b')?.expanded).toEqual(['b1']);
  });
});
