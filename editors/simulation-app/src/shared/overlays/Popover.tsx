/**
 * Popover — anchored, non-occluding overlay primitive (ninebar Phase 1).
 *
 * The in-loop-action surface (plan audit F1/F9): variable overrides, the
 * event injector, "apply & re-run" — actions performed *next to* the
 * element being observed, never inside a modal that hides it. Positioned
 * adjacent to `anchorEl` on the requested `placement`, flipping to the
 * opposite side when the preferred side doesn't fit the viewport. Unlike
 * `Modal`, a Popover never covers its anchor.
 *
 * Portalled to `document.body`; `--surface-overlay` / `--radius-md` /
 * `--shadow-float` match the rest of the overlay system (one step down
 * from Modal's `--radius-lg`, since a popover is a lighter-weight
 * surface). Escape and an outside click (outside both the panel and the
 * anchor) both close it. No focus trap and no `role="dialog"` — a
 * popover is a transient action surface, not a modal; content is
 * responsible for its own focus management if it needs any.
 */
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

export type PopoverPlacement = 'top' | 'bottom' | 'left' | 'right';

export interface PopoverProps {
  anchorEl: HTMLElement | null;
  open: boolean;
  onClose: () => void;
  /** Preferred side, relative to the anchor. Defaults to 'bottom'. */
  placement?: PopoverPlacement;
  children: React.ReactNode;
}

/** Screen-edge and anchor clearance, in px. */
const GAP = 8;

interface ResolvedPosition {
  top: number;
  left: number;
  placement: PopoverPlacement;
}

export function Popover({
  anchorEl,
  open,
  onClose,
  placement = 'bottom',
  children,
}: PopoverProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<ResolvedPosition | null>(null);

  // Position (and re-position on resize/scroll) against the anchor.
  useLayoutEffect(() => {
    if (!open || !anchorEl) {
      setPos(null);
      return;
    }
    const compute = () => {
      const anchorRect = anchorEl.getBoundingClientRect();
      const panelRect = panelRef.current?.getBoundingClientRect();
      const size = panelRect ?? { width: 280, height: 160 };
      setPos(resolvePosition(anchorRect, size, placement));
    };
    compute();
    window.addEventListener('resize', compute);
    window.addEventListener('scroll', compute, true);
    return () => {
      window.removeEventListener('resize', compute);
      window.removeEventListener('scroll', compute, true);
    };
  }, [open, anchorEl, placement]);

  // Escape + outside-click close.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    }
    function onPointerDown(e: MouseEvent) {
      const target = e.target as Node;
      if (panelRef.current?.contains(target)) return;
      if (anchorEl?.contains(target)) return;
      onClose();
    }
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('mousedown', onPointerDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('mousedown', onPointerDown);
    };
  }, [open, onClose, anchorEl]);

  if (!open || !anchorEl) return null;

  return createPortal(
    <div
      ref={panelRef}
      data-testid="popover-panel"
      data-placement={pos?.placement ?? placement}
      style={{
        position: 'fixed',
        top: pos?.top ?? -9999,
        left: pos?.left ?? -9999,
        visibility: pos ? 'visible' : 'hidden',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-md)',
        boxShadow: 'var(--shadow-float)',
        zIndex: 1000,
        maxWidth: '92vw',
      }}
    >
      {children}
    </div>,
    document.body,
  );
}

/** Pick a side that fits the viewport (falling back to the opposite side,
 *  then to the preferred side if neither fits) and compute top/left. */
function resolvePosition(
  anchor: DOMRect,
  panel: { width: number; height: number },
  preferred: PopoverPlacement,
): ResolvedPosition {
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  const fits = (p: PopoverPlacement): boolean => {
    switch (p) {
      case 'bottom':
        return anchor.bottom + GAP + panel.height <= vh;
      case 'top':
        return anchor.top - GAP - panel.height >= 0;
      case 'right':
        return anchor.right + GAP + panel.width <= vw;
      case 'left':
        return anchor.left - GAP - panel.width >= 0;
    }
  };

  const opposite: Record<PopoverPlacement, PopoverPlacement> = {
    top: 'bottom',
    bottom: 'top',
    left: 'right',
    right: 'left',
  };

  const placement = fits(preferred)
    ? preferred
    : fits(opposite[preferred])
      ? opposite[preferred]
      : preferred;

  let top: number;
  let left: number;
  if (placement === 'bottom' || placement === 'top') {
    left = clamp(anchor.left + anchor.width / 2 - panel.width / 2, GAP, vw - panel.width - GAP);
    top = placement === 'bottom' ? anchor.bottom + GAP : anchor.top - GAP - panel.height;
  } else {
    top = clamp(anchor.top + anchor.height / 2 - panel.height / 2, GAP, vh - panel.height - GAP);
    left = placement === 'right' ? anchor.right + GAP : anchor.left - GAP - panel.width;
  }
  return { top, left, placement };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}
