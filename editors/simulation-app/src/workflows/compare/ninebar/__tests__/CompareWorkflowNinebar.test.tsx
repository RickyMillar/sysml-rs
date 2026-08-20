/**
 * CompareWorkflowNinebar — flag-on Compare surface tests (Phase 6).
 *
 * The data layer is mocked at the compareData/useSessionList seams so
 * these specs pin the SHELL contract: teaching state, pair-mode
 * banners (history_truncated), the fork-anchor snap, diff-token value
 * chips (missing dimming), and the one-playhead wiring.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type {
  SessionSummary,
  SessionTimelineDivergence,
} from '@/features/sessions/types';

// ninebar is default-on; pin explicitly so the suite never drifts.
window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: true };

// ── Mock data plumbing ──────────────────────────────────────────────

let mockSessions: SessionSummary[] = [];
let mockPairDiff: SessionTimelineDivergence | null = null;
let mockGoldenRef: {
  series: Record<string, Array<{ t: number; v: number }>>;
  label: string;
  snapshotCount: number;
} | null = null;
let mockGoldenList: Array<{ id: string; label: string; golden_label?: string }> = [];

vi.mock('@/features/sessions/queries', () => ({
  useSessionList: () => ({ data: mockSessions, isLoading: false }),
}));

// The rail's fork-at-playhead affordance (W4). Controllable per test.
const forkMutate = vi.fn();
let mockForkPending = false;
vi.mock('@/features/sessions/mutations', () => ({
  useForkSession: () => ({ mutate: forkMutate, isPending: mockForkPending }),
}));

// GoldenStrip's archive picker + the history-modal opener.
vi.mock('@/features/archive/useArchiveList', () => ({
  useArchiveList: () => ({ data: mockGoldenList, isLoading: false }),
}));

vi.mock('../compareData', async () => {
  const actual = await vi.importActual<typeof import('../compareData')>('../compareData');
  return {
    ...actual,
    useGoldenReference: () => ({ data: mockGoldenRef }),
    useTimelineDiff: (a: SessionSummary | null, b: SessionSummary | null) => ({
      data: a && b ? mockPairDiff : undefined,
    }),
    useUnionVariableNames: (summaries: SessionSummary[]) => ({
      names: summaries.length > 0 ? ['current', 'voltage'] : [],
      namesBySession: new Map(
        summaries.map((s) => [s.id, new Set(['current', 'voltage'])]),
      ),
      isLoading: false,
    }),
    useCompareSeries: (summaries: SessionSummary[]) => ({
      samplesByVar: {
        current: summaries.map((_, i) => [0, 1 + i, 2 + i, 3 + i]),
        voltage: summaries.map((_, i) => (i === 0 ? [5, 5, 5, 5] : [5, 5, NaN, NaN])),
      },
      maxTick: 3,
      isLoading: false,
    }),
  };
});

import { CompareWorkflow } from '../../CompareWorkflow';
import { useCompareStore } from '../../useCompareStore';

function summary(overrides: Partial<SessionSummary>): SessionSummary {
  return {
    id: 'sess-a',
    kind: 'orchestrator',
    uri: '__workspace__',
    subsystem_name: null,
    label: null,
    created_at_ms: 0,
    elapsed_ms: 0,
    tick: 3,
    time_ms: 0.3,
    current_state: null,
    completed: true,
    is_expired: false,
    history_len: 4,
    subsystem_count: 1,
    fork_point_tick: null,
    forkable_ticks: [],
    paused: false,
    ticks_advanced: 0,
    ...overrides,
  };
}

function resetStore() {
  useCompareStore.setState({
    pickedSessionIds: [],
    sharedTick: 0,
    isPlaying: false,
    layout: null,
    activeModeId: null,
    pickedVariables: null,
    goldenArchiveId: null,
    goldenToleranceRel: 0.05,
  });
}

beforeEach(() => {
  resetStore();
  mockSessions = [];
  mockPairDiff = null;
  mockForkPending = false;
  forkMutate.mockReset();
  mockGoldenRef = null;
  mockGoldenList = [];
});
afterEach(cleanup);

describe('CompareWorkflowNinebar — shell', () => {
  it('renders the flag-on surface with rail + playhead', () => {
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-workflow-ninebar')).toBeTruthy();
    expect(screen.getByTestId('compare-session-rail')).toBeTruthy();
    expect(screen.getByTestId('compare-playhead')).toBeTruthy();
  });

  it('shows the teaching state below 2 picks', () => {
    mockSessions = [summary({ id: 'a' })];
    useCompareStore.setState({ pickedSessionIds: ['a'] });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-teaching')).toBeTruthy();
  });

  it('renders the diff canvas with variable rows and value chips at 2 picks', () => {
    mockSessions = [summary({ id: 'a' }), summary({ id: 'b', label: 'faulted' })];
    useCompareStore.setState({ pickedSessionIds: ['a', 'b'] });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-diff-canvas')).toBeTruthy();
    expect(screen.getByTestId('compare-variable-row-current')).toBeTruthy();
    expect(screen.getByTestId('compare-envelope-current')).toBeTruthy();
    expect(screen.getByTestId('compare-value-current-a')).toBeTruthy();
    expect(screen.getByTestId('compare-value-current-b')).toBeTruthy();
  });

  it('dims a missing sample as "—" (diff-missing), never zero', () => {
    mockSessions = [summary({ id: 'a' }), summary({ id: 'b' })];
    useCompareStore.setState({ pickedSessionIds: ['a', 'b'], sharedTick: 3 });
    render(<CompareWorkflow />);
    // voltage for session b is NaN at tick 3 (mock above).
    const chip = screen.getByTestId('compare-value-voltage-b');
    expect(chip.textContent).toContain('—');
  });

  it('reports picked ids that no longer resolve instead of dropping them', () => {
    mockSessions = [summary({ id: 'a' })];
    useCompareStore.setState({ pickedSessionIds: ['a', 'gone-id'] });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-missing-banner')).toBeTruthy();
    expect(screen.getByTestId('compare-session-rail-missing').textContent).toContain(
      'no longer available',
    );
  });
});

describe('CompareWorkflowNinebar — pair mode honesty + markers', () => {
  const pairDiff = (over: Partial<SessionTimelineDivergence>): SessionTimelineDivergence => ({
    a_id: 'a',
    b_id: 'b',
    shared_start_tick: 0,
    shared_end_tick: 3,
    first_divergence_tick: 2,
    tick_diffs: [
      {
        tick: 2,
        subsystem_diffs: [{ name: 'Breaker', a_state: 'Closed', b_state: 'Open' }],
        variable_diffs: [{ name: 'current', a_value: 2, b_value: null }],
      },
    ],
    history_truncated: false,
    ...over,
  });

  beforeEach(() => {
    mockSessions = [summary({ id: 'a' }), summary({ id: 'b' })];
    useCompareStore.setState({ pickedSessionIds: ['a', 'b'] });
  });

  it('renders the history_truncated banner only when the backend flags it', () => {
    mockPairDiff = pairDiff({ history_truncated: true });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-history-truncated').textContent).toContain(
      'history truncated',
    );
    cleanup();
    mockPairDiff = pairDiff({ history_truncated: false });
    render(<CompareWorkflow />);
    expect(screen.queryByTestId('compare-history-truncated')).toBeNull();
  });

  it('marks first divergence and jumps the playhead to it', () => {
    mockPairDiff = pairDiff({});
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-marker-first-divergence')).toBeTruthy();
    fireEvent.click(screen.getByTestId('compare-jump-divergence'));
    expect(useCompareStore.getState().sharedTick).toBe(2);
  });

  it('shows subsystem state divergence and the variable pair-kind at the playhead', () => {
    mockPairDiff = pairDiff({});
    useCompareStore.setState({ sharedTick: 2 });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-state-diff-Breaker').textContent).toContain(
      'Closed ⇥ Open',
    );
    // current: a_value=2, b_value=null → 'removed' manufacture.
    expect(
      screen.getByTestId('compare-variable-pairkind-current').textContent,
    ).toBe('removed');
  });
});

describe('CompareWorkflowNinebar — fork anchoring (F8)', () => {
  it('snaps the playhead to fork_point_tick when a fork joins the picks', () => {
    mockSessions = [
      summary({ id: 'parent' }),
      summary({ id: 'child', fork_point_tick: 2, forkable_ticks: [0, 2] }),
    ];
    useCompareStore.setState({ pickedSessionIds: ['parent', 'child'], sharedTick: 0 });
    render(<CompareWorkflow />);
    expect(useCompareStore.getState().sharedTick).toBe(2);
    expect(screen.getByTestId('compare-marker-fork-anchor')).toBeTruthy();
  });
});

describe('CompareWorkflowNinebar — fork affordances at forkable_ticks (F8)', () => {
  beforeEach(() => {
    mockSessions = [
      summary({ id: 'a', forkable_ticks: [0, 2] }),
      summary({ id: 'b', forkable_ticks: [] }),
    ];
    useCompareStore.setState({ pickedSessionIds: ['a', 'b'] });
  });

  it('offers "fork here" ONLY when the playhead sits on an archived tick', () => {
    useCompareStore.setState({ sharedTick: 2 });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-fork-here-a')).toBeTruthy();
    // b has no archived ticks — no affordance, no guessing.
    expect(screen.queryByTestId('compare-fork-here-b')).toBeNull();
    cleanup();
    // Off-archive tick: no affordance anywhere.
    useCompareStore.setState({ sharedTick: 1 });
    render(<CompareWorkflow />);
    expect(screen.queryByTestId('compare-fork-here-a')).toBeNull();
  });

  it('forks at the exact playhead tick and pulls the child into the picks', () => {
    useCompareStore.setState({ sharedTick: 2 });
    forkMutate.mockImplementation((args, opts) => {
      opts?.onSuccess?.(summary({ id: 'child-1', fork_point_tick: 2 }));
    });
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-fork-here-a'));
    expect(forkMutate).toHaveBeenCalledWith(
      { sessionId: 'a', atTick: 2 },
      expect.anything(),
    );
    expect(useCompareStore.getState().pickedSessionIds).toContain('child-1');
  });

  it('consumes the structured SnapshotMissing error and names the valid ticks', () => {
    useCompareStore.setState({ sharedTick: 2 });
    forkMutate.mockImplementation((args, opts) => {
      opts?.onError?.(
        new Error(
          'API 500 /api/command: {"kind":"SnapshotMissing","tick":2,"earliest_available":4,"valid_ticks":[4,6]}',
        ),
      );
    });
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-fork-here-a'));
    const err = screen.getByTestId('compare-fork-error-a');
    expect(err.textContent).toContain('tick 2 is not archived');
    expect(err.textContent).toContain('4, 6');
  });

  it('renders forkable-tick dot rows on the playhead marker strip', () => {
    useCompareStore.setState({ sharedTick: 0 });
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-forkable-row-a')).toBeTruthy();
  });
});

describe('CompareWorkflowNinebar — mode switch (no extra panels)', () => {
  beforeEach(() => {
    mockSessions = [summary({ id: 'a' }), summary({ id: 'b' })];
    useCompareStore.setState({ pickedSessionIds: ['a', 'b'] });
  });

  it('defaults to Diff with all four tabs present', () => {
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-mode-tabs')).toBeTruthy();
    expect(
      screen.getByTestId('compare-mode-tab-diff').getAttribute('data-active'),
    ).toBe('true');
    expect(screen.queryByTestId('compare-ensemble-strip')).toBeNull();
    expect(screen.queryByTestId('compare-golden-strip')).toBeNull();
    expect(screen.queryByTestId('compare-twodesign-strip')).toBeNull();
  });

  it('ensemble mode shows per-variable stats at the playhead', () => {
    useCompareStore.setState({ sharedTick: 2 });
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-mode-tab-ensemble'));
    expect(screen.getByTestId('compare-ensemble-strip')).toBeTruthy();
    // 'current' mock series: session a → 2, session b → 3 at tick 2.
    const row = screen.getByTestId('compare-ensemble-row-current');
    expect(row.textContent).toContain('2.500'); // mean of 2 and 3
  });

  it('two-design mode shows the per-variable delta table with peak jump', () => {
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-mode-tab-two-design'));
    expect(screen.getByTestId('compare-twodesign-strip')).toBeTruthy();
    const peak = screen.getByTestId('compare-twodesign-peak-current');
    fireEvent.click(peak);
    // 'current': A=[0,1,2,3], B=[0,2,3,4] → |A−B| first peaks (=1) at tick 1.
    expect(useCompareStore.getState().sharedTick).toBe(1);
  });

  it('golden mode: picker lists golden-pinned runs, verdicts use verdict tokens', () => {
    mockGoldenList = [{ id: 'arch-1', label: 'nightly', golden_label: 'v1.0 ref' }];
    mockGoldenRef = {
      label: 'nightly',
      snapshotCount: 4,
      // Matches session a's 'current' series exactly → pass for a;
      // session b runs +1 higher → fail at 5% relative tolerance.
      series: { current: [0, 1, 2, 3].map((t) => ({ t, v: t })), voltage: [] },
    };
    useCompareStore.setState({ goldenArchiveId: 'arch-1' });
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-mode-tab-golden'));
    expect(screen.getByTestId('compare-golden-picker')).toBeTruthy();
    expect(
      screen.getByTestId('compare-golden-verdict-current-a').textContent,
    ).toBe('pass');
    expect(
      screen.getByTestId('compare-golden-verdict-current-b').textContent,
    ).toBe('fail');
  });

  it('golden mode teaches when nothing is pinned yet', () => {
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-mode-tab-golden'));
    expect(screen.getByTestId('compare-golden-empty').textContent).toContain(
      'Mark Golden',
    );
    expect(screen.getByTestId('compare-golden-manage')).toBeTruthy();
  });
});
