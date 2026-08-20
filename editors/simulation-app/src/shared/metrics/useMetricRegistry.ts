/**
 * useMetricRegistryVersion — re-render when the shared MetricRegistry changes.
 *
 * The registry is a plain mutable class deliberately kept out of React state
 * (see `registry.ts`), so consumers need an explicit bridge.
 * `useSyncExternalStore` is that bridge: it subscribes, reads the monotonic
 * version as the snapshot, and re-renders on any mutation.
 *
 * Replaces the `const [metricTick] = useState(0)` idiom, which never updated —
 * a `useMemo` keyed on it therefore snapshotted the registry once and went
 * stale for the life of the component.
 */

import { useSyncExternalStore } from 'react';
import { metricRegistry, type MetricRegistry } from './registry';

export function useMetricRegistryVersion(registry: MetricRegistry = metricRegistry): number {
  return useSyncExternalStore(
    (listener) => registry.subscribe(listener),
    () => registry.getVersion(),
    () => registry.getVersion(),
  );
}
