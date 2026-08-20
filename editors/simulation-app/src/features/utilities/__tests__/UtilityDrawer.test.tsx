import { afterEach, describe, expect, it } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { UtilityDrawer } from '../UtilityDrawer';
import { useBreakpointStore } from '@/features/breakpoints/useBreakpointStore';

function renderDrawer() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <UtilityDrawer />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  useBreakpointStore.getState().clearAll();
});

describe('UtilityDrawer', () => {
  it('exposes diagnostics, archive, breakpoints, and views as shell utilities', () => {
    renderDrawer();
    expect(screen.getByTestId('utility-toolbar')).toBeInTheDocument();
    expect(screen.getByTestId('utility-toggle-diagnostics')).toHaveTextContent('Diagnostics');
    expect(screen.getByTestId('utility-toggle-archive')).toHaveTextContent('Archive');
    expect(screen.getByTestId('utility-toggle-breakpoints')).toHaveTextContent('Breakpoints');
    expect(screen.getByTestId('utility-toggle-views')).toHaveTextContent('Views');
    expect(screen.queryByTestId('utility-toggle-outline')).toBeNull();
    expect(screen.queryByTestId('utility-drawer')).toBeNull();
  });

  it('opens and closes a utility drawer', () => {
    renderDrawer();
    fireEvent.click(screen.getByTestId('utility-toggle-breakpoints'));
    expect(screen.getByTestId('utility-drawer')).toHaveTextContent('Breakpoints');
    expect(screen.getByTestId('breakpoints-panel')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('utility-drawer-close'));
    expect(screen.queryByTestId('utility-drawer')).toBeNull();
  });

  it('shows a badge for armed breakpoints', () => {
    useBreakpointStore.getState().addLocal({
      breakpoint: {
        kind: 'state-entry',
        target: 'Engine.Running',
        label: 'Engine.Running',
      },
    });
    renderDrawer();
    expect(screen.getByTestId('utility-badge-breakpoints')).toHaveTextContent('1');
  });
});
