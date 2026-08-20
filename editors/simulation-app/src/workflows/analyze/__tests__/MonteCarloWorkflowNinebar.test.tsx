/**
 * MonteCarloWorkflowNinebar — flag-on Monte Carlo surface tests
 * (ninebar Phase 5).
 *
 * Pins:
 *   1. The route body is the viewer hero — the legacy resident config
 *      column (`montecarlo-config`) must NOT mount flag-on.
 *   2. Config lives in the 'analyze-montecarlo-config' modal (registry),
 *      reusing DistributionEditor; edits write the shared study store
 *      and survive close (no apply step).
 *   3. The study store's derived validity + distribution summaries.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';

window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: true };

import { MonteCarloWorkflowNinebar, summariseDistribution } from '../montecarlo/MonteCarloWorkflowNinebar';
import { useMcStudyStore, mcStudyIsValid } from '../montecarlo/useMcStudyStore';
import { ModalHost } from '@/shared/overlays/ModalHost';
import { useModalStore } from '@/shared/overlays/modalStore';

function renderNinebarMc() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/analyze/montecarlo']}>
        <MonteCarloWorkflowNinebar />
        <ModalHost />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  act(() => {
    useMcStudyStore.setState({ distributions: {}, sampleCount: 10, seed: null });
    useModalStore.getState().closeModal();
  });
});

afterEach(() => {
  cleanup();
});

describe('MonteCarloWorkflowNinebar — recomposed layout', () => {
  it('mounts the viewer hero with the empty teaching state, not the legacy config column', () => {
    renderNinebarMc();
    expect(screen.getByTestId('montecarlo-workflow-ninebar')).toBeTruthy();
    expect(screen.getByTestId('mc-hero-empty')).toBeTruthy();
    expect(screen.queryByTestId('montecarlo-config')).toBeNull();
    expect(screen.queryByTestId('montecarlo-results-empty')).toBeNull();
  });

  it('the empty-state CTA opens the Configure Monte Carlo modal (reused DistributionEditor)', () => {
    renderNinebarMc();
    fireEvent.click(screen.getByTestId('mc-hero-configure'));
    expect(screen.getByTestId('mc-config-modal')).toBeTruthy();
    expect(screen.getByTestId('modal-title').textContent).toBe('Configure Monte Carlo');

    // Free-form add mounts the REUSED DistributionEditor for the param.
    fireEvent.change(screen.getByTestId('mc-modal-parameter-search'), {
      target: { value: 'I_residual' },
    });
    fireEvent.click(screen.getByTestId('mc-modal-parameter-add'));
    expect(Object.keys(useMcStudyStore.getState().distributions)).toEqual(['I_residual']);
    expect(screen.getByTestId('mc-modal-distributions')).toBeTruthy();

    // Closing keeps the study (no apply step).
    fireEvent.click(screen.getByTestId('modal-close'));
    expect(screen.queryByTestId('mc-config-modal')).toBeNull();
    expect(Object.keys(useMcStudyStore.getState().distributions)).toEqual(['I_residual']);
  });
});

describe('useMcStudyStore derivations', () => {
  it('a default normal distribution validates; empty study does not', () => {
    expect(mcStudyIsValid({})).toBe(false);
    act(() => useMcStudyStore.getState().addParameter('x'));
    expect(mcStudyIsValid(useMcStudyStore.getState().distributions)).toBe(true);
  });

  it('sample count clamps to [1, 20]', () => {
    act(() => useMcStudyStore.getState().setSampleCount(500));
    expect(useMcStudyStore.getState().sampleCount).toBe(20);
    act(() => useMcStudyStore.getState().setSampleCount(-3));
    expect(useMcStudyStore.getState().sampleCount).toBe(1);
  });

  it('summarises each distribution kind compactly for the rail', () => {
    expect(summariseDistribution({ kind: 'normal', mean: 0, sigma: 1 })).toBe('N(0, 1)');
    expect(summariseDistribution({ kind: 'uniform', min: 2, max: 4 })).toBe('U(2, 4)');
    expect(summariseDistribution({ kind: 'triangular', min: 0, mode: 1, max: 2 })).toBe('T(0, 1, 2)');
    expect(summariseDistribution({ kind: 'custom-cdf', raw: '', points: [] })).toBe('CDF · 0 pts');
  });
});
