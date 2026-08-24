/**
 * Workspace/model query hooks shared by the shell and workflows.
 *
 * All backend calls go through `httpPost` / `httpGet` from the shared
 * API layer, and are cached via @tanstack/react-query.
 */

import { useQuery, useMutation, useQueryClient, useIsMutating } from '@tanstack/react-query';
import { httpPost, httpGet } from '@/shared/api/http';
import { WORKSPACE_URI } from '@/shared/api/model';
import { fetchWorkspaceCapabilities } from '@/shared/api/capabilities';
import type { TreeNode } from '@/types/element';
import { useWorkspaceStore, type FileState } from '@/store/workspace';
import { useWorkspaceUIStore } from '@/features/workspace/store';

// ── Types ────────────────────────────────────────────────────────────────

/** Shape returned by the sysml.load_workspace backend command. */
interface WorkspaceLoadRaw {
  loaded_uris: string[];
  error_count: number;
  errors: string[];
}

export interface WorkspaceLoadResult {
  uris: string[];
}

/** Normalise the raw backend response into the shape the UI expects. */
function normaliseWorkspaceLoad(raw: WorkspaceLoadRaw): WorkspaceLoadResult {
  return {
    uris: (raw.loaded_uris ?? []).filter((u) => u !== '__workspace__'),
  };
}

export interface ModelStats {
  elements_by_kind?: Record<string, number>;
  [key: string]: unknown;
}

// ── Query keys ───────────────────────────────────────────────────────────

export const packageKeys = {
  workspace: (root: string) => ['workspace', root] as const,
  tree: (uri: string) => ['tree', uri] as const,
  stats: (uri: string) => ['stats', uri] as const,
};

/** Shared mutation key so all useLoadWorkspace instances are observable. */
export const LOAD_WORKSPACE_MUTATION_KEY = ['load-workspace'] as const;

// ── Hooks ────────────────────────────────────────────────────────────────

/**
 * Load a workspace directory. Returns the list of loaded file URIs.
 * This is a mutation (side-effectful: backend loads and parses files),
 * but we cache the result as a query for downstream reads.
 */
export function useLoadWorkspace() {
  const qc = useQueryClient();

  return useMutation({
    mutationKey: LOAD_WORKSPACE_MUTATION_KEY,
    mutationFn: async (root: string) => {
      // Bucket 5-followup (2026-05-05): wipe stale FE state BEFORE the
      // backend round-trip so a workspace switch never paints the old
      // root's diagram while the new root's tree is still loading. The
      // backend's `sysml.load_workspace` already drops doc graphs that
      // sit outside the new root; this mirrors that on the FE side.
      // We only clear when the root actually changes — re-loading the
      // same root is a refresh, not a switch, and shouldn't drop
      // mid-flight panel state.
      const prevRoot = useWorkspaceStore.getState().workspaceRoot;
      if (prevRoot !== null && prevRoot !== root) {
        useWorkspaceStore.getState().clearWorkspace();
      }
      const raw = await httpPost<WorkspaceLoadRaw>('/api/command', {
        command: 'sysml.load_workspace',
        params: { root },
      });
      return raw;
    },
    onSuccess: (raw, root) => {
      const data = normaliseWorkspaceLoad(raw);
      // Cache the result so useWorkspaceTree can read it synchronously
      qc.setQueryData(packageKeys.workspace(root), data);

      // Bucket 5-followup (2026-05-05): surface parse errors as a
      // dismissible banner. The Diagnostics drawer already lists them
      // per-file via `useDiagnostics`, but a blank Views drawer with
      // no clue why is the symptom we're fixing — the banner names
      // the count and points the user there.
      useWorkspaceUIStore.getState().setLoadStatus({
        errorCount: raw.error_count ?? 0,
        errors: raw.errors ?? [],
        dismissed: false,
      });

      // Re-loading the workspace re-parses every file, which regenerates
      // every element UUID. Any cached per-file tree, element detail,
      // stats, OR sysml.query view-list holds stale IDs that no longer exist in
      // the backend graph — clicking those rows 404s on
      // /views/<old-id>/render. Invalidate so the next render refetches
      // with the new IDs. Bucket 5-followup (2026-05-05) added the
      // sysml.query view-list / views.render entries; without them, ViewsPanel
      // showed views with stale UUIDs and every click hit 404.
      void qc.invalidateQueries({ queryKey: ['workspace-tree'] });
      void qc.invalidateQueries({ queryKey: ['views', 'list'] });
      void qc.invalidateQueries({ queryKey: ['views', 'render'] });
      void qc.invalidateQueries({ queryKey: ['diagnostics'] });
      // Requirement rows carry element UUIDs (row ids + link refs) —
      // stale after a reload just like the tree/views above.
      void qc.invalidateQueries({ queryKey: ['requirement-rows'] });
      void qc.invalidateQueries({ queryKey: ['requirement-detail'] });
      // Suspect records key on those same ids; baselines list is cheap
      // to refresh and re-runs the save_workspace snapshot (idempotent).
      void qc.invalidateQueries({ queryKey: ['baselines'] });
      void qc.invalidateQueries({ queryKey: ['suspects'] });
      for (const uri of data.uris) {
        void qc.invalidateQueries({ queryKey: packageKeys.tree(uri) });
        void qc.invalidateQueries({ queryKey: packageKeys.stats(uri) });
      }

      // Bridge react-query → legacy Zustand workspace store: components
      // such as SessionStatusBar, SessionTree, and useModelCapabilities
      // read loadedFiles / statsCache from useWorkspaceStore. Without
      // this bridge, those views show "No runnable capabilities".
      //
      // Same-root reload (2026-07-16 staleness fix): when the root is
      // unchanged this is a disk re-sync, not a switch — every URI must
      // re-hydrate (tree/stats regenerate with new element UUIDs, and
      // cached source text may predate an on-disk edit). workspaceRoot
      // still holds the PREVIOUS root here; hydrate updates it.
      //
      // Fire-and-forget: the per-URI tree/stats fetches happen in the
      // background so the mutation settles immediately and the loading
      // spinner clears as soon as URIs are known.
      const isReload = useWorkspaceStore.getState().workspaceRoot === root;
      hydrateWorkspaceStore(root, data.uris, { reload: isReload }).catch((e) => {
        console.error('[workspace] hydrate failed unexpectedly', e);
      });
    },
  });
}

