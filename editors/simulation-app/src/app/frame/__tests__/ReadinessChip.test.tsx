/**
 * ReadinessChip — render-nothing-when-unknown, the three visible level
 * states (ready/warnings/errors), the drill popover, row selection, and
 * the "Verify workspace (static)" modal trigger (ninebar Phase 1.5).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReadinessSummary } from '@/features/readiness/types';

afterEach(cleanup);

let mockReadiness: ReadinessSummary;
const selectMock = vi.fn();
const openModalMock = vi.fn();

vi.mock('@/features/readiness/useModelReadiness', () => ({
  useModelReadiness: () => mockReadiness,
}));
vi.mock('@/features/selection/store', () => ({
  useSelectionStore: (selector: (s: unknown) => unknown) => selector({ select: selectMock }),
}));
vi.mock('@/shared/overlays/modalStore', () => ({
  useModalStore: (selector: (s: unknown) => unknown) => selector({ openModal: openModalMock }),
}));
vi.mock('@/features/readiness/StaticVerifyModal', () => ({
  READINESS_STATIC_VERIFY_MODAL_ID: 'readiness-static-verify',
}));

import { ReadinessChip } from '../ReadinessChip';

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

describe('ReadinessChip — render states', () => {
  it('renders nothing when level is unknown (no workspace loaded)', () => {
    mockReadiness = summary({ level: 'unknown' });
    const { container } = render(<ReadinessChip />);
    expect(container.firstChild).toBeNull();
  });

  it('shows a green-dot "Ready" chip when ready', () => {
    mockReadiness = summary({ level: 'ready' });
    render(<ReadinessChip />);
    const chip = screen.getByTestId('readiness-chip');
    expect(chip).toHaveAttribute('data-level', 'ready');
    expect(chip).toHaveTextContent('Ready');
    expect(screen.getByTestId('readiness-chip-dot').style.background).toContain('health-nominal');
  });

  it('shows an amber "n warnings" chip when warnings', () => {
    mockReadiness = summary({ level: 'warnings', counts: { errors: 0, warnings: 3 } });
    render(<ReadinessChip />);
    const chip = screen.getByTestId('readiness-chip');
    expect(chip).toHaveTextContent('3 warnings');
    expect(screen.getByTestId('readiness-chip-dot').style.background).toContain('severity-warning');
  });

  it('shows a red "n errors" chip when errors, folding unresolved deps into the count', () => {
    mockReadiness = summary({
      level: 'errors',
      counts: { errors: 2, warnings: 0 },
      unresolvedDeps: ['missing-pkg'],
    });
    render(<ReadinessChip />);
    const chip = screen.getByTestId('readiness-chip');
    expect(chip).toHaveTextContent('3 errors');
    expect(screen.getByTestId('readiness-chip-dot').style.background).toContain('severity-error');
  });

  it('singularizes "1 warning" / "1 error"', () => {
    mockReadiness = summary({ level: 'warnings', counts: { errors: 0, warnings: 1 } });
    render(<ReadinessChip />);
    expect(screen.getByTestId('readiness-chip')).toHaveTextContent('1 warning');
  });
});

describe('ReadinessChip — drill popover', () => {
  it('renders "No issues" when the drill list is empty', () => {
    mockReadiness = summary({ level: 'ready' });
    render(<ReadinessChip />);
    fireEvent.click(screen.getByTestId('readiness-chip'));
    expect(screen.getByTestId('readiness-drill-list')).toHaveTextContent('No issues');
  });

  it('lists file / severity / message drill rows', () => {
    mockReadiness = summary({
      level: 'errors',
      counts: { errors: 1, warnings: 0 },
      drill: [
        { file: 'file:///a.sysml', severity: 'error', message: 'unresolved reference' },
      ],
    });
    render(<ReadinessChip />);
    fireEvent.click(screen.getByTestId('readiness-chip'));
    const row = screen.getByTestId('readiness-drill-row-0');
    expect(row).toHaveTextContent('error');
    expect(row).toHaveTextContent('unresolved reference');
    expect(row).toHaveTextContent('a.sysml');
  });

  it('clicking a drill row selects the element via the shared selection store and closes the popover', () => {
    mockReadiness = summary({
      level: 'errors',
      counts: { errors: 1, warnings: 0 },
      drill: [
        { file: 'file:///a.sysml', severity: 'error', message: 'bad ref', elementId: 'el-1' },
      ],
    });
    render(<ReadinessChip />);
    fireEvent.click(screen.getByTestId('readiness-chip'));
    fireEvent.click(screen.getByTestId('readiness-drill-row-0'));
    expect(selectMock).toHaveBeenCalledWith('file:///a.sysml', 'el-1', 'ui');
    expect(screen.queryByTestId('readiness-drill-list')).toBeNull();
  });

  it('"Verify workspace (static)" opens the static verify modal and closes the popover', () => {
    mockReadiness = summary({ level: 'ready' });
    render(<ReadinessChip />);
    fireEvent.click(screen.getByTestId('readiness-chip'));
    fireEvent.click(screen.getByTestId('readiness-verify-action'));
    expect(openModalMock).toHaveBeenCalledWith('readiness-static-verify');
    expect(screen.queryByTestId('readiness-drill-list')).toBeNull();
  });
});
