/**
 * Pure builders for the Phase B session tree.
 *
 * `classifyTreeNode` reads the backend-projected `archetype` field
 * (R2.4) and applies one upgrade — `calc → ode` when the backend's
 * `is_ode` flag is set. The static `KIND_MAP` dictionary that lived
 * here previously is gone; the backend's `classify_archetype` helper
 * is now the single source of truth (the SysML v2 type hierarchy
 * was already encoded there, the FE was just duplicating it).
 *
 * `buildModelTree` walks the per-file `TreeNode[]` returned by
 * `/models/{uri}/tree?view=user_facing`, tags every element, and emits
 * `ModelTreeNode`s that the live-merge layer can decorate with per-node
 * values without having to understand the model taxonomy again.
 *
 * R2.1 of the backend-first cleansing audit: noise filtering (the old
 * `PRUNE_KINDS` set + `dropEmptyOther` heuristic) lives server-side in
 * `is_user_facing_noise` (`crates/tooling/sysml-service/src/query.rs`).
 *
 * R3.2 + R3.4 of the backend-first cleansing audit: sibling
 * (name, kind) dedupe and archetype-rank sort live server-side in
 * `build_tree_node` and `model_tree_with_resolver`
 * (`crates/tooling/sysml-service/src/query.rs`). The frontend trusts
 * the backend's order verbatim and does only display-layer work here:
 * name fallback for anonymous rows and the calc → ode upgrade. The
 * single exception is `sortSiblingsByKind`, which the cross-file merge
 * (`useSessionModelTree`) uses to interleave roots from different
 * files into one ordered list — see its doc comment.
 *
 * Purely structural — no session or live data is consulted here. Tests
 * pin the kind mapping; the consumer (Phase B2 hook / Phase B1
 * renderer) feeds live snapshots in afterwards.
 */

import type { TreeNode } from '@/types/element';
import type {
  ModelTreeNode,
  ModelTreeNodeKind,
  SmStateDescriptor,
  SmTransitionDescriptor,
} from './types';

/**
 * Map a backend TreeNode to the FE's tree archetype.
 *
 * R2.4 of the backend-first cleansing audit lifted the static
 * `KIND_MAP` dictionary into a backend `Archetype` projection
 * (`crates/tooling/sysml-service/src/query.rs::classify_archetype`):
 * the SysML v2 type hierarchy already encodes which kinds are parts /
 * attributes / SMs / constraints / calcs, so the FE was duplicating
 * that. The backend now stamps each TreeNode with the right
 * archetype using `is_subtype_of` family checks (catching
 * RequirementUsage / EnumerationUsage / ExhibitStateUsage etc.
 * without enumerating them).
 *
 * The only piece left in the FE is the calc → ode upgrade based on
 * the existing `is_ode` flag, since `is_ode` is per-node info the
 * backend already carries and we don't want a separate
 * `Archetype::Ode` enum variant rippling through every other
 * consumer.
 */
export function classifyTreeNode(node: TreeNode): ModelTreeNodeKind {
  if (node.archetype === 'calc' && node.is_ode === true) return 'ode';
  return node.archetype;
}

/**
 * Sort siblings by archetype rank without deduping. The ONLY remaining
 * FE-side reordering (R3.4): the cross-file merge in
 * `useSessionModelTree` (when `groupByPackage = false`) concatenates
 * each file's already-sorted root list into one merged Vec — which
 * lands a structurally-sorted block per file but interleaves wrong
 * across files (file A's attributes precede file B's parts). This
 * function applies one final stable sort to interleave them
 * correctly. Within-file ordering is fully backend-authoritative
 * (`build_tree_node` + `model_tree_with_resolver` apply the
 * (name, kind) dedupe + archetype-rank sort server-side); the FE
 * never re-sorts a single file's tree.
 *
 * The rank table here mirrors `Archetype::sort_rank` in
 * `crates/tooling/sysml-service/src/types.rs`. `'ode'` is FE-derived
 * (calc + is_ode) and shares rank 3 with `'calc'`, since the backend
 * doesn't distinguish them as separate archetypes today.
 */
const FE_KIND_RANK: Readonly<Record<ModelTreeNodeKind, number>> = {
  part: 0,
  port: 1,
  sm: 2,
  action: 3,
  case: 4,
  constraint: 5,
  ode: 6,
  calc: 6,
  attribute: 7,
  connection: 8,
  section: 9,
  other: 10,
};

export function sortSiblingsByKind(
  children: readonly ModelTreeNode[],
): ModelTreeNode[] {
  return [...children].sort(
    (a, b) => FE_KIND_RANK[a.kind] - FE_KIND_RANK[b.kind],
  );
}

