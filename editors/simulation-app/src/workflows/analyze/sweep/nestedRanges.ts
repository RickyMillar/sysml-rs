/**
 * nestedRanges — pure helper for sweep param range expansion (R5.4).
 *
 * The sweep workflow lets a user sweep one parameter across a set of
 * discrete values. A "nested" range is a range whose outer iterator is
 * itself another range: for each outer value, the inner range runs its
 * full sweep. The cartesian product honours parent → child containment.
 *
 * Example:
 *   outer  temperature ∈ [20, 40]
 *   inner  voltage     ∈ [10, 12, 14], parent = "temperature"
 *
 *   generateNestedChildrenParams([outer, inner])
 *   → 2 × 3 = 6 children:
 *     { temperature: 20, voltage: 10 }
 *     { temperature: 20, voltage: 12 }
 *     { temperature: 20, voltage: 14 }
 *     { temperature: 40, voltage: 10 }
 *     { temperature: 40, voltage: 12 }
 *     { temperature: 40, voltage: 14 }
 *
 * Boundaries:
 *  - Pure. No React, no side-effects, no backend calls. Trivially unit-testable.
 *  - Does NOT build `ChildDescriptor`s. It returns the `params` map only.
 *    Wrapping into descriptors (ids, status, session_id) is the caller's
 *    job — typically Agent BB's `useSweepConfig.ts`.
 *  - Deterministic ordering: parents fully qualify before any descendant
 *    advances. The iteration order of a parent's values is the order
 *    given in `values`, not sorted. This keeps the UI row order stable.
 */

/**
 * A single range entry. `parent` names another range in the same list
 * that the iteration nests inside. A range with no `parent` is a root.
 * Multiple roots are supported; every root is expanded independently and
 * their cartesian product is concatenated.
 */
export interface NestedRange {
  /** Unique name of the sweep parameter. Used as key in the resulting `params` map. */
  param: string;
  /** Discrete values the parameter will take. */
  values: number[];
  /** Optional name of the parent range this range nests inside. */
  parent?: string;
}

/** Error thrown when the input graph is malformed (cycle or missing parent). */
export class NestedRangeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'NestedRangeError';
  }
}

/**
 * Expand nested ranges into the cartesian product of their values,
 * honouring the parent → child nesting order.
 *
 * @param ranges  List of {@link NestedRange} entries. Order does not
 *                matter — the function builds a dependency ordering from
 *                the `parent` links.
 * @returns       A list of `Record<param, value>` entries. Empty when
 *                any range has zero values or when `ranges` itself is
 *                empty. The identity element of a cartesian product is
 *                the empty tuple, but the sweep workflow treats "no
 *                params" as "no children", which is what the callers
 *                want.
 */
export function generateNestedChildrenParams(
  ranges: NestedRange[],
): Array<Record<string, number>> {
  if (ranges.length === 0) return [];

  // Fast-path: any range with empty values → cartesian product is empty.
  if (ranges.some((r) => r.values.length === 0)) return [];

  // Validate names are unique — duplicates make parent references ambiguous.
  const byName = new Map<string, NestedRange>();
  for (const r of ranges) {
    if (byName.has(r.param)) {
      throw new NestedRangeError(
        `duplicate range param name: "${r.param}"`,
      );
    }
    byName.set(r.param, r);
  }

  // Validate every `parent` points at a known range and detect cycles.
  for (const r of ranges) {
    if (r.parent !== undefined) {
      if (!byName.has(r.parent)) {
        throw new NestedRangeError(
          `range "${r.param}" references unknown parent "${r.parent}"`,
        );
      }
    }
  }
  detectCycles(ranges, byName);

  // Topological order: parents before children. Roots come first, and
  // within a level the order follows the input order.
  const ordered = topoSort(ranges, byName);

  // Cartesian product honouring the order. Because nesting is strict (a
  // child's iteration is contained inside a parent's iteration), the
  // product reduces to a plain cartesian product of the ordered ranges.
  // The `parent` relationship only constrains position in `ordered`;
  // product semantics do not change.
  let acc: Array<Record<string, number>> = [{}];
  for (const r of ordered) {
    const next: Array<Record<string, number>> = [];
    for (const row of acc) {
      for (const v of r.values) {
        next.push({ ...row, [r.param]: v });
      }
    }
    acc = next;
  }
  return acc;
}

// ── internal helpers ────────────────────────────────────────────────

function detectCycles(
  ranges: NestedRange[],
  byName: Map<string, NestedRange>,
): void {
  // Standard white/grey/black DFS.
  const WHITE = 0, GREY = 1, BLACK = 2;
  const colour = new Map<string, number>();
  for (const r of ranges) colour.set(r.param, WHITE);

  function visit(name: string, trail: string[]): void {
    const c = colour.get(name);
    if (c === BLACK) return;
    if (c === GREY) {
      throw new NestedRangeError(
        `cycle detected in range parent chain: ${[...trail, name].join(' → ')}`,
      );
    }
    colour.set(name, GREY);
    const r = byName.get(name)!;
    if (r.parent !== undefined) {
      visit(r.parent, [...trail, name]);
    }
    colour.set(name, BLACK);
  }

  for (const r of ranges) visit(r.param, []);
}

function topoSort(
  ranges: NestedRange[],
  byName: Map<string, NestedRange>,
): NestedRange[] {
  // DFS post-order: parents first.
  const visited = new Set<string>();
  const out: NestedRange[] = [];

  function visit(r: NestedRange) {
    if (visited.has(r.param)) return;
    if (r.parent !== undefined) {
      visit(byName.get(r.parent)!);
    }
    visited.add(r.param);
    out.push(r);
  }

  for (const r of ranges) visit(r);
  return out;
}
