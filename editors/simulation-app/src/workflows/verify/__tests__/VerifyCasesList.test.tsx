/**
 * Tests for VerifyCasesList — the Cases sub-view list (design 1a).
 *
 * One row per case: a verdict glyph (verdict colour lives ONLY here),
 * the name, a check count, and the neutral mode glyph. Selection + the
 * empty states are honest.
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { VerifyCasesList } from '../VerifyCasesList';
import type { VerificationCaseRow } from '../useVerificationCases';

const CASES: VerificationCaseRow[] = [
  { case_id: 'c1', case_name: 'ClauseFourReview', verdict: 'Fail', evaluation_mode: 'static', total_requirements: 1 },
  { case_id: 'c2', case_name: 'CreepageInspection', verdict: 'Pass', evaluation_mode: 'static', total_requirements: 2 },
  { case_id: 'c3', case_name: 'BareCase', verdict: 'Inconclusive', evaluation_mode: 'static', total_requirements: 0 },
];

function renderList(props: Partial<React.ComponentProps<typeof VerifyCasesList>> = {}) {
  const defaults = {
    cases: CASES,
    hasWorkspace: true,
    selectedCaseId: null,
    onSelectCase: vi.fn(),
  };
  const merged = { ...defaults, ...props };
  render(<VerifyCasesList {...merged} />);
  return merged;
}

afterEach(() => cleanup());

describe('VerifyCasesList', () => {
  it('renders a row per case with the verdict glyph, req count, and mode glyph', () => {
    renderList();
    const row = screen.getByTestId('verify-cases-row-CreepageInspection');
    expect(within(row).getByLabelText('verdict: pass')).toHaveAttribute('data-verdict', 'pass');
    expect(row).toHaveTextContent('2 req');
    expect(within(row).getByLabelText('evaluation mode: static')).toHaveTextContent('=');
  });

  it('mints no verdict glyph for a bare objective (1e)', () => {
    renderList();
    const bare = screen.getByTestId('verify-cases-row-BareCase');
    expect(within(bare).getByLabelText('no verdict — bare objective')).toBeInTheDocument();
    expect(within(bare).queryByLabelText(/verdict:/)).not.toBeInTheDocument();
  });

  it('selects a case on click', () => {
    const props = renderList();
    fireEvent.click(screen.getByTestId('verify-cases-row-ClauseFourReview'));
    expect(props.onSelectCase).toHaveBeenCalledWith('c1');
  });

  it('marks the selected row', () => {
    renderList({ selectedCaseId: 'c1' });
    expect(screen.getByTestId('verify-cases-row-ClauseFourReview')).toHaveAttribute('data-selected', 'true');
  });

  it('shows the model digest in the footer when present', () => {
    renderList({ modelDigest: 'c7e2a1abcdef' });
    expect(screen.getByTestId('verify-cases-list')).toHaveTextContent('c7e2a1');
  });

  it('renders honest empty states', () => {
    renderList({ hasWorkspace: false });
    expect(screen.getByText('No workspace loaded')).toBeInTheDocument();
    cleanup();
    renderList({ cases: [] });
    expect(screen.getByText('No verification cases')).toBeInTheDocument();
  });
});
