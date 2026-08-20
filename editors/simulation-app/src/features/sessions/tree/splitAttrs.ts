/**
 * splitAttributesByActivity — partition each part's attribute
 * children into two virtual "section" rows: `Outputs` (values
 * changing in the last N ticks) and `Parameters` (static or
 * never-observed).
 *
 * Runs after mergeLiveState (which stamps `lastChangedTick` on
 * attribute nodes) and after the display filter. Pure —
 * structural-only transformation on the tree; no store reads.
 *
 * Behaviour:
 * - Walks every part node. For its direct children:
 *    * 'attribute' children get bucketed.
 *    * Non-attribute children (sub-parts, SMs, constraints, ODEs,
 *      calcs, other) keep their original position and relative
 *      order after the two section rows.
 * - Insert Outputs + Parameters as the first two children,
 *   skipping either if the bucket is empty. A part with only 2
 *   static attributes just gets a "Parameters (2)" section; no
 *   empty "Outputs (0)".
 * - Attributes with no `lastChangedTick` → parameters. Attributes
 *   whose lastChangedTick is within `staleTicks` of the current
 *   tick → outputs.
 * - Section node ids are synthesised from the parent part id so
 *   they round-trip through expand/collapse cleanly.
 */
import type {
  AttributeTreeNode,
  CalcTreeNode,
  ModelTreeNode,
  SectionTreeNode,
} from './types';

/** Nodes that belong to the outputs/parameters split (attribute-like). */
type SplitCandidate = AttributeTreeNode | CalcTreeNode;

function isSplitCandidate(n: ModelTreeNode): n is SplitCandidate {
  return n.kind === 'attribute' || n.kind === 'calc';
}

export interface SplitOptions {
  /**
   * How long after `lastChangedTick` an attribute stays in the
   * outputs bucket. Default 20. Set to `Infinity` to keep every
   * observed attribute as an output forever; set to 0 to bucket
   * everything as parameters (useful for debug).
   */
  staleTicks?: number;
}

const DEFAULT_STALE_TICKS = 20;

export function splitAttributesByActivity(
  tree: readonly ModelTreeNode[],
  currentTick: number,
  options: SplitOptions = {},
): ModelTreeNode[] {
  const staleTicks = options.staleTicks ?? DEFAULT_STALE_TICKS;

  const walk = (nodes: readonly ModelTreeNode[]): ModelTreeNode[] => {
    return nodes.map((node) => {
      const recursedChildren = walk(node.children);
      if (node.kind !== 'part') {
        return { ...node, children: recursedChildren } as ModelTreeNode;
      }
      // Partition the (already-recursed) children. Attributes AND
      // plain calcs are both "attribute-like" and bucket into
      // Outputs/Parameters. ODE nodes stay as top-level non-attribute
      // siblings so their integrator chip isn't hidden behind the
      // Parameters section by accident.
      const outputs: SplitCandidate[] = [];
      const params: SplitCandidate[] = [];
      const others: ModelTreeNode[] = [];
      for (const c of recursedChildren) {
        if (!isSplitCandidate(c)) {
          others.push(c);
          continue;
        }
        if (
          c.lastChangedTick !== undefined &&
          currentTick - c.lastChangedTick <= staleTicks
        ) {
          outputs.push(c);
        } else {
          params.push(c);
        }
      }

      // If the part has no attribute children at all, don't insert
      // any section headers — nothing to label.
      if (outputs.length === 0 && params.length === 0) {
        return { ...node, children: others } as ModelTreeNode;
      }

      // "Parameters (N)" alone (no outputs) is header clutter: it
      // says the same thing as the part-header itself. Render the
      // parameters flat under the part. The header only earns its
      // keep when it's actually partitioning outputs from parameters.
      if (outputs.length === 0) {
        return {
          ...node,
          children: [...(params as ModelTreeNode[]), ...others],
        } as ModelTreeNode;
      }

      const sections: ModelTreeNode[] = [
        makeSection(node, 'outputs', outputs as ModelTreeNode[]),
      ];
      if (params.length > 0) {
        sections.push(
          makeSection(node, 'parameters', params as ModelTreeNode[]),
        );
      }
      return {
        ...node,
        children: [...sections, ...others],
      } as ModelTreeNode;
    });
  };

  return walk(tree);
}

function makeSection(
  parent: ModelTreeNode,
  kind: 'outputs' | 'parameters',
  children: ModelTreeNode[],
): SectionTreeNode {
  const label = kind === 'outputs' ? 'Outputs' : 'Parameters';
  return {
    id: `${parent.id}__${kind}`,
    // Section is a synthetic grouping with no backing element —
    // mirror its id so consumers reading `elementId` don't get undefined.
    elementId: `${parent.id}__${kind}`,
    uri: parent.uri,
    name: `${label} (${children.length})`,
    rawKind: 'Section',
    kind: 'section',
    depth: parent.depth + 1,
    ownerPath: parent.ownerPath
      ? `${parent.ownerPath}.${parent.name}`
      : parent.name,
    children,
    sectionKind: kind,
    count: children.length,
  };
}

/** Return the synthetic id we'd give the parameters section for a
 *  part — callers use it to seed `expandedSet=false` by default. */
export function parametersSectionId(partId: string): string {
  return `${partId}__parameters`;
}

/** Same for outputs — surfaces the default-open id so consumers
 *  can pre-expand. */
export function outputsSectionId(partId: string): string {
  return `${partId}__outputs`;
}
