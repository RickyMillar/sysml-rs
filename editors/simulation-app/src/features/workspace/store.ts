/**
 * Workspace UI store — local client state only.
 *
 * Per ADR-004 / simulation-polish-plan section 2, this store holds
 * synchronous client-only state. No remote data.
 *
 * `workspaceRoot` is persisted to localStorage so a browser reload
 * (or hard-navigation to a deep link) doesn't lose which workspace
 * the user was looking at. Everything else stays session-scoped.
 */

import { create } from 'zustand';

const WORKSPACE_ROOT_KEY = 'sysml.workspaceRoot';

function readPersistedWorkspaceRoot(): string | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(WORKSPACE_ROOT_KEY);
    return raw && raw.length > 0 ? raw : null;
  } catch {
    return null;
  }
}

function writePersistedWorkspaceRoot(root: string | null): void {
  if (typeof window === 'undefined') return;
  try {
    if (root == null || root.length === 0) {
      window.localStorage.removeItem(WORKSPACE_ROOT_KEY);
    } else {
      window.localStorage.setItem(WORKSPACE_ROOT_KEY, root);
    }
  } catch {
    // localStorage full / blocked — silently degrade; the in-memory
    // store still works for the lifetime of the tab.
  }
}

/**
 * Which top-level workflow surface the user is on. Kept in sync with
 * the URL by `useActiveToolRouteSync` in `App.tsx`. Values MUST match
 * the `id` field of a `WorkflowDescriptor` in `workflows/routes.ts` —
 * any mismatch sends the router into a redirect loop.
 *
 * `session` remains the historical id for the Run workflow; the rest
 * match the current workflow descriptor ids.
 */
export type ActiveTool =
  | 'session'
  | 'compare'
  | 'browse'
  | 'requirements'
  | 'verify'
  | 'analyze'
  | 'sweep'
  | 'montecarlo'
  | 'trade-study'
  | 'sensitivity';

/**
 * Which utility drawer panel is currently open in the right-side
 * drawer. `null` means no drawer. Lifted out of `UtilityDrawer`'s
 * local state in Phase 3 so source-preview clicks can promote into
 * the Source panel from anywhere in the app (diagnostics rows, view
 * rows, tree rows, traceability cells).
 */
export type ActiveUtility =
  | 'diagnostics'
  | 'archive'
  | 'breakpoints'
  | 'views'
  | 'source'
  | 'integrations'
  | 'debug';

/**
 * Result surfaced to the user after the most recent workspace load.
 *
 * Bucket 5-followup (2026-05-05): when `sysml.load_workspace` returns
 * with `error_count > 0` we want a visible banner pointing at the
 * Diagnostics drawer — silently dropping into the Views panel with
 * fewer rows is the bug we're fixing.
 */
export interface WorkspaceLoadStatus {
  errorCount: number;
  /** Pre-formatted strings: "<file>:<line>: <message>". */
  errors: string[];
  /** Whether the user has dismissed the banner for this load result. */
  dismissed: boolean;
}

interface WorkspaceUIState {
  /** Which top-level tool surface is active. */
  activeTool: ActiveTool;

  /** Workspace root path (from URL params or explicit load). */
  workspaceRoot: string | null;

  /** ID of the selected runnable model target. */
  activeSessionTarget: string | null;

  /** Panel sizing state. */
  panelSizes: {
    treePanelWidth: number;
    rightPanelWidth: number;
    bottomPanelHeight: number;
  };

  /** Whether the source-text panel is visible. */
  sourceVisible: boolean;

  /**
   * Which utility panel is currently open in the right-side drawer.
   * Phase 3: lifted out of `UtilityDrawer` so source-preview clicks
   * can promote into the Source panel from any row in the app.
   */
  activeUtility: ActiveUtility | null;

  /** Most recent workspace-load result. `null` until first load. */
  loadStatus: WorkspaceLoadStatus | null;

  // Actions
  setActiveTool: (tool: ActiveTool) => void;
  setWorkspaceRoot: (root: string | null) => void;
  setActiveSessionTarget: (targetId: string | null) => void;
  setPanelSize: (key: keyof WorkspaceUIState['panelSizes'], value: number) => void;
  toggleSource: () => void;
  setActiveUtility: (utility: ActiveUtility | null) => void;
  setLoadStatus: (status: WorkspaceLoadStatus | null) => void;
  dismissLoadStatus: () => void;
}

export const useWorkspaceUIStore = create<WorkspaceUIState>((set) => ({
  activeTool: 'session',
  workspaceRoot: readPersistedWorkspaceRoot(),
  activeSessionTarget: null,
  panelSizes: {
    treePanelWidth: 240,
    rightPanelWidth: 320,
    bottomPanelHeight: 200,
  },
  sourceVisible: false,
  activeUtility: null,
  loadStatus: null,

  setActiveTool: (activeTool) => set({ activeTool }),
  setWorkspaceRoot: (workspaceRoot) => {
    writePersistedWorkspaceRoot(workspaceRoot);
    set({ workspaceRoot });
  },
  setActiveSessionTarget: (activeSessionTarget) => set({ activeSessionTarget }),
  setPanelSize: (key, value) =>
    set((s) => ({ panelSizes: { ...s.panelSizes, [key]: value } })),
  toggleSource: () => set((s) => ({ sourceVisible: !s.sourceVisible })),
  setActiveUtility: (activeUtility) => set({ activeUtility }),
  setLoadStatus: (loadStatus) => set({ loadStatus }),
  dismissLoadStatus: () =>
    set((s) =>
      s.loadStatus ? { loadStatus: { ...s.loadStatus, dismissed: true } } : {},
    ),
}));
