/**
 * Phase 3 — shared hover popover that renders a `<SneakPeek>` next to a
 * panel row. Wired into DiagnosticsPanel, TraceabilityMatrixViewer,
 * ViewsPanel, and ModelTreeNodeRow.
 *
 * Lifecycle (per panel row):
 *   - Consumer holds a `triggerHovered` boolean keyed on the row.
 *   - This popover combines that with its own `popoverHovered` so the
 *     user can slide from the row into the preview without it closing.
 *   - `useSourcePreview` debounces the combined hover, so scrolling
 *     past rows doesn't fire `sysml.get_source`.
 *   - Click anywhere on the popover → `onPromote`, which the consumer
 *     uses to set selection + focused URI + open the Source drawer.
 *
 * Positioning is intentionally minimal: fixed-position below the
 * trigger rect, clamped into the viewport. No portals, no flip logic —
 * the SneakPeek itself is ~160px tall so it almost always fits below
 * the row. If the trigger is in the bottom 200px of the viewport we
 * flip above instead.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react';
import { SneakPeek } from '@/features/hover/SneakPeek';
import { useSourcePreview } from './useSourcePreview';

export interface SourcePreviewPopoverProps {
  /** Ref to the row / cell the user is hovering. */
  triggerRef: RefObject<HTMLElement | null>;
  /** Whether the trigger is currently hovered (or keyboard-focused). */
  triggerHovered: boolean;
  /** URI of the element to preview. */
  uri: string | null;
  /** Element id within `uri`. */
  elementId: string | null;
  /**
   * Click handler — the consumer typically pushes the URI + element id
   * onto the selection store, sets the focused file, and opens the
   * Source utility drawer. The popover closes immediately afterwards
   * (controlled by the trigger losing hover when the row navigates).
   */
  onPromote?: () => void;
  /** Pixel height of the embedded Monaco. Defaults to 160. */
  heightPx?: number;
  /** Pixel width of the popover card. Defaults to 380. */
  widthPx?: number;
  /** Override the debounce delay (ms). */
  debounceMs?: number;
  /** data-testid on the popover root. Default `source-preview-popover`. */
  testId?: string;
}

const DEFAULT_HEIGHT = 160;
const DEFAULT_WIDTH = 380;
/**
 * Approx popover height; used for the bottom-of-viewport flip check.
 * Slightly larger than the Monaco height to cover the action footer.
 */
const POPOVER_PADDING = 56;

export function SourcePreviewPopover({
  triggerRef,
  triggerHovered,
  uri,
  elementId,
  onPromote,
  heightPx = DEFAULT_HEIGHT,
  widthPx = DEFAULT_WIDTH,
  debounceMs,
  testId,
}: SourcePreviewPopoverProps) {
  const [popoverHovered, setPopoverHovered] = useState(false);
  const hovering = triggerHovered || popoverHovered;
  const { armed } = useSourcePreview(uri, elementId, { hovering, debounceMs });

  const [coords, setCoords] = useState<{ top: number; left: number } | null>(
    null,
  );
  const popoverRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    if (!armed) {
      setCoords(null);
      return;
    }
    const el = triggerRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const totalHeight = heightPx + POPOVER_PADDING;
    const flipAbove =
      rect.bottom + totalHeight > window.innerHeight && rect.top > totalHeight;
    const top = flipAbove ? rect.top - totalHeight - 4 : rect.bottom + 4;
    const maxLeft = Math.max(8, window.innerWidth - widthPx - 8);
    const left = Math.min(Math.max(8, rect.left), maxLeft);
    setCoords({ top, left });
  }, [armed, heightPx, widthPx, triggerRef]);

  // Recompute on scroll / resize so the popover tracks the row when
  // the user nudges the page underneath it. Skip when disarmed.
  useEffect(() => {
    if (!armed) return;
    const recalc = () => {
      const el = triggerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const totalHeight = heightPx + POPOVER_PADDING;
      const flipAbove =
        rect.bottom + totalHeight > window.innerHeight && rect.top > totalHeight;
      const top = flipAbove ? rect.top - totalHeight - 4 : rect.bottom + 4;
      const maxLeft = Math.max(8, window.innerWidth - widthPx - 8);
      const left = Math.min(Math.max(8, rect.left), maxLeft);
      setCoords({ top, left });
    };
    window.addEventListener('scroll', recalc, true);
    window.addEventListener('resize', recalc);
    return () => {
      window.removeEventListener('scroll', recalc, true);
      window.removeEventListener('resize', recalc);
    };
  }, [armed, heightPx, widthPx, triggerRef]);

  const handleClick = useCallback(() => {
    if (!onPromote) return;
    onPromote();
  }, [onPromote]);

  if (!armed || !coords || !uri || !elementId) return null;

  const baseStyle: CSSProperties = {
    position: 'fixed',
    top: coords.top,
    left: coords.left,
    width: widthPx,
    zIndex: 200,
    background: 'var(--surface-panel)',
    border: '1px solid var(--border-default)',
    borderRadius: 6,
    boxShadow: '0 10px 30px rgba(0,0,0,0.35), 0 4px 12px rgba(0,0,0,0.25)',
    overflow: 'hidden',
    cursor: onPromote ? 'pointer' : 'default',
  };

  return (
    <div
      ref={popoverRef}
      data-testid={testId ?? 'source-preview-popover'}
      data-uri={uri}
      data-element-id={elementId}
      style={baseStyle}
      onMouseEnter={() => setPopoverHovered(true)}
      onMouseLeave={() => setPopoverHovered(false)}
      onClick={handleClick}
    >
      <div style={{ padding: 8 }}>
        <SneakPeek uri={uri} elementId={elementId} heightPx={heightPx} />
      </div>
      {onPromote ? (
        <div
          data-testid="source-preview-promote"
          style={{
            padding: '4px 10px',
            borderTop: '1px solid var(--border-default)',
            background: 'var(--surface-raised)',
            fontSize: 10,
            // "Open in Source" is a clickable link/action affordance —
            // a link genuinely reads as "you can act here" primacy.
            color: 'var(--accent-fg)',
            fontWeight: 700,
            letterSpacing: 0.3,
            textTransform: 'uppercase',
            display: 'flex',
            alignItems: 'center',
            gap: 4,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 12 }}>
            open_in_new
          </span>
          Open in Source
        </div>
      ) : null}
    </div>
  );
}
