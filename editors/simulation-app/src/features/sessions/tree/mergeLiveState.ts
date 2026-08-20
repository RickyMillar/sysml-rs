/**
 * mergeLiveState — overlay a `NormalizedSnapshot` onto a structural
 * `ModelTreeNode[]`.
 *
 * Split from `buildModelTree` so the structural pass stays pure and
 * cacheable across ticks — the tree shape only needs to rebuild when
 * the workspace / model graph changes, while live-state overlays can
 * rebuild every tick from the same immutable structure.
 *
 * Lookup strategy (names → snapshot keys):
 *   1. Attribute: try `${ownerPath}.${name}` first — the "fully
 *      qualified" form the backend usually emits. Fall back to the
 *      bare `name` so orphan / workspace-level attributes still land.
 *      scalar_vars wins over string_vars when both carry the key.
 *   2. SM: match `subsystems[name]`. Most SMs are workspace-scope
 *      named uniquely, so we don't bother with ownerPath.name yet —
 *      adding it as a fallback is safe once we see collisions in the
 *      wild.
 *   3. Constraint: match `constraint_results[].name` against the
 *      element name. `ConstraintView.verdict` carries the full
 *      four-valued `VerdictKind`, so all of pass / fail / inconclusive
 *      / error reach the tree — the mapping is just a case fold from
 *      the wire's PascalCase to this layer's lowercase vocabulary.
 *      A constraint the run could not decide badges `inconclusive`,
 *      never `fail`.
 */
import type {
  NormalizedSnapshot,
  ConstraintView,
  SubsystemView,
} from '@/features/sessions/sessionLiveStore';
import type {
  ConstraintVerdict,
  VariableValue,
} from '@/features/variables/VariableTree';
import type { AttributeTreeNode, ModelTreeNode } from './types';

/**
 * Per-caller change tracker for `lastChangedTick` stamping. Consumers
 * that want the output/parameter split (Phase-B Task 3) pass a
 * persistent `Map<attributeKey, { value, tick }>` between calls —
 * when the incoming snapshot's value differs from the prior one, the
 * attribute's `lastChangedTick` stamps to the current tick; otherwise
 * it carries forward the previously-stamped tick. First-seen values
 * count as a change.
 *
 * The consumer is responsible for lifecycle: recycle the map on
 * session switch to avoid stale ticks surviving across sessions.
 */
export type ChangeTracker = Map<
  string,
  { value: VariableValue; tick: number }
>;

export interface MergeOptions {
  /** Optional tracker; when supplied `lastChangedTick` is stamped on
   *  every attribute whose value is visible in this snapshot. */
  changeTracker?: ChangeTracker;
}

