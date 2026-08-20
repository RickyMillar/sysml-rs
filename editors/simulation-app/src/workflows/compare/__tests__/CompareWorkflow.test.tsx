/**
 * CompareWorkflow — mount-level smoke test.
 *
 * Goal: pick + playhead + gutter + mode slot all mount and render without
 * crashing when the archive list is stubbed. No real IndexedDB is touched.
 *
 * The SessionArchive module is mocked so `listArchivedSessions` and
 * `loadArchivedSession` return fixed data deterministically.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

// These specs cover the LEGACY Compare shell. ninebar is default-on
// since the Phase 3 flip, so pin it off for this suite (the flag-on
// surface is covered by CompareWorkflowNinebar.test.tsx).
window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: false };

// Mock must be set up before importing the module under test.
vi.mock('@/shared/data/SessionArchive', () => {
  const archive = [
    {
      id: 'sess-a',
      label: 'Session A',
      uri: 'file:///a.sysml',
      kind: 'simulation' as const,
      archivedAt: 1,
      tick: 4,
      timeMs: 40,
    },
    {
      id: 'sess-b',
      label: 'Session B',
      uri: 'file:///b.sysml',
      kind: 'simulation' as const,
      archivedAt: 2,
      tick: 3,
      timeMs: 30,
    },
  ];
  const fullRecords: Record<string, unknown> = {
    'sess-a': {
      id: 'sess-a',
      label: 'Session A',
      uri: 'file:///a.sysml',
      kind: 'simulation',
      archivedAt: 1,
      tick: 4,
      timeMs: 40,
      detail: {},
      topology: null,
      snapshotHistory: [
        { time_ms: 0, variables: { v: 0, w: 10 } },
        { time_ms: 10, variables: { v: 1, w: 9 } },
        { time_ms: 20, variables: { v: 2, w: 8 } },
        { time_ms: 30, variables: { v: 3, w: 7 } },
      ],
    },
    'sess-b': {
      id: 'sess-b',
      label: 'Session B',
      uri: 'file:///b.sysml',
      kind: 'simulation',
      archivedAt: 2,
      tick: 3,
      timeMs: 30,
      detail: {},
      topology: null,
      snapshotHistory: [
        { time_ms: 0, variables: { v: 0, w: 10 } },
        { time_ms: 10, variables: { v: 5, w: 5 } },
        { time_ms: 20, variables: { v: 10, w: 0 } },
      ],
    },
  };
  return {
    listArchivedSessions: vi.fn(async () => archive),
    loadArchivedSession: vi.fn(async (id: string) => fullRecords[id] ?? null),
  };
});

import {
  CompareWorkflow,
  __resetCompareModesForTesting,
  registerCompareMode,
} from '../CompareWorkflow';
import { useCompareStore } from '../useCompareStore';

function resetState() {
  __resetCompareModesForTesting();
  useCompareStore.setState({
    pickedSessionIds: [],
    sharedTick: 0,
    isPlaying: false,
    layout: null,
    activeModeId: null,
    pickedVariables: null,
  });
}

beforeEach(() => {
  resetState();
});

afterEach(() => {
  cleanup();
});

describe('CompareWorkflow — mount', () => {
  it('renders the picker, playhead, variable picker, and mode slot', async () => {
    render(<CompareWorkflow />);

    expect(screen.getByTestId('compare-workflow')).toBeInTheDocument();
    expect(screen.getByTestId('compare-session-picker')).toBeInTheDocument();
    expect(screen.getByTestId('shared-playhead')).toBeInTheDocument();
    expect(screen.getByTestId('compare-variable-picker')).toBeInTheDocument();
    expect(screen.getByTestId('compare-mode-config')).toBeInTheDocument();
    expect(screen.getByTestId('compare-mode-switcher')).toBeInTheDocument();
  });

  it('shows the placeholder mode when no modes are registered', () => {
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-mode-placeholder')).toBeInTheDocument();
  });

  it('shows "need more sessions" empty state until 2 picked', () => {
    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-need-more')).toBeInTheDocument();
  });
});

describe('CompareWorkflow — layout switching', () => {
  it('exposes both layout buttons with overlay active by default for 2 picks', () => {
    useCompareStore.getState().setPickedSessionIds(['sess-a', 'sess-b']);
    render(<CompareWorkflow />);
    const overlay = screen.getByTestId('compare-layout-overlay');
    const side = screen.getByTestId('compare-layout-side-by-side');
    expect(overlay).toHaveAttribute('data-active', 'true');
    expect(side).toHaveAttribute('data-active', 'false');
  });

  it('switches active chip when the user overrides layout', () => {
    useCompareStore.getState().setPickedSessionIds(['sess-a', 'sess-b']);
    render(<CompareWorkflow />);
    fireEvent.click(screen.getByTestId('compare-layout-side-by-side'));
    expect(useCompareStore.getState().layout).toBe('side-by-side');
  });
});

describe('CompareWorkflow — mode registry', () => {
  it('renders registered modes in the switcher and honors activeModeId', async () => {
    registerCompareMode({
      id: 'ensemble',
      label: 'Ensemble',
      description: 'Compare N sessions as an ensemble.',
      configRender: () => <div data-testid="mode-ensemble-config">ensemble cfg</div>,
    });
    registerCompareMode({
      id: 'golden',
      label: 'Golden',
      description: 'Diff every run against a designated golden session.',
      configRender: () => <div data-testid="mode-golden-config">golden cfg</div>,
    });

    render(<CompareWorkflow />);
    expect(screen.getByTestId('compare-mode-ensemble')).toBeInTheDocument();
    expect(screen.getByTestId('compare-mode-golden')).toBeInTheDocument();

    // First registered mode is active by default.
    expect(screen.getByTestId('mode-ensemble-config')).toBeInTheDocument();

    // Switch mode through the switcher → config swaps.
    await act(async () => {
      fireEvent.click(screen.getByTestId('compare-mode-golden'));
    });
    expect(screen.getByTestId('mode-golden-config')).toBeInTheDocument();
  });

  it('delegates mainRender to the active mode when provided', async () => {
    registerCompareMode({
      id: 'ensemble',
      label: 'Ensemble',
      description: 'Compare N sessions as an ensemble.',
      configRender: () => null,
      mainRender: () => <div data-testid="ensemble-main">ensemble main</div>,
    });

    useCompareStore.getState().setPickedSessionIds(['sess-a', 'sess-b']);
    render(<CompareWorkflow />);

    // Wait a frame so the async loadArchivedSession resolves.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByTestId('compare-main-custom')).toBeInTheDocument();
    expect(screen.getByTestId('ensemble-main')).toBeInTheDocument();
  });
});
