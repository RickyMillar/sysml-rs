/**
 * The registry notifies its consumers.
 *
 * `MetricRegistry` is deliberately a plain mutable class outside React state,
 * so a consumer needs an explicit change signal. There wasn't one: Sweep
 * snapshotted `list()` through a `useMemo` keyed on `useState(0)` that nothing
 * ever updated, so metrics registered after first paint never showed up — the
 * picker stayed empty even once a run had populated the catalogue.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';

import { MetricRegistry } from '../registry';
import { useMetricRegistryVersion } from '../useMetricRegistry';

afterEach(cleanup);

function Consumer({ registry }: { registry: MetricRegistry }) {
  useMetricRegistryVersion(registry);
  return <span data-testid="names">{registry.list().map((m) => m.name).join(',')}</span>;
}

const metric = (name: string) => ({
  id: name,
  name,
  source: 'variable' as const,
  expression: name,
});

describe('useMetricRegistryVersion', () => {
  it('re-renders when a metric is registered after mount', () => {
    const registry = new MetricRegistry();
    render(<Consumer registry={registry} />);
    expect(screen.getByTestId('names')).toHaveTextContent('');

    act(() => registry.register(metric('temperature')));
    expect(screen.getByTestId('names')).toHaveTextContent('temperature');

    act(() => registry.register(metric('pressure')));
    expect(screen.getByTestId('names')).toHaveTextContent('temperature,pressure');
  });

  it('re-renders on unregister and clear', () => {
    const registry = new MetricRegistry();
    registry.register(metric('a'));
    registry.register(metric('b'));
    render(<Consumer registry={registry} />);
    expect(screen.getByTestId('names')).toHaveTextContent('a,b');

    act(() => registry.unregister('a'));
    expect(screen.getByTestId('names')).toHaveTextContent('b');

    act(() => registry.clear());
    expect(screen.getByTestId('names')).toHaveTextContent('');
  });

  it('does not bump the version for a no-op mutation', () => {
    // Re-render churn on every poll tick is exactly what the registry's
    // "plain class, no React state" design was avoiding.
    const registry = new MetricRegistry();
    const before = registry.getVersion();
    registry.unregister('absent');
    registry.clear();
    expect(registry.getVersion()).toBe(before);
  });

  it('stops notifying after unsubscribe', () => {
    const registry = new MetricRegistry();
    let hits = 0;
    const unsubscribe = registry.subscribe(() => {
      hits += 1;
    });
    registry.register(metric('a'));
    expect(hits).toBe(1);
    unsubscribe();
    registry.register(metric('b'));
    expect(hits).toBe(1);
  });
});