export function mergeLiveState(
  tree: readonly ModelTreeNode[],
  snapshot: NormalizedSnapshot | null | undefined,
  options: MergeOptions = {},
): ModelTreeNode[] {
  if (!snapshot) {
    // No session yet — consumer still gets a usable tree with
    // undefined live fields so the renderer can show "— K" / blank
    // without special-casing.
    return tree.map((n) => cloneShallow(n));
  }

  // Id-keyed lookups are the correctness fix: a SysML element's
  // `name` is unique only within its containing scope, so nested
  // packages routinely declare two constraints (or two state usages)
  // sharing a short name. Matching by name silently picks one of
  // them at random. The backend now ships `element_id` on every
  // `ConstraintView` / `SubsystemView` it can resolve — prefer that.
  // Name-keyed lookups remain only as a transitional fallback for
  // older cached frames and legacy subsystems with no recorded id.
  const constraintByElementId = new Map<string, ConstraintView>();
  const constraintByName = new Map<string, ConstraintView>();
  for (const c of snapshot.constraint_results) {
    if (c.element_id) constraintByElementId.set(c.element_id, c);
    else constraintByName.set(c.name, c);
  }

  const subsystemsByElementId: Record<string, SubsystemView> = {};
  for (const s of Object.values(snapshot.subsystems)) {
    if (s.element_id) subsystemsByElementId[s.element_id] = s;
  }

  const tracker = options.changeTracker;
  const tick = snapshot.tick;

  const merge = (node: ModelTreeNode): ModelTreeNode => {
    const children = node.children.map(merge);
    switch (node.kind) {
      case 'attribute': {
        const fullPath = node.ownerPath
          ? `${node.ownerPath}.${node.name}`
          : node.name;
        const value = lookupVar(snapshot, fullPath) ?? lookupVar(snapshot, node.name);
        const lastChangedTick = tracker
          ? stampChange(tracker, fullPath, value, tick, node as AttributeTreeNode)
          : (node as AttributeTreeNode).lastChangedTick;
        return { ...node, children, value, lastChangedTick };
      }
      case 'calc': {
        // Option-A ODE detection (Task 4): if the calc's name is
        // carried by scalar_vars / string_vars, an integrator is
        // producing state for it → upgrade to 'ode'. Otherwise keep
        // as 'calc' and render like an attribute row.
        const fullPath = node.ownerPath
          ? `${node.ownerPath}.${node.name}`
          : node.name;
        const value = lookupVar(snapshot, fullPath) ?? lookupVar(snapshot, node.name);
        const lastChangedTick = tracker
          ? stampChange(tracker, fullPath, value, tick, node)
          : node.lastChangedTick;
        if (value !== undefined) {
          return {
            ...node,
            children,
            kind: 'ode',
            value,
          } as ModelTreeNode;
        }
        return { ...node, children, value, lastChangedTick };
      }
      case 'sm': {
        // Prefer id-keyed lookup; fall back to name only when the
        // backend didn't ship an element id for this subsystem.
        const sub =
          subsystemsByElementId[node.elementId] ??
          snapshot.subsystems[node.name];
        return {
          ...node,
          children,
          currentState: sub?.current_state,
          availableTransitions: sub?.available_transitions,
        };
      }
      case 'constraint': {
        // Prefer id-keyed lookup; fall back to name only when the
        // backend didn't ship an element id for this constraint result.
        const result =
          constraintByElementId.get(node.elementId) ??
          constraintByName.get(node.name);
        // Normalised, not verbatim: the wire spells `VerdictKind` in
        // PascalCase (serde derive) while the tree's `ConstraintVerdict`
        // vocabulary is lowercase. An undecidable constraint badges as
        // `inconclusive`, distinct from both a pass and a violation.
        //
        // Guard on the FIELD, not just the row: a row arriving without a
        // verdict is backend/frontend skew, and skew should leave the node
        // unbadged — the same "no determination" state as a constraint the
        // snapshot never carried. Guarding only on `result` would turn skew
        // into a TypeError inside the tree merge, i.e. a React render
        // failure with no diagnosable message attached.
        const verdict: ConstraintVerdict | undefined =
          result?.verdict === undefined
            ? undefined
            : (result.verdict.toLowerCase() as ConstraintVerdict);
        return {
          ...node,
          children,
          verdict,
          // Forward the per-identifier operand map so ConstraintDetail
          // can render the live-value overlay (GAP-CONSTR-002). Leaves
          // the field undefined when the snapshot didn't ship one so
          // the UI can distinguish "not yet evaluated" from "no operands".
          operands: result?.operands,
        };
      }
      case 'part':
      case 'port':
      case 'connection':
      case 'action':
      case 'case':
      case 'ode':
      case 'section':
      case 'other':
        return { ...node, children };
    }
  };

  return tree.map(merge);
}

/**
 * Record / update the change tracker and return the tick this
 * attribute/calc was LAST observed changing. Returns `undefined`
 * when no value is yet available (parameter-bucket candidate).
 */
function stampChange(
  tracker: ChangeTracker,
  key: string,
  value: VariableValue | undefined,
  currentTick: number,
  node: { lastChangedTick?: number },
): number | undefined {
  if (value === undefined) {
    // Attribute not present in this snapshot — preserve whatever
    // tick we stamped on the last observation (if any) so a
    // transiently-missing value doesn't demote it back to
    // "parameter".
    return node.lastChangedTick ?? tracker.get(key)?.tick;
  }
  const prior = tracker.get(key);
  if (!prior) {
    // First-seen — count as a change so the attribute lands in
    // `outputs` at least once.
    tracker.set(key, { value, tick: currentTick });
    return currentTick;
  }
  if (!valuesEqual(prior.value, value)) {
    tracker.set(key, { value, tick: currentTick });
    return currentTick;
  }
  // Unchanged — surface the previous change-tick so the bucket
  // assignment stays stable across ticks.
  return prior.tick;
}

function valuesEqual(a: VariableValue, b: VariableValue): boolean {
  if (a === b) return true;
  if (a == null || b == null) return false;
  if (typeof a !== typeof b) return false;
  if (typeof a === 'object' && typeof b === 'object') {
    // Compare by JSON string — ModelTreeNode values are small
    // scalars / short records, so the cost is negligible.
    try {
      return JSON.stringify(a) === JSON.stringify(b);
    } catch {
      return false;
    }
  }
  return false;
}

function lookupVar(
  snapshot: NormalizedSnapshot,
  key: string,
): number | string | undefined {
  if (key in snapshot.scalar_vars) return snapshot.scalar_vars[key];
  if (key in snapshot.string_vars) return snapshot.string_vars[key];
  return undefined;
}

function cloneShallow(n: ModelTreeNode): ModelTreeNode {
  return { ...n, children: n.children.map(cloneShallow) } as ModelTreeNode;
}
