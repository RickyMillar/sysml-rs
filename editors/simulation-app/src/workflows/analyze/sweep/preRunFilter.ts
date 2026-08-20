/**
 * preRunFilter — drop cartesian-product points that fail a user predicate.
 *
 * Used by the sweep workflow's "conditional filter" (R5.4). The filter is
 * applied at the client before any child run is dispatched, so expensive
 * points ("voltage / current > 10 A would exceed fuse rating") are never
 * requested from the backend.
 *
 * Contract:
 *   - Pure, deterministic.
 *   - The predicate reads from `ChildDescriptor.params`. If the named
 *     parameter is not present on a child, we warn (once per param) and
 *     retain the child — silently dropping every row because of a typo'd
 *     key would be a terrible UX.
 *   - Applies to the `ChildDescriptor[]` after nestedRanges expansion;
 *     keeping the shape symmetric with post-hoc slicing simplifies reuse
 *     of the same predicate builder UI on both sides.
 */

import type { ChildDescriptor, ParamPredicate, CompareOp } from '@/engine/types';

/** Evaluate a `CompareOp` against two numbers. Exported for tests + reuse. */
export function compareNumbers(lhs: number, op: CompareOp, rhs: number): boolean {
  switch (op) {
    case 'lt': return lhs < rhs;
    case 'le': return lhs <= rhs;
    case 'gt': return lhs > rhs;
    case 'ge': return lhs >= rhs;
    case 'eq': return lhs === rhs;
    case 'ne': return lhs !== rhs;
  }
}

/**
 * Options for `applyPreRunFilter`. Exported so callers can inject a
 * test-friendly warn sink (tests typically want to assert on it rather
 * than let real console output leak).
 */
export interface PreRunFilterOpts {
  /** Sink for warnings. Defaults to `console.warn`. */
  warn?: (msg: string) => void;
}

/**
 * Apply a predicate to a list of pre-generated sweep children. Returns a
 * new array; never mutates input.
 *
 * @param children   Pre-generated child descriptors.
 * @param predicate  Predicate to evaluate. `null` / `undefined` is a
 *                   no-op — every child is retained unchanged. This
 *                   matches the "no active filter" state of the UI.
 * @param opts       Optional overrides for warning behaviour.
 */
export function applyPreRunFilter(
  children: ChildDescriptor[],
  predicate: ParamPredicate | null | undefined,
  opts: PreRunFilterOpts = {},
): ChildDescriptor[] {
  if (!predicate) return children.slice();

  const warn = opts.warn ?? ((m) => console.warn(m));

  // Detect missing param up front (track which names have already been
  // warned about so big sweeps don't produce N identical log lines).
  let everySawParam = true;
  for (const c of children) {
    if (!(predicate.param in c.params)) {
      everySawParam = false;
      break;
    }
  }
  if (!everySawParam) {
    warn(
      `pre-run filter: predicate references param "${predicate.param}" ` +
        `which is not present on every child — retaining all children`,
    );
    return children.slice();
  }

  return children.filter((c) => {
    const v = c.params[predicate.param];
    if (typeof v !== 'number') {
      // Non-numeric values slip past the early check above because
      // `predicate.param in c.params` does not check the value type.
      return true;
    }
    return compareNumbers(v, predicate.op, predicate.value);
  });
}
