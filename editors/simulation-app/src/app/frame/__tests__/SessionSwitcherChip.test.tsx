/**
 * SessionSwitcherChip — trigger label, popover list rendering off mocked
 * `sessions.list` query data, expired dimming, session selection, and
 * the "Clear stale" reap action (ninebar Phase 1 frame chips, audit F2).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import type { SessionSummary } from '@/features/sessions/types';

afterEach(cleanup);

let mockSummaries: SessionSummary[] = [];
let mockActiveSessionId: string | null = null;
const setActiveSession = vi.fn();
const reapMutate = vi.fn();
let mockReapPending = false;

vi.mock('@/features/sessions/queries', () => ({
  useSessionList: () => ({ data: mockSummaries }),
}));
vi.mock('@/features/sessions/mutations', () => ({
  useReapSessions: () => ({ mutate: reapMutate, isPending: mockReapPending }),
}));
vi.mock('@/features/sessions/store', () => ({
  useSessionStore: (selector: (s: unknown) => unknown) =>
    selector({ activeSessionId: mockActiveSessionId, setActiveSession }),
}));

import { SessionSwitcherChip } from '../SessionSwitcherChip';

function summary(overrides: Partial<SessionSummary>): SessionSummary {
  return {
    id: 'sess-default',
    kind: 'simulation',
    uri: 'file:///a.sysml',
    subsystem_name: null,
    label: null,
    created_at_ms: 0,
    elapsed_ms: 0,
    tick: 0,
    time_ms: 0,
    current_state: null,
    completed: false,
    is_expired: false,
    history_len: 0,
    subsystem_count: 0,
    fork_point_tick: null,
    paused: false,
    ticks_advanced: 0,
    ...overrides,
  };
}

describe('SessionSwitcherChip — trigger', () => {
  it('shows "no session" quiet text when nothing is active', () => {
    mockSummaries = [];
    mockActiveSessionId = null;
    render(<MemoryRouter><SessionSwitcherChip /></MemoryRouter>);
    expect(screen.getByTestId('session-switcher-chip')).toHaveTextContent('no session');
  });

  it('shows the active session short id + kind when active', () => {
    mockSummaries = [summary({ id: 'abcdefgh1234', kind: 'action' })];
    mockActiveSessionId = 'abcdefgh1234';
    render(<MemoryRouter><SessionSwitcherChip /></MemoryRouter>);
    const chip = screen.getByTestId('session-switcher-chip');
    expect(chip).toHaveTextContent('abcdefgh');
    expect(chip).toHaveTextContent('action');
  });
});

describe('SessionSwitcherChip — popover list', () => {
  it('lists sessions with kind + status badges, dims expired rows', () => {
    mockSummaries = [
      summary({ id: 'live-1', kind: 'simulation', is_expired: false }),
      summary({ id: 'gone-1', kind: 'orchestrator', is_expired: true }),
    ];
    mockActiveSessionId = 'live-1';
    render(<MemoryRouter><SessionSwitcherChip /></MemoryRouter>);

    fireEvent.click(screen.getByTestId('session-switcher-chip'));

    const liveRow = screen.getByTestId('session-switcher-row-live-1');
    expect(liveRow).toHaveTextContent('simulation');
    expect(liveRow).toHaveTextContent('active');

    const expiredRow = screen.getByTestId('session-switcher-row-gone-1');
    expect(expiredRow).toHaveTextContent('orchestrator');
    expect(expiredRow).toHaveTextContent('expired');
    expect(expiredRow.style.opacity).toBe('0.6');
  });

  it('renders "no session" inside the popover when the list is empty', () => {
    mockSummaries = [];
    mockActiveSessionId = null;
    render(<MemoryRouter><SessionSwitcherChip /></MemoryRouter>);
    fireEvent.click(screen.getByTestId('session-switcher-chip'));
    expect(screen.getByTestId('session-switcher-list')).toHaveTextContent('no session');
  });

  it('selecting a row calls setActiveSession and closes the popover', () => {
    mockSummaries = [summary({ id: 'pick-me', kind: 'simulation' })];
    mockActiveSessionId = null;
    render(<MemoryRouter><SessionSwitcherChip /></MemoryRouter>);
    fireEvent.click(screen.getByTestId('session-switcher-chip'));
    fireEvent.click(screen.getByTestId('session-switcher-row-pick-me'));
    expect(setActiveSession).toHaveBeenCalledWith('pick-me');
    expect(screen.queryByTestId('session-switcher-list')).toBeNull();
  });

  it('"Clear stale" fires the reap mutation', () => {
    mockSummaries = [summary({ id: 's1' })];
    mockActiveSessionId = null;
    render(<MemoryRouter><SessionSwitcherChip /></MemoryRouter>);
    fireEvent.click(screen.getByTestId('session-switcher-chip'));
    fireEvent.click(screen.getByTestId('session-switcher-clear-stale'));
    expect(reapMutate).toHaveBeenCalled();
  });
});
