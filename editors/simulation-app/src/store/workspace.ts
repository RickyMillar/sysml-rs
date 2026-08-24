/**
 * Workspace store — owns the loaded model files, diagram state, and capabilities.
 *
 * Workspace-centric: multiple files loaded simultaneously, cross-file import
 * resolution via the backend, activities survive file switches.
 */

import { create } from 'zustand';
import type { TreeNode, WorkspaceTreeNode } from '../types/element';
import type { GeometryModel, TableModel, TreeModel } from '../shared/api/model';

// ── Types ──────────────────────────────────────────────────────────────

export interface Capabilities {
  hasStateMachines: boolean;
  hasActionFlows: boolean;
  hasOdeDynamics: boolean;
  hasPortFlows: boolean;
  hasMultipleSubsystems: boolean;
  hasConstraints: boolean;
  hasRequirements: boolean;
  hasTradeStudies: boolean;
  stateMachineNames: string[];
  actionFlowNames: string[];
  tradeStudyNames: string[];
}

export interface FileState {
  uri: string;
  source: string;
  dirty: boolean;
  tree: TreeNode[];
}

// ── Store ──────────────────────────────────────────────────────────────

interface WorkspaceState {
  // Connection
  apiConnected: boolean;

  // Workspace (multi-file)
  workspaceRoot: string | null;
  loadedFiles: Map<string, FileState>;
  focusedUri: string | null;

  /**
   * Bumped on every same-root workspace reload (queries.ts::
   * hydrateWorkspaceStore with `reload: true`). Consumers that cache
   * file-derived state outside this store key on it so a reload drops
   * their caches: SourcePanel remounts its Monaco (fresh LSP
   * didClose/didOpen re-syncs the server-side editor overlay from
   * disk) and re-fetches source under a new react-query key instead
   * of seeding from the pre-reload cache.
   */
  reloadEpoch: number;

  // Non-graph ViewModel data for the focused view. At most one is non-null;
  // graph views render directly from ViewModel.scene in SvgCanvas.
  tableModel: TableModel | null;
  geometryModel: GeometryModel | null;
  treeModel: TreeModel | null;

  /**
   * Currently selected ViewDefinition / ViewUsage ElementId. Drives
   * which view the ViewsPanel highlights and which one the URL
   * encodes (`?view_id=<id>`). `null` means no declared view is
   * picked — the diagram pane shows its empty state rather than a
   * silent default render. (Bucket 5 anti-pattern: "Do not synthesize
   * an implicit default view".)
   */
  selectedViewId: string | null;

  // Merged model tree
  workspaceTree: WorkspaceTreeNode[];

  // Capabilities (union across all loaded files)
  capabilities: Capabilities;

  // Per-URI stats from backend (element kind → count)
  statsCache: Map<string, Record<string, number>>;

  // Actions
  setApiConnected: (connected: boolean) => void;
  setWorkspaceRoot: (root: string | null) => void;

  /** Switch diagram to show a different file. Activities on other files unaffected. */
  focusFile: (uri: string) => Promise<void>;

  /**
   * Set the selected ViewDefinition / ViewUsage. ViewsPanel drives this
   * when the user picks a declared view; URL sync mirrors it through
   * `?view_id=`. Non-graph data arrives through `setNonGraphModel`.
   */
  setSelectedViewId: (id: string | null) => void;

  /** Update source text for a file. */
  updateSource: (uri: string, source: string) => void;
  /**
   * Seed the buffer for a freshly-hydrated file. Workspace hydrate
   * (queries.ts::hydrateWorkspaceStore) only pulls tree + stats — file
   * source stays `''` until the user opens the file. `seedSource` writes
   * the fetched text into the store WITHOUT marking the file dirty, so
   * the LSP doesn't think the user edited it on first open.
   */
  seedSource: (uri: string, source: string) => void;

  /**
   * Phase 3 — promote a URI to the focused file without clearing the
   * diagram or selected view. `focusFile` is the workflow-level
   * "switch which file I'm editing" action; this is the surgical
   * "show this URI in the Source drawer alongside whatever I'm
   * already doing" set. Skips if the URI isn't in `loadedFiles` so
   * the live editor doesn't mount on a blank buffer.
   */
  setFocusedUri: (uri: string) => void;

