/**
 * ReadinessTeachingLine — the one-line "open Browse" nudge shown only
 * when the session is idle/no-target AND readiness is red (ninebar
 * Phase 1.5, audit F12).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import type { ReadinessSummary } from '@/features/readiness/types';
import type { SessionPhase } from '@/features/sessions/types';

afterEach(cleanup);

let mockPhase: SessionPhase;
let mockActiveSessionId: string | null;
let mockActiveSessionTarget: string | null;
let mockReadiness: ReadinessSummary;

vi.mock('@/features/sessions/store', () => ({
  useSessionStore: (selector: (s: unknown) => unknown) =>
    selector({ phase: mockPhase, activeSessionId: mockActiveSessionId }),
}));
vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (selector: (s: unknown) => unknown) =>
    selector({ activeSessionTarget: mockActiveSessionTarget }),
}));
vi.mock('@/features/readiness/useModelReadiness', () => ({
  useModelReadiness: () => mockReadiness,
}));

import { ReadinessTeachingLine } from '../ReadinessTeachingLine';

function summary(overrides: Partial<ReadinessSummary> = {}): ReadinessSummary {
  return {
    level: 'ready',
    counts: { errors: 0, warnings: 0 },
    unresolvedDeps: [],
    missingCapabilities: [],
    drill: [],
    ...overrides,
  };
}

function renderLine() {
  return render(
    <MemoryRouter>
      <ReadinessTeachingLine />
    </MemoryRouter>,
  );
}

describe('ReadinessTeachingLine', () => {
  it('renders nothing when idle/no-target but readiness is ready', () => {
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockReadiness = summary({ level: 'ready' });
    const { container } = renderLine();
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when readiness is errors but a session is already active', () => {
    mockPhase = 'running';
    mockActiveSessionId = 'sess-1';
    mockActiveSessionTarget = null;
    mockReadiness = summary({ level: 'errors', counts: { errors: 2, warnings: 0 } });
    const { container } = renderLine();
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when readiness is only warnings', () => {
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockReadiness = summary({ level: 'warnings', counts: { errors: 0, warnings: 1 } });
    const { container } = renderLine();
    expect(container.firstChild).toBeNull();
  });

  it('renders the teaching line + Browse link when idle/no-target and readiness is errors', () => {
    mockPhase = 'idle';
    mockActiveSessionId = null;
    mockActiveSessionTarget = null;
    mockReadiness = summary({ level: 'errors', counts: { errors: 2, warnings: 0 } });
    renderLine();
    const line = screen.getByTestId('readiness-teaching-line');
    expect(line).toHaveTextContent(/This model has diagnostics that fail at load/);
    expect(line).toHaveTextContent(/to review before running\./);
    const link = screen.getByTestId('readiness-teaching-line-link');
    expect(link).toHaveAttribute('href', '/browse');
  });
});
