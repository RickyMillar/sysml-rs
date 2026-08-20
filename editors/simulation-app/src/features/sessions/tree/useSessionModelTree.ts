/**
 * useSessionModelTree — the hook the Phase B1 renderer consumes.
 *
 * Composes three already-existing data sources:
 *   1. `useWorkspaceUris` — the set of loaded model files (used as the
 *      reload-sensitive query key + enabled gate).
 *   2. `useWorkspaceScopedTree` — the ONE workspace-scoped `TreeNode[]`
 *      from `GET /models/__workspace__/tree` (cached by react-query).
 *   3. `useSessionLiveStore` — the streaming snapshot mirror.
 *
 * The tree comes from the backend's merged workspace graph
 * (`workspace_model_tree_best`), so cross-file structure is already
 * correct at the source: a package reopened across files is ONE
 * element, ownership is intact across file boundaries, and per-node
 * file attribution rides on `TreeNode.source_uri`. The hook's job is
 * projection, not merging:
 *   - drop the standard library's `LibraryPackage` roots (Run shows
 *     the user's model, not the stdlib),
 *   - package mode (default): keep real `Package` containers,
 *     normalised by `mergeTopLevelPackages`,
 *   - flat mode: flatten packages and archetype-sort the roots.
 *
 * (Historical: this used to fetch N per-file trees and merge them
 * client-side — cross-file package unification re-implemented in TS.
 * The workspace primitive is the one home for that merge.)
 *
 * Memoisation:
 *   - Structural tree rebuilds only when the workspace tree response
 *     changes (cheap equality via react-query's cached data identity).
 *   - Live-state overlay re-applies on every snapshot change.
 *     Zustand's slice subscription already scopes re-renders to
 *     the snapshot reference, so components calling this hook
 *     re-render at ≤tick rate.
 */
import { useEffect, useMemo, useRef } from 'react';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import {
  useWorkspaceUris,
  useWorkspaceScopedTree,
} from '@/features/packages/queries';
import { useSessionLiveStore } from '../sessionLiveStore';
import { buildModelTree, sortSiblingsByKind } from './buildModelTree';
import { mergeLiveState, type ChangeTracker } from './mergeLiveState';
import type { ModelTreeNode } from './types';

export interface UseSessionModelTreeOptions {
  /** When true (default), the model's real SysML `Package` nodes render
   *  as top-level containers with ownership nesting beneath them. When
   *  false, packages are flattened away and every element lands in one
   *  flat, archetype-sorted root list — the by-type view. */
  groupByPackage?: boolean;
  /**
   * UX closeout #4 (reset-path guard): the session id the caller
   * *expects* to be live right now (typically `useSessionStore`'s
   * `activeSessionId`). When supplied and it doesn't match the live
   * store's own `sessionId`, the live overlay is skipped for this
   * render — the merge falls back to the cheap "no snapshot" path
   * instead of walking a stale (possibly huge) snapshot that belongs
   * to a session the caller has already moved on from.
   *
   * This matters because `RunWorkflow`'s target-switch effect nulls
   * `activeSessionId` a render *before* `useSessionStream`'s
   * enable/disable effect actually calls `sessionLiveStore.reset()` —
   * without this guard, that gap reprocesses the outgoing session's
   * full snapshot at least once more right at the moment of switching,
   * on top of whatever cost the new (idle) render already has.
   *
   * Omit (default) to preserve prior behaviour exactly — always merge
   * whatever the live store currently holds. Existing callers that
   * don't pass this are unaffected.
   */
  expectedSessionId?: string | null;
}

export interface UseSessionModelTreeResult {
  /** The polymorphic tree. Empty when no workspace is loaded. */
  tree: ModelTreeNode[];
  /** True while the workspace tree is being fetched. */
  isLoading: boolean;
  /** The session id the live overlay is currently tracking (null = no stream). */
  liveSessionId: string | null;
  /** The latest snapshot's tick, or 0 when no session is active. Used
   *  by the output/parameter split to compute staleness. */
  currentTick: number;
}

