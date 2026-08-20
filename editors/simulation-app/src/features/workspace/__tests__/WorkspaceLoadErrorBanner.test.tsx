/**
 * WorkspaceLoadErrorBanner — Bucket 5-followup (2026-05-05).
 *
 * Pins the banner contract: hidden when no errors, visible when
 * `error_count > 0`, dismissible.
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { WorkspaceLoadErrorBanner } from '@/features/workspace/WorkspaceLoadErrorBanner';

afterEach(() => {
  cleanup();
  useWorkspaceUIStore.setState({ loadStatus: null });
});

function renderBanner() {
  return render(
    <MemoryRouter>
      <WorkspaceLoadErrorBanner />
    </MemoryRouter>,
  );
}

describe('WorkspaceLoadErrorBanner', () => {
  it('renders nothing when loadStatus is null', () => {
    renderBanner();
    expect(screen.queryByTestId('workspace-load-error-banner')).toBeNull();
  });

  it('renders nothing when errorCount is 0', () => {
    useWorkspaceUIStore.setState({
      loadStatus: { errorCount: 0, errors: [], dismissed: false },
    });
    renderBanner();
    expect(screen.queryByTestId('workspace-load-error-banner')).toBeNull();
  });

  it('renders nothing when dismissed', () => {
    useWorkspaceUIStore.setState({
      loadStatus: {
        errorCount: 1,
        errors: ['x.sysml:1: bad'],
        dismissed: true,
      },
    });
    renderBanner();
    expect(screen.queryByTestId('workspace-load-error-banner')).toBeNull();
  });

  it('shows the count, the diagnostics link, and the first error preview', () => {
    useWorkspaceUIStore.setState({
      loadStatus: {
        errorCount: 2,
        errors: [
          '/a/views.sysml:81: syntax error: expected …',
          '/a/connections.sysml:29: syntax error: expected …',
        ],
        dismissed: false,
      },
    });
    renderBanner();
    expect(screen.getByTestId('workspace-load-error-count').textContent).toMatch(
      /2 files? failed to parse/,
    );
    expect(screen.getByTestId('workspace-load-error-link')).toBeTruthy();
    expect(
      screen.getByTestId('workspace-load-error-banner').textContent,
    ).toContain('/a/views.sysml:81');
  });

  it('dismisses on click', async () => {
    useWorkspaceUIStore.setState({
      loadStatus: {
        errorCount: 1,
        errors: ['x.sysml:1: bad'],
        dismissed: false,
      },
    });
    renderBanner();
    await userEvent.click(screen.getByTestId('workspace-load-error-dismiss'));
    expect(screen.queryByTestId('workspace-load-error-banner')).toBeNull();
    expect(useWorkspaceUIStore.getState().loadStatus?.dismissed).toBe(true);
  });

  it('uses singular phrasing for one error', () => {
    useWorkspaceUIStore.setState({
      loadStatus: {
        errorCount: 1,
        errors: ['x.sysml:1: bad'],
        dismissed: false,
      },
    });
    renderBanner();
    expect(screen.getByTestId('workspace-load-error-count').textContent).toMatch(
      /1 file failed to parse/,
    );
  });
});