export interface BuildOptions {
  /**
   * Prune packages (and their children are promoted to the parent's
   * level — i.e. the package becomes transparent). Default `true`
   * because packages are pure namespacing and don't belong in a
   * Run-page tree.
   *
   * `flattenPackages` is a user-facing display setting (the Group-by-
   * package toggle) — explicitly out of scope for the R2.1 cleansing
   * audit, kept FE-side.
   */
  flattenPackages?: boolean;
}

export function buildModelTree(
  nodes: readonly TreeNode[],
  uri: string,
  options: BuildOptions = {},
): ModelTreeNode[] {
  const { flattenPackages = true } = options;

  const build = (
    node: TreeNode,
    depth: number,
    ownerPath: string,
  ): ModelTreeNode[] => {
    // R2.1: noise kinds (membership edges, type bindings, expression
    // AST, ports/flows, transitions, etc.) are dropped server-side
    // when the FE asks for `?view=user_facing`. The FE no longer
    // re-enumerates `PRUNE_KINDS`. Backend keeps `name: null` on
    // anonymous nodes; we substitute `(unnamed)` as a pure display
    // concern.
    const name = node.name ?? '(unnamed)';

    if (flattenPackages && node.kind === 'Package') {
      // Treat the package as transparent: its children slot into the
      // current depth / ownerPath rather than nesting under a node
      // whose only contribution is namespacing.
      return node.children.flatMap((c) => build(c, depth, ownerPath));
    }

    // Backend-projected archetype (R2.4) — `classifyTreeNode` reads
    // the projection directly and upgrades `calc → ode` when the
    // backend's `is_ode` flag is set (GAP-ODE-001 / `GetDerivative`
    // subsetting). `mergeLiveState` still has the runtime
    // scalar_vars heuristic as a safety net for trees that don't
    // carry the flag.
    const kind = classifyTreeNode(node);
    const childOwner = ownerPath ? `${ownerPath}.${name}` : name;
    // R3.2 + R3.4: backend is authoritative for sibling order and
    // (name, kind) dedupe — `build_tree_node` applies both server-
    // side. The FE walks children in the order it receives them.
    const children = node.children.flatMap((c) =>
      build(c, depth + 1, childOwner),
    );

    const base = {
      id: node.id,
      // Backend's dedupe pass rewrites `id` when the same element
      // surfaces at multiple tree positions (typed-def inlining);
      // `element_id` carries the real id for backend lookups. Falls
      // back to `id` for direct children where no rewrite happened.
      elementId: node.element_id ?? node.id,
      // Per-node file attribution (backend-stamped from the element's
      // span) beats the per-response uri: the workspace-scoped tree
      // has no meaningful response uri at all, and even per-file
      // trees inline typed-def children declared in other files.
      uri: node.source_uri ?? uri,
      name,
      rawKind: node.kind,
      depth,
      ownerPath,
      children,
      // Backend resolves the usage → definition typing once, via
      // sysml-core's `find_feature_type` index; we just carry the
      // id through. Absent for definitions and untypable kinds.
      typedAs: node.typed_as,
      // Backend default-collapse hint (commit 2 of the model-tree
      // rework). Set true for Port / Connection nodes with children
      // so the FE doesn't bury structural rows under heavy typed-def
      // inlining fan-out. Mapped from snake_case → camelCase here.
      defaultCollapsed: node.default_collapsed,
    } as const;

    // Discriminate — every branch constructs the right variant of the
    // union. Live-state fields (value, currentState, verdict, …) are
    // deliberately absent; the merge step fills them in.
    switch (kind) {
      case 'part':
        return [{ ...base, kind: 'part' }];
      case 'port':
        return [{ ...base, kind: 'port' }];
      case 'connection':
        return [{ ...base, kind: 'connection' }];
      case 'attribute': {
        // R3.3: backend-projected bound markers come in as
        // `constraint_name` (snake_case wire format); rename to
        // `constraintName` so the rest of the FE keeps its
        // camelCase convention. Empty array stays absent so
        // downstream `bounds?` typing remains exact.
        const bounds = node.bounds?.map((b) => ({
          y: b.y,
          kind: b.kind,
          constraintName: b.constraint_name,
          operator: b.operator,
        }));
        return [{ ...base, kind: 'attribute', bounds }];
      }
      case 'sm': {
        // Snapshot static SM topology. States come from the SM's
        // StateUsage children; transitions are read from the
        // backend-projected `transitions` field on the SM TreeNode
        // (R2.1 fixup) — works in both `view=user_facing` and
        // `view=full`.
        const { states, transitions } = extractSmTopology(node);
        return [{ ...base, kind: 'sm', states, transitions }];
      }
      case 'action':
        return [{ ...base, kind: 'action' }];
      case 'case':
        return [{ ...base, kind: 'case' }];
      case 'constraint':
        return [{ ...base, kind: 'constraint' }];
      case 'ode':
        return [{ ...base, kind: 'ode' }];
      case 'calc':
        return [{ ...base, kind: 'calc' }];
      case 'section':
        // Structural build doesn't emit sections — they come from
        // splitAttrs later. Treat this branch as unreachable but
        // keep it in the switch for exhaustiveness.
        return [{ ...base, kind: 'other' }];
      default:
        return [{ ...base, kind: 'other' }];
    }
  };

  // R3.2 + R3.4: roots are dedupe + archetype-sorted server-side in
  // `model_tree_with_resolver`. The FE preserves backend order.
  return nodes.flatMap((n) => build(n, 0, ''));
}