  /** Set the typed non-graph data carried by a ViewModel, if any. */
  setNonGraphModel: (
    model:
      | { kind: 'table'; data: TableModel }
      | { kind: 'geometry'; data: GeometryModel }
      | { kind: 'tree'; data: TreeModel }
      | null,
  ) => void;

  /** Reset entire workspace. */
  reset: () => void;

  /**
   * Wipe every per-workspace slot in the FE store: loaded files, stats,
   * focused diagram models, selected view, capabilities, workspace tree.
   * `workspaceRoot` is preserved so the URL bar / load-button copy
   * doesn't flicker; the workspace-load action sets the new root after
   * this clear runs.
   *
   * Bucket 5-followup (2026-05-05): without this, switching workspaces
   * left the previous root's loadedFiles / focusedUri / view data intact,
   * so the diagram pane kept rendering the old espresso-production-cell while
   * the Views drawer started populating from the new root. Mirror of
   * the backend's scope-prefix retain in `sysml.load_workspace`.
   */
  clearWorkspace: () => void;
}

function emptyCapabilities(): Capabilities {
  return {
    hasStateMachines: false,
    hasActionFlows: false,
    hasOdeDynamics: false,
    hasPortFlows: false,
    hasMultipleSubsystems: false,
    hasConstraints: false,
    hasRequirements: false,
    hasTradeStudies: false,
    stateMachineNames: [],
    actionFlowNames: [],
    tradeStudyNames: [],
  };
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  apiConnected: false,
  workspaceRoot: null,
  loadedFiles: new Map(),
  focusedUri: null,
  reloadEpoch: 0,
  tableModel: null,
  geometryModel: null,
  treeModel: null,
  selectedViewId: null,
  workspaceTree: [],
  capabilities: emptyCapabilities(),
  statsCache: new Map(),

  setApiConnected: (apiConnected) => set({ apiConnected }),
  setWorkspaceRoot: (workspaceRoot) => set({ workspaceRoot }),

  focusFile: async (uri) => {
    const { loadedFiles } = get();
    if (!loadedFiles.has(uri)) return;
    // Switching the focused file clears the previously rendered
    // diagram so the user gets the empty state until they pick a
    // declared view for the new file. No backend round-trip.
    set({
      focusedUri: uri,
      tableModel: null,
      geometryModel: null,
      treeModel: null,
      selectedViewId: null,
    });
  },

  setSelectedViewId: (id) => set({ selectedViewId: id }),

  setFocusedUri: (uri) => {
    const { loadedFiles } = get();
    if (!loadedFiles.has(uri)) return;
    set({ focusedUri: uri });
  },

  updateSource: (uri, source) => {
    set((state) => {
      const next = new Map(state.loadedFiles);
      const file = next.get(uri);
      if (file) {
        next.set(uri, { ...file, source, dirty: true });
      }
      return { loadedFiles: next };
    });
  },

  seedSource: (uri, source) => {
    set((state) => {
      const next = new Map(state.loadedFiles);
      const file = next.get(uri);
      if (file && file.source === '') {
        next.set(uri, { ...file, source, dirty: false });
        return { loadedFiles: next };
      }
      return {};
    });
  },

  setNonGraphModel: (model) => {
    if (model === null) {
      set({ tableModel: null, geometryModel: null, treeModel: null });
      return;
    }
    switch (model.kind) {
      case 'table':
        set({ tableModel: model.data, geometryModel: null, treeModel: null });
        return;
      case 'geometry':
        set({ tableModel: null, geometryModel: model.data, treeModel: null });
        return;
      case 'tree':
        set({ tableModel: null, geometryModel: null, treeModel: model.data });
        return;
    }
  },

  reset: () =>
    set({
      workspaceRoot: null,
      loadedFiles: new Map(),
      focusedUri: null,
      reloadEpoch: 0,
      tableModel: null,
      geometryModel: null,
      treeModel: null,
      selectedViewId: null,
      workspaceTree: [],
      capabilities: emptyCapabilities(),
      statsCache: new Map(),
    }),

  clearWorkspace: () =>
    set({
      loadedFiles: new Map(),
      focusedUri: null,
      tableModel: null,
      geometryModel: null,
      treeModel: null,
      selectedViewId: null,
      workspaceTree: [],
      capabilities: emptyCapabilities(),
      statsCache: new Map(),
    }),
}));

