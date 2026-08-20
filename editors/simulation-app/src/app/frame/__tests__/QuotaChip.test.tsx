/**
 * QuotaChip — "used/cap sessions" summary off mocked `sessions.quota`
 * data, the near-limit warning treatment, and the render-nothing cases
 * (no data yet / zero cap) (ninebar Phase 1 frame chips, audit F7).
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import type { SessionQuota } from '@/features/sessions/types';

afterEach(cleanup);

let mockQuota: SessionQuota | undefined;
vi.mock('@/features/sessions/queries', () => ({
  useSessionQuota: () => ({ data: mockQuota }),
}));

import { QuotaChip } from '../QuotaChip';

describe('QuotaChip', () => {
  it('renders nothing while quota has not loaded', () => {
    mockQuota = undefined;
    const { container } = render(<QuotaChip />);
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when every bucket cap is zero (quota disabled)', () => {
    mockQuota = {
      simulation: { used: 0, cap: 0 },
      action: { used: 0, cap: 0 },
      orchestrator: { used: 0, cap: 0 },
    };
    const { container } = render(<QuotaChip />);
    expect(container.firstChild).toBeNull();
  });

  it('sums used/cap across all three buckets', () => {
    mockQuota = {
      simulation: { used: 2, cap: 30 },
      action: { used: 1, cap: 10 },
      orchestrator: { used: 0, cap: 5 },
    };
    render(<QuotaChip />);
    expect(screen.getByTestId('quota-chip')).toHaveTextContent('3/45 sessions');
  });

  it('applies the warning treatment at/near the cap, quiet otherwise', () => {
    mockQuota = {
      simulation: { used: 1, cap: 10 },
      action: { used: 0, cap: 0 },
      orchestrator: { used: 0, cap: 0 },
    };
    render(<QuotaChip />);
    const quiet = screen.getByTestId('quota-chip');
    expect(quiet).toHaveAttribute('data-warn', 'false');
    expect(quiet.style.color).not.toContain('severity-warning');

    mockQuota = {
      simulation: { used: 9, cap: 10 },
      action: { used: 0, cap: 0 },
      orchestrator: { used: 0, cap: 0 },
    };
    render(<QuotaChip />);
    const warn = screen.getAllByTestId('quota-chip')[1];
    expect(warn).toHaveAttribute('data-warn', 'true');
    expect(warn.style.color).toContain('severity-warning');
  });
});
