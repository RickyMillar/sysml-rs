/**
 * DiagnosticsPanel — sidebar panel surfacing backend diagnostics (R6.1).
 *
 * Registered in `shared/panels/registry.ts` as a `'utility'` panel (id
 * `'diagnostics'`, icon `bug_report`). Backed by the `sysml.diagnostics`
 * service command; fans out one query per loaded URI and merges into a
 * single flat list the filter helpers operate on.
 *
 * Filter layering:
 *   - Severity checkboxes (error / warning / info / hint)  — client only
 *   - Search box (case-insensitive, matches message + code) — client only
 *   - Scope toggle (current file vs. workspace)             — client only
 *
 * Activate action:
 *   Click a row (or Enter / Space) → `useSelectionStore.select(uri, id?)`
 *   + `navigate('/run')`. If the diagnostic's span resolves to a file
 *   URI, that URI drives the selection — this reveals the file in the
 *   model tree. If the panel owner wants to wire element-id extraction
 *   from the span, the `testHooks.extractElementId` injection point
 *   covers it without baking a heuristic into the component.
 */

import {
  useCallback,
  useMemo,
  useState,
  type CSSProperties,
  type ChangeEvent,
  type ReactNode,
} from 'react';
import { useNavigate } from 'react-router-dom';
import { useSelectionStore } from '@/features/selection/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import { useWorkspaceUris } from '@/features/packages/queries';
import { DiagnosticRow } from './DiagnosticRow';
import {
  filterDiagnostics,
  groupDiagnosticsByUri,
} from './filterDiagnostics';
import { useDiagnostics } from './useDiagnostics';
import { shortFileName } from './DiagnosticRow';
import {
  DEFAULT_DIAGNOSTICS_FILTER,
  DIAGNOSTIC_SEVERITY_LABELS,
  DIAGNOSTIC_SEVERITY_OPTIONS,
  type DiagnosticEntry,
  type DiagnosticsFilter,
  type DiagnosticsScope,
  type DiagnosticSeverity,
} from './types';

export interface DiagnosticsPanelProps {
  /** Width in px (default 300). */
  width?: number | string;
  /**
   * Override the active URI picked up from the selection store. Mostly
   * useful for tests and embedded previews — production callers let the
   * panel follow selection automatically.
   */
  activeUri?: string | null;
  /**
   * Override the loaded-URI list. When omitted, the panel pulls the list
   * from `useWorkspaceUris` keyed on the workspace root (same source
   * every other workflow uses for `wsData.uris`).
   */
  uris?: string[];
  /**
   * Injectable navigate + selection + element-id extractor for tests.
   * Omit in production — the panel wires to react-router + zustand
   * automatically.
   */
  testHooks?: {
    navigate?: (path: string) => void;
    select?: (uri: string | null, elementId: string | null) => void;
    /** Optional extractor that maps a diagnostic to an element id. */
    extractElementId?: (entry: DiagnosticEntry) => string | null;
  };
}

