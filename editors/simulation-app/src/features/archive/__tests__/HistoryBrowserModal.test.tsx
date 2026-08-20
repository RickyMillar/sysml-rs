/**
 * HistoryBrowserModal — registration + body contract (Phase 6, plan
 * §1 row 24). ArchivePanel's own behaviour (filters, golden
 * mark/unmark, restore) is covered by ArchivePanel.test.tsx; here we
 * pin that the modal registers under the stable id and hosts the
 * exact panel (unforked) inside the archived-runs region.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

vi.mock('../ArchivePanel', () => ({
  ArchivePanel: ({ width }: { width?: number | string }) => (
    <div data-testid="archive-panel-stub" data-width={String(width)} />
  ),
}));

import { getModal } from '@/shared/overlays/modalStore';
import {
  HISTORY_BROWSER_MODAL_ID,
  HistoryBrowserModal,
} from '../HistoryBrowserModal';

afterEach(cleanup);

describe('HistoryBrowserModal', () => {
  it('registers in the modal registry under the stable id', () => {
    const descriptor = getModal(HISTORY_BROWSER_MODAL_ID);
    expect(descriptor).toBeDefined();
    expect(descriptor?.title).toContain('archived runs');
    expect(descriptor?.component).toBe(HistoryBrowserModal);
  });

  it('hosts the exact ArchivePanel (full width) in the archived-runs region', () => {
    render(<HistoryBrowserModal />);
    expect(screen.getByTestId('history-browser-modal')).toBeTruthy();
    const region = screen.getByTestId('history-browser-archived-runs');
    expect(region).toBeTruthy();
    const panel = screen.getByTestId('archive-panel-stub');
    expect(panel.getAttribute('data-width')).toBe('100%');
  });

  it('renders no tab chrome while there is a single tab', () => {
    render(<HistoryBrowserModal />);
    expect(screen.queryByTestId('history-browser-tabs')).toBeNull();
  });
});
