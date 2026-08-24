/**
 * Render tests for SweepWorkflow (R5.1).
 *
 * Validates the two required acceptance points from the R5.1 brief:
 *
 *   1. Empty state — with no workspace loaded, the workflow mounts
 *      with the config panel and the `sweep-results-empty` placeholder.
 *   2. Populated state — after seeding a config state via the
 *      `useSweepConfig` hook (simulated by picking a parameter through
 *      the free-entry input + editing its range), the Run button
 *      enables and the child-count summary reflects the cartesian
 *      expansion.
 *
 * Tests render against an empty React Query cache and an in-memory
 * router so `useWorkspaceUris` / `useRunTargets` resolve to no data
 * (they are gated on `workspaceRoot`, which defaults to null).
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';

// Stub EmbeddedDiagram — Phase 6 added it as a shared sidebar that pulls
// in DiagramHost → the diagram canvas → sysml-layout (WASM). The WASM module
// has a Vite alias in dev/prod but no resolution path under jsdom, so
// the workflow-level test would fail to load the module graph. The
// behavioural assertions below don't depend on diagram contents.
vi.mock('@/components/diagram/EmbeddedDiagram', () => ({
  EmbeddedDiagram: () => null,
}));

// These specs cover the LEGACY Sweep body. ninebar is default-on since
// the Phase 3 flip, so pin it OFF for this suite (Phase 4/5 test pattern).
window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: false };

import { SweepWorkflow } from '../SweepWorkflow';
import { useWorkspaceUIStore } from '@/features/workspace/store';

function renderSweep() {
  // Fresh client per test so query cache never leaks.
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/analyze/sweep']}>
        <SweepWorkflow />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  // Reset Zustand workspace store between tests so the "no workspace"
  // path is exercised reliably.
  act(() => {
    useWorkspaceUIStore.getState().setWorkspaceRoot(null);
  });
});

describe('SweepWorkflow — empty state', () => {
  it('mounts with the config panel and empty results shell', () => {
    renderSweep();
    expect(screen.getByTestId('sweep-workflow')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-config')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-results-empty')).toBeInTheDocument();
  });

  it('shows "Pick a parameter" hint when no ranges have been added', () => {
    renderSweep();
    expect(screen.getByTestId('sweep-ranges-empty')).toBeInTheDocument();
  });

  it('renders 0 runs and a disabled Run Sweep button', () => {
    renderSweep();
    const summary = screen.getByTestId('sweep-config-summary');
    expect(summary.textContent).toMatch(/0 runs/);
    const runBtn = screen.getByTestId('sweep-run') as HTMLButtonElement;
    expect(runBtn.disabled).toBe(true);
  });

  it('defaults to parallel run mode', () => {
    renderSweep();
    const parallel = screen.getByTestId('sweep-run-mode-parallel');
    const sequential = screen.getByTestId('sweep-run-mode-sequential');
    expect(parallel.getAttribute('data-active')).toBe('true');
    expect(sequential.getAttribute('data-active')).toBe('false');
  });
});

describe('SweepWorkflow — populated state', () => {
  it('adds a parameter via free-entry, shows the range editor, and updates child-count', () => {
    renderSweep();

    // Type a parameter name into the picker and press Enter — this is
    // the "free-entry" path (no candidates, no workspace loaded).
    const search = screen.getByTestId('sweep-parameter-search') as HTMLInputElement;
    // The input is disabled when no workspace is loaded. Flip the store
    // so it enables — the Run button still gates separately on
    // `hasWorkspace`, which is exercised in the next test.
    act(() => {
      useWorkspaceUIStore.getState().setWorkspaceRoot('/tmp/test-ws');
    });
    fireEvent.change(search, { target: { value: 'my_param' } });
    fireEvent.keyDown(search, { key: 'Enter' });

    // Range editor row rendered; summary reflects default 5-point grid
    // (min=0, max=1, step=0.25 → 5 samples).
    expect(screen.getByTestId('sweep-range-row-my_param')).toBeInTheDocument();
    const summary = screen.getByTestId('sweep-config-summary');
    expect(summary.textContent).toMatch(/5 runs/);
  });

  it('switching to a list spec and editing values reflects in the child-count', () => {
    renderSweep();
    act(() => {
      useWorkspaceUIStore.getState().setWorkspaceRoot('/tmp/test-ws');
    });
    const search = screen.getByTestId('sweep-parameter-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'p' } });
    fireEvent.keyDown(search, { key: 'Enter' });

    // Default is Grid → flip to List → set three values.
    fireEvent.click(screen.getByTestId('sweep-range-kind-list-p'));
    const valuesInput = screen.getByTestId('sweep-range-values-p') as HTMLInputElement;
    fireEvent.change(valuesInput, { target: { value: '1, 2, 3' } });

    const summary = screen.getByTestId('sweep-config-summary');
    expect(summary.textContent).toMatch(/3 runs/);
  });

  it('Run button enables once a workspace is loaded and at least one non-empty range exists', () => {
    renderSweep();
    // No workspace yet → button disabled, no config yet.
    const runBtn = () => screen.getByTestId('sweep-run') as HTMLButtonElement;
    expect(runBtn().disabled).toBe(true);

    act(() => {
      useWorkspaceUIStore.getState().setWorkspaceRoot('/tmp/test-ws');
    });
    // Still disabled — no ranges yet.
    expect(runBtn().disabled).toBe(true);

    const search = screen.getByTestId('sweep-parameter-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'p' } });
    fireEvent.keyDown(search, { key: 'Enter' });

    // Now: workspace loaded + a 5-point grid → Run is armed.
    expect(runBtn().disabled).toBe(false);
  });

  it('removing the only range disables Run again', () => {
    renderSweep();
    act(() => {
      useWorkspaceUIStore.getState().setWorkspaceRoot('/tmp/test-ws');
    });
    const search = screen.getByTestId('sweep-parameter-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'p' } });
    fireEvent.keyDown(search, { key: 'Enter' });
    expect((screen.getByTestId('sweep-run') as HTMLButtonElement).disabled).toBe(false);

    fireEvent.click(screen.getByTestId('sweep-range-remove-p'));
    expect(screen.getByTestId('sweep-ranges-empty')).toBeInTheDocument();
    expect((screen.getByTestId('sweep-run') as HTMLButtonElement).disabled).toBe(true);
  });

  it('run-mode toggle flips between parallel and sequential', () => {
    renderSweep();
    fireEvent.click(screen.getByTestId('sweep-run-mode-sequential'));
    expect(
      screen.getByTestId('sweep-run-mode-sequential').getAttribute('data-active'),
    ).toBe('true');
    expect(
      screen.getByTestId('sweep-run-mode-parallel').getAttribute('data-active'),
    ).toBe('false');
  });
});
