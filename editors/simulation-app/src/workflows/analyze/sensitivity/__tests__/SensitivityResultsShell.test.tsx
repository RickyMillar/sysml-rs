import { afterEach, describe, it, expect } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import {
  SensitivityResultsShell,
  rankForSummary,
} from '../SensitivityResultsShell';
import type { SensitivityAnalyzeResult } from '@/engine/types';

describe('SensitivityResultsShell', () => {
  afterEach(cleanup);

  it('renders an empty-copy message when idle', () => {
    render(
      <SensitivityResultsShell
        batchId={null}
        children={[]}
        state="idle"
        results={null}
      />,
    );
    expect(screen.getByTestId('sensitivity-empty-copy')).toBeInTheDocument();
  });

  it('renders an error banner when state is error', () => {
    render(
      <SensitivityResultsShell
        batchId="abc"
        children={[]}
        state="error"
        results={null}
        error="boom"
      />,
    );
    expect(screen.getByTestId('sensitivity-error')).toHaveTextContent('boom');
  });

  it('renders a MorrisScatter when morris results arrive', () => {
    const results: SensitivityAnalyzeResult = {
      method: 'morris',
      parameters: [
        { name: 'a', mu: 3, sigma: 0.1 },
        { name: 'b', mu: 1, sigma: 0.05 },
      ],
    };
    render(
      <SensitivityResultsShell
        batchId="abc"
        children={[]}
        state="complete"
        results={results}
      />,
    );
    expect(screen.getByTestId('morris-scatter')).toBeInTheDocument();
    // Tornado shows both parameters.
    expect(screen.getByTestId('sensitivity-tornado-row-a')).toBeInTheDocument();
    expect(screen.getByTestId('sensitivity-tornado-row-b')).toBeInTheDocument();
  });

  it('renders a SobolBarChart when sobol results arrive', () => {
    const results: SensitivityAnalyzeResult = {
      method: 'sobol',
      parameters: [
        { name: 'x1', s1: 0.3, st: 0.5 },
        { name: 'x2', s1: 0.4, st: 0.4 },
      ],
    };
    render(
      <SensitivityResultsShell
        batchId="abc"
        children={[]}
        state="complete"
        results={results}
      />,
    );
    expect(screen.getByTestId('sobol-bar-chart')).toBeInTheDocument();
    expect(screen.getByTestId('sensitivity-tornado')).toHaveAttribute(
      'data-method',
      'sobol',
    );
  });
});

describe('rankForSummary', () => {
  it('sorts Morris rows by |μ*| descending', () => {
    const ranked = rankForSummary({
      method: 'morris',
      parameters: [
        { name: 'low', mu: 0.1, sigma: 0 },
        { name: 'high', mu: 2, sigma: 0 },
        { name: 'mid', mu: 1, sigma: 0 },
      ],
    });
    expect(ranked.map((r) => r.name)).toEqual(['high', 'mid', 'low']);
  });

  it('sorts Sobol rows by S_Ti descending', () => {
    const ranked = rankForSummary({
      method: 'sobol',
      parameters: [
        { name: 'a', s1: 0.2, st: 0.3 },
        { name: 'b', s1: 0.01, st: 0.5 },
      ],
    });
    expect(ranked.map((r) => r.name)).toEqual(['b', 'a']);
  });
});
