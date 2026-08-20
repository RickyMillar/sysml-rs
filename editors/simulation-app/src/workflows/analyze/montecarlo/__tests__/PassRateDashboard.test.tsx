/**
 * PassRateDashboard — the overall card carries the evaluation-mode
 * badge (B10 §2.1a(d): the mode is a BINDING label wherever verdicts
 * read; MC pass-rates roll up per-child session runs → trajectory).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { PassRateDashboard } from '../PassRateDashboard';
import type { ChildDescriptor } from '../passRateHelpers';

afterEach(cleanup);

const child = (verdict: 'pass' | 'fail'): ChildDescriptor => ({
  session_id: 's1',
  index: 0,
  params: {},
  status: 'complete',
  verdicts: [{ verdict, id: 'C1' }],
});

describe('PassRateDashboard — evaluation-mode badge', () => {
  it('labels the overall card trajectory (session-run rollup)', () => {
    render(
      <PassRateDashboard
        children={[child('pass'), child('fail')]}
        constraints={['C1']}
      />,
    );
    const badge = screen.getByTestId('pass-rate-overall-mode');
    expect(badge).toHaveTextContent(/trajectory/i);
  });

  it('renders no badge in the constraints-empty state (no verdicts to label)', () => {
    render(<PassRateDashboard children={[]} constraints={[]} />);
    expect(screen.queryByTestId('pass-rate-overall-mode')).toBeNull();
  });
});
