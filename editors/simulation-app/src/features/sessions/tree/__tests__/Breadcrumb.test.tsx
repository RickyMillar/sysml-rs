/**
 * Breadcrumb — contract tests.
 *
 * Verifies the strict-prefix semantics the plan promised (clicking
 * any segment navigates to that depth, never appends) and the
 * home-chip + leaf-is-not-a-link affordances.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { Breadcrumb } from '../Breadcrumb';

afterEach(() => {
  cleanup();
});

const THREE_SEGMENTS = [
  { id: 'sb', label: 'ProductionCell' },
  { id: 'c1', label: 'Station 1' },
  { id: 'groupHead', label: 'GroupHead (C16)' },
];

describe('Breadcrumb — render', () => {
  it('at depth 0 (empty segments) renders only Home chip', () => {
    render(<Breadcrumb segments={[]} onNavigateToDepth={vi.fn()} />);
    expect(screen.getByTestId('breadcrumb')).toHaveAttribute(
      'data-depth',
      '0',
    );
    expect(screen.getByTestId('breadcrumb-home')).toBeInTheDocument();
    expect(screen.queryByTestId('breadcrumb-leaf')).toBeNull();
  });

  it('renders every segment in order with the last marked aria-current', () => {
    render(
      <Breadcrumb
        segments={THREE_SEGMENTS}
        onNavigateToDepth={vi.fn()}
      />,
    );
    expect(screen.getByTestId('breadcrumb-segment-0')).toHaveTextContent(
      'ProductionCell',
    );
    expect(screen.getByTestId('breadcrumb-segment-1')).toHaveTextContent(
      'Station 1',
    );
    const leaf = screen.getByTestId('breadcrumb-leaf');
    expect(leaf).toHaveTextContent('GroupHead (C16)');
    expect(leaf).toHaveAttribute('aria-current', 'page');
    // Leaf has no segment-N testid (it's a <span>, not a <button>).
    expect(screen.queryByTestId('breadcrumb-segment-2')).toBeNull();
  });

  it('stamps data-segment-id on every segment chip and the leaf', () => {
    render(
      <Breadcrumb
        segments={THREE_SEGMENTS}
        onNavigateToDepth={vi.fn()}
      />,
    );
    expect(screen.getByTestId('breadcrumb-segment-0')).toHaveAttribute(
      'data-segment-id',
      'sb',
    );
    expect(screen.getByTestId('breadcrumb-segment-1')).toHaveAttribute(
      'data-segment-id',
      'c1',
    );
    expect(screen.getByTestId('breadcrumb-leaf')).toHaveAttribute(
      'data-segment-id',
      'groupHead',
    );
  });
});

describe('Breadcrumb — navigation', () => {
  it('clicking Home navigates to depth 0 (root)', () => {
    const onNav = vi.fn();
    render(
      <Breadcrumb segments={THREE_SEGMENTS} onNavigateToDepth={onNav} />,
    );
    fireEvent.click(screen.getByTestId('breadcrumb-home'));
    expect(onNav).toHaveBeenCalledWith(0);
  });

  it('Home is disabled at depth 0 (no-op when already at root)', () => {
    const onNav = vi.fn();
    render(<Breadcrumb segments={[]} onNavigateToDepth={onNav} />);
    const home = screen.getByTestId('breadcrumb-home');
    expect(home).toBeDisabled();
    fireEvent.click(home);
    // Disabled buttons do not fire click handlers in jsdom.
    expect(onNav).not.toHaveBeenCalled();
  });

  it('clicking a middle segment navigates to that depth (1-based truncation)', () => {
    const onNav = vi.fn();
    render(
      <Breadcrumb segments={THREE_SEGMENTS} onNavigateToDepth={onNav} />,
    );
    // Depth semantics: segment idx 0 → depth 1 (keep 1 segment),
    // segment idx 1 → depth 2. Matches `focusPath.slice(0, N)`.
    fireEvent.click(screen.getByTestId('breadcrumb-segment-0'));
    expect(onNav).toHaveBeenLastCalledWith(1);
    fireEvent.click(screen.getByTestId('breadcrumb-segment-1'));
    expect(onNav).toHaveBeenLastCalledWith(2);
  });

  it('leaf is not clickable (aria-current="page", not a <button>)', () => {
    const onNav = vi.fn();
    render(
      <Breadcrumb segments={THREE_SEGMENTS} onNavigateToDepth={onNav} />,
    );
    const leaf = screen.getByTestId('breadcrumb-leaf');
    // Click it anyway — nothing should fire.
    fireEvent.click(leaf);
    expect(onNav).not.toHaveBeenCalled();
  });
});

describe('Breadcrumb — single segment', () => {
  it('renders only Home chip + leaf (no middle segments)', () => {
    render(
      <Breadcrumb
        segments={[{ id: 'sb', label: 'ProductionCell' }]}
        onNavigateToDepth={vi.fn()}
      />,
    );
    expect(screen.getByTestId('breadcrumb-home')).toBeInTheDocument();
    expect(screen.getByTestId('breadcrumb-leaf')).toHaveTextContent(
      'ProductionCell',
    );
    expect(screen.queryByTestId('breadcrumb-segment-0')).toBeNull();
  });
});

describe('Breadcrumb — custom testIdPrefix', () => {
  it('namespaces every testid under the custom prefix', () => {
    render(
      <Breadcrumb
        segments={THREE_SEGMENTS}
        onNavigateToDepth={vi.fn()}
        testIdPrefix="run-page-crumb"
      />,
    );
    expect(screen.getByTestId('run-page-crumb')).toBeInTheDocument();
    expect(screen.getByTestId('run-page-crumb-home')).toBeInTheDocument();
    expect(screen.getByTestId('run-page-crumb-segment-0')).toBeInTheDocument();
    expect(screen.getByTestId('run-page-crumb-leaf')).toBeInTheDocument();
    expect(screen.queryByTestId('breadcrumb')).toBeNull();
  });
});
