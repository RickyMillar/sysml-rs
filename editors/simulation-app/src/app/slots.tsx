/**
 * slots — portal targets for the AppShell's left rail and bottom strip
 * (ninebar Phase 1.5, Browse floor).
 *
 * `AppShell` mounts the *targets* (`<LeftRailSlot/>` / `<BottomStripSlot/>`)
 * — stable DOM ids the shell owns, inside its left-rail `<aside>` and
 * bottom-strip `<footer>`. A workflow mounts the *content* wrappers
 * (`<LeftRailContent>` / `<BottomStripContent>`) anywhere in its own
 * component tree; each portals its children into the matching target
 * via `createPortal`, so a workflow's left-rail/bottom-strip UI lives
 * with the workflow's own code — AppShell never imports per-workflow
 * components.
 *
 * Deliberately dumb: no descriptor registry (contrast `app/rail/
 * railRegistry.ts`, which stores named, always-resolvable panel
 * descriptors so the right rail can look one up by id at any time). A
 * content wrapper here just announces "I'm mounted" via
 * `useSlotPresenceStore` so `AppShell` can decide two things without
 * ever knowing WHAT was portaled:
 *   - the left-rail fallback: the interim `WorkspaceBar` section shows
 *     only when no workflow has mounted `<LeftRailContent>` (it stays
 *     visible on routes like `/run` today that don't use the slot yet).
 *   - the bottom-strip open/closed state: the strip opens (height auto
 *     within `--strip-min-height`/`--strip-max-height`) while
 *     `<BottomStripContent>` is mounted, and collapses to 0 otherwise.
 */
import { useEffect, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { create } from 'zustand';

export const LEFT_RAIL_SLOT_ID = 'ninebar-left-rail-slot';
export const BOTTOM_STRIP_SLOT_ID = 'ninebar-bottom-strip-slot';

interface SlotPresenceState {
  /** True while some workflow has a `<LeftRailContent>` mounted. */
  leftRailActive: boolean;
  /** True while some workflow has a `<BottomStripContent>` mounted. */
  bottomStripActive: boolean;
  /**
   * Ghost mode (plan §0: "rest are ghost/collapsed"): the strip stays
   * OPEN (its content — e.g. the transport row — remains reachable) but
   * drops its `--strip-min-height` floor so it shrinks to the content's
   * intrinsic height. Run sets this while no session exists.
   */
  bottomStripCollapsed: boolean;
  setLeftRailActive: (active: boolean) => void;
  setBottomStripActive: (active: boolean) => void;
  setBottomStripCollapsed: (collapsed: boolean) => void;
}

/** Not a registry — just presence booleans. See file doc comment. */
export const useSlotPresenceStore = create<SlotPresenceState>((set) => ({
  leftRailActive: false,
  bottomStripActive: false,
  bottomStripCollapsed: false,
  setLeftRailActive: (active) => set({ leftRailActive: active }),
  setBottomStripActive: (active) =>
    set(active ? { bottomStripActive: true } : { bottomStripActive: false, bottomStripCollapsed: false }),
  setBottomStripCollapsed: (collapsed) => set({ bottomStripCollapsed: collapsed }),
}));

/** Mounted once by `AppShell`, inside the left-rail `<aside>`. */
export function LeftRailSlot() {
  return (
    <div
      id={LEFT_RAIL_SLOT_ID}
      data-testid="left-rail-slot"
      className="flex flex-col flex-1 overflow-hidden"
      style={{ minHeight: 0 }}
    />
  );
}

/** Mounted once by `AppShell`, inside the bottom-strip `<footer>`. */
export function BottomStripSlot() {
  return (
    <div
      id={BOTTOM_STRIP_SLOT_ID}
      data-testid="bottom-strip-slot"
      className="flex flex-col flex-1 overflow-hidden"
      style={{ minHeight: 0 }}
    />
  );
}

/** Finds the DOM target by id and toggles the presence flag around the
 *  content wrapper's mounted lifetime. Renders nothing (portal target
 *  `null`) when the target isn't in the DOM — e.g. the legacy
 *  `AppLayout` shell, which never mounts `<LeftRailSlot/>`/
 *  `<BottomStripSlot/>`. A workflow that needs presence under BOTH
 *  shells must provide its own fallback; ninebar-only workflows don't. */
function usePortalTarget(id: string, announce: (active: boolean) => void): HTMLElement | null {
  const [target, setTarget] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setTarget(document.getElementById(id));
    announce(true);
    return () => announce(false);
  }, [id, announce]);
  return target;
}

/**
 * Portals `children` into the shell's left-rail slot. Mounting this
 * REPLACES the interim `WorkspaceBar` section for as long as it stays
 * mounted (see `AppShell.tsx`).
 */
export function LeftRailContent({ children }: { children: ReactNode }) {
  const setActive = useSlotPresenceStore((s) => s.setLeftRailActive);
  const target = usePortalTarget(LEFT_RAIL_SLOT_ID, setActive);
  if (!target) return null;
  return createPortal(children, target);
}

/**
 * Portals `children` into the shell's bottom-strip slot. The strip
 * opens while this is mounted and collapses to 0 otherwise. Pass
 * `collapsed` to keep it open but ghost-height (no min-height floor) —
 * e.g. Run before any session exists: the transport row stays
 * reachable, the chart area doesn't reserve dead space.
 */
export function BottomStripContent({
  children,
  collapsed = false,
}: {
  children: ReactNode;
  collapsed?: boolean;
}) {
  const setActive = useSlotPresenceStore((s) => s.setBottomStripActive);
  const setCollapsed = useSlotPresenceStore((s) => s.setBottomStripCollapsed);
  const target = usePortalTarget(BOTTOM_STRIP_SLOT_ID, setActive);
  useEffect(() => {
    setCollapsed(collapsed);
  }, [collapsed, setCollapsed]);
  if (!target) return null;
  return createPortal(children, target);
}
