/**
 * SweepWorkflowNinebar — flag-on Sweep surface tests (ninebar Phase 5).
 *
 * Pins the recomposition contract:
 *   1. The route body is the viewer hero — no resident config column
 *      (`sweep-config` aside must NOT mount flag-on).
 *   2. Config lives in the 'analyze-sweep-config' modal: the hero's
 *      empty-state CTA opens it via the modal registry, edits write the
 *      shared study store, and closing is enough (no apply step).
 *   3. The rail summary derives combinations from the store.
 *   4. `AnalyzeBatchStrip` helpers: status counts, verdict-failing
 *      split, promote-to-Compare pick order (failing first).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';

// ninebar is default-on; make it explicit so the suite stays pinned even
// if another suite in the same worker flips the runtime override off.
window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: true };

import { SweepWorkflowNinebar } from '../sweep/SweepWorkflowNinebar';
import { unwrapWireChild } from '../sweep/useSweepRunner';
import { useSweepStudyStore, expandStudyChildren } from '../sweep/useSweepStudyStore';
import {
  countByStatus,
  failingChildren,
  promotedSessionIds,
} from '../ninebar/AnalyzeBatchStrip';
import { ModalHost } from '@/shared/overlays/ModalHost';
import { useModalStore } from '@/shared/overlays/modalStore';
import type { ChildDescriptor, Verdict } from '@/engine/types';

function renderNinebarSweep() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/analyze/sweep']}>
        <SweepWorkflowNinebar />
        <ModalHost />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  act(() => {
    useSweepStudyStore.setState({ ranges: [], selectedMetricIds: [], runMode: 'parallel' });
    useModalStore.getState().closeModal();
  });
});

afterEach(() => {
  cleanup();
});

describe('SweepWorkflowNinebar — recomposed layout', () => {
  it('mounts the viewer hero with the empty teaching state, not the legacy config column', () => {
    renderNinebarSweep();
    expect(screen.getByTestId('sweep-workflow-ninebar')).toBeTruthy();
    expect(screen.getByTestId('sweep-hero-empty')).toBeTruthy();
    // The legacy resident config panel must not mount flag-on.
    expect(screen.queryByTestId('sweep-config')).toBeNull();
    expect(screen.queryByTestId('sweep-results-empty')).toBeNull();
  });

  it('the empty-state CTA opens the Configure sweep modal from the registry', () => {
    renderNinebarSweep();
    fireEvent.click(screen.getByTestId('sweep-hero-configure'));
    expect(screen.getByTestId('sweep-config-modal')).toBeTruthy();
    expect(screen.getByTestId('modal-title').textContent).toBe('Configure sweep');
  });

  it('modal edits write the shared store — closing is enough (no apply step)', () => {
    renderNinebarSweep();
    fireEvent.click(screen.getByTestId('sweep-hero-configure'));

    // Free-form add (no workspace loaded → no discovered candidates).
    fireEvent.change(screen.getByTestId('sweep-modal-parameter-search'), {
      target: { value: 'I_residual' },
    });
    fireEvent.click(screen.getByTestId('sweep-modal-parameter-add'));

    expect(useSweepStudyStore.getState().ranges).toHaveLength(1);
    expect(useSweepStudyStore.getState().ranges[0]!.parameterId).toBe('I_residual');
    // Default grid 0→1 step 0.25 = 5 combinations.
    expect(screen.getByTestId('sweep-modal-child-count').textContent).toContain('5');

    // Close — the store keeps the study; no apply step.
    fireEvent.click(screen.getByTestId('modal-close'));
    expect(screen.queryByTestId('sweep-config-modal')).toBeNull();
    expect(useSweepStudyStore.getState().ranges).toHaveLength(1);
  });

  it('rail summary reflects store-derived combinations', () => {
    act(() => {
      useSweepStudyStore.getState().addRange({
        parameterId: 'a',
        spec: { kind: 'list', values: [1, 2, 3] },
      });
      useSweepStudyStore.getState().addRange({
        parameterId: 'b',
        spec: { kind: 'list', values: [10, 20] },
      });
    });
    expect(expandStudyChildren(useSweepStudyStore.getState().ranges)).toHaveLength(6);
  });
});

describe('AnalyzeBatchStrip helpers', () => {
  const failVerdict: Verdict = {
    verdict: 'fail',
    actual: null,
    expected: null,
    margin: null,
    error: null,
    sensitivity: null,
    evidence: null,
    metadata: {},
    label: 'x',
  };

  const children: ChildDescriptor[] = [
    { session_id: 's1', index: 0, params: {}, status: 'complete', verdicts: [failVerdict] },
    { session_id: 's2', index: 1, params: {}, status: 'complete', verdicts: [] },
    { session_id: null, index: 2, params: {}, status: 'pending' },
    { session_id: 's4', index: 3, params: {}, status: 'failed' },
    { session_id: 's5', index: 4, params: {}, status: 'running' },
  ];

  it('counts children per execution status', () => {
    expect(countByStatus(children)).toEqual({ pending: 1, running: 1, complete: 2, failed: 1 });
  });

  it('splits verdict-failing (verdict ladder) from execution-failed', () => {
    const failing = failingChildren(children);
    expect(failing).toHaveLength(1);
    expect(failing[0]!.session_id).toBe('s1');
  });

  it('promotes failing children first, then completed, dropping session-less rows', () => {
    expect(promotedSessionIds(children)).toEqual(['s1', 's2']);
  });
});

describe('unwrapWireChild — sysml.batch.status wire contract', () => {
  // Live-caught (Phase 5 acceptance run): the backend serialises child
  // status as a serde-tagged OBJECT, which the sweep/sensitivity runners
  // stored verbatim — blank status cells, all-zero strip counts.
  it('unwraps the tag-wrapped object form', () => {
    const c = unwrapWireChild({ session_id: 's', index: 0, params: {}, status: { status: 'running' } });
    expect(c.status).toBe('running');
  });

  it('passes the flat string form through', () => {
    const c = unwrapWireChild({ session_id: 's', index: 0, params: {}, status: 'complete' });
    expect(c.status).toBe('complete');
  });

  it('warns + defaults to pending on contract drift', () => {
    const c = unwrapWireChild({ session_id: 's', index: 0, params: {}, status: { status: 'exploded' } });
    expect(c.status).toBe('pending');
  });
});
