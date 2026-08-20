import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { VerifyResultsShell } from '../VerifyResultsShell';
import type { Verdict } from '@/engine/types';

const navigate = vi.fn();
const httpPost = vi.fn();
vi.mock('@/shared/api/http', () => ({ httpPost: (...args: unknown[]) => httpPost(...args) }));
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => navigate };
});

afterEach(() => {
  cleanup();
  navigate.mockClear();
  httpPost.mockReset();
});

const verdicts: Verdict[] = [
  {
    verdict: 'pass',
    id: 'r1',
    label: 'Requirement 1',
    actual: 9,
    expected: 10,
    margin: -1,
    metadata: { case_name: 'Case A', requirement_id: 'REQ-1' },
  },
  {
    verdict: 'fail',
    id: 'r2',
    label: 'Requirement 2',
    actual: 12,
    expected: 10,
    margin: 2,
    reason: 'too high',
    evidence: { session_id: 'sess-1', tick: 7, element_id: 'el-1' },
    metadata: {
      case_name: 'Case A',
      requirement_id: 'REQ-2',
      element_id: 'constraint-2',
      methods: ['test', 'demo'],
      uri: 'file:///vehicle.sysml',
      requirements: [
        {
          requirement_id: 'REQ-2',
          requirement_text: 'Temperature shall remain below limit',
          verdict: 'fail',
          message: 'failed: constraint[0]',
          constraints: [{ expression: 'temp < 90', satisfied: false }],
        },
      ],
    },
  },
];

function renderShell(vs = verdicts) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <VerifyResultsShell verdicts={vs} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe('VerifyResultsShell', () => {
  it('renders the inspectable workbench and selects the failing verdict by default', () => {
    renderShell();
    expect(screen.getByTestId('verify-results-workbench')).toBeInTheDocument();
    expect(screen.getByTestId('verify-results-matrix')).toBeInTheDocument();
    const detail = screen.getByTestId('verify-verdict-detail');
    expect(within(detail).getByText('Requirement 2')).toBeInTheDocument();
    expect(within(detail).getByText('too high')).toBeInTheDocument();
    expect(within(detail).getByText('12')).toBeInTheDocument();
    expect(within(detail).getByText('10')).toBeInTheDocument();
    expect(within(detail).getByText('Temperature shall remain below limit')).toBeInTheDocument();
    expect(within(detail).getAllByText(/temp < 90/).length).toBeGreaterThan(0);
  });

  it('shows the declared method line for cases that declare one (B4)', () => {
    renderShell();
    const method = screen.getByTestId('verify-verdict-method');
    expect(method.textContent).toBe('declared: test · demo');
    expect(method.getAttribute('title')).toContain('model intent');
  });

  it('drills to run evidence when evidence is present', () => {
    renderShell();
    fireEvent.click(screen.getByTestId('verify-verdict-drill'));
    expect(navigate).toHaveBeenCalledWith('/run?session=sess-1&tick=7&element=el-1');
  });

  it('offers tree/equation actions and evaluates the selected expression', async () => {
    httpPost.mockResolvedValueOnce({
      element_id: 'constraint-2',
      display: 'false',
      value: false,
      verdict: 'fail',
    });

    renderShell();

    fireEvent.click(screen.getByTestId('verify-action-show-tree'));
    expect(navigate).toHaveBeenCalledWith('/run?session=sess-1&tick=7&element=constraint-2');

    fireEvent.click(screen.getByTestId('verify-action-open-equation'));
    expect(navigate).toHaveBeenCalledWith('/run?session=sess-1&tick=7&element=constraint-2&result_tab=equations&equation=constraint-2');

    fireEvent.click(screen.getByTestId('verify-action-evaluate-expression'));
    await waitFor(() => expect(httpPost).toHaveBeenCalledWith('/api/command', {
      command: 'sysml.evaluate.expression',
      params: { element_id: 'constraint-2', overrides: [] },
    }));
    expect(await screen.findByTestId('verify-action-evaluate-result')).toHaveTextContent('false');
  });

  it('renders the empty state unchanged for no verdicts', () => {
    renderShell([]);
    expect(screen.getByTestId('verify-results-empty')).toHaveTextContent(/Select cases and click Run/i);
  });
});
