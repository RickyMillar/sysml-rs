/**
 * flattenAttributePhysics — flatten the per-file `TreeNode[]` into a
 * dotted-path → physics-info map for the Variables pane (R3.1).
 *
 * Backend now projects `unit` and `isq_dimension` onto every
 * AttributeUsage TreeNode (see `crates/tooling/sysml-service/src/query.rs`
 * `attribute_physics_info`). The Variables pane keys variables by their
 * dotted path (`circuit1.T_busbar`), but the model tree is hierarchical
 * with short names per node — this helper joins ancestor short names
 * with `.` so the lookup composes.
 *
 * Behaviour:
 *   - Only `AttributeUsage` (kind === 'AttributeUsage') nodes contribute
 *     entries; structural containers (parts, packages) are walked but
 *     not recorded.
 *   - Nodes whose `name` is null/empty are skipped — there is no
 *     legitimate dotted path that lands on an unnamed attribute.
 *   - Nodes without `unit` *and* without `isq_dimension` are skipped
 *     entirely; entering them with both fields undefined would just be
 *     dead weight in the lookup map.
 *   - Cross-file callers can `Map.set`-merge multiple flatten results
 *     because the dotted path keys are unique per file (root attributes
 *     sit at `name`, nested at `parent.name`).
 */
import type { TreeNode } from '@/types/element';

export interface AttributePhysicsInfo {
  unit?: string;
  isq_dimension?: string;
}

/**
 * Walk an array of root `TreeNode`s and emit a `Map` from dotted path
 * to `AttributePhysicsInfo` for every `AttributeUsage` node carrying
 * backend-projected unit / ISQ data.
 */
export function flattenAttributePhysics(
  trees: ReadonlyArray<TreeNode>,
): Map<string, AttributePhysicsInfo> {
  const out = new Map<string, AttributePhysicsInfo>();

  const walk = (node: TreeNode, parents: ReadonlyArray<string>): void => {
    const name = node.name ?? '';
    const path = name.length > 0
      ? (parents.length === 0 ? name : `${parents.join('.')}.${name}`)
      : '';

    if (
      node.kind === 'AttributeUsage'
      && path.length > 0
      && (node.unit !== undefined || node.isq_dimension !== undefined)
    ) {
      out.set(path, {
        unit: node.unit,
        isq_dimension: node.isq_dimension,
      });
    }

    // Recurse into children. We always extend the parent path, including
    // through structural containers (parts, packages), so an attribute
    // like `circuit1.busbar.temperature` lands at the right key even
    // though the intermediate `busbar` is a PartUsage, not an attribute.
    if (node.children && node.children.length > 0) {
      const nextParents = name.length > 0 ? [...parents, name] : parents;
      for (const child of node.children) {
        walk(child, nextParents);
      }
    }
  };

  for (const root of trees) {
    walk(root, []);
  }
  return out;
}
