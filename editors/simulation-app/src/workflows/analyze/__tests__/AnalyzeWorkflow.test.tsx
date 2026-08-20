/**
 * AnalyzeWorkflow shell — Phase 6 contract test.
 *
 * Pins the tab nav: every analysis mode the codebase ships
 * (Cases / Sweep / Monte Carlo / Trade Study / Sensitivity) is
 * addressable as a `/analyze/<mode>` route and visually flips
 * `aria-current`-style styling when the current pathname matches.
 *
 * Tabs are plain react-router `<Link>`s, so we lean on `MemoryRouter`
 * to drive the pathname and inspect the rendered chips.
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';

// These specs cover the LEGACY Analyze shell. ninebar is default-on since
// the Phase 3 flip, so pin it OFF for this suite (Phase 4/5 test pattern).
window.__sysmlFlags = { ...(window.__sysmlFlags ?? {}), ninebar: false };

import { AnalyzeWorkflow } from '../AnalyzeWorkflow';

afterEach(() => {
  cleanup();
});

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/analyze" element={<AnalyzeWorkflow />}>
          <Route index element={<div data-testid="child-cases">cases</div>} />
          <Route path="sweep" element={<div data-testid="child-sweep">sweep</div>} />
          <Route
            path="montecarlo"
            element={<div data-testid="child-montecarlo">mc</div>}
          />
          <Route
            path="trade-study"
            element={<div data-testid="child-trade-study">trade</div>}
          />
          <Route
            path="sensitivity"
            element={<div data-testid="child-sensitivity">sens</div>}
          />
        </Route>
      </Routes>
    </MemoryRouter>,
  );
}

const MODE_IDS = ['cases', 'sweep', 'montecarlo', 'trade-study', 'sensitivity'];

describe('AnalyzeWorkflow shell', () => {
  it('renders all five analysis-mode tabs', () => {
    renderAt('/analyze');
    expect(screen.getByTestId('analyze-mode-tabs')).toBeInTheDocument();
    for (const id of MODE_IDS) {
      expect(screen.getByTestId(`analyze-mode-${id}`)).toBeInTheDocument();
    }
  });

  it('mounts the index child (Cases landing) when path is /analyze', () => {
    renderAt('/analyze');
    expect(screen.getByTestId('child-cases')).toBeInTheDocument();
    expect(screen.queryByTestId('child-sweep')).not.toBeInTheDocument();
  });

  it.each([
    ['sweep', '/analyze/sweep'],
    ['montecarlo', '/analyze/montecarlo'],
    ['trade-study', '/analyze/trade-study'],
    ['sensitivity', '/analyze/sensitivity'],
  ])('mounts the %s child at %s', (mode, path) => {
    renderAt(path);
    expect(screen.getByTestId(`child-${mode}`)).toBeInTheDocument();
    // Sibling children are not rendered.
    for (const other of MODE_IDS.filter((m) => m !== mode && m !== 'cases')) {
      expect(screen.queryByTestId(`child-${other}`)).not.toBeInTheDocument();
    }
  });

  it('flags the active tab via inline style — primary-container background', () => {
    renderAt('/analyze/montecarlo');
    const active = screen.getByTestId('analyze-mode-montecarlo');
    // The shell renders the active tab with `var(--primary-container)`; the
    // others fall back to `var(--surface-container)`. Pin the contract via
    // the inline style attribute string — it's the only signal the shell
    // emits today, since there's no aria-current.
    expect(active.getAttribute('style') ?? '').toContain('primary-container');
    const inactive = screen.getByTestId('analyze-mode-sweep');
    expect(inactive.getAttribute('style') ?? '').toContain('surface-container');
    expect(inactive.getAttribute('style') ?? '').not.toContain(
      'primary-container',
    );
  });

  it('each tab links to its absolute /analyze/<mode> path', () => {
    renderAt('/analyze');
    expect(screen.getByTestId('analyze-mode-cases')).toHaveAttribute(
      'href',
      '/analyze',
    );
    expect(screen.getByTestId('analyze-mode-sweep')).toHaveAttribute(
      'href',
      '/analyze/sweep',
    );
    expect(screen.getByTestId('analyze-mode-montecarlo')).toHaveAttribute(
      'href',
      '/analyze/montecarlo',
    );
    expect(screen.getByTestId('analyze-mode-trade-study')).toHaveAttribute(
      'href',
      '/analyze/trade-study',
    );
    expect(screen.getByTestId('analyze-mode-sensitivity')).toHaveAttribute(
      'href',
      '/analyze/sensitivity',
    );
  });
});
