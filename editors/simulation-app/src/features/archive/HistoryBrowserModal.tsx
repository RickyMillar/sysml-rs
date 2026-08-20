/**
 * HistoryBrowserModal — the archive's PERMANENT home (ninebar Phase 6,
 * plan §1 row 24 / audit F4): "History-browser modal: archived-runs
 * tab + mark/unmark-golden action + Compare golden picker".
 *
 * The body renders the exact `ArchivePanel` (unforked — filters,
 * golden star, three-dot mark/unmark-golden and restore actions all
 * ride along), replacing the interim `archive` right-rail context from
 * Phase 1. Openers: Cmd-K (`modal.history`), the frame session
 * switcher's "Archived runs…" row, and the Compare golden mode's
 * "manage archive" affordance.
 *
 * `TABS` is a real array so future tabs (e.g. a cross-workspace view)
 * slot in without restructuring, but the tab strip only renders once
 * a second tab exists — a one-tab tab bar is dead chrome.
 */
import { registerModal } from '@/shared/overlays/modalStore';
import { ArchivePanel } from './ArchivePanel';

export const HISTORY_BROWSER_MODAL_ID = 'history-browser';

const TABS = [{ id: 'archived-runs', label: 'Archived runs' }] as const;

export function HistoryBrowserModal() {
  return (
    <div
      data-testid="history-browser-modal"
      className="flex flex-col overflow-hidden"
      style={{ width: 'min(720px, 84vw)', height: 'min(520px, 76vh)' }}
    >
      {TABS.length > 1 && (
        <nav
          data-testid="history-browser-tabs"
          className="flex items-center gap-1 shrink-0"
          style={{ height: 30, borderBottom: '1px solid var(--border-hairline)' }}
        >
          {TABS.map((t) => (
            <span key={t.id} style={{ fontSize: 'var(--text-sm)', padding: '2px 8px' }}>
              {t.label}
            </span>
          ))}
        </nav>
      )}
      <div className="flex-1 min-h-0 overflow-hidden" data-testid="history-browser-archived-runs">
        <ArchivePanel width="100%" />
      </div>
    </div>
  );
}

registerModal({
  id: HISTORY_BROWSER_MODAL_ID,
  title: 'History — archived runs',
  component: HistoryBrowserModal,
});
