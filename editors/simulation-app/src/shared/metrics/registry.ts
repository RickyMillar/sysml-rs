/**
 * MetricRegistry — shared, mutable catalogue of MetricDescriptors.
 *
 * Extensibility plan EP1.7: one registry, many producers (session
 * variables auto-registered today; expression/constraint sources land in
 * later rounds) and many consumers (PlotsTab picker today;
 * Sweep/Monte Carlo/KPI/verification tomorrow).
 *
 * Design notes:
 *   - Plain class, no Zustand / React state. Consumers that need
 *     reactivity wrap the registry in a React hook; this keeps the
 *     primitive pure and testable without a render environment.
 *   - `register` is idempotent by `id`: re-registering replaces the
 *     previous descriptor. That lets producers refresh metadata (unit,
 *     domain) without the caller having to unregister first.
 *   - Insertion order is preserved via a Map, so `list()` yields a stable
 *     order for UI iteration (picker groups, chip order).
 */

import type { MetricDescriptor } from './types';

export class MetricRegistry {
  /** Storage keyed by id; Map preserves insertion order. */
  private readonly metrics = new Map<string, MetricDescriptor>();

  /** Bumped on every mutation; the change signal for `subscribe`. */
  private version = 0;

  /** Listeners notified after any mutation. */
  private readonly listeners = new Set<() => void>();

  /**
   * Subscribe to mutations. Returns an unsubscribe function, so this plugs
   * straight into `useSyncExternalStore`.
   *
   * The registry is push-only and mutated from render effects (PlotsTab,
   * WaveformCard), with no way for a consumer to learn about it. Sweep
   * snapshotted `list()` through a `useMemo` keyed on a `useState(0)` that was
   * never updated, so metrics registered after the modal first rendered never
   * appeared — the list stayed empty even once a run had populated it.
   */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Monotonic mutation counter — a stable `getSnapshot` for React. */
  getVersion(): number {
    return this.version;
  }

  private notify(): void {
    this.version += 1;
    for (const listener of this.listeners) listener();
  }

  /** Upsert a descriptor. Idempotent by `id`. */
  register(metric: MetricDescriptor): void {
    this.metrics.set(metric.id, metric);
    this.notify();
  }

  /** Remove a descriptor. No-op when the id is absent. */
  unregister(id: string): void {
    if (this.metrics.delete(id)) this.notify();
  }

  /** Every registered descriptor in insertion order. */
  list(): MetricDescriptor[] {
    return Array.from(this.metrics.values());
  }

  /** Single lookup — returns `undefined` when no metric matches. */
  get(id: string): MetricDescriptor | undefined {
    return this.metrics.get(id);
  }

  /**
   * Predicate-filtered subset in insertion order. Kept thin on purpose —
   * callers that need fancier queries (domain groupings, aggregator
   * filtering, etc.) build them on top.
   */
  filter(predicate: (m: MetricDescriptor) => boolean): MetricDescriptor[] {
    const out: MetricDescriptor[] = [];
    for (const m of this.metrics.values()) if (predicate(m)) out.push(m);
    return out;
  }

  /** Purge everything. Used by tests + session reset helpers. */
  clear(): void {
    if (this.metrics.size === 0) return;
    this.metrics.clear();
    this.notify();
  }

  /** Number of registered descriptors. */
  get size(): number {
    return this.metrics.size;
  }
}

/**
 * Application-wide default registry. The Round-1 PlotsTab reads from
 * this instance; later rounds layer tool-specific registries on top (the
 * class is exported above so isolated registries are possible).
 */
export const metricRegistry = new MetricRegistry();

/**
 * Helper: rebuild the registry from a flat list of variable names. Used
 * by PlotsTab / the session ingest path to mirror `Object.keys(timeSeries)`
 * into the registry with `source: 'variable'` every tick.
 *
 * The helper diffs the registry against the incoming list so unchanged
 * entries stay put (preserving any domain / unit metadata producers may
 * have enriched them with) and removed variables are evicted.
 */
export function syncVariableMetrics(
  registry: MetricRegistry,
  variableNames: Iterable<string>,
  classify?: (name: string) => string | undefined,
): void {
  const next = new Set<string>();
  for (const name of variableNames) {
    next.add(name);
    const existing = registry.get(name);
    if (existing && existing.source === 'variable') {
      // Already registered with the same source — leave it alone so any
      // producer-added metadata (domain, unit) survives the sync.
      continue;
    }
    registry.register({
      id: name,
      name,
      source: 'variable',
      expression: name,
      domain: classify?.(name),
    });
  }
  // Evict variable-sourced metrics that are no longer present. Leave
  // expression/constraint-sourced ones alone — they have their own
  // lifecycle owned by whichever producer registered them.
  for (const m of registry.list()) {
    if (m.source === 'variable' && !next.has(m.id)) {
      registry.unregister(m.id);
    }
  }
}
