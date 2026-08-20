/**
 * VariablesPane — subscription-shape regression (ninebar Phase 1, F15).
 *
 * The pane must not subscribe to the whole `s.snapshot` object (nor to
 * `useTimeSeriesStore.revision`) at the top level — see the "Live-store
 * subscription shape" block in VariablesPane.tsx. This suite proves the
 * guardrail holds: when an unrelated variable's value changes but the
 * live tick and the variable name set stay the same, the pane's own
 * render count must not move, while the row whose value actually
 * changed still reflects the new value (via its own `useVar` subscription
 * in VariableRow.tsx).
 *
 * Network-backed hooks (useSessionDetail / useSessionTopology /
 * useWorkspaceUris / useWorkspaceTree) are neutralised the same way
 * CausalTracePanel.test.tsx does it: mock the shared http transport and
 * return a never-resolving promise so those queries just sit in
 * "loading" without affecting the assertions below. `__testEntries`
 * supplies the structural entry list directly, matching how the pane's
 * own test hook is meant to be used.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, cleanup, act, fireEvent } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { VariablesPane, __resetVariablesPaneShortcutForTests } from '../VariablesPane';
import { useSessionStore } from '@/features/sessions/store';
import { useSessionLiveStore } from '@/features/sessions/sessionLiveStore';
import type { VariableEntry } from '../VariableTree';

vi.mock('@/shared/api/http', () => ({
  httpGet: vi.fn(),
  httpPost: vi.fn(),
}));

import { httpPost, httpGet } from '@/shared/api/http';
const mockPost = httpPost as unknown as ReturnType<typeof vi.fn>;
const mockGet = httpGet as unknown as ReturnType<typeof vi.fn>;

const SESSION_ID = 'sess-f15';

function makeClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
}

function Wrap({ children }: { children: ReactNode }) {
  const client = makeClient();
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

const TEST_ENTRIES: VariableEntry[] = [
  { name: 'circuit.watched', value: 10 },
  { name: 'circuit.unrelated', value: 1 },
];

beforeEach(() => {
  // Never-resolving promises keep useSessionDetail / useSessionTopology /
  // useWorkspaceTree in "loading" — irrelevant to this suite since
  // __testEntries bypasses entry derivation entirely.
  mockPost.mockReturnValue(new Promise(() => {}));
  mockGet.mockReturnValue(new Promise(() => {}));

  useSessionStore.setState({
    activeSessionId: SESSION_ID,
    phase: 'running',
    selectedScope: [],
  });

  useSessionLiveStore.getState().reset();
  __resetVariablesPaneShortcutForTests();
  act(() => {
    useSessionLiveStore.getState().applyHello(SESSION_ID, {
      tick: 0,
      time_ms: 0,
      completed: false,
      subsystems: {},
      scalar_vars: { 'circuit.watched': 10, 'circuit.unrelated': 1 },
      string_vars: {},
      constraint_results: [],
    });
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useSessionStore.setState({ activeSessionId: null, phase: 'idle', selectedScope: [] });
  useSessionLiveStore.getState().reset();
});

describe('VariablesPane — F15 subscription shape', () => {
  it('does not re-render the pane root when an unrelated variable changes value but names/tick stay put', () => {
    render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} __testEntries={TEST_ENTRIES} />
      </Wrap>,
    );

    const pane = () => screen.getByTestId('variables-pane');
    const renderCountBefore = pane().getAttribute('data-render-count');
    expect(renderCountBefore).toBe('1');

    // Change a scalar's value WITHOUT advancing the tick (same tick
    // number => the pane's `liveTick` selector output is unchanged) and
    // without adding/removing any names (=> `namesKey` is unchanged
    // too). Neither of the pane's two per-key selectors should fire.
    act(() => {
      useSessionLiveStore.getState().applyTick({
        tick: 0,
        time_ms: 0,
        completed: false,
        scalar_changed: { 'circuit.unrelated': 999 },
      });
    });

    expect(pane().getAttribute('data-render-count')).toBe(renderCountBefore);
  });

  it('still reflects the changed value on the affected row via its own live subscription', () => {
    render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} __testEntries={TEST_ENTRIES} />
      </Wrap>,
    );

    const row = () => screen.getByTestId('variable-row-circuit.unrelated');
    expect(row().textContent).toContain('1');

    act(() => {
      useSessionLiveStore.getState().applyTick({
        tick: 0,
        time_ms: 0,
        completed: false,
        scalar_changed: { 'circuit.unrelated': 999 },
      });
    });

    // The row picked up the fresh value straight from the store even
    // though the pane itself never re-rendered (previous test).
    expect(row().textContent).toContain('999');
  });

  it('re-renders the pane chrome (and rebuilds structure) when a variable is added — namesKey changes', () => {
    render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} />
      </Wrap>,
    );

    expect(screen.queryByTestId('variable-row-circuit.newVar')).toBeNull();

    act(() => {
      useSessionLiveStore.getState().applyTick({
        tick: 1,
        time_ms: 10,
        completed: false,
        scalar_changed: { 'circuit.newVar': 5 },
      });
    });

    expect(screen.getByTestId('variable-row-circuit.newVar')).toBeDefined();
  });
});

describe('VariablesPane — keyboard shortcut singleton (dual mount)', () => {
  it('attaches exactly one window keydown listener across two concurrent mounts', () => {
    const addSpy = vi.spyOn(window, 'addEventListener');

    const { unmount: unmountA } = render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} __testEntries={TEST_ENTRIES} />
      </Wrap>,
    );
    const { unmount: unmountB } = render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} __testEntries={TEST_ENTRIES} />
      </Wrap>,
    );

    const keydownRegistrations = addSpy.mock.calls.filter(([type]) => type === 'keydown');
    expect(keydownRegistrations).toHaveLength(1);

    unmountA();
    unmountB();
    addSpy.mockRestore();
  });

  it('routes Cmd/Ctrl+Shift+V to whichever pane most recently mounted', () => {
    render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} __testEntries={TEST_ENTRIES} />
      </Wrap>,
    );
    // A second concurrent mount (rail + workbench during ninebar migration).
    render(
      <Wrap>
        <VariablesPane sessionId={SESSION_ID} __testEntries={TEST_ENTRIES} />
      </Wrap>,
    );

    const searchInputs = screen.getAllByTestId('variables-pane-search');
    expect(searchInputs).toHaveLength(2);
    expect(searchInputs.every((el) => el !== document.activeElement)).toBe(true);

    fireEvent.keyDown(window, { key: 'v', metaKey: true, shiftKey: true });

    // Exactly one pane (the most-recently-mounted one) claims focus — not
    // both, and not neither.
    const focused = searchInputs.filter((el) => el === document.activeElement);
    expect(focused).toHaveLength(1);
    expect(focused[0]).toBe(searchInputs[1]);
  });
});
