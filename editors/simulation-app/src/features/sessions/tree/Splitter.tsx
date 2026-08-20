/**
 * Splitter — minimal horizontal drag handle for vertically stacked
 * panes.
 *
 * Round 2 Task #142. Zone 1 gets a tree (top) + detail region
 * (bottom) split. Users drag this handle to resize the split. The
 * parent owns the current split-pixel state; this component is pure
 * presentation + pointer-event plumbing.
 *
 * Pointer events (not mouse) so the drag works on touch devices as
 * well as the common trackpad / mouse case. setPointerCapture lets
 * the drag continue even if the pointer leaves the tiny handle.
 *
 * Clamping is OWNED BY THE PARENT — the splitter fires a raw
 * `onPositionChange(newHeight)` and the parent decides min / max.
 * Keeping the clamp pure + testable on the parent side meant we
 * didn't need to pass both min and max props in here.
 */
import type { PointerEvent as ReactPointerEvent } from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';

export interface SplitterProps {
  /**
   * Current height of the detail pane (pixels from the bottom of
   * the container). Displayed read-only here; the parent re-renders
   * with a new value after each drag tick.
   */
  detailHeightPx: number;
  /**
   * Called on every pointer move while dragging. Receives the
   * caller's suggestion for the new detail height in pixels — the
   * parent is expected to clamp + commit it.
   */
  onPositionChange: (nextHeightPx: number) => void;
  /**
   * A ref to the container whose bottom edge anchors the split.
   * We compute the new height as `containerBottom - pointerY`.
   */
  containerRef: React.RefObject<HTMLDivElement | null>;
  /** data-testid (default `tree-splitter`). */
  testId?: string;
}

export function Splitter({
  detailHeightPx,
  onPositionChange,
  containerRef,
  testId = 'tree-splitter',
}: SplitterProps) {
  const [dragging, setDragging] = useState(false);
  const originRef = useRef<{ startY: number; startHeight: number } | null>(null);

  const handlePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      (e.target as HTMLDivElement).setPointerCapture(e.pointerId);
      originRef.current = { startY: e.clientY, startHeight: detailHeightPx };
      setDragging(true);
    },
    [detailHeightPx],
  );

  const handlePointerMove = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (!dragging || !originRef.current) return;
      const origin = originRef.current;
      const container = containerRef.current;
      if (!container) return;
      // Upward drag (smaller Y) → grow the detail pane.
      const delta = origin.startY - e.clientY;
      onPositionChange(origin.startHeight + delta);
    },
    [dragging, containerRef, onPositionChange],
  );

  const stopDrag = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      try {
        (e.target as HTMLDivElement).releasePointerCapture(e.pointerId);
      } catch {
        /* pointer already released — drag ended via pointercancel */
      }
      originRef.current = null;
      setDragging(false);
    },
    [],
  );

  // Global escape fallback — if the browser fires pointercancel,
  // we need to stop the drag too so the splitter doesn't look
  // stuck.
  useEffect(() => {
    if (!dragging) return;
    const cancel = () => {
      originRef.current = null;
      setDragging(false);
    };
    window.addEventListener('pointercancel', cancel);
    return () => window.removeEventListener('pointercancel', cancel);
  }, [dragging]);

  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-valuenow={detailHeightPx}
      data-testid={testId}
      data-dragging={dragging || undefined}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={stopDrag}
      style={{
        height: 6,
        flexShrink: 0,
        cursor: 'row-resize',
        background: dragging
          ? 'var(--primary)'
          : 'var(--outline-variant)',
        opacity: dragging ? 0.9 : 0.6,
        transition: dragging ? 'none' : 'opacity 0.15s, background 0.15s',
        touchAction: 'none',
      }}
      onMouseEnter={(e) => {
        if (!dragging) {
          (e.currentTarget as HTMLDivElement).style.opacity = '0.9';
        }
      }}
      onMouseLeave={(e) => {
        if (!dragging) {
          (e.currentTarget as HTMLDivElement).style.opacity = '0.6';
        }
      }}
    />
  );
}

/**
 * Clamp a proposed detail-pane height against the container height.
 * Pure + exported so callers can test + unit-test the clamping
 * behaviour without driving the drag UI.
 *
 * Rules:
 *  - never below `minPx`
 *  - never above `maxFraction * containerHeight`
 *  - when containerHeight is 0 (pre-layout), returns the proposal
 *    unclamped so the first paint doesn't collapse to minPx.
 */
export function clampSplitPosition(
  proposalPx: number,
  containerHeightPx: number,
  opts: { minPx?: number; maxFraction?: number } = {},
): number {
  const { minPx = 100, maxFraction = 0.6 } = opts;
  if (!Number.isFinite(proposalPx)) return minPx;
  if (containerHeightPx <= 0) return proposalPx;
  const maxPx = Math.max(minPx, Math.floor(containerHeightPx * maxFraction));
  if (proposalPx < minPx) return minPx;
  if (proposalPx > maxPx) return maxPx;
  return Math.floor(proposalPx);
}