/**
 * Walk every node in a tree in DFS order. Consumers that want a flat
 * list (search, pin registry, status bar counts) use this — cheaper
 * than the consumer building its own traversal.
 */
export function walkModelTree(
  tree: readonly ModelTreeNode[],
  visit: (node: ModelTreeNode, parent: ModelTreeNode | null) => void,
): void {
  const stack: Array<{ node: ModelTreeNode; parent: ModelTreeNode | null }> = [];
  for (let i = tree.length - 1; i >= 0; i--) {
    stack.push({ node: tree[i], parent: null });
  }
  while (stack.length > 0) {
    const { node, parent } = stack.pop()!;
    visit(node, parent);
    for (let i = node.children.length - 1; i >= 0; i--) {
      stack.push({ node: node.children[i], parent: node });
    }
  }
}

/**
 * Scan a raw SM element for its static topology — the StateUsage
 * children (states) + the backend-projected `transitions` field on
 * the SM TreeNode itself.
 *
 * As of R2.1 of the backend-first cleansing audit, the
 * `user_facing` model-tree view filters out `TransitionUsage`
 * children. To keep the static state-graph SVG renderable, the
 * backend now projects an `SmTransitionDescriptor` list directly
 * onto every SM TreeNode (`StateDefinition` / `StateUsage` /
 * `ExhibitStateUsage`). This function reads from that list rather
 * than walking children.
 *
 * Transition source / target come from the same server-side
 * resolution that powers the per-TransitionUsage `source` /
 * `target` fields (R2.2): `Value::Ref` → owning element name, with
 * `unresolved_source` / `unresolved_target` as a fallback.
 */
export function extractSmTopology(smNode: TreeNode): {
  states: SmStateDescriptor[];
  transitions: SmTransitionDescriptor[];
} {
  const states: SmStateDescriptor[] = [];
  for (const child of smNode.children) {
    if (child.kind === 'StateUsage' || child.kind === 'ExhibitStateUsage') {
      if (child.name) {
        states.push({ id: child.id, name: child.name });
      }
    }
  }
  const transitions: SmTransitionDescriptor[] = (smNode.transitions ?? []).map(
    (t) => ({
      id: t.id,
      name: t.name && t.name.length > 0 ? t.name : '(unnamed)',
      source: t.source,
      target: t.target,
    }),
  );
  return { states, transitions };
}

/** Convenience: gather every node whose kind matches the predicate. */
export function collectByKind(
  tree: readonly ModelTreeNode[],
  kind: ModelTreeNodeKind,
): ModelTreeNode[] {
  const out: ModelTreeNode[] = [];
  walkModelTree(tree, (n) => {
    if (n.kind === kind) out.push(n);
  });
  return out;
}

/**
 * Find the ancestor chain (root → target, inclusive) for a given id.
 * Returns `null` if the id isn't in the tree. Used by the Run page to
 * turn a click on any node into the `focusPath: ElementId[]` the store
 * + breadcrumb expect.
 */
export function findPathToNode(
  tree: readonly ModelTreeNode[],
  targetId: string,
): ModelTreeNode[] | null {
  for (const node of tree) {
    if (node.id === targetId) return [node];
    const inner = findPathToNode(node.children, targetId);
    if (inner) return [node, ...inner];
  }
  return null;
}

/**
 * Resolve a `focusPath: string[]` back to its corresponding node
 * chain. Returns a partial chain if the path goes stale (a node id
 * was removed) so the UI still shows what it can.
 */
export function resolveFocusPath(
  tree: readonly ModelTreeNode[],
  focusPath: readonly string[],
): ModelTreeNode[] {
  const out: ModelTreeNode[] = [];
  let cursor: readonly ModelTreeNode[] = tree;
  for (const id of focusPath) {
    const next = cursor.find((n) => n.id === id);
    if (!next) break;
    out.push(next);
    cursor = next.children;
  }
  return out;
}
