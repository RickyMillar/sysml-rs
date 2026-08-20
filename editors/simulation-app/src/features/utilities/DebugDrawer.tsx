/**
 * DebugDrawer — dev-only utility surface (Phase 8).
 *
 * Four collapsible sections, each on its own react-query hook against
 * a backend introspection command. Default polling is off; each
 * section has a manual "Refresh" button plus a section-local
 * "Auto-refresh (1s)" toggle.
 *
 * Gated behind `import.meta.env.VITE_DEBUG_DRAWER === '1'` (see
 * `shared/panels/debug.ts::isDebugDrawerEnabled`). When the flag is
 * off, `UtilityDrawer` never registers the toolbar affordance and
 * this component is never mounted.
 */

import { useCallback, useState, type ReactNode } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { httpPost } from '@/shared/api/http';

interface SalsaStats {
  executions: number;
  validations: number;
  hit_ratio: number;
}

interface WorkspaceUriInfo {
  uri: string;
  tree: unknown[];
  stats: Record<string, number | undefined>;
}

type CacheStatus = Record<string, unknown>;
type DependencyStatus = Record<string, unknown>;

async function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Section state ───────────────────────────────────────────────────

const AUTO_REFRESH_MS = 1000;

interface SectionShellProps {
  id: string;
  title: string;
  loading: boolean;
  error: unknown;
  autoRefresh: boolean;
  onToggleAutoRefresh: () => void;
  onRefresh: () => void;
  extraActions?: ReactNode;
  children: ReactNode;
}

function SectionShell({
  id,
  title,
  loading,
  error,
  autoRefresh,
  onToggleAutoRefresh,
  onRefresh,
  extraActions,
  children,
}: SectionShellProps) {
  const [open, setOpen] = useState(true);
  return (
    <section
      data-testid={`debug-section-${id}`}
      style={{
        border: '1px solid var(--outline-variant)',
        borderRadius: 6,
        margin: '6px 8px',
        background: 'var(--surface-container)',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '6px 8px',
          borderBottom: open ? '1px solid var(--outline-variant)' : 'none',
        }}
      >
        <button
          type="button"
          data-testid={`debug-section-${id}-toggle`}
          onClick={() => setOpen((o) => !o)}
          style={{
            border: 'none',
            background: 'transparent',
            color: 'var(--on-surface)',
            font: 'inherit',
            fontWeight: 700,
            fontSize: 11,
            cursor: 'pointer',
            flex: 1,
            textAlign: 'left',
          }}
          aria-expanded={open}
        >
          {open ? '▾' : '▸'} {title}
        </button>
        {extraActions}
        <label
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 3,
            fontSize: 10,
            color: 'var(--on-surface-variant)',
          }}
        >
          <input
            type="checkbox"
            data-testid={`debug-section-${id}-autorefresh`}
            checked={autoRefresh}
            onChange={onToggleAutoRefresh}
          />
          auto 1s
        </label>
        <button
          type="button"
          data-testid={`debug-section-${id}-refresh`}
          onClick={onRefresh}
          disabled={loading}
          style={{
            border: '1px solid var(--outline-variant)',
            background: 'var(--surface-container-high)',
            color: 'var(--on-surface)',
            borderRadius: 3,
            padding: '1px 6px',
            fontSize: 10,
            cursor: loading ? 'wait' : 'pointer',
          }}
        >
          {loading ? '…' : 'Refresh'}
        </button>
      </header>
      {open && (
        <div style={{ padding: '6px 8px', fontSize: 11 }}>
          {error ? (
            <div style={{ color: 'var(--error)' }}>
              Error: {error instanceof Error ? error.message : String(error)}
            </div>
          ) : (
            children
          )}
        </div>
      )}
    </section>
  );
}

function KeyValueGrid({ rows }: { rows: Array<[string, ReactNode]> }) {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: 'auto 1fr',
        columnGap: 10,
        rowGap: 2,
        fontFamily: 'var(--font-mono, ui-monospace, monospace)',
      }}
    >
      {rows.map(([k, v]) => (
        <>
          <div key={`${k}-k`} style={{ color: 'var(--on-surface-variant)' }}>{k}</div>
          <div key={`${k}-v`} style={{ color: 'var(--on-surface)' }}>{v}</div>
        </>
      ))}
    </div>
  );
}

