/**
 * Pure helpers for PartDetail: constraint health aggregation + key-
 * signal selection.
 *
 * Both functions walk the subtree rooted at a PartTreeNode; keeping
 * them pure means we can unit-test without React and the component
 * layer stays a thin renderer.
 */
import { collectByKind } from '../buildModelTree';
import type { AttributeTreeNode, ModelTreeNode } from '../types';

export type HealthLevel = 'pass' | 'fail' | 'inconclusive' | 'unobserved';

/**
 * Aggregate constraint verdicts under a subtree into one dot colour.
 *
 * Rules (fail dominates):
 *   - any 'fail' / 'error' → 'fail'
 *   - any 'inconclusive'   → 'inconclusive'
 *   - all 'pass'           → 'pass'
 *   - no constraints OR all undefined verdicts → 'unobserved'
 */
export function aggregateHealth(subtree: ModelTreeNode): HealthLevel {
  const constraints = collectByKind([subtree], 'constraint');
  if (constraints.length === 0) return 'unobserved';

  let sawPass = false;
  let sawInconclusive = false;
  for (const c of constraints) {
    if (c.kind !== 'constraint') continue;
    const verdict = c.verdict;
    if (verdict === 'fail' || verdict === 'error') return 'fail';
    if (verdict === 'inconclusive') sawInconclusive = true;
    else if (verdict === 'pass') sawPass = true;
  }
  if (sawInconclusive) return 'inconclusive';
  if (sawPass) return 'pass';
  return 'unobserved';
}

export interface KeySignal {
  node: AttributeTreeNode;
  lastChangedTick: number;
}

/**
 * Pick the top-N most recently changed attribute descendants. Drops
 * attributes that haven't been observed at all (no lastChangedTick)
 * so the list stays actionable. Ties break by depth (shallower first,
 * surface-level signals are more interesting) then by name.
 */
export function pickKeySignals(
  subtree: ModelTreeNode,
  limit = 4,
): KeySignal[] {
  const attrs = collectByKind([subtree], 'attribute') as AttributeTreeNode[];
  const observed = attrs.filter(
    (a) => typeof a.lastChangedTick === 'number',
  );
  observed.sort((a, b) => {
    const tickDiff = (b.lastChangedTick ?? 0) - (a.lastChangedTick ?? 0);
    if (tickDiff !== 0) return tickDiff;
    if (a.depth !== b.depth) return a.depth - b.depth;
    return a.name.localeCompare(b.name);
  });
  return observed
    .slice(0, limit)
    .map((node) => ({ node, lastChangedTick: node.lastChangedTick! }));
}

export interface PartCounts {
  subParts: number;
  attributes: number;
  stateMachines: number;
  constraints: number;
  odes: number;
  calcs: number;
}

export function countArchetypes(subtree: ModelTreeNode): PartCounts {
  return {
    // Subtract 1 for self.
    subParts: Math.max(0, collectByKind([subtree], 'part').length - 1),
    attributes: collectByKind([subtree], 'attribute').length,
    stateMachines: collectByKind([subtree], 'sm').length,
    constraints: collectByKind([subtree], 'constraint').length,
    odes: collectByKind([subtree], 'ode').length,
    calcs: collectByKind([subtree], 'calc').length,
  };
}
