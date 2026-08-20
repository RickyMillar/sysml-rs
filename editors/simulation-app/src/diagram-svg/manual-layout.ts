/**
 * Manual-layout store — the persistent drag layer of the 3-layer position
 * model (elk base position + manual delta + viewport transform).
 *
 * Contract (D-I1): a node the user drags stays EXACTLY where it was dropped.
 * elk's INTERACTIVE mode cannot pin coordinates — position seeds only bias
 * layering/ordering, so a post-drop reflow re-derives placement and snaps the
 * node back. The dropped delta therefore persists HERE, applied at render
 * time on top of the elk base layout (edges re-anchor via
 * `adjustedEdgePoints`), and is never fed back through elk.
 *
 * Deltas are keyed per view: they survive view reselects and route switches,
 * and one view's moves never leak into another's scene.
 */
import { create } from 'zustand';

export interface ManualDelta {
  dx: number;
  dy: number;
}

interface ManualLayoutState {
  /** view element_id → node element_id → manual delta from the elk base. */
  deltasByView: Record<string, Record<string, ManualDelta>>;
  setDelta: (viewId: string, elementId: string, delta: ManualDelta) => void;
}

export const useManualLayoutStore = create<ManualLayoutState>((set) => ({
  deltasByView: {},
  setDelta: (viewId, elementId, delta) =>
    set((s) => ({
      deltasByView: {
        ...s.deltasByView,
        [viewId]: { ...s.deltasByView[viewId], [elementId]: delta },
      },
    })),
}));
