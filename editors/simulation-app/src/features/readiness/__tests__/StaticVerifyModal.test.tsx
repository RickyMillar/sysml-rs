/**
 * StaticVerifyModal — fires `sysml.workspace.verify` on mount, renders
 * the pending/error/result states, and — the plan's hard requirement —
 * always carries a persistent "Static / pre-run" label so this is never
 * mistaken for live run evidence (ninebar Phase 1.5).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, fireEvent } from '@testing-library/react';
import type { WorkspaceVerifyResult } from '@/features/packages/queries';

afterEach(cleanup);

const mutateMock = vi.fn();
let mockState: {
  isPending: boolean;
  isError: boolean;
  error: unknown;
  data: WorkspaceVerifyResult | undefined;
};

vi.mock('@/features/packages/queries', () => ({
  useWorkspaceVerify: () => ({ mutate: mutateMock, ...mockState }),
}));

import { StaticVerifyModal, STATIC_PRE_RUN_LABEL } from '../StaticVerifyModal';

function setState(overrides: Partial<typeof mockState>) {
  mockState = { isPending: false, isError: false, error: null, data: undefined, ...overrides };
}

describe('StaticVerifyModal', () => {
  it('fires the verify mutation once on mount', () => {
    setState({});
    mutateMock.mockClear();
    render(<StaticVerifyModal />);
    expect(mutateMock).toHaveBeenCalledTimes(1);
  });

  it('always renders the "Static / pre-run" label, in every state', () => {
    setState({ isPending: true });
    render(<StaticVerifyModal />);
    expect(screen.getByTestId('static-verify-label')).toHaveTextContent(STATIC_PRE_RUN_LABEL);
  });

  it('shows the Ninebar indeterminate glyph while pending', () => {
    setState({ isPending: true });
    render(<StaticVerifyModal />);
    expect(screen.getByTestId('static-verify-loading')).toBeInTheDocument();
    expect(screen.getByRole('status', { name: 'running static verification' })).toBeInTheDocument();
  });

  it('shows an error message on failure', () => {
    setState({ isError: true, error: new Error('backend unreachable') });
    render(<StaticVerifyModal />);
    expect(screen.getByTestId('static-verify-error')).toHaveTextContent('backend unreachable');
  });

  it('renders a pass VerdictBadge + counts when every case passed', () => {
    setState({ data: { total_cases: 5, passed: 5, failed: 0, elapsed_ms: 1234, per_file: [] } });
    render(<StaticVerifyModal />);
    expect(screen.getByTestId('verdict-badge-pass')).toBeInTheDocument();
    expect(screen.getByTestId('static-verify-result')).toHaveTextContent('5/5 cases passed');
  });

  it('renders a fail VerdictBadge + affected-file list when some cases failed', () => {
    setState({
      data: {
        total_cases: 5,
        passed: 3,
        failed: 2,
        elapsed_ms: 500,
        per_file: ['file:///a.sysml', 'file:///b.sysml'],
      },
    });
    render(<StaticVerifyModal />);
    expect(screen.getByTestId('verdict-badge-fail')).toBeInTheDocument();
    expect(screen.getByTestId('static-verify-result')).toHaveTextContent('2 failed');
    expect(screen.getAllByTestId('static-verify-file-row')).toHaveLength(2);
  });

  it('renders an inconclusive VerdictBadge when there are no verification cases', () => {
    setState({ data: { total_cases: 0, passed: 0, failed: 0, elapsed_ms: 10, per_file: [] } });
    render(<StaticVerifyModal />);
    expect(screen.getByTestId('verdict-badge-inconclusive')).toBeInTheDocument();
  });

  it('"Re-run" fires the mutation again', () => {
    setState({ data: { total_cases: 1, passed: 1, failed: 0, elapsed_ms: 10, per_file: [] } });
    render(<StaticVerifyModal />);
    mutateMock.mockClear(); // drop the mount-time call
    fireEvent.click(screen.getByTestId('static-verify-rerun'));
    expect(mutateMock).toHaveBeenCalledTimes(1);
  });
});
