/**
 * appServices — shared app-level services mounted by BOTH shells
 * (`AppLayout` legacy, `AppShell` ninebar). Lifted verbatim out of
 * App.tsx in ninebar Phase 1 so the flag-gated shells can share them
 * without a circular App ↔ AppShell import.
 *
 * Exactly one shell renders at a time, so each hook/component here still
 * mounts once per app — the same contract as before the lift.
 */

import { useEffect, useRef, useState } from 'react';
import { useLocation } from 'react-router-dom';
import { useWorkspaceStore } from '@/store/workspace';
import {
  useWorkspaceUIStore,
  type ActiveTool,
} from '@/features/workspace/store';
import { useLoadWorkspace } from '@/features/packages/queries';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { useSessionStore } from '@/features/sessions/store';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';
import { useSelectionStore } from '@/features/selection/store';
import { workflowIdForPath } from '@/workflows';

// ── Test hooks (programmatic data access) ────────────────────────────
// Exposes read-only helpers on `window` so Playwright tests can assert
// against the same data the UI is rendering, without having to scrape
// the canvas. Mounting them once at app boot keeps them stable across
// re-renders.

export function useTestHooks() {
  useEffect(() => {
    (window as any).__getTimeSeries = () =>
      useTimeSeriesStore.getState().getTimeSeries();
    (window as any).__getActiveSessionId = () =>
      useSessionStore.getState().activeSessionId;
    (window as any).__setPlotSelection = (names: string[]) => {
      const sid = useSessionStore.getState().activeSessionId;
      if (!sid) return false;
      usePlotSelectionStore.getState().setSelected(sid, names);
      return true;
    };
    // S4.T4 — minimal selection-store hook so Playwright specs can drive
    // the sneak-peek / source-panel flow without rendering a full tree.
    (window as any).__selectionStoreForTests = {
      select: (uri: string | null, id: string | null) =>
        useSelectionStore.getState().select(uri, id),
    };
    // S4.T5 — workspace-store hook lets Playwright pin the focused URI
    // for the sneak-peek path (the source-preview popover reads `focusedUri`
    // to pass into `<SneakPeek>`). No-op outside test runs.
    (window as any).__workspaceStoreForTests = {
      setFocusedUri: (uri: string | null) => useWorkspaceStore.setState({ focusedUri: uri }),
      /** Drive the diagram view picker (the documented test surface for
       *  selecting a declared view by ElementId). */
      setSelectedViewId: (id: string | null) => useWorkspaceStore.getState().setSelectedViewId(id),
      /** Read the current selection back — lets a test verify its
       *  setSelectedViewId survived async workspace-load resets. */
      getSelectedViewId: () => useWorkspaceStore.getState().selectedViewId,
      /**
       * Phase 1 — Playwright hook so monaco-live specs can hydrate a
       * focused file into the workspace store without spinning up a
       * real backend load. Inserts a FileState into `loadedFiles` so
       * `SourcePanel` mounts its live editor.
       */
      setFocusedFile: (uri: string, source: string) => {
        const next = new Map(useWorkspaceStore.getState().loadedFiles);
        next.set(uri, { uri, source, dirty: false, tree: [] });
        useWorkspaceStore.setState({ loadedFiles: next, focusedUri: uri });
      },
    };
    return () => {
      delete (window as any).__getTimeSeries;
      delete (window as any).__getActiveSessionId;
      delete (window as any).__setPlotSelection;
      delete (window as any).__selectionStoreForTests;
      delete (window as any).__workspaceStoreForTests;
    };
  }, []);
}

// ── Auto-load workspace from URL query param ─────────────────────────
// Must render INSIDE <Providers> so react-query context is available.

