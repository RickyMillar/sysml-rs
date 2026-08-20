/**
 * SmDetail — accepted-events rail + transition table tests.
 *
 * Mocks `useInjectEvent` and `useSessionStore` so the tests don't
 * touch the React Query client or the real Zustand store.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

const injectMutate = vi.fn();
const mockUseSessionStore = vi.fn<(selector: (s: unknown) => unknown) => unknown>();

vi.mock('@/features/sessions/mutations', () => ({
  useInjectEvent: () => ({
    mutate: injectMutate,
    isPending: false,
  }),
}));

vi.mock('../../store', () => ({
  useSessionStore: (selector: (s: unknown) => unknown) =>
    mockUseSessionStore(selector),
}));

import { SmDetail } from '../detail/SmDetail';
import type { SmTreeNode } from '../types';

function smNode(overrides: Partial<SmTreeNode> = {}): SmTreeNode {
  return {
    id: 'sm1',
    uri: 'file:///w.sysml',
    name: 'breaker_1',
    rawKind: 'StateDefinition',
    kind: 'sm',
    depth: 2,
    ownerPath: 'ProductionCell',
    children: [],
    currentState: 'armed',
    states: [
      { id: 's-armed', name: 'armed' },
      { id: 's-tripped', name: 'tripped' },
      { id: 's-off', name: 'off' },
    ],
    transitions: [
      { id: 't-trip', name: 'armed_to_tripped', source: 'armed', target: 'tripped' },
      { id: 't-reset', name: 'tripped_to_armed', source: 'tripped', target: 'armed' },
      { id: 't-off', name: 'armed_to_off', source: 'armed', target: 'off' },
    ],
    availableTransitions: [
      ['armed_to_tripped', 'tripped'],
      ['armed_to_off', 'off'],
    ],
    ...overrides,
  } as SmTreeNode;
}

afterEach(() => {
  cleanup();
  injectMutate.mockReset();
  mockUseSessionStore.mockReset();
});

describe('SmDetail — accepted-events rail', () => {
  it('renders a chip per unique available event', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(<SmDetail node={smNode()} testIdPrefix="d" />);
    expect(screen.getByTestId('d-sm-rail')).toBeInTheDocument();
    expect(screen.getByTestId('d-sm-rail-armed_to_tripped')).toBeInTheDocument();
    expect(screen.getByTestId('d-sm-rail-armed_to_off')).toBeInTheDocument();
  });

  it('dedupes repeated events (same event, multiple targets)', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(
      <SmDetail
        node={smNode({
          availableTransitions: [
            ['fault', 'tripped'],
            ['fault', 'off'],
          ],
        })}
        testIdPrefix="d"
      />,
    );
    // Only one chip for the `fault` event.
    expect(screen.queryAllByTestId('d-sm-rail-fault')).toHaveLength(1);
  });

  it('shows the empty placeholder when no events are accepted', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(
      <SmDetail
        node={smNode({ availableTransitions: [] })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-sm-rail-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('d-sm-rail')).toBeNull();
  });

  it('clicking a chip invokes useInjectEvent with sessionId + subsystem + event', () => {
    mockUseSessionStore.mockReturnValue('sess-xyz');
    render(<SmDetail node={smNode()} testIdPrefix="d" />);
    fireEvent.click(screen.getByTestId('d-sm-rail-armed_to_tripped'));
    expect(injectMutate).toHaveBeenCalledTimes(1);
    expect(injectMutate).toHaveBeenCalledWith({
      sessionId: 'sess-xyz',
      subsystem: 'breaker_1',
      event: 'armed_to_tripped',
    });
  });

  it('disables chips and skips inject when no active session', () => {
    mockUseSessionStore.mockReturnValue(null);
    render(<SmDetail node={smNode()} testIdPrefix="d" />);
    const chip = screen.getByTestId('d-sm-rail-armed_to_tripped') as HTMLButtonElement;
    expect(chip.disabled).toBe(true);
    fireEvent.click(chip);
    expect(injectMutate).not.toHaveBeenCalled();
  });
});

describe('SmDetail — transition table', () => {
  it('renders one row per static transition with from / event / to columns', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(<SmDetail node={smNode()} testIdPrefix="d" />);
    expect(screen.getByTestId('d-sm-ttbl')).toBeInTheDocument();
    expect(screen.getByTestId('d-sm-ttbl-row-t-trip')).toBeInTheDocument();
    expect(screen.getByTestId('d-sm-ttbl-row-t-reset')).toBeInTheDocument();
    expect(screen.getByTestId('d-sm-ttbl-row-t-off')).toBeInTheDocument();
  });

  it('flags rows from the current state that match availableTransitions as live', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(<SmDetail node={smNode()} testIdPrefix="d" />);
    const liveRow = screen.getByTestId('d-sm-ttbl-row-t-trip');
    expect(liveRow.getAttribute('data-live')).toBe('true');
    // Not from the current state → not live.
    const offState = screen.getByTestId('d-sm-ttbl-row-t-reset');
    expect(offState.getAttribute('data-live')).toBeNull();
  });

  it('renders an inject button only on live rows and fires the mutation on click', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(<SmDetail node={smNode()} testIdPrefix="d" />);
    // Live row has an inject button.
    const live = screen.getByTestId('d-sm-ttbl-inject-t-trip');
    expect(live).toBeInTheDocument();
    // Non-current-state rows have no inject button.
    expect(screen.queryByTestId('d-sm-ttbl-inject-t-reset')).toBeNull();
    fireEvent.click(live);
    expect(injectMutate).toHaveBeenCalledWith({
      sessionId: 'sess-1',
      subsystem: 'breaker_1',
      event: 'armed_to_tripped',
    });
  });

  it('shows the empty placeholder when no transitions are declared', () => {
    mockUseSessionStore.mockReturnValue('sess-1');
    render(
      <SmDetail
        node={smNode({ transitions: [] })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-sm-ttbl-empty')).toBeInTheDocument();
  });
});
