/**
 * useStableSortedKeys — UX closeout #4 / #17 (big-model freeze).
 *
 * `PlotsTab` / `KpisTab` both derive `variableNames = Object.keys(timeSeries).sort()`
 * from the `timeSeries` prop, which gets a fresh object identity every
 * tick (ResultsWorkbench rebuilds it from the ring buffer on every
 * `tsRevision` bump). That made `variableNames` a brand-new array every
 * tick even though the actual SET of variable names is stable for the
 * entire life of a running session — only the VALUES change per tick.
 * Every downstream `useMemo`/`useEffect` keyed on `variableNames`
 * (notably `syncVariableMetrics`, which walks the whole metric
 * registry) was re-running on every tick as a result.
 *
 * `computeStableSortedKeys` returns the *previous* array when the sorted
 * key set is unchanged, so callers that memoize/effect off the returned
 * reference only re-fire when a variable is actually added or removed —
 * not on every value update.
 */

/** Pure comparison + stabilization step. Exported for direct unit testing. */
export function computeStableSortedKeys(
  obj: Record<string, unknown>,
  prev: string[] | null,
): string[] {
  const next = Object.keys(obj).sort();
  if (prev && prev.length === next.length && prev.every((k, i) => k === next[i])) {
    return prev;
  }
  return next;
}
