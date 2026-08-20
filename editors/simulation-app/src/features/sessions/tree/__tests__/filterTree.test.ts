import { describe, it, expect } from 'vitest';
import type { TreeNode } from '@/types/element';
import { buildModelTree } from '../buildModelTree';
import { mergeLiveState } from '../mergeLiveState';
import {
  filterTree,
  isDefinitionKind,
  isMachineGeneratedName,
  isUnnamedAttributeName,
  isUsageKind,
} from '../filterTree';
import type { ModelTreeNode } from '../types';
import { archetypeForKind } from './testHelpers';

function n(
  id: string,
  name: string | null,
  kind: string,
  children: TreeNode[] = [],
): TreeNode {
  return { id, name, kind, archetype: archetypeForKind(kind), children };
}

const URI = 'file:///w.sysml';

function sample(): ModelTreeNode[] {
  return buildModelTree(
    [
      n('sb', 'ProductionCell', 'PartUsage', [
        n('c1', 'Station1', 'PartUsage', [
          n('temp', 'bimetalTemp', 'AttributeUsage'),
          n('v', 'temperature', 'AttributeUsage'),
          n('cur', 'flow', 'AttributeUsage'),
          n('anon', null, 'AttributeUsage'), // unnamed attribute
          n(
            'uuid',
            'b4a7e6a0-1234-5678-9abc-def012345678',
            'AttributeUsage',
          ),
          n('sm', 'StationStates', 'StateDefinition'),
        ]),
      ]),
    ],
    URI,
  );
}

function namesUnder(tree: readonly ModelTreeNode[]): string[] {
  const out: string[] = [];
  const walk = (nodes: readonly ModelTreeNode[]) => {
    for (const n of nodes) {
      out.push(n.name);
      walk(n.children);
    }
  };
  walk(tree);
  return out;
}

describe('filterTree — unnamed attribute drop', () => {
  it('drops unnamed AttributeUsage + uuid-shaped names by default', () => {
    const out = filterTree(sample(), { mode: 'all', pinnedIds: new Set() });
    const names = namesUnder(out);
    expect(names).toContain('bimetalTemp');
    expect(names).toContain('temperature');
    expect(names).not.toContain('(unnamed)');
    expect(names.find((n) => /^[0-9a-f]{8}-/i.test(n))).toBeUndefined();
  });

  it('keeps unnamed attributes when dropUnnamedAttributes=false', () => {
    const out = filterTree(sample(), {
      mode: 'all',
      pinnedIds: new Set(),
      dropUnnamedAttributes: false,
    });
    const names = namesUnder(out);
    expect(names).toContain('(unnamed)');
  });

  it('promotes children of an unnamed non-attribute parent (transparent container)', () => {
    // Prior behaviour kept the "(unnamed)" part as a visible row;
    // that put gibberish in the tree. Now unnamed parts get promoted
    // so their named descendants rise to the parent's level.
    const tree = buildModelTree(
      [n('sb', 'ProductionCell', 'PartUsage', [
        n('p', null, 'PartUsage', [
          n('a', 'temperature', 'AttributeUsage'),
        ]),
      ])],
      URI,
    );
    const out = filterTree(tree, { mode: 'all', pinnedIds: new Set() });
    expect(namesUnder(out)).toContain('temperature');
    expect(namesUnder(out)).not.toContain('(unnamed)');
  });

  it('keeping unnamed intact via dropUnnamedAttributes:false (debug toggle)', () => {
    const tree = buildModelTree(
      [n('sb', 'ProductionCell', 'PartUsage', [
        n('p', null, 'PartUsage', [
          n('a', 'temperature', 'AttributeUsage'),
        ]),
      ])],
      URI,
    );
    const out = filterTree(tree, {
      mode: 'all',
      pinnedIds: new Set(),
      dropUnnamedAttributes: false,
    });
    expect(namesUnder(out)).toContain('(unnamed)');
  });
});

describe('filterTree — LIVE mode', () => {
  it('keeps only attributes with a live value; ancestor parts preserved', () => {
    const tree = mergeLiveState(sample(), {
      tick: 0,
      time_ms: 0,
      completed: false,
      subsystems: {},
      scalar_vars: {
        'ProductionCell.Station1.temperature': 12.5,
        'ProductionCell.Station1.flow': 0.5,
      },
      string_vars: {},
      constraint_results: [],
    });
    const out = filterTree(tree, { mode: 'live', pinnedIds: new Set() });
    const names = namesUnder(out);
    expect(names).toContain('ProductionCell'); // ancestor kept
    expect(names).toContain('Station1');
    expect(names).toContain('temperature');
    expect(names).toContain('flow');
    expect(names).not.toContain('bimetalTemp'); // no live value
  });

  it('drops an empty subtree entirely when no live values + no other kinds remain', () => {
    const tree = mergeLiveState(sample(), null);
    const out = filterTree(tree, { mode: 'live', pinnedIds: new Set() });
    // SM is a non-attribute, so ProductionCell > Station1 > StationStates stays.
    expect(namesUnder(out)).toContain('StationStates');
    // But none of the attributes survive in live mode when nothing
    // has been observed yet.
    expect(namesUnder(out)).not.toContain('bimetalTemp');
    expect(namesUnder(out)).not.toContain('temperature');
  });
});

