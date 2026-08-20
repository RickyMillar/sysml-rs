/**
 * SneakPeek — render-state coverage for the read-only Monaco preview
 * embedded in the hover popup (S4.T5).
 *
 * The Monaco editor is mocked: a populated render reaches the wrapped
 * `<MonacoSysmlEditor>` element which is enough to assert wiring without
 * pulling the real monaco bundle into jsdom.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';

afterEach(() => cleanup());

// Stub MonacoSysmlEditor — keeps Monaco's browser bundle out of jsdom and
// lets us assert the props the hover popup hands down.
vi.mock('@/features/editor/MonacoSysmlEditor', () => ({
  MonacoSysmlEditor: ({
    value,
    readOnly,
    revealLineCol,
    height,
  }: {
    value: string;
    readOnly?: boolean;
    revealLineCol?: { line: number; col?: number };
    height?: string | number;
  }) => (
    <div
      data-testid="mock-monaco"
      data-readonly={String(!!readOnly)}
      data-height={String(height ?? '')}
      data-reveal-line={revealLineCol ? String(revealLineCol.line) : ''}
      data-reveal-col={revealLineCol?.col != null ? String(revealLineCol.col) : ''}
    >
      {value}
    </div>
  ),
}));

// Mock the get-source hook — controlled per-test via re-assigning the
// returned value before each render.
const mockUseGetSource = vi.fn();
vi.mock('@/features/editor/useGetSource', () => ({
  useGetSource: (...args: unknown[]) => mockUseGetSource(...args),
}));

import { SneakPeek } from '../SneakPeek';

function wrap(node: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>;
}

describe('SneakPeek', () => {
  it('renders nothing when uri or elementId is missing', () => {
    mockUseGetSource.mockReturnValue({ isLoading: false, isError: false, data: null });
    const { container } = render(wrap(<SneakPeek uri={null} elementId={null} />));
    expect(container.firstChild).toBe(null);
  });

  it('renders the loading state while the source query is in flight', () => {
    mockUseGetSource.mockReturnValue({ isLoading: true, isError: false, data: undefined });
    render(wrap(<SneakPeek uri="file:///a.sysml" elementId="E1" />));
    expect(screen.getByTestId('sneak-peek-loading')).toBeInTheDocument();
  });

  it('renders the error state when the query fails', () => {
    mockUseGetSource.mockReturnValue({ isLoading: false, isError: true, data: undefined });
    render(wrap(<SneakPeek uri="file:///a.sysml" elementId="E1" />));
    expect(screen.getByTestId('sneak-peek-error')).toBeInTheDocument();
  });

  it('renders the no-span placeholder when the backend returns null', () => {
    mockUseGetSource.mockReturnValue({ isLoading: false, isError: false, data: null });
    render(wrap(<SneakPeek uri="file:///a.sysml" elementId="E1" />));
    expect(screen.getByTestId('sneak-peek-no-span')).toBeInTheDocument();
  });

  it('mounts Monaco read-only with the slice text + reveal coordinates', () => {
    mockUseGetSource.mockReturnValue({
      isLoading: false,
      isError: false,
      data: { text: 'part p : P;', start: 0, end: 11, line: 4, col: 3 },
    });
    render(wrap(<SneakPeek uri="file:///a.sysml" elementId="E1" />));
    const monaco = screen.getByTestId('mock-monaco');
    expect(monaco).toHaveTextContent('part p : P;');
    expect(monaco).toHaveAttribute('data-readonly', 'true');
    expect(monaco).toHaveAttribute('data-reveal-line', '4');
    expect(monaco).toHaveAttribute('data-reveal-col', '3');
  });

  it('omits revealLineCol when the backend did not return line info', () => {
    mockUseGetSource.mockReturnValue({
      isLoading: false,
      isError: false,
      data: { text: 'x', start: 0, end: 1 },
    });
    render(wrap(<SneakPeek uri="file:///a.sysml" elementId="E1" />));
    const monaco = screen.getByTestId('mock-monaco');
    expect(monaco).toHaveAttribute('data-reveal-line', '');
  });
});