/** Hydrate the legacy useWorkspaceStore with tree + stats for each URI
 *  returned by sysml.load_workspace. Runs in the background so it does
 *  not block the mutation from settling. Idempotent: skips already-
 *  loaded URIs and merges into the existing maps — EXCEPT on a
 *  same-root reload (`reload: true`), where every URI re-hydrates:
 *  the backend re-parsed from disk, so cached trees hold dead element
 *  UUIDs and cached source text may predate an on-disk edit. Reload
 *  also bumps `reloadEpoch`, which drops SourcePanel's Monaco buffer +
 *  file-source cache (see the store doc on `reloadEpoch`). */
async function hydrateWorkspaceStore(
  root: string,
  uris: string[],
  opts: { reload: boolean } = { reload: false },
): Promise<void> {
  const state = useWorkspaceStore.getState();
  if (state.workspaceRoot !== root) {
    useWorkspaceStore.setState({ workspaceRoot: root });
  }
  const newUris = opts.reload ? uris : uris.filter((u) => !state.loadedFiles.has(u));
  if (newUris.length === 0) return;

  // Single batched call replaces N×(GET /tree + POST sysml.stats). Backend
  // implements this as `sysml.workspace.info`.
  interface WorkspaceInfoEntry {
    uri: string;
    tree?: TreeNode[];
    stats?: { elements_by_kind?: Record<string, number> } & Record<string, unknown>;
  }
  let infoEntries: WorkspaceInfoEntry[] = [];
  try {
    infoEntries = await httpPost<WorkspaceInfoEntry[]>('/api/command', {
      command: 'sysml.workspace.info',
      params: { uris: newUris },
    });
  } catch (e) {
    // Was previously a silent warn — upgrade to error so a broken
    // backend surfaces clearly in dev (the UI shows empty panels
    // otherwise with no hint why).
    console.error('[workspace] sysml.workspace.info failed; aborting hydrate', e);
    return;
  }

  const byUri = new Map(infoEntries.map((e) => [e.uri, e] as const));
  const results = newUris.map((uri) => {
    const entry = byUri.get(uri);
    const tree = (entry?.tree ?? []) as TreeNode[];
    const rawStats = entry?.stats;
    const stats: Record<string, number> = rawStats
      ? ((rawStats.elements_by_kind ?? rawStats) as Record<string, number>)
      : {};
    return { uri, tree, stats };
  });

  const hadFocusBeforeHydrate = state.focusedUri !== null;

  useWorkspaceStore.setState((s) => {
    // Reload rebuilds from scratch so files deleted on disk drop out;
    // incremental hydrate merges into the existing maps.
    const nextFiles = opts.reload ? new Map<string, FileState>() : new Map(s.loadedFiles);
    const nextStats = opts.reload
      ? new Map<string, Record<string, number>>()
      : new Map(s.statsCache);
    for (const { uri, tree, stats } of results) {
      // On reload this overwrite intentionally resets `source` to ''
      // (and `dirty` — reload is an explicit re-sync from disk), so
      // SourcePanel lazily re-fetches fresh text instead of keeping a
      // pre-reload buffer.
      nextFiles.set(uri, { uri, source: '', dirty: false, tree });
      nextStats.set(uri, stats);
    }
    // A reloaded root may have lost the focused file (deleted on disk).
    const focusStillLoaded = s.focusedUri !== null && nextFiles.has(s.focusedUri);
    return {
      loadedFiles: nextFiles,
      statsCache: nextStats,
      focusedUri: (focusStillLoaded ? s.focusedUri : null) ?? results[0]?.uri ?? null,
      ...(opts.reload ? { reloadEpoch: s.reloadEpoch + 1 } : {}),
    };
  });

  // Capabilities are backend-owned (S4.T6) — fetch after the loaded
  // files are registered so `sysml.workspace.capabilities`' Salsa
  // query sees the elaborated workspace. Best-effort: leave the
  // existing slot untouched if the round-trip fails so the UI keeps
  // whatever state the previous load produced.
  try {
    const capabilities = await fetchWorkspaceCapabilities();
    useWorkspaceStore.setState({ capabilities });
  } catch (e) {
    console.warn('[workspace] sysml.workspace.capabilities failed during hydrate', e);
  }

  // If THIS hydration call promoted a focused URI from null, fetch its
  // diagram so the renderers (which read the view state from the
  // workspace store) actually render something on workspace load.
  // focusFile loads the focused file's default view.
  if (!hadFocusBeforeHydrate) {
    const newlyFocusedUri = useWorkspaceStore.getState().focusedUri;
    if (newlyFocusedUri) {
      await useWorkspaceStore.getState().focusFile(newlyFocusedUri);
    }
  }
}