describe('filterTree — PINNED mode', () => {
  it('keeps only pinned attributes + their ancestors', () => {
    const out = filterTree(sample(), {
      mode: 'pinned',
      pinnedIds: new Set(['v']),
    });
    const names = namesUnder(out);
    expect(names).toContain('ProductionCell');
    expect(names).toContain('Station1');
    expect(names).toContain('temperature');
    expect(names).not.toContain('bimetalTemp');
    expect(names).not.toContain('flow');
  });

  it('returns an empty tree when nothing is pinned', () => {
    const out = filterTree(sample(), {
      mode: 'pinned',
      pinnedIds: new Set(),
    });
    // Non-attribute nodes (SM) survive the attribute-only chip filter
    // — they're not "pinned" but the chip filter never demands they be.
    expect(namesUnder(out)).toContain('StationStates');
    // Attributes are all filtered out.
    expect(namesUnder(out)).not.toContain('temperature');
  });
});

describe('filterTree — search', () => {
  it('keeps matching nodes + ancestors (case-insensitive)', () => {
    const out = filterTree(sample(), {
      mode: 'all',
      pinnedIds: new Set(),
      searchQuery: 'TEMPERATURE',
    });
    const names = namesUnder(out);
    expect(names).toContain('ProductionCell');
    expect(names).toContain('Station1');
    expect(names).toContain('temperature');
    expect(names).not.toContain('bimetalTemp');
    expect(names).not.toContain('flow');
  });

  it('matches ownerPath segments too', () => {
    const out = filterTree(sample(), {
      mode: 'all',
      pinnedIds: new Set(),
      searchQuery: 'station1',
    });
    // Everything under Station1 matches because their ownerPath contains 'Station1'.
    expect(namesUnder(out)).toContain('Station1');
    expect(namesUnder(out)).toContain('temperature');
    expect(namesUnder(out)).toContain('bimetalTemp');
  });

  it('returns an empty tree when nothing matches', () => {
    const out = filterTree(sample(), {
      mode: 'all',
      pinnedIds: new Set(),
      searchQuery: 'zzzzzz',
    });
    expect(out).toEqual([]);
  });
});

describe('isMachineGeneratedName', () => {
  it('flags empty + (unnamed) + full uuid + anon_', () => {
    expect(isMachineGeneratedName('')).toBe(true);
    expect(isMachineGeneratedName('(unnamed)')).toBe(true);
    expect(
      isMachineGeneratedName('b4a7e6a0-1234-5678-9abc-def012345678'),
    ).toBe(true);
    expect(isMachineGeneratedName('anon_5e5abc')).toBe(true);
    expect(isMachineGeneratedName('ANON-5E5ABC')).toBe(true);
  });

  it('flags names with an embedded full UUID (authored-prefix + uuid tail)', () => {
    // User-reported: "lighting" gets a uuid tail appended somewhere,
    // so "li..." + uuid shows in the tree. Dropping these fully.
    expect(
      isMachineGeneratedName(
        'lighting_b4a7e6a0-1234-5678-9abc-def012345678',
      ),
    ).toBe(true);
    expect(
      isMachineGeneratedName('b4a7e6a0-1234-5678-9abc-def012345678-suffix'),
    ).toBe(true);
  });

  it('flags dual hex-run shapes like "abc12345-def67890"', () => {
    expect(isMachineGeneratedName('abc12345-def67890')).toBe(true);
    expect(isMachineGeneratedName('lighting-abc12345-def67890')).toBe(true);
    // Three-letter hex runs don't qualify — too short to be a UUID.
    expect(isMachineGeneratedName('foo-bar')).toBe(false);
  });

  it('flags long unbroken hex runs (16+ chars) — id-as-name fallback', () => {
    expect(isMachineGeneratedName('abc1234567890def')).toBe(true);
    expect(isMachineGeneratedName('feed1234deadbeef')).toBe(true);
    // A short hex-looking string is fine — "abcdef" is a valid name.
    expect(isMachineGeneratedName('abcdef')).toBe(false);
  });

  it('keeps real human names intact', () => {
    expect(isMachineGeneratedName('temperature')).toBe(false);
    expect(isMachineGeneratedName('bimetalTemp')).toBe(false);
    expect(isMachineGeneratedName('Q_circuits')).toBe(false);
    expect(isMachineGeneratedName('ThermalProtectionModel')).toBe(false);
    // Even names with a digit sequence are fine — "feed1" is a valid
    // identifier, just 5 hex chars.
    expect(isMachineGeneratedName('sensor12')).toBe(false);
    expect(isMachineGeneratedName('phase1Voltage')).toBe(false);
  });

  it('back-compat: isUnnamedAttributeName aliases isMachineGeneratedName', () => {
    expect(isUnnamedAttributeName('(unnamed)')).toBe(true);
    expect(isUnnamedAttributeName('temperature')).toBe(false);
  });
});

