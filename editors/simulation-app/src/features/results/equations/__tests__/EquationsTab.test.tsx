import { afterEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactElement } from 'react';
import { EquationsTab } from '../EquationsTab';
import type { ExpressionAstResult } from '@sysml-rs/expression-view';

const httpPost = vi.fn();
vi.mock('@/shared/api/http', () => ({ httpPost: (...args: unknown[]) => httpPost(...args) }));

vi.mock('@/components/cards/ExpressionViewReact', () => ({
  ExpressionViewReact: ({ source, testId }: { source: ExpressionAstResult; testId?: string }) => (
    <div data-testid={testId ?? 'expression-view'}>{source.source ?? source.element_name}</div>
  ),
}));

afterEach(() => {
  cleanup();
  httpPost.mockReset();
});

function renderTab(ui: ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

const results: ExpressionAstResult[] = [
  {
    element_id: 'constraint-1',
    element_name: 'limitTemp',
    element_kind: 'ConstraintUsage',
    source: 'temp < 350',
    ast: {
      kind: 'OperatorExpression',
      props: { operator: '<' },
      children: [
        { kind: 'FeatureReferenceExpression', name: 'temp', props: {}, children: [] },
        { kind: 'LiteralInteger', props: { value: 350 }, children: [] },
      ],
    },
  },
  {
    element_id: 'calc-1',
    element_name: 'powerCalc',
    element_kind: 'CalculationDefinition',
    source: 'v * i',
    ast: {
      kind: 'OperatorExpression',
      props: { operator: '*' },
      children: [
        { kind: 'FeatureReferenceExpression', name: 'v', props: {}, children: [] },
        { kind: 'FeatureReferenceExpression', name: 'i', props: {}, children: [] },
      ],
    },
  },
];

describe('EquationsTab', () => {
  it('renders searchable equation groups and detail', () => {
    renderTab(
      <EquationsTab
        results={results}
        timeSeries={{ temp: [{ t: 0, v: 300 }, { t: 1, v: 340 }] }}
        expanded
      />,
    );

    expect(screen.getByTestId('equations-tab')).toBeInTheDocument();
    expect(screen.getByText('Constraints (1)')).toBeInTheDocument();
    expect(screen.getByText('Calculations (1)')).toBeInTheDocument();
    expect(screen.getByTestId('equation-detail-source')).toHaveTextContent('temp < 350');
    expect(screen.getByText('340')).toBeInTheDocument();
  });

  it('filters equations by search query', () => {
    renderTab(<EquationsTab results={results} timeSeries={{}} expanded />);

    fireEvent.change(screen.getByTestId('equations-search'), { target: { value: 'power' } });
    expect(screen.getAllByText('powerCalc').length).toBeGreaterThan(0);
    expect(screen.queryByText('limitTemp')).toBeNull();
  });

  it('selects a different equation detail', () => {
    renderTab(<EquationsTab results={results} timeSeries={{ v: [{ t: 1, v: 12 }], i: [{ t: 1, v: 3 }] }} expanded />);

    fireEvent.click(screen.getByTestId('equation-select-calc-1'));
    const detail = screen.getByTestId('equation-detail');
    expect(within(detail).getByText('powerCalc')).toBeInTheDocument();
    expect(within(detail).getByText('12')).toBeInTheDocument();
    expect(within(detail).getByText('3')).toBeInTheDocument();
  });

  it('evaluates the selected equation with overrides', async () => {
    httpPost.mockResolvedValueOnce({
      element_id: 'constraint-1',
      verdict: 'fail',
      value: false,
      display: 'false',
      value_type: 'Bool',
      context: { temp: 400 },
      diagnostics: [],
    });

    renderTab(<EquationsTab results={results} timeSeries={{ temp: [{ t: 1, v: 340 }] }} uri="file://model.sysml" expanded />);

    fireEvent.change(screen.getByTestId('equation-override-temp'), { target: { value: '400' } });
    fireEvent.click(screen.getByTestId('equation-evaluate-run'));

    await waitFor(() => expect(httpPost).toHaveBeenCalled());
    expect(httpPost).toHaveBeenCalledWith('/api/command', {
      command: 'sysml.evaluate.expression',
      params: {
        element_id: 'constraint-1',
        overrides: [['temp', '400']],
      },
    });
    expect(await screen.findByTestId('equation-evaluate-result')).toHaveTextContent('false');
    expect(screen.getByTestId('equation-evaluate-result')).toHaveTextContent('Fail');
  });

  it('hides bare constants and hex-named noise by default, with a toggle to reveal them', () => {
    const noisy: ExpressionAstResult[] = [
      // Genuine equation — kept.
      {
        element_id: 'calc-1',
        element_name: 'powerCalc',
        element_kind: 'CalculationDefinition',
        source: 'v * i',
        ast: {
          kind: 'OperatorExpression',
          props: { operator: '*' },
          children: [
            { kind: 'FeatureReferenceExpression', name: 'v', props: {}, children: [] },
            { kind: 'FeatureReferenceExpression', name: 'i', props: {}, children: [] },
          ],
        },
      },
      // Bare constant ("maxTime_ms = 40") — noise.
      {
        element_id: 'const-1',
        element_name: 'maxTime_ms',
        element_kind: 'AttributeUsage',
        source: '40',
        ast: { kind: 'LiteralRational', props: { value: 40 }, children: [] },
      },
      // Hex-id name — noise even though it carries an operator expression.
      {
        element_id: 'hex-1',
        element_name: '71b9ccee',
        element_kind: 'AttributeUsage',
        source: 'x + 1',
        ast: {
          kind: 'OperatorExpression',
          props: { operator: '+' },
          children: [
            { kind: 'FeatureReferenceExpression', name: 'x', props: {}, children: [] },
            { kind: 'LiteralInteger', props: { value: 1 }, children: [] },
          ],
        },
      },
    ];

    renderTab(<EquationsTab results={noisy} timeSeries={{}} expanded />);

    // Default: only the genuine equation is listed; the toggle shows the count.
    expect(screen.getByTestId('equation-select-calc-1')).toBeInTheDocument();
    expect(screen.queryByTestId('equation-select-const-1')).toBeNull();
    expect(screen.queryByTestId('equation-select-hex-1')).toBeNull();
    const toggle = screen.getByTestId('equations-hide-noise');
    expect(toggle).toHaveTextContent('(2)');

    // Untick → all three appear.
    fireEvent.click(within(toggle).getByRole('checkbox'));
    expect(screen.getByTestId('equation-select-const-1')).toBeInTheDocument();
    expect(screen.getByTestId('equation-select-hex-1')).toBeInTheDocument();
  });
});
