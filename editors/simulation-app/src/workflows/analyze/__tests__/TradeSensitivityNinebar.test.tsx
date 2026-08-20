/**
 * Trade Study + Sensitivity ninebar bodies (Phase 5 W3).
 *
 * Pins the recomposition contract for both: the route body is the
 * viewer hero (no resident config column flag-on), and configuration
 * lives in a directly-rendered Modal that REUSES the existing editors /
 * config-hook state (threaded as props — see the bodies' doc comments
 * for why these two don't use the id registry).
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';

window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: true };

import { TradeStudyWorkflowNinebar } from '../tradestudy/TradeStudyWorkflowNinebar';
import { SensitivityWorkflowNinebar } from '../sensitivity/SensitivityWorkflowNinebar';

function renderBody(body: React.ReactElement, path: string) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[path]}>{body}</MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
});

describe('TradeStudyWorkflowNinebar', () => {
  it('mounts the hero empty state, not the legacy config column', () => {
    renderBody(<TradeStudyWorkflowNinebar />, '/analyze/trade-study');
    expect(screen.getByTestId('tradestudy-workflow-ninebar')).toBeTruthy();
    expect(screen.getByTestId('tradestudy-hero-empty')).toBeTruthy();
    expect(screen.queryByTestId('tradestudy-config')).toBeNull();
  });

  it('the CTA opens the config modal hosting the reused Alternatives/Criteria editors', () => {
    renderBody(<TradeStudyWorkflowNinebar />, '/analyze/trade-study');
    fireEvent.click(screen.getByTestId('tradestudy-hero-configure'));
    expect(screen.getByTestId('tradestudy-config-modal')).toBeTruthy();
    // The reused editors mount inside the modal (their own testids).
    expect(screen.getByTestId('modal-title').textContent).toBe('Configure trade study');
    // <2 alternatives → the validation reason renders.
    expect(screen.getByTestId('tradestudy-modal-validation')).toBeTruthy();
  });
});

describe('SensitivityWorkflowNinebar', () => {
  it('mounts the hero empty state, not the legacy config column', () => {
    renderBody(<SensitivityWorkflowNinebar />, '/analyze/sensitivity');
    expect(screen.getByTestId('sensitivity-workflow-ninebar')).toBeTruthy();
    expect(screen.getByTestId('sensitivity-hero-empty')).toBeTruthy();
    expect(screen.queryByTestId('sensitivity-config')).toBeNull();
  });

  it('the CTA opens the config modal; method pills flip the sampler knobs', () => {
    renderBody(<SensitivityWorkflowNinebar />, '/analyze/sensitivity');
    fireEvent.click(screen.getByTestId('sensitivity-hero-configure'));
    expect(screen.getByTestId('sensitivity-config-modal')).toBeTruthy();

    // Morris (default) shows r/p; Sobol swaps to n.
    expect(screen.getByTestId('sensitivity-modal-morris-r')).toBeTruthy();
    fireEvent.click(screen.getByTestId('sensitivity-modal-method-sobol'));
    expect(screen.queryByTestId('sensitivity-modal-morris-r')).toBeNull();
    expect(screen.getByTestId('sensitivity-modal-sobol-n')).toBeTruthy();

    // Free-form parameter add lands in the rail list + range rows.
    fireEvent.change(screen.getByTestId('sensitivity-modal-parameter-search'), {
      target: { value: 'I_residual' },
    });
    fireEvent.click(screen.getByTestId('sensitivity-modal-parameter-add'));
    expect(screen.getByTestId('sensitivity-modal-range-I_residual')).toBeTruthy();
  });
});
