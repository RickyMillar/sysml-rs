import { useMemo } from 'react';
import { panelRegistry } from '@/shared/panels/registry';
import type { PanelDescriptor, PanelProps } from '@/shared/panels/types';
import { useWorkspaceUIStore, type ActiveUtility } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useDiagnostics } from '@/features/diagnostics/useDiagnostics';
import { useArchiveList } from '@/features/archive/useArchiveList';
import { DEFAULT_ARCHIVE_FILTER } from '@/features/archive/types';
import { selectArmedCount, useBreakpointStore } from '@/features/breakpoints/useBreakpointStore';
import { useViewsList } from '@/features/views/queries';
import { isDebugDrawerEnabled } from '@/shared/panels/debug';

type UtilityId = ActiveUtility;

const BASE_UTILITY_IDS: UtilityId[] = [
  'diagnostics',
  'archive',
  'breakpoints',
  'views',
  'source',
  'integrations',
];

// Phase 8: append 'debug' only when the env flag is on. Computed at
// module-load time so the production bundle elides the affordance
// entirely (Vite inlines `import.meta.env.VITE_DEBUG_DRAWER`).
const UTILITY_IDS: UtilityId[] = isDebugDrawerEnabled()
  ? [...BASE_UTILITY_IDS, 'debug']
  : BASE_UTILITY_IDS;

export function UtilityDrawer() {
  const activeId = useWorkspaceUIStore((s) => s.activeUtility);
  const setActiveId = useWorkspaceUIStore((s) => s.setActiveUtility);
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const workspaceUris = useWorkspaceUris(workspaceRoot ?? null);
  const uris = workspaceUris.data?.uris ?? [];
  const diagnostics = useDiagnostics({
    uris,
    enabled: !!workspaceRoot && uris.length > 0,
  });
  const archive = useArchiveList(DEFAULT_ARCHIVE_FILTER, {
    workspaceUri: workspaceRoot,
    enabled: !!workspaceRoot,
  });
  const armedBreakpoints = useBreakpointStore(selectArmedCount);
  const viewsList = useViewsList(workspaceRoot ? '__workspace__' : null);

  const panels = useMemo(() => {
    const byId = new Map(panelRegistry.map((panel) => [panel.id, panel]));
    return UTILITY_IDS.map((id) => byId.get(id)).filter(Boolean) as PanelDescriptor[];
  }, []);
  const active = activeId ? panels.find((panel) => panel.id === activeId) ?? null : null;

  const badges: Record<UtilityId, string | null> = {
    diagnostics: diagnostics.isError ? '!' : diagnostics.entries.length > 0 ? String(diagnostics.entries.length) : null,
    archive: archive.isError ? '!' : archive.data?.length ? String(archive.data.length) : null,
    breakpoints: armedBreakpoints > 0 ? String(armedBreakpoints) : null,
    views: viewsList.isError ? '!' : viewsList.data?.length ? String(viewsList.data.length) : null,
    source: null,
    integrations: null,
    debug: null,
  };

  return (
    <>
      <div
        data-testid="utility-toolbar"
        className="flex items-center gap-1 px-3 py-1 shrink-0"
        style={{
          background: 'var(--surface-container-lowest)',
          borderBottom: '1px solid var(--outline-variant)',
        }}
      >
        <span style={{ fontSize: 10, color: 'var(--outline)', fontWeight: 800, marginRight: 4 }}>
          Utilities
        </span>
        {panels.map((panel) => {
          const id = panel.id as UtilityId;
          const open = activeId === id;
          const badge = badges[id];
          return (
            <button
              key={panel.id}
              type="button"
              data-testid={`utility-toggle-${panel.id}`}
              onClick={() => setActiveId(open ? null : (id as ActiveUtility))}
              className="inline-flex items-center gap-1 rounded"
              style={{
                border: '1px solid var(--outline-variant)',
                background: open ? 'var(--secondary-container)' : 'var(--surface-container)',
                color: open ? 'var(--on-secondary-container)' : 'var(--on-surface-variant)',
                padding: '3px 8px',
                fontSize: 10,
                fontWeight: 700,
                cursor: 'pointer',
              }}
              aria-pressed={open}
            >
              <span className="material-symbols-outlined" style={{ fontSize: 13, color: panel.accentColor }}>
                {panel.icon}
              </span>
              {panel.title}
              {badge && (
                <span
                  data-testid={`utility-badge-${panel.id}`}
                  className="mono-text"
                  style={{
                    borderRadius: 999,
                    background: 'var(--surface-container-highest)',
                    color: badge === '!' ? 'var(--error)' : 'var(--outline)',
                    padding: '0 5px',
                    fontSize: 9,
                  }}
                >
                  {badge}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {active && (
        <div
          data-testid="utility-drawer"
          className="fixed top-0 right-0 h-screen flex flex-col shadow-xl"
          style={{
            width: 420,
            zIndex: 80,
            background: 'var(--surface-container-low)',
            borderLeft: '1px solid var(--outline-variant)',
            color: 'var(--on-surface)',
          }}
        >
          <header
            className="flex items-center gap-2 px-3 py-2 shrink-0"
            style={{ borderBottom: '1px solid var(--outline-variant)' }}
          >
            <span className="material-symbols-outlined" style={{ fontSize: 16, color: active.accentColor }}>
              {active.icon}
            </span>
            <span style={{ fontSize: 12, fontWeight: 800 }}>{active.title}</span>
            <span style={{ fontSize: 10, color: 'var(--outline)' }}>utility drawer</span>
            <button
              type="button"
              data-testid="utility-drawer-close"
              onClick={() => setActiveId(null)}
              style={{
                marginLeft: 'auto',
                border: '1px solid var(--outline-variant)',
                background: 'var(--surface-container)',
                color: 'var(--on-surface-variant)',
                borderRadius: 4,
                padding: '2px 6px',
                cursor: 'pointer',
              }}
              aria-label="Close utility drawer"
            >
              <span className="material-symbols-outlined" style={{ fontSize: 14 }}>close</span>
            </button>
          </header>
          <div className="flex-1 min-h-0 overflow-hidden">{active.render(emptyPanelProps)}</div>
        </div>
      )}
    </>
  );
}

const emptyPanelProps: PanelProps = {
  expanded: true,
  onHeaderClick: undefined,
  running: false,
  tick: 0,
  clockTime: 0,
  timeSeries: {},
  getFullTimeSeries: () => ({}),
  timelineEntries: [],
  constraintResults: [],
  streamingActions: [],
  expressionResults: [],
};