/**
 * Unify top-level SysML packages across the merged per-file roots.
 *
 * With `flattenPackages: false`, every file contributes its real
 * `Package` node(s) as containers. A package can legally be reopened
 * across files (same qualified name in two sources); the backend's
 * merged workspace graph already unifies those into one element, so
 * the merge arm here is a defensive no-op in practice — the pass's
 * real job is display normalisation. Bare top-level elements (members
 * of the anonymous root namespace that aren't inside any package)
 * stay as root siblings — no synthetic "uncategorized" bucket.
 *
 * Order: packages first, sorted by name (the model's structure reads
 * top-down); then any bare elements, archetype-sorted. Package rows
 * are re-tagged `kind: 'part'` so they render with the container
 * styling (`rawKind: 'Package'` still drives the folder icon), the
 * same treatment the old group-by-file view used.
 */
export function mergeTopLevelPackages(
  roots: readonly ModelTreeNode[],
): ModelTreeNode[] {
  const isPackage = (n: ModelTreeNode) =>
    n.rawKind === 'Package' || n.rawKind === 'LibraryPackage';
  const packagesByName = new Map<string, ModelTreeNode>();
  const packageOrder: string[] = [];
  const bare: ModelTreeNode[] = [];

  for (const r of roots) {
    if (!isPackage(r)) {
      bare.push(r);
      continue;
    }
    const key = r.name ?? '(unnamed package)';
    const existing = packagesByName.get(key);
    if (existing) {
      existing.children = [...existing.children, ...r.children];
    } else {
      packagesByName.set(key, {
        ...r,
        kind: 'part',
        name: key,
        children: [...r.children],
      });
      packageOrder.push(key);
    }
  }

  const packages = packageOrder
    .map((k) => packagesByName.get(k)!)
    .sort((a, b) => (a.name ?? '').localeCompare(b.name ?? ''));
  return [...packages, ...sortSiblingsByKind(bare)];
}

/**
 * Walk an array of model-tree roots depth-first, ensuring every node's
 * `id` is unique across the whole workspace. Duplicates can reach the
 * frontend whenever two files import a shared definition — each file's
 * per-URI tree is already deduped by the backend, but the merged
 * cross-file view can legitimately surface the same element's inlined
 * typed-def children under PortUsages in either file. First-seen keeps
 * its id; later occurrences get a fresh uuid + the original moved into
 * `elementId` so detail / hover / AST lookups still resolve to the
 * real element. Mutates in place.
 */
export function dedupeTreeIds(roots: ModelTreeNode[]): void {
  // Deterministic rewrite — every consumer (SessionTreeV2,
  // DetailPanel, breadcrumb, the focusPath store) calls
  // `useSessionModelTree` independently and each builds its own
  // deduped tree. If we used `randomUUID()` here the rewrites would
  // diverge between consumers, and a click in SessionTreeV2 would
  // store a focusPath whose ids no longer exist in DetailPanel's
  // tree — the panel would silently fail to update for deep nodes
  // whose id had to be rewritten. Counter-suffix scheme keeps the
  // first occurrence of every id stable and the Nth duplicate gets
  // `${original}#${N-1}` — same input → same output every call.
  const counts = new Map<string, number>();
  const walk = (n: ModelTreeNode): void => {
    const original = n.id;
    const seenCount = counts.get(original) ?? 0;
    if (seenCount > 0) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (n as any).id = `${original}#${seenCount}`;
      if (!n.elementId || n.elementId === original) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        (n as any).elementId = original;
      }
    }
    counts.set(original, seenCount + 1);
    for (const c of n.children) walk(c);
  };
  for (const r of roots) walk(r);
}