describe('filterTree — machine-generated name drop (all kinds)', () => {
  it('drops a Part whose name is a uuid, preserves its named descendants', () => {
    const tree = buildModelTree(
      [
        n('p1', 'b4a7e6a0-1234-5678-9abc-def012345678', 'PartUsage', [
          n('a1', 'temperature', 'AttributeUsage'),
        ]),
      ],
      URI,
    );
    const out = filterTree(tree, { mode: 'all', pinnedIds: new Set() });
    const names = namesUnder(out);
    // The uuid-named part itself is gone; its named attribute promotes.
    expect(names.filter((n) => /[0-9a-f]{8}-/i.test(n))).toEqual([]);
    expect(names).toContain('temperature');
  });

  it('drops a constraint whose name is machine-generated', () => {
    const tree = buildModelTree(
      [
        n('p', 'ProductionCell', 'PartUsage', [
          n('c', 'abc12345-def67890', 'ConstraintUsage'),
          n('a', 'temperature', 'AttributeUsage'),
        ]),
      ],
      URI,
    );
    const out = filterTree(tree, { mode: 'all', pinnedIds: new Set() });
    const names = namesUnder(out);
    expect(names).toContain('temperature');
    expect(names).not.toContain('abc12345-def67890');
  });
});

describe('filterTree — Definition / Usage mode', () => {
  function defVsUsageTree() {
    // PascalCase definition paired with camelCase usage — the common
    // SysML idiom (class `GroupHead`, instance `groupHead: GroupHead`). The usage
    // carries `typed_as` pointing at the definition id; that's the
    // authoritative link the backend (sysml-core `find_feature_type`)
    // produces and the only mechanism this filter consults.
    return buildModelTree(
      [
        {
          id: 'sbDef',
          name: 'ProductionCell',
          kind: 'PartDefinition',
          archetype: 'part',
          children: [
            { id: 'breakerDef', name: 'GroupHead', kind: 'PartDefinition', archetype: 'part', children: [] },
            { id: 'ormDef', name: 'Orphan', kind: 'PartDefinition', archetype: 'part', children: [] },
            {
              id: 'breakerInst',
              name: 'groupHead',
              kind: 'PartUsage',
              archetype: 'part',
              typed_as: 'breakerDef',
              children: [
                { id: 'temp', name: 'bimetalTemp', kind: 'AttributeUsage', archetype: 'attribute', children: [] },
              ],
            },
          ],
        },
      ],
      URI,
    );
  }

  it('usages (default) drops GroupHead def when groupHead usage exists, keeps Orphan def', () => {
    const out = filterTree(defVsUsageTree(), {
      mode: 'all',
      pinnedIds: new Set(),
    });
    const names = namesUnder(out);
    // GroupHead def paired with groupHead usage — dropped.
    expect(names).not.toContain('GroupHead');
    // Orphan has no usage — kept.
    expect(names).toContain('Orphan');
    expect(names).toContain('groupHead');
    expect(names).toContain('bimetalTemp');
    // ProductionCell def is the entry point — also kept (no `production cell`
    // usage in the tree).
    expect(names).toContain('ProductionCell');
  });

  it('definitions mode drops all *Usage rows — spec-first view', () => {
    const out = filterTree(defVsUsageTree(), {
      mode: 'all',
      pinnedIds: new Set(),
      definitionMode: 'definitions',
    });
    const names = namesUnder(out);
    expect(names).toContain('GroupHead');
    expect(names).toContain('Orphan');
    expect(names).toContain('ProductionCell');
    // groupHead usage + its attribute gone.
    expect(names).not.toContain('groupHead');
    expect(names).not.toContain('bimetalTemp');
  });

  it('both mode keeps everything — def + usage visible together', () => {
    const out = filterTree(defVsUsageTree(), {
      mode: 'all',
      pinnedIds: new Set(),
      definitionMode: 'both',
    });
    const names = namesUnder(out);
    expect(names).toContain('GroupHead');
    expect(names).toContain('groupHead');
    expect(names).toContain('Orphan');
    expect(names).toContain('bimetalTemp');
  });

  it('id-based typedAs drop catches defs the name heuristic misses', () => {
    // Exactly the espresso-production-cell pattern: `part thermalModel :
    // ThermalProtectionModel` — the usage name (`thermalModel`) bears no
    // relation to the definition name (`ThermalProtectionModel`). Only
    // the id-based `typedAs` link can drop the def cleanly.
    const defId = 'def-groupHead-thermal';
    const tree = buildModelTree(
      [
        {
          id: defId,
          name: 'ThermalProtectionModel',
          kind: 'PartDefinition',
          archetype: 'part',
          children: [],
        },
        {
          id: 'use-thermal',
          name: 'thermalModel',
          kind: 'PartUsage',
          archetype: 'part',
          typed_as: defId,
          children: [],
        },
      ],
      URI,
    );
    const out = filterTree(tree, { mode: 'all', pinnedIds: new Set() });
    const names = namesUnder(out);
    // Definition hidden because a usage points at it; usage itself
    // renders as normal.
    expect(names).not.toContain('ThermalProtectionModel');
    expect(names).toContain('thermalModel');
  });

  it('drops the entire def subtree in usages mode — children of a hidden def must not bob back up via ancestor preservation', () => {
    // Reproduction of the espresso-production-cell regression: a
    // `HallEffectSensor` PartDefinition has its OWN attribute
    // children (`sensitivity`, `range_low`, `range_high`). A
    // `hallSensor` PartUsage somewhere in the workspace types
    // against it. In usages mode the def must vanish entirely —
    // ancestor preservation (which keeps a parent when any child
    // matches the chip / search) used to resurrect the def via its
    // surviving attribute children.
    const defId = 'def-hall';
    const tree = buildModelTree(
      [
        {
          id: defId,
          name: 'HallEffectSensor',
          kind: 'PartDefinition',
          archetype: 'part',
          children: [
            {
              id: 'a-sens',
              name: 'sensitivity',
              kind: 'AttributeUsage',
              archetype: 'attribute',
              children: [],
            },
            {
              id: 'a-rl',
              name: 'range_low',
              kind: 'AttributeUsage',
              archetype: 'attribute',
              children: [],
            },
          ],
        },
        {
          id: 'station-use',
          name: 'station1',
          kind: 'PartUsage',
          archetype: 'part',
          children: [
            {
              id: 'use-hs',
              name: 'hallSensor',
              kind: 'PartUsage',
              archetype: 'part',
              typed_as: defId,
              children: [],
            },
          ],
        },
      ],
      URI,
    );
    const out = filterTree(tree, { mode: 'all', pinnedIds: new Set() });
    const names = namesUnder(out);
    expect(names).not.toContain('HallEffectSensor');
    // The def's own attributes go away with it — they belong to the
    // hidden def, not to the workspace root.
    expect(names).not.toContain('sensitivity');
    expect(names).not.toContain('range_low');
    // Usage stays; it's the visible representative of the type.
    expect(names).toContain('hallSensor');
  });

  it('defs with no typed_as link stay visible in usages mode — name overlap alone does not dedupe', () => {
    // Backend-first: if the usage doesn't carry a resolved `typed_as`,
    // the filter keeps the def. A name-matching camelCase usage is
    // NOT enough — that would mask a real backend gap
    // (`find_feature_type` failing to resolve the typing).
    const tree = buildModelTree(
      [
        n('def', 'Router', 'PartDefinition'),
        n('use', 'router', 'PartUsage'), // no typed_as wired
        n('def2', 'Exact', 'PartDefinition'),
        n('use2', 'Exact', 'PartUsage'), // no typed_as wired
      ],
      URI,
    );
    const out = filterTree(tree, { mode: 'all', pinnedIds: new Set() });
    const names = namesUnder(out);
    expect(names).toContain('Router');
    expect(names).toContain('router');
    // Both "Exact" def and "Exact" usage survive — no typed_as, no drop.
    expect(names.filter((n) => n === 'Exact')).toHaveLength(2);
  });
});

describe('isDefinitionKind / isUsageKind', () => {
  it('classifies raw kinds by suffix', () => {
    expect(isDefinitionKind('PartDefinition')).toBe(true);
    expect(isDefinitionKind('AttributeDefinition')).toBe(true);
    expect(isDefinitionKind('PartUsage')).toBe(false);
    expect(isUsageKind('PartUsage')).toBe(true);
    expect(isUsageKind('AttributeUsage')).toBe(true);
    expect(isUsageKind('PartDefinition')).toBe(false);
    // Neither — structural chrome (already pruned by build, but the
    // helpers answer honestly for anything).
    expect(isDefinitionKind('Package')).toBe(false);
    expect(isUsageKind('Package')).toBe(false);
  });
});