function RawJson({ value }: { value: unknown }) {
  const [open, setOpen] = useState(false);
  return (
    <details
      onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}
      style={{ marginTop: 6 }}
    >
      <summary style={{ cursor: 'pointer', fontSize: 10, color: 'var(--outline)' }}>
        {open ? 'hide' : 'show'} raw JSON
      </summary>
      <pre
        style={{
          marginTop: 4,
          padding: 6,
          background: 'var(--surface-container-low)',
          borderRadius: 4,
          fontSize: 10,
          overflowX: 'auto',
          maxHeight: 220,
        }}
      >
        {JSON.stringify(value, null, 2)}
      </pre>
    </details>
  );
}

// ── Section: Workspace info ─────────────────────────────────────────

function WorkspaceInfoSection() {
  const [auto, setAuto] = useState(false);
  const q = useQuery<WorkspaceUriInfo[]>({
    queryKey: ['debug', 'sysml.workspace.info'],
    queryFn: () => cmd<WorkspaceUriInfo[]>('sysml.workspace.info'),
    refetchInterval: auto ? AUTO_REFRESH_MS : false,
    staleTime: 0,
  });

  const rows = q.data ?? [];
  const totalElements = rows.reduce(
    (acc, r) => acc + Number(r.stats?.element_count ?? 0),
    0,
  );
  const totalRelationships = rows.reduce(
    (acc, r) => acc + Number(r.stats?.relationship_count ?? 0),
    0,
  );

  return (
    <SectionShell
      id="workspace-info"
      title="Workspace info"
      loading={q.isFetching}
      error={q.error}
      autoRefresh={auto}
      onToggleAutoRefresh={() => setAuto((a) => !a)}
      onRefresh={() => void q.refetch()}
    >
      <KeyValueGrid
        rows={[
          ['Loaded URIs', String(rows.length)],
          ['Total elements', String(totalElements)],
          ['Total relationships', String(totalRelationships)],
        ]}
      />
      {rows.length > 0 && (
        <ul
          style={{
            margin: '6px 0 0',
            padding: 0,
            listStyle: 'none',
            fontSize: 10,
            color: 'var(--on-surface-variant)',
          }}
        >
          {rows.slice(0, 10).map((r) => (
            <li key={r.uri} style={{ fontFamily: 'var(--font-mono, monospace)' }}>
              {r.uri} — {String(r.stats?.element_count ?? 0)} elems
            </li>
          ))}
          {rows.length > 10 && <li>… +{rows.length - 10} more</li>}
        </ul>
      )}
      {q.data !== undefined && <RawJson value={q.data} />}
    </SectionShell>
  );
}

// ── Section: Salsa stats ────────────────────────────────────────────

function SalsaStatsSection() {
  const [auto, setAuto] = useState(false);
  const qc = useQueryClient();
  const q = useQuery<SalsaStats>({
    queryKey: ['debug', 'sysml.salsa.stats'],
    queryFn: () => cmd<SalsaStats>('sysml.salsa.stats'),
    refetchInterval: auto ? AUTO_REFRESH_MS : false,
    staleTime: 0,
  });
  const reset = useMutation({
    mutationFn: () => cmd<{ status: string }>('sysml.salsa.stats.reset'),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['debug', 'sysml.salsa.stats'] }),
  });

  const stats = q.data;
  return (
    <SectionShell
      id="salsa-stats"
      title="Salsa stats"
      loading={q.isFetching || reset.isPending}
      error={q.error ?? reset.error}
      autoRefresh={auto}
      onToggleAutoRefresh={() => setAuto((a) => !a)}
      onRefresh={() => void q.refetch()}
      extraActions={
        <button
          type="button"
          data-testid="debug-section-salsa-stats-reset"
          onClick={() => reset.mutate()}
          disabled={reset.isPending}
          style={{
            border: '1px solid var(--outline-variant)',
            background: 'var(--surface-container-high)',
            color: 'var(--on-surface)',
            borderRadius: 3,
            padding: '1px 6px',
            fontSize: 10,
            cursor: reset.isPending ? 'wait' : 'pointer',
          }}
        >
          Reset
        </button>
      }
    >
      {stats && (
        <KeyValueGrid
          rows={[
            ['Executions', String(stats.executions)],
            ['Validations', String(stats.validations)],
            ['Hit ratio', `${(stats.hit_ratio * 100).toFixed(1)}%`],
          ]}
        />
      )}
      {q.data !== undefined && <RawJson value={q.data} />}
    </SectionShell>
  );
}

