/**
 * useDiagramUrlSync — Phase 4 step 2 (Bucket 5.R3 rewrite).
 *
 * Bi-directional sync between the diagram pane state and `/run`'s URL
 * search string:
 *
 *   `/run?uri=<focused-file>&view_id=<view-element-id>`
 *
 * The previous version carried `?view=<view-kind>` (general / state /
 * interconnection / …). That encoded the FE's view-kind enum, which
 * the views-first-class roadmap deletes. Diagrams are now driven by
 * **declared** `ViewDefinition` / `ViewUsage` elements; a sharable URL
 * has to point at one of them by ElementId. There is no "view kind"
 * to share independent of an authored view.
 *
 * On mount the hook reads the params and applies them to the workspace
 * store (via `focusFile` / `setSelectedViewId`). After mount it watches
 * the store and writes URL params back via `history.replaceState` so
 * the URL stays in sync without polluting browser history.
 *
 * The hook is intentionally narrow — it does NOT touch session state
 * (`useDrillReceiver` owns that handshake) and ignores unknown query
 * keys so callers can stack their own params on the same URL without
 * collision.
 */

import { useEffect, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useWorkspaceStore } from '@/store/workspace';

/**
 * Pure parser — pulls the diagram-state keys out of a URL search
 * string. Returns null when neither key is present so callers can
 * skip the apply step entirely.
 */
export function parseDiagramUrlParams(search: string): {
  uri: string | null;
  viewId: string | null;
} | null {
  const params = new URLSearchParams(search);
  const uri = params.get('uri');
  const viewId = params.get('view_id');
  if (!uri && !viewId) return null;
  return { uri, viewId };
}

/**
 * Build the next search string by overlaying `uri` / `view_id` on top
 * of the current params. Existing keys (workspace, drill-from-verdict
 * handshake) are preserved untouched; absent values are removed so
 * the URL stays minimal.
 */
export function applyDiagramUrlParams(
  search: string,
  uri: string | null,
  viewId: string | null,
): string {
  const params = new URLSearchParams(search);
  if (uri) params.set('uri', uri);
  else params.delete('uri');
  if (viewId) params.set('view_id', viewId);
  else params.delete('view_id');
  const out = params.toString();
  return out.length > 0 ? `?${out}` : '';
}

export function useDiagramUrlSync() {
  const { search, pathname } = useLocation();
  const navigate = useNavigate();
  const consumedRef = useRef(false);

  // ── Mount: URL → store ──────────────────────────────────────────
  useEffect(() => {
    if (consumedRef.current) return;
    consumedRef.current = true;
    const parsed = parseDiagramUrlParams(search);
    if (!parsed) return;
    const ws = useWorkspaceStore.getState();
    if (parsed.uri && ws.focusedUri !== parsed.uri) {
      void ws.focusFile(parsed.uri);
    }
    if (parsed.viewId && ws.selectedViewId !== parsed.viewId) {
      ws.setSelectedViewId(parsed.viewId);
    }
  }, [search]);

  // ── Store → URL ─────────────────────────────────────────────────
  // Subscribe outside of React rendering so the URL update doesn't
  // require a rerender of consumers that don't otherwise care about
  // the focused file or selected view.
  useEffect(() => {
    const sync = (focusedUri: string | null, viewId: string | null) => {
      // `window.location.search` always reflects the live URL — using
      // the React Router snapshot here would lose params written by
      // sibling effects within the same tick.
      const nextSearch = applyDiagramUrlParams(
        window.location.search,
        focusedUri,
        viewId,
      );
      if (nextSearch === window.location.search) return;
      navigate({ pathname, search: nextSearch }, { replace: true });
    };
    // Push the current snapshot so the URL reflects whatever the store
    // already holds — covers the case where the user picked a file
    // before this hook mounted (e.g. via the workspace loader).
    const snap = useWorkspaceStore.getState();
    sync(snap.focusedUri, snap.selectedViewId);

    return useWorkspaceStore.subscribe((state, prev) => {
      if (
        state.focusedUri === prev.focusedUri &&
        state.selectedViewId === prev.selectedViewId
      ) {
        return;
      }
      sync(state.focusedUri, state.selectedViewId);
    });
  }, [navigate, pathname]);
}