/**
 * Returns true if ANY useLoadWorkspace mutation is currently in-flight,
 * regardless of which component fired it.
 */
export function useIsWorkspaceLoading(): boolean {
  return useIsMutating({ mutationKey: LOAD_WORKSPACE_MUTATION_KEY }) > 0;
}

/**
 * Read the cached workspace load result.
 *
 * This is a pure cache reader — it never fires its own network request.
 * The cache is populated by `useLoadWorkspace`'s mutation `onSuccess`
 * via `queryClient.setQueryData`.
 *
 * `enabled: !!root` ensures the query observer subscribes to cache
 * updates so components re-render when setQueryData writes new data.
 * `staleTime: Infinity` prevents react-query from refetching (the
 * queryFn is only a fallback that returns an empty list).
 */
export function useWorkspaceUris(root: string | null) {
  return useQuery({
    queryKey: packageKeys.workspace(root ?? ''),
    queryFn: () => Promise.resolve({ uris: [] as string[] }),
    enabled: !!root,
    staleTime: Infinity,
  });
}

/**
 * Fetch the element tree for a single loaded file URI.
 *
 * Always asks the backend for the `user_facing` projection — that
 * mirrors what the simulation UI is built to render. Spec-mandated
 * wrappers (memberships, FeatureTyping), expression AST, ports,
 * flows, transitions, and chrome (comments, imports) are filtered
 * server-side via `is_user_facing_noise` (R2.1 of the backend-first
 * cleansing audit).
 */
export function useModelTree(uri: string | null) {
  return useQuery({
    queryKey: packageKeys.tree(uri ?? ''),
    queryFn: async () => {
      if (!uri) return [] as TreeNode[];
      return httpGet<TreeNode[]>(
        `/models/${encodeURIComponent(uri)}/tree?view=user_facing`,
      );
    },
    enabled: !!uri,
  });
}

/**
 * Fetch per-file trees for all loaded URIs in one batch.
 * Returns Map<uri, TreeNode[]>.
 *
 * Same `view=user_facing` projection as `useModelTree`.
 */
export function useWorkspaceTree(uris: string[]) {
  return useQuery({
    queryKey: ['workspace-tree', ...uris],
    queryFn: async () => {
      const entries = await Promise.all(
        uris.map(async (uri) => {
          try {
            const tree = await httpGet<TreeNode[]>(
              `/models/${encodeURIComponent(uri)}/tree?view=user_facing`,
            );
            return [uri, tree] as const;
          } catch {
            return [uri, [] as TreeNode[]] as const;
          }
        }),
      );
      return new Map(entries);
    },
    enabled: uris.length > 0,
  });
}

