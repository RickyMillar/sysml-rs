/**
 * PhasePill — state rendering per SessionPhase (ninebar Phase 1 frame
 * chips). Verifies the label/dot for every non-running phase, that
 * `running` swaps in the `<Ninebar/>` glyph instead of a dot, and that
 * `error` is the only phase carrying `--severity-error`.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import type { SessionPhase } from '@/features/sessions/types';

afterEach(cleanup);

let mockPhase: SessionPhase = 'idle';
vi.mock('@/features/sessions/store', () => ({
  useSessionStore: (selector: (s: { phase: SessionPhase }) => unknown) =>
    selector({ phase: mockPhase }),
}));

import { PhasePill } from '../PhasePill';

describe('PhasePill', () => {
  it.each<[SessionPhase, string]>([
    ['idle', 'Idle'],
    ['configuring', 'Configuring'],
    ['paused', 'Paused'],
    ['completed', 'Done'],
    ['error', 'Error'],
  ])('renders the %s label as "%s"', (phase, label) => {
    mockPhase = phase;
    render(<PhasePill />);
    const pill = screen.getByTestId('phase-pill');
    expect(pill).toHaveTextContent(label);
    expect(pill).toHaveAttribute('data-phase', phase);
  });

  it('renders a neutral dot for idle (no --severity-error color)', () => {
    mockPhase = 'idle';
    render(<PhasePill />);
    const dot = screen.getByTestId('phase-pill-dot');
    expect(dot).toBeInTheDocument();
    expect(dot.style.background).not.toContain('severity-error');
  });

  it('swaps the dot for the <Ninebar/> indeterminate glyph while running', () => {
    mockPhase = 'running';
    render(<PhasePill />);
    expect(screen.getByTestId('phase-pill')).toHaveTextContent('Running');
    expect(screen.queryByTestId('phase-pill-dot')).toBeNull();
    // Ninebar's indeterminate mode renders role="status".
    expect(screen.getByRole('status')).toBeInTheDocument();
  });

  it('colors the error phase with --severity-error, never amber', () => {
    mockPhase = 'error';
    render(<PhasePill />);
    const pill = screen.getByTestId('phase-pill');
    expect(pill.style.color).toContain('severity-error');
    expect(pill.style.color).not.toContain('accent');
  });
});
