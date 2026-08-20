/**
 * ArchivePanel — sidebar panel listing archived simulation sessions (R4.1).
 *
 * Registered in `shared/panels/registry.ts` as a `'utility'` panel and
 * surfaced alongside Variables + Breakpoints. Backed by the (parallel-
 * built) backend commands:
 *
 *   - sysml.sessions.archive.list           → `useArchiveList`
 *   - sysml.sessions.archive.get            → `useArchivedSession`
 *   - sysml.sessions.archive.mark_golden    → `useMarkGolden`
 *   - sysml.sessions.archive.unmark_golden  → `useUnmarkGolden`
 *
 * Filter layering:
 *   - Server-side  : origin · since · only_golden · workspace_uri
 *   - Client-side  : free-text search (label / workspace_uri substring)
 *
 * Restore action:
 *   Click a row → `setActiveSession(id)` + `navigate('/run')`.
 *   The RunWorkflow picks up the new `activeSessionId` via its existing
 *   session-store subscription.
 */

import {
  useCallback,
  useMemo,
  useState,
  type CSSProperties,
  type ChangeEvent,
} from 'react';
import { useNavigate } from 'react-router-dom';
import { useSessionStore } from '@/features/sessions/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { ArchiveRow } from './ArchiveRow';
import { filterArchive } from './filterArchive';
import { useArchiveList } from './useArchiveList';
import { useMarkGolden, useUnmarkGolden } from './useMarkGolden';
import {
  ARCHIVE_ORIGIN_LABELS,
  ARCHIVE_ORIGIN_OPTIONS,
  ARCHIVE_SINCE_LABELS,
  ARCHIVE_SINCE_OPTIONS,
  DEFAULT_ARCHIVE_FILTER,
  type ArchiveFilter,
  type ArchiveOriginFilter,
  type ArchiveSinceKey,
  type ArchivedSessionSummary,
} from './types';

export interface ArchivePanelProps {
  /** Width in px (default 300). */
  width?: number | string;
  /**
   * When provided, overrides the hook's workspace narrowing. The default
   * reads from the workspace UI store so the panel stays scoped to the
   * active workspace.
   */
  workspaceUri?: string | null;
  /**
   * Injectable navigate + setActiveSession + copy for tests. Omit in
   * production — the panel wires to react-router + zustand + the
   * clipboard API automatically.
   */
  testHooks?: {
    navigate?: (path: string) => void;
    setActiveSession?: (id: string | null) => void;
    copyToClipboard?: (text: string) => void;
    now?: number;
  };
}

/**
 * Body renderer. Four states are covered: loading, error, populated,
 * and two empty-state variants:
 *   - no entries at all → "No archived sessions yet."
 *   - entries filtered down to zero → "No matches. Clear filters."
 */