// ── Section: Cache status ───────────────────────────────────────────

function CacheStatusSection() {
  const [auto, setAuto] = useState(false);
  const q = useQuery<CacheStatus>({
    queryKey: ['debug', 'sysml.cache.status'],
    queryFn: () => cmd<CacheStatus>('sysml.cache.status'),
    refetchInterval: auto ? AUTO_REFRESH_MS : false,
    staleTime: 0,
  });

  const c = q.data;
  return (
    <SectionShell
      id="cache-status"
      title="Library cache"
      loading={q.isFetching}
      error={q.error}
      autoRefresh={auto}
      onToggleAutoRefresh={() => setAuto((a) => !a)}
      onRefresh={() => void q.refetch()}
    >
      {c && (
        <KeyValueGrid
          rows={Object.entries(c).map(([k, v]) => [
            k,
            typeof v === 'object' && v !== null ? JSON.stringify(v) : String(v),
          ])}
        />
      )}
      {q.data !== undefined && <RawJson value={q.data} />}
    </SectionShell>
  );
}

// ── Section: Dependency status ──────────────────────────────────────

function DependencyStatusSection() {
  const [auto, setAuto] = useState(false);
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const q = useQuery<DependencyStatus>({
    queryKey: ['debug', 'sysml.dependency.status', workspaceRoot],
    queryFn: () =>
      cmd<DependencyStatus>('sysml.dependency.status', {
        roots: workspaceRoot ? [workspaceRoot] : [],
      }),
    refetchInterval: auto ? AUTO_REFRESH_MS : false,
    staleTime: 0,
  });

  const summary = (q.data?.summary ?? null) as Record<string, number> | null;
  return (
    <SectionShell
      id="dependency-status"
      title="Dependency status"
      loading={q.isFetching}
      error={q.error}
      autoRefresh={auto}
      onToggleAutoRefresh={() => setAuto((a) => !a)}
      onRefresh={() => void q.refetch()}
    >
      {!workspaceRoot && (
        <div style={{ color: 'var(--on-surface-variant)' }}>
          No workspace root loaded — dependency status requires a workspace.
        </div>
      )}
      {summary && (
        <KeyValueGrid
          rows={Object.entries(summary).map(([k, v]) => [k, String(v)])}
        />
      )}
      {q.data !== undefined && <RawJson value={q.data} />}
    </SectionShell>
  );
}

// ── Root ────────────────────────────────────────────────────────────

export function DebugDrawer() {
  // Single mount-time invalidate so a stale react-query cache from an
  // earlier visit doesn't cause sections to appear pre-populated. Each
  // section will then fetch on mount.
  const qc = useQueryClient();
  const drop = useCallback(() => {
    qc.invalidateQueries({ queryKey: ['debug'] });
  }, [qc]);

  return (
    <div
      data-testid="debug-drawer"
      style={{
        flex: 1,
        minHeight: 0,
        overflowY: 'auto',
        color: 'var(--on-surface)',
        background: 'var(--surface-container-low)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          padding: '6px 10px',
          borderBottom: '1px solid var(--outline-variant)',
          fontSize: 10,
          color: 'var(--on-surface-variant)',
        }}
      >
        <span style={{ flex: 1 }}>
          Dev-only. Backend introspection commands. Not visible without
          <code style={{ margin: '0 4px' }}>VITE_DEBUG_DRAWER=1</code>.
        </span>
        <button
          type="button"
          data-testid="debug-drawer-invalidate"
          onClick={drop}
          style={{
            border: '1px solid var(--outline-variant)',
            background: 'var(--surface-container-high)',
            color: 'var(--on-surface)',
            borderRadius: 3,
            padding: '1px 6px',
            fontSize: 10,
            cursor: 'pointer',
          }}
        >
          Refetch all
        </button>
      </div>
      <WorkspaceInfoSection />
      <SalsaStatsSection />
      <CacheStatusSection />
      <DependencyStatusSection />
    </div>
  );
}
