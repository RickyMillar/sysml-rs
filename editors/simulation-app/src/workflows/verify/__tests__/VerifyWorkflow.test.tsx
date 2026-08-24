/**
 * Render tests for VerifyWorkflow (R3.1).
 *
 * Validates the three acceptance points from the R3.1 scope:
 *
 *   1. The workflow mounts at /verify.
 *   2. The empty-state results shell renders its "Select cases and
 *      click Run" copy when no run has produced verdicts yet.
 *   3. The Run button is disabled until at least one case is
 *      selected (i.e. with 0 selected it is inert).
 *
 * Also smoke-tests the suite selector so the config panel's wiring is
 * exercised once end-to-end.
 */

import { describe, it, expect, afterEach, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';

// These specs cover the LEGACY Verify body. ninebar is default-on since
// the flip — pin it off for this file (runtime override, no storage).
window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: false };

// Stub EmbeddedDiagram — Phase 6 added it as a shared sidebar that pulls
// in DiagramHost → the diagram canvas → sysml-layout (WASM). The WASM module
// has a Vite alias in dev/prod but no resolution path under jsdom, so
// the workflow-level test would fail to load the module graph. The
// behavioural assertions below don't depend on diagram contents.
vi.mock('@/components/diagram/EmbeddedDiagram', () => ({
  EmbeddedDiagram: () => null,
}));

import { VerifyWorkflow } from '../VerifyWorkflow';

function renderVerify() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
    },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/verify']}>
        <VerifyWorkflow />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
});

describe('VerifyWorkflow — mount', () => {
  it('mounts at /verify with the config panel and the results shell', () => {
    renderVerify();
    expect(screen.getByTestId('verify-workflow')).toBeInTheDocument();
    expect(screen.getByTestId('verify-config')).toBeInTheDocument();
    expect(screen.getByTestId('verify-results')).toBeInTheDocument();
  });

  it('surfaces the suite selector with every canonical suite option', () => {
    renderVerify();
    const select = screen.getByTestId('verify-suite-select') as HTMLSelectElement;
    const values = Array.from(select.options).map((o) => o.value);
    expect(values).toEqual([
      'evaluate_verification_cases',
      'evaluate_constraints',
      'evaluate_calculations',
    ]);
  });

});

describe('VerifyWorkflow — empty state', () => {
  it('renders the results empty-state with the "Select cases and click Run" copy', () => {
    renderVerify();
    const empty = screen.getByTestId('verify-results-empty');
    expect(empty).toBeInTheDocument();
    expect(empty).toHaveTextContent(/Select cases and click Run/i);
  });

  it('shows the no-workspace empty state for the case picker when workspace is unset', () => {
    renderVerify();
    expect(screen.getByTestId('verify-cases-no-workspace')).toBeInTheDocument();
  });
});

describe('VerifyWorkflow — Run button gating', () => {
  it('disables Run when zero cases are selected', () => {
    renderVerify();
    const run = screen.getByTestId('verify-run') as HTMLButtonElement;
    expect(run).toBeDisabled();
  });

  it('running summary reports 0 cases selected by default', () => {
    renderVerify();
    const summary = screen.getByTestId('verify-running-summary');
    // "0 cases selected" — count is first, matching the rendered template.
    expect(summary.textContent).toMatch(/0 cases selected/);
    // Suite default (Verification Cases) is echoed into the summary.
    expect(summary.textContent).toMatch(/Verification Cases/);
  });

  it('clicking the disabled Run button does not flip isRunning', () => {
    renderVerify();
    const run = screen.getByTestId('verify-run') as HTMLButtonElement;
    // Button is disabled → fireEvent.click is a no-op and the empty
    // state stays mounted (no skeleton, no spinner).
    fireEvent.click(run);
    expect(screen.getByTestId('verify-results-empty')).toBeInTheDocument();
  });
});