export function DiagnosticsPanel({
  width = 300,
  activeUri: activeUriProp,
  uris: urisProp,
  testHooks,
}: DiagnosticsPanelProps = {}) {
  const [filter, setFilter] = useState<DiagnosticsFilter>(
    DEFAULT_DIAGNOSTICS_FILTER,
  );
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  // ── Data sources ────────────────────────────────────────────────────
  const storeSelectedUri = useSelectionStore((s) => s.selectedUri);
  const activeUri =
    activeUriProp !== undefined ? activeUriProp : storeSelectedUri;

  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const uris = useMemo(
    () => urisProp ?? wsData?.uris ?? [],
    [urisProp, wsData?.uris],
  );

  const diagnosticsQuery = useDiagnostics({ uris });
  const entries = diagnosticsQuery.entries;

  const visible = useMemo(
    () => filterDiagnostics(entries, { ...filter, activeUri }),
    [entries, filter, activeUri],
  );

  const grouped = useMemo(() => groupDiagnosticsByUri(visible), [visible]);

  // ── Navigation / selection wiring ───────────────────────────────────
  const routerNavigate = useNavigate();
  const storeSelect = useSelectionStore((s) => s.select);
  const setActiveUtility = useWorkspaceUIStore((s) => s.setActiveUtility);
  const setFocusedUri = useWorkspaceStore((s) => s.setFocusedUri);
  const navigate =
    testHooks?.navigate ?? ((path: string) => routerNavigate(path));
  const select = testHooks?.select ?? storeSelect;
  const extractElementId = testHooks?.extractElementId ?? defaultExtractor;

  const handleActivate = useCallback(
    (entry: DiagnosticEntry) => {
      const spanFile = entry.diagnostic.span?.file;
      // Prefer the span's file when present — semantic diagnostics can
      // refer to a file other than the parent URI the diagnostic was
      // fetched under. Fall back to the parent URI when the span is
      // absent so parse errors still resolve to a file.
      const uri = spanFile ?? entry.uri ?? null;
      if (!uri) return;
      const elementId = extractElementId(entry);
      select(uri, elementId);
      navigate('/run');
    },
    [extractElementId, navigate, select],
  );

  /**
   * Phase 3 — promote a hover-preview into the Source utility drawer.
   * Different from `handleActivate` in that it doesn't change the
   * route; the user has already chosen to inspect source, so we just
   * push selection + focused URI and open the Source drawer.
   */
  const handlePromotePreview = useCallback(
    (entry: DiagnosticEntry) => {
      const spanFile = entry.diagnostic.span?.file;
      const uri = spanFile ?? entry.uri ?? null;
      if (!uri) return;
      const elementId = extractElementId(entry);
      select(uri, elementId);
      setFocusedUri(uri);
      setActiveUtility('source');
    },
    [extractElementId, select, setActiveUtility, setFocusedUri],
  );

  // ── Filter control handlers ─────────────────────────────────────────
  const setSearch = useCallback((event: ChangeEvent<HTMLInputElement>) => {
    const next = event.target.value;
    setFilter((prev) => ({ ...prev, search: next }));
  }, []);

  const toggleSeverity = useCallback((severity: DiagnosticSeverity) => {
    setFilter((prev) => ({
      ...prev,
      severity: { ...prev.severity, [severity]: !prev.severity[severity] },
    }));
  }, []);

  const setScope = useCallback((scope: DiagnosticsScope) => {
    setFilter((prev) => ({ ...prev, scope }));
  }, []);

  const clearFilters = useCallback(() => {
    setFilter(DEFAULT_DIAGNOSTICS_FILTER);
  }, []);

  const toggleGroup = useCallback((uri: string) => {
    setCollapsed((prev) => ({ ...prev, [uri]: !prev[uri] }));
  }, []);

  // ── Render ──────────────────────────────────────────────────────────
  return (
    <div
      data-testid="diagnostics-panel"
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
          bug_report
        </span>
        <strong
          data-testid="diagnostics-panel-title"
          style={{ fontSize: 'var(--text-xs, 11px)', letterSpacing: 0.3 }}
        >
          Diagnostics ({entries.length})
        </strong>
        <div style={{ flex: 1 }} />
        {diagnosticsQuery.isFetching ? (
          <span
            data-testid="diagnostics-panel-fetching"
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
          placeholder="Filter by message or code…"
          data-testid="diagnostics-panel-search"
          aria-label="Search diagnostics"
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
          role="group"
          aria-label="Filter by severity"
          data-testid="diagnostics-panel-severity-group"
          style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}
        >
          {DIAGNOSTIC_SEVERITY_OPTIONS.map((severity) => (
            <SeverityCheckbox
              key={severity}
              severity={severity}
              active={filter.severity[severity]}
              onToggle={() => toggleSeverity(severity)}
            />
          ))}
        </div>

        <div
          role="radiogroup"
          aria-label="Filter by scope"
          data-testid="diagnostics-panel-scope-group"
          style={{ display: 'flex', gap: 4 }}
        >
          <SegmentedButton
            label="Current file"
            active={filter.scope === 'current-file'}
            onClick={() => setScope('current-file')}
            testId="diagnostics-panel-scope-current"
          />
          <SegmentedButton
            label="Workspace"
            active={filter.scope === 'workspace'}
            onClick={() => setScope('workspace')}
            testId="diagnostics-panel-scope-workspace"
          />
        </div>
      </div>

      <div
        style={{
          flex: 1,
          overflowY: 'auto',
          minHeight: 0,
        }}
      >
        {diagnosticsQuery.isLoading ? (
          <LoadingState />
        ) : diagnosticsQuery.isError ? (
          <ErrorState
            message={
              diagnosticsQuery.error instanceof Error
                ? diagnosticsQuery.error.message
                : String(diagnosticsQuery.error)
            }
            onRetry={diagnosticsQuery.refetch}
          />
        ) : entries.length === 0 ? (
          <EmptyState testId="diagnostics-panel-empty-total">
            No diagnostics. The loaded workspace is clean.
          </EmptyState>
        ) : visible.length === 0 ? (
          <EmptyState testId="diagnostics-panel-empty-filtered">
            <div style={{ marginBottom: 6 }}>
              No diagnostics match the current filters.
            </div>
            <SmallButton
              onClick={clearFilters}
              testId="diagnostics-panel-clear-filters"
            >
              Clear filters
            </SmallButton>
          </EmptyState>
        ) : (
          <ul
            role="list"
            data-testid="diagnostics-panel-list"
            style={{
              listStyle: 'none',
              margin: 0,
              padding: 0,
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            {Array.from(grouped.entries()).map(([uri, items]) => {
              const isCollapsed = !!collapsed[uri];
              return (
                <li
                  key={uri}
                  data-testid={`diagnostics-group-${uri}`}
                  style={{ margin: 0, padding: 0 }}
                >
                  <GroupHeader
                    uri={uri}
                    count={items.length}
                    collapsed={isCollapsed}
                    onToggle={() => toggleGroup(uri)}
                  />
                  {!isCollapsed ? (
                    <ul
                      role="list"
                      data-testid={`diagnostics-group-items-${uri}`}
                      style={{
                        listStyle: 'none',
                        margin: 0,
                        padding: 0,
                      }}
                    >
                      {items.map((entry, idx) => (
                        <li
                          key={`${uri}-${idx}`}
                          style={{ margin: 0, padding: 0 }}
                        >
                          <DiagnosticRow
                            entry={entry}
                            onActivate={handleActivate}
                            previewElementId={extractElementId(entry)}
                            onPromotePreview={handlePromotePreview}
                          />
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

// ── Internals ────────────────────────────────────────────────────────

/**
 * Default element-id extractor — the wire diagnostic carries no element
 * id field today, so the default returns `null`. Hosts can override via
 * `testHooks.extractElementId` without the panel baking in a heuristic.
 */
function defaultExtractor(_entry: DiagnosticEntry): string | null {
  return null;
}

function SeverityCheckbox({
  severity,
  active,
  onToggle,
}: {
  severity: DiagnosticSeverity;
  active: boolean;
  onToggle: () => void;
}) {
  return (
    <label
      data-testid={`diagnostics-panel-severity-${severity}-label`}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        padding: '3px 8px',
        fontSize: 'var(--text-xs, 11px)',
        border: `1px solid ${active ? 'var(--accent-fg)' : 'var(--border-default)'}`,
        background: active
          ? 'color-mix(in srgb, var(--accent-fg) 18%, transparent)'
          : 'transparent',
        color: active ? 'var(--accent-fg)' : 'var(--text-muted)',
        borderRadius: 3,
        cursor: 'pointer',
      }}
    >
      <input
        type="checkbox"
        checked={active}
        onChange={onToggle}
        data-testid={`diagnostics-panel-severity-${severity}`}
        aria-label={`Show ${DIAGNOSTIC_SEVERITY_LABELS[severity]}`}
        style={{ margin: 0 }}
      />
      {DIAGNOSTIC_SEVERITY_LABELS[severity]}
    </label>
  );
}

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
  const style: CSSProperties = {
    flex: 1,
    padding: '3px 8px',
    fontSize: 'var(--text-xs, 11px)',
    border: `1px solid ${active ? 'var(--accent-fg)' : 'var(--border-default)'}`,
    background: active
      ? 'color-mix(in srgb, var(--accent-fg) 18%, transparent)'
      : 'transparent',
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

function GroupHeader({
  uri,
  count,
  collapsed,
  onToggle,
}: {
  uri: string;
  count: number;
  collapsed: boolean;
  onToggle: () => void;
}) {
  const file = shortFileName(uri);
  return (
    <button
      type="button"
      data-testid={`diagnostics-group-header-${uri}`}
      aria-expanded={!collapsed}
      onClick={onToggle}
      title={uri}
      style={{
        display: 'flex',
        width: '100%',
        alignItems: 'center',
        gap: 6,
        padding: '6px 10px',
        background: 'var(--surface-panel)',
        border: 'none',
        borderBottom: '1px solid var(--border-default)',
        color: 'var(--text-primary)',
        cursor: 'pointer',
        textAlign: 'left',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          fontSize: 11,
          color: 'var(--text-muted)',
          transform: collapsed ? 'rotate(-90deg)' : undefined,
          transition: 'transform 120ms ease',
        }}
      >
        {'\u25BE'}
      </span>
      <span
        style={{
          fontSize: 'var(--text-xs, 11px)',
          fontWeight: 600,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          flex: 1,
        }}
      >
        {file}
      </span>
      <span
        data-testid={`diagnostics-group-count-${uri}`}
        style={{
          fontSize: 10,
          padding: '0 6px',
          borderRadius: 8,
          background: 'var(--surface-raised)',
          color: 'var(--text-muted)',
          letterSpacing: 0.3,
        }}
      >
        {count}
      </span>
    </button>
  );
}

function SmallButton({
  children,
  onClick,
  testId,
}: {
  children: ReactNode;
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
      data-testid="diagnostics-panel-loading"
      style={{
        padding: 16,
        color: 'var(--text-muted)',
        fontSize: 'var(--text-xs, 11px)',
        textAlign: 'center',
      }}
    >
      Loading diagnostics…
    </div>
  );
}

function ErrorState({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div
      data-testid="diagnostics-panel-error"
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
      <div>Failed to load diagnostics.</div>
      <div style={{ color: 'var(--text-muted)' }}>{message}</div>
      <SmallButton onClick={onRetry} testId="diagnostics-panel-retry">
        Retry
      </SmallButton>
    </div>
  );
}

function EmptyState({
  children,
  testId,
}: {
  children: ReactNode;
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