export function useSessionModelTree(
  options: UseSessionModelTreeOptions = {},
): UseSessionModelTreeResult {
  const storeWorkspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(storeWorkspaceRoot);
  const uris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);

  const { data: workspaceTree, isLoading } = useWorkspaceScopedTree(uris);

  // Structural pass. Depends ONLY on the workspace tree response —
  // independent of tick frequency so we pay it once per model load.
  //
  // Two shapes over the same merged-graph response:
  //   groupByPackage = true (default) → package + ownership. Keep the
  //     model's real SysML `Package` nodes as containers (don't
  //     flatten) and nest by ownership beneath them. Cross-file
  //     unification already happened in the backend's merged graph;
  //     `mergeTopLevelPackages` is the display normaliser (container
  //     re-tag + package-first ordering + bare-root handling).
  //   groupByPackage = false → flat by type. Flatten packages away and
  //     archetype-sort the promoted roots into one list ("Parts",
  //     "State machines", … root section headers). The
  //     capability-oriented view.
  const groupByPackage = options.groupByPackage ?? true;
  const structural = useMemo<ModelTreeNode[]>(() => {
    if (!workspaceTree) return [];
    // Run shows the user's model, not the stdlib: when the standard
    // library is loaded, the merged workspace graph includes its
    // LibraryPackage roots (~90+ packages) — drop them here. A user
    // element's tree position is unaffected (library content only
    // ever roots under LibraryPackage).
    const userRoots = workspaceTree.filter(
      (n) => n.kind !== 'LibraryPackage',
    );
    let out: ModelTreeNode[];
    if (groupByPackage) {
      out = mergeTopLevelPackages(
        buildModelTree(userRoots, '', { flattenPackages: false }),
      );
    } else {
      // Backend root-sort is by archetype already, but flattening
      // promotes package children to the root — re-sort the merged
      // list so the section headers group correctly.
      out = sortSiblingsByKind(buildModelTree(userRoots, ''));
    }
    // Id-uniqueness backstop. The backend dedupes ids within the
    // response (`dedupe_tree_node_ids`), so this is normally a no-op —
    // kept because React keys, expansion state and pin state all key
    // on `id` and a collision would corrupt them silently.
    dedupeTreeIds(out);
    return out;
  }, [workspaceTree, groupByPackage]);

  // Live overlay. We subscribe to `snapshot` directly so the hook
  // re-renders once per tick rather than once per scalar_vars entry.
  const rawSnapshot = useSessionLiveStore((s) => s.snapshot);
  const liveSessionId = useSessionLiveStore((s) => s.sessionId);

  // Reset-path guard (UX closeout #4 / #17): when the caller tells us
  // which session it expects to be live and the live store hasn't
  // caught up yet (or has already moved past it), treat this render as
  // "no snapshot" rather than merging a snapshot that belongs to a
  // session the caller no longer cares about. See the option's doc
  // comment for the exact race this closes. `expectedSessionId ===
  // undefined` (option omitted) always passes through unchanged.
  const sessionMismatch =
    options.expectedSessionId !== undefined &&
    liveSessionId !== options.expectedSessionId;
  const snapshot = sessionMismatch ? null : rawSnapshot;

  // Persistent change tracker for Task-3's outputs/parameters split.
  // Lives across ticks so `lastChangedTick` can compare new values
  // against the previous snapshot. Reset on session-id change so
  // ticks from a previous session don't leak.
  //
  // Also reset on in-place session reset (tick regresses backward):
  // the per-attribute "last change @ tick N" stamps are otherwise
  // pinned to ticks that no longer exist after the reset, and the
  // outputs/parameters bucket assignment stays stuck on the
  // pre-reset state.
  const trackerRef = useRef<ChangeTracker>(new Map());
  const lastTickRef = useRef<number>(0);
  useEffect(() => {
    trackerRef.current = new Map();
    lastTickRef.current = 0;
  }, [liveSessionId]);
  const currentTick = snapshot?.tick ?? 0;
  if (currentTick < lastTickRef.current) {
    trackerRef.current = new Map();
  }
  lastTickRef.current = currentTick;

  const tree = useMemo<ModelTreeNode[]>(
    () =>
      mergeLiveState(structural, snapshot, {
        changeTracker: trackerRef.current,
      }),
    [structural, snapshot],
  );

  return {
    tree,
    isLoading,
    liveSessionId,
    currentTick,
  };
}