/**
 * Fetch the ONE workspace-scoped tree (`GET /models/__workspace__/tree`).
 *
 * Routes to the backend's merged workspace graph
 * (`workspace_model_tree_best`): every loaded file elaborated into a
 * single graph, so a package reopened across files is unified by
 * construction and ownership is intact across file boundaries — no
 * FE-side cross-file merging. Per-node file attribution comes from
 * `TreeNode.source_uri` (stamped from each element's span), since a
 * merged response has no meaningful per-response uri.
 *
 * NOTE: when the standard library is loaded the merged graph includes
 * its `LibraryPackage` roots — consumers that only want user-model
 * content (the Run session tree) filter those out.
 *
 * Keyed on the loaded-uris list — the same reload-sensitive signal
 * `useWorkspaceTree` uses — so a workspace (re)load invalidates both
 * paths identically.
 */
export function useWorkspaceScopedTree(uris: string[]) {
  return useQuery({
    queryKey: ['workspace-scoped-tree', ...uris],
    queryFn: async () =>
      httpGet<TreeNode[]>(
        `/models/${encodeURIComponent(WORKSPACE_URI)}/tree?view=user_facing`,
      ),
    enabled: uris.length > 0,
  });
}

/**
 * Fetch element stats (kind counts) for a loaded file URI.
 */
export function useModelStats(uri: string | null) {
  return useQuery({
    queryKey: packageKeys.stats(uri ?? ''),
    queryFn: async () => {
      if (!uri) return {} as ModelStats;
      return httpPost<ModelStats>('/api/command', {
        command: 'sysml.stats',
        params: { uri },
      });
    },
    enabled: !!uri,
  });
}

// ── Dependency status (ninebar Phase 1.5 — readiness aggregation) ────────

/** One dependency-resolution failure inside a root's report. */
export interface DependencyFailureWire {
  name: string;
  source: string;
  reason: string;
  message: string;
  action?: string;
}

/**
 * One workspace root's dependency report from `sysml.dependency.status`.
 * Shape branches on whether a manifest was found and hydrated —
 * `failed_dependencies` only appears on the success-with-manifest arm.
 */
export type DependencyStatusRootWire =
  | {
      root: string;
      manifest: string;
      project: string;
      dependency_count: number;
      failed_dependencies: DependencyFailureWire[];
    }
  | { root: string; status: 'no_manifest' }
  | { root: string; status: 'error'; error: string };

/** Wire shape returned by `sysml.dependency.status`. */
export interface DependencyStatusWire {
  roots: DependencyStatusRootWire[];
  summary: Record<string, number>;
}

/**
 * Walk every `root`'s manifest and report per-dependency resolution
 * outcomes (`sysml.dependency.status`). Roots without a discoverable
 * manifest come back as `{status: "no_manifest"}` entries rather than
 * throwing — the query itself never errors on a missing manifest.
 *
 * Parked (`enabled: false`) when `roots` is empty, mirroring
 * `useWorkspaceTree`'s empty-input guard.
 *
 * Consumed by `features/readiness/useModelReadiness` for the readiness
 * aggregation; `features/utilities/DebugDrawer` has its own ad-hoc
 * inline query for the same command (auto-refresh toggle + raw JSON
 * dump) predating this hook — left as-is, out of scope here.
 */
export function useDependencyStatus(roots: string[]) {
  return useQuery({
    queryKey: ['dependency-status', ...roots],
    queryFn: () =>
      httpPost<DependencyStatusWire>('/api/command', {
        command: 'sysml.dependency.status',
        params: { roots },
      }),
    enabled: roots.length > 0,
  });
}

// ── Workspace verify (ninebar Phase 1.5 — static / pre-run) ──────────────

/** Wire shape returned by `sysml.workspace.verify`. */
export interface WorkspaceVerifyResult {
  total_cases: number;
  passed: number;
  failed: number;
  elapsed_ms: number;
  /** File URIs that had at least one failing verification case attributed to them. */
  per_file: string[];
  /**
   * Digest of the current model this verdict set was computed against
   * (B6 provenance root). Landing on the wire in parallel; the case view's
   * `@ <digest7>` chip renders it truncated when present, nothing when
   * absent (older server) — never fabricated (§E coordination pin).
   */
  model_digest?: string;
}

/**
 * Run cross-file static workspace verification — merges every loaded
 * document graph with the library and evaluates every verification
 * case, with NO session involved. This is a validation act, distinct
 * from Phase 4's live verdict matrix (per-run evidence); callers must
 * label it "Static / pre-run" wherever it renders (plan requirement —
 * see `features/readiness/StaticVerifyModal.tsx`).
 */
export function useWorkspaceVerify() {
  return useMutation({
    mutationFn: (timeoutSecs?: number) =>
      httpPost<WorkspaceVerifyResult>('/api/command', {
        command: 'sysml.workspace.verify',
        params: timeoutSecs ? { timeout_secs: timeoutSecs } : {},
      }),
  });
}