export function AutoLoadWorkspace() {
  const setWorkspaceRoot = useWorkspaceUIStore((s) => s.setWorkspaceRoot);
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const loadWorkspace = useLoadWorkspace();
  const didAutoLoad = useRef(false);
  const location = useLocation();

  useEffect(() => {
    if (didAutoLoad.current) return;

    // Two ways to pick a root:
    //   1. A persisted root from a previous session (via B12 localStorage).
    //      In that case the store is already populated on mount, and the
    //      backend needs to be re-primed for this fresh page load — the
    //      backend process has no memory of the last session's load.
    //   2. A `?workspace=<path>` query param on the URL. Always wins so
    //      deep links and shared URLs behave predictably.
    const raw = location.search || window.location.search;
    const params = new URLSearchParams(raw);
    const rootFromUrl = params.get('workspace');

    if (rootFromUrl) {
      didAutoLoad.current = true;
      setWorkspaceRoot(rootFromUrl);
      loadWorkspace.mutate(rootFromUrl);
      return;
    }

    if (workspaceRoot) {
      didAutoLoad.current = true;
      loadWorkspace.mutate(workspaceRoot);
    }
  }, [setWorkspaceRoot, loadWorkspace, workspaceRoot, location.search]);

  return null;
}

// ── activeTool <-> route sync ────────────────────────────────────────
// The legacy `useWorkspaceUIStore.activeTool` remains a derived signal
// for older call-sites that still read the current workflow from the
// workspace UI store. Mirror URL → store so those reads stay current.

export function useActiveToolRouteSync() {
  const location = useLocation();
  const activeTool = useWorkspaceUIStore((s) => s.activeTool);
  const setActiveTool = useWorkspaceUIStore((s) => s.setActiveTool);

  // URL → store (one-way). `activeTool` is a derived signal — it
  // mirrors the current route so legacy readers can still ask "what
  // workflow am I on?" through the zustand slice. External callers that
  // used to call `setActiveTool('x')` to *navigate* must now call
  // `navigate('/x')` directly.
  useEffect(() => {
    const id = workflowIdForPath(location.pathname);
    if (!id) return;
    if (id !== activeTool) {
      setActiveTool(id as ActiveTool);
    }
  }, [location.pathname, activeTool, setActiveTool]);
}

// ── Global workspace bar ─────────────────────────────────────────────

export function WorkspaceBar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const setWorkspaceRoot = useWorkspaceUIStore((s) => s.setWorkspaceRoot);
  const loadWorkspace = useLoadWorkspace();
  const [draft, setDraft] = useState(workspaceRoot ?? '');

  useEffect(() => {
    setDraft(workspaceRoot ?? '');
  }, [workspaceRoot]);

  const load = () => {
    const root = draft.trim();
    if (!root) return;
    setWorkspaceRoot(root);
    loadWorkspace.mutate(root);
  };

  return (
    <div
      data-testid="workspace-bar"
      className="flex items-center gap-2 px-3 shrink-0"
      style={{
        height: 34,
        background: 'var(--surface-container-lowest)',
        borderBottom: '1px solid var(--outline-variant)',
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 15, color: 'var(--primary)' }}>
        folder_open
      </span>
      <span style={{ fontSize: 11, color: 'var(--outline)', fontWeight: 700 }}>Workspace</span>
      <input
        data-testid="workspace-bar-input"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') load();
        }}
        placeholder="/path/to/workspace"
        style={{
          flex: 1,
          minWidth: 180,
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          padding: '4px 8px',
          fontSize: 11,
        }}
      />
      <button
        type="button"
        data-testid="workspace-bar-load"
        onClick={load}
        disabled={!draft.trim() || loadWorkspace.isPending}
        className="inline-flex items-center gap-1 rounded"
        style={{
          border: '1px solid var(--outline-variant)',
          background: 'var(--primary-container)',
          color: 'var(--on-primary-container)',
          padding: '4px 8px',
          fontSize: 11,
          fontWeight: 700,
          cursor: !draft.trim() || loadWorkspace.isPending ? 'not-allowed' : 'pointer',
          opacity: !draft.trim() || loadWorkspace.isPending ? 0.6 : 1,
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: 13 }}>
          {loadWorkspace.isPending ? 'progress_activity' : 'refresh'}
        </span>
        {workspaceRoot ? 'Reload' : 'Load'}
      </button>
      {workspaceRoot && (
        <span
          data-testid="workspace-bar-current"
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', maxWidth: 260, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
          title={workspaceRoot}
        >
          {workspaceRoot}
        </span>
      )}
    </div>
  );
}
