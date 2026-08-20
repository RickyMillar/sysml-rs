/**
 * Integration tests for VerifyWorkflowNinebar (design 1a).
 *
 * Asserts the consolidated navigation (Matrix · Cases · History · Report —
 * Aggregate retired), that opening a case (Cases tab, or matrix row
 * double-click) shows the case document, and that the right rail is dead
 * (no verdict-detail rail panel is ever registered/opened for Verify).
 *
 * Data hooks are mocked so the render is deterministic and network-free.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { Verdict } from '@/engine/types';
import type { VerificationCaseRow } from '../useVerificationCases';

// ── Mocks: the data layer + the networked timeline ───────────────────

const CASE: VerificationCaseRow = {
  case_id: 'c1',
  element_id: 'e1',
  case_name: 'ClauseFourReview',
  subject: 'bench',
  methods: [],
  evaluation_mode: 'static',
  verdict: 'Fail',
  display: 'FAIL (1/1 failed)',
  total_requirements: 1,
  requirements: [
    { requirement_id: 'protectionSpec', requirement_name: 'protectionSpec', requirement_element_id: 'r1', verdict: 'fail', message: 'nope' },
  ],
};

const VERDICT: Verdict = {
  verdict: 'fail',
  metadata: { case_id: 'c1', case_name: 'ClauseFourReview', requirement_id: 'protectionSpec', requirement_name: 'protectionSpec', evaluation_mode: 'static' },
};

vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (sel: (s: { workspaceRoot: string }) => unknown) => sel({ workspaceRoot: '/ws' }),
}));
vi.mock('@/features/packages/queries', () => ({
  useWorkspaceUris: () => ({ data: { uris: ['file://x.sysml'] } }),
}));
vi.mock('@/features/run-targets/queries', () => ({
  useRunTargets: () => ({ data: [], isLoading: false }),
}));
vi.mock('@/features/sessions/queries', () => ({
  useSessionList: () => ({ data: [] }),
}));
vi.mock('../runner/useVerifyRunner', () => ({
  useVerifyRunner: () => ({ state: 'idle', verdicts: [VERDICT], progress: null, run: vi.fn(), error: null, lastResult: null }),
}));
vi.mock('../useVerificationCases', async (orig) => ({
  ...(await orig<typeof import('../useVerificationCases')>()),
  useVerificationCases: () => ({ data: [CASE], isLoading: false, isError: false, isFetching: false, refetch: vi.fn() }),
}));
vi.mock('../VerdictTimelinePanel', () => ({
  VerdictTimelinePanel: (props: { testId?: string }) => <div data-testid={props.testId ?? 'verdict-timeline-panel'} />,
}));

import { VerifyWorkflowNinebar } from '../VerifyWorkflowNinebar';
import { LeftRailSlot, BottomStripSlot } from '@/app/slots';

function renderNinebar() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <VerifyWorkflowNinebar />
        <LeftRailSlot />
        <BottomStripSlot />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => cleanup());

describe('VerifyWorkflowNinebar — consolidated nav', () => {
  it('has the Matrix · Cases · History · Report sub-views and no Aggregate', () => {
    renderNinebar();
    expect(screen.getByTestId('verify-subview-matrix')).toBeInTheDocument();
    expect(screen.getByTestId('verify-subview-cases')).toBeInTheDocument();
    expect(screen.getByTestId('verify-subview-history')).toBeInTheDocument();
    expect(screen.getByTestId('verify-subview-report')).toBeInTheDocument();
    expect(screen.queryByTestId('verify-subview-aggregate')).not.toBeInTheDocument();
  });

  it('renders the suite header rollup from the static case read', () => {
    renderNinebar();
    expect(screen.getByTestId('verify-suite-rollup')).toHaveTextContent('1 fail');
  });

  it('opens the case document from the Cases tab', () => {
    renderNinebar();
    fireEvent.click(screen.getByTestId('verify-subview-cases'));
    expect(screen.getByTestId('verify-cases-list')).toBeInTheDocument();
    // No case selected yet → the empty prompt.
    expect(screen.getByTestId('verify-case-view-empty')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('verify-cases-row-ClauseFourReview'));
    expect(screen.getByTestId('verify-case-view')).toHaveAttribute('data-case-id', 'c1');
  });

  it('opens the case document on a matrix row double-click', () => {
    renderNinebar();
    fireEvent.doubleClick(screen.getByTestId('verdict-matrix-row-ClauseFourReview'));
    expect(screen.getByTestId('verify-case-view')).toHaveAttribute('data-case-id', 'c1');
  });

  it('never opens a verdict-detail right rail (the rail is dead in Verify)', () => {
    renderNinebar();
    fireEvent.click(screen.getByTestId('verify-subview-cases'));
    fireEvent.click(screen.getByTestId('verify-cases-row-ClauseFourReview'));
    expect(screen.queryByTestId('verify-verdict-detail')).not.toBeInTheDocument();
  });
});