export function ArchivePanel({
  width = 300,
  workspaceUri,
  testHooks,
}: ArchivePanelProps = {}) {
  const [filter, setFilter] = useState<ArchiveFilter>(DEFAULT_ARCHIVE_FILTER);

  const storeWorkspaceUri = useWorkspaceUIStore((s) => s.workspaceRoot);
  const effectiveWorkspaceUri =
    workspaceUri !== undefined ? workspaceUri : storeWorkspaceUri;

  const listQuery = useArchiveList(filter, {
    workspaceUri: effectiveWorkspaceUri,
  });

  const entries = listQuery.data ?? [];
  const visible = useMemo(
    () => filterArchive(entries, filter, testHooks?.now ?? Date.now()),
    [entries, filter, testHooks?.now],
  );

  // ── Mutations (mark / unmark golden) ───────────────────────────────
  const markGolden = useMarkGolden();
  const unmarkGolden = useUnmarkGolden();

  // ── Navigation wiring ──────────────────────────────────────────────
  const routerNavigate = useNavigate();
  const storeSetActive = useSessionStore((s) => s.setActiveSession);
  const navigate = testHooks?.navigate ?? ((path: string) => routerNavigate(path));
  const setActiveSession = testHooks?.setActiveSession ?? storeSetActive;
  const copyToClipboard =
    testHooks?.copyToClipboard ??
    ((text: string) => {
      if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
        void navigator.clipboard.writeText(text).catch(() => {});
      }
    });

  // ── Row action handlers ────────────────────────────────────────────
  const handleRestore = useCallback(
    (entry: ArchivedSessionSummary) => {
      setActiveSession(entry.id);
      navigate('/run');
    },
    [navigate, setActiveSession],
  );

  const handleMarkGolden = useCallback(
    (entry: ArchivedSessionSummary) => {
      markGolden.mutate(
        { id: entry.id, label: entry.label },
        {
          onSuccess: () => {
            void listQuery.refetch();
          },
        },
      );
    },
    [listQuery, markGolden],
  );

  const handleUnmarkGolden = useCallback(
    (entry: ArchivedSessionSummary) => {
      unmarkGolden.mutate(
        { id: entry.id },
        {
          onSuccess: () => {
            void listQuery.refetch();
          },
        },
      );
    },
    [listQuery, unmarkGolden],
  );

  const handleCopyId = useCallback(
    (entry: ArchivedSessionSummary) => {
      copyToClipboard(entry.id);
    },
    [copyToClipboard],
  );

  // ── Filter control handlers ────────────────────────────────────────
  const setSearch = useCallback((event: ChangeEvent<HTMLInputElement>) => {
    const next = event.target.value;
    setFilter((prev) => ({ ...prev, search: next }));
  }, []);

  const setOrigin = useCallback((origin: ArchiveOriginFilter) => {
    setFilter((prev) => ({ ...prev, origin }));
  }, []);

  const setSince = useCallback((since: ArchiveSinceKey) => {
    setFilter((prev) => ({ ...prev, since }));
  }, []);

  const toggleOnlyGolden = useCallback(() => {
    setFilter((prev) => ({ ...prev, onlyGolden: !prev.onlyGolden }));
  }, []);

  const clearFilters = useCallback(() => {
    setFilter(DEFAULT_ARCHIVE_FILTER);
  }, []);

  // ── Render ─────────────────────────────────────────────────────────
  return (
    <div
      data-testid="archive-panel"
      style={{
        width,
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--surface-sunken)',
        borderLeft: '1px solid var(--border-default)',
        color: 'var(--text-primary)',
        overflow: 'hidden',
        height: '100%',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '8px 10px',
          borderBottom: '1px solid var(--border-default)',
          background: 'var(--surface-panel)',
        }}
      >
        <span
          className="material-symbols-outlined"
          aria-hidden="true"
          style={{ fontSize: 16, color: 'var(--text-secondary)' }}
        >
          archive
        </span>
        <strong
          data-testid="archive-panel-title"
          style={{ fontSize: 'var(--text-xs, 11px)', letterSpacing: 0.3 }}
        >
          Archive ({entries.length})
        </strong>
        <div style={{ flex: 1 }} />
        {listQuery.isFetching ? (
          <span
            data-testid="archive-panel-fetching"
            style={{ fontSize: 10, color: 'var(--text-muted)' }}
          >
            loading…
          </span>
        ) : null}
      </header>

      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
          padding: '8px 10px',
          borderBottom: '1px solid var(--border-default)',
        }}
      >
        <input
          type="search"
          value={filter.search}
          onChange={setSearch}
          placeholder="Search label or workspace…"
          data-testid="archive-panel-search"
          aria-label="Search archived sessions"
          style={{
            padding: '5px 8px',
            fontSize: 'var(--text-xs, 11px)',
            background: 'var(--surface-raised)',
            border: '1px solid var(--border-default)',
            borderRadius: 3,
            color: 'var(--text-primary)',
          }}
        />

        <div
          role="radiogroup"
          aria-label="Filter by origin"
          data-testid="archive-panel-origin-group"
          style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}
        >
          {ARCHIVE_ORIGIN_OPTIONS.map((origin) => (
            <SegmentedButton
              key={origin}
              label={ARCHIVE_ORIGIN_LABELS[origin]}
              active={filter.origin === origin}
              onClick={() => setOrigin(origin)}
              testId={`archive-panel-origin-${origin}`}
            />
          ))}
        </div>

        <div
          role="radiogroup"
          aria-label="Filter by time range"
          data-testid="archive-panel-since-group"
          style={{ display: 'flex', gap: 4 }}
        >
          {ARCHIVE_SINCE_OPTIONS.map((since) => (
            <SegmentedButton
              key={since}
              label={ARCHIVE_SINCE_LABELS[since]}
              active={filter.since === since}
              onClick={() => setSince(since)}
              testId={`archive-panel-since-${since}`}
            />
          ))}
          <div style={{ flex: 1 }} />
          <label
            data-testid="archive-panel-only-golden-label"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 4,
              fontSize: 'var(--text-xs, 11px)',
              color: 'var(--text-muted)',
              cursor: 'pointer',
            }}
          >
            <input
              type="checkbox"
              checked={filter.onlyGolden}
              onChange={toggleOnlyGolden}
              data-testid="archive-panel-only-golden"
            />
            Golden only
          </label>
        </div>
      </div>

      <div
        style={{
          flex: 1,
          overflowY: 'auto',
          minHeight: 0,
        }}
      >
        {listQuery.isLoading ? (
          <LoadingState />
        ) : listQuery.isError ? (
          <ErrorState
            message={listQuery.error instanceof Error ? listQuery.error.message : String(listQuery.error)}
            onRetry={() => void listQuery.refetch()}
          />
        ) : entries.length === 0 ? (
          <EmptyState testId="archive-panel-empty-total">
            No archived sessions yet. Run, verify, or compare to create one.
          </EmptyState>
        ) : visible.length === 0 ? (
          <EmptyState testId="archive-panel-empty-filtered">
            <div style={{ marginBottom: 6 }}>No sessions match the current filters.</div>
            <SmallButton onClick={clearFilters} testId="archive-panel-clear-filters">
              Clear filters
            </SmallButton>
          </EmptyState>
        ) : (
          <ul
            role="list"
            data-testid="archive-panel-list"
            style={{
              listStyle: 'none',
              margin: 0,
              padding: 0,
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            {visible.map((entry) => (
              <li key={entry.id} style={{ margin: 0, padding: 0 }}>
                <ArchiveRow
                  entry={entry}
                  onRestore={handleRestore}
                  onMarkGolden={handleMarkGolden}
                  onUnmarkGolden={handleUnmarkGolden}
                  onCopyId={handleCopyId}
                  now={testHooks?.now}
                />
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

// ── Internals ────────────────────────────────────────────────────────

function SegmentedButton({
  label,
  active,
  onClick,
  testId,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  testId?: string;
}) {
  // Active state reads as a genuine selection/primacy affordance (a
  // selected filter chip in a radio group), so it takes accent per
  // the "active tab/highlighted item" carve-out — accent-tint mirrors
  // the row-selection tint tokens.css defines for exactly this use.
  const style: CSSProperties = {
    padding: '3px 8px',
    fontSize: 'var(--text-xs, 11px)',
    border: `1px solid ${active ? 'var(--accent)' : 'var(--border-default)'}`,
    background: active ? 'var(--accent-tint)' : 'transparent',
    color: active ? 'var(--accent-fg)' : 'var(--text-muted)',
    borderRadius: 3,
    cursor: 'pointer',
  };
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      data-active={active ? 'true' : 'false'}
      data-testid={testId}
      onClick={onClick}
      style={style}
    >
      {label}
    </button>
  );
}

function SmallButton({
  children,
  onClick,
  testId,
}: {
  children: React.ReactNode;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      data-testid={testId}
      style={{
        padding: '3px 10px',
        fontSize: 'var(--text-xs, 11px)',
        background: 'var(--accent)',
        color: 'var(--on-accent)',
        border: 'none',
        borderRadius: 3,
        cursor: 'pointer',
      }}
    >
      {children}
    </button>
  );
}

function LoadingState() {
  return (
    <div
      data-testid="archive-panel-loading"
      style={{
        padding: 16,
        color: 'var(--text-muted)',
        fontSize: 'var(--text-xs, 11px)',
        textAlign: 'center',
      }}
    >
      Loading archive…
    </div>
  );
}

function ErrorState({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div
      data-testid="archive-panel-error"
      style={{
        padding: 16,
        color: 'var(--severity-error)',
        fontSize: 'var(--text-xs, 11px)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 8,
      }}
    >
      <div>Failed to load archive.</div>
      <div style={{ color: 'var(--text-muted)' }}>{message}</div>
      <SmallButton onClick={onRetry} testId="archive-panel-retry">
        Retry
      </SmallButton>
    </div>
  );
}

function EmptyState({
  children,
  testId,
}: {
  children: React.ReactNode;
  testId: string;
}) {
  return (
    <div
      data-testid={testId}
      style={{
        padding: '20px 16px',
        color: 'var(--text-muted)',
        fontSize: 'var(--text-xs, 11px)',
        textAlign: 'center',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 4,
      }}
    >
      {children}
    </div>
  );
}
