/**
 * Breadcrumb — hierarchical path bar (Phase B3).
 *
 * Replaces the old back-button "drilled into X" banner that could
 * grow cyclically (parent → child → parent → child …). This bar is
 * always a strict PREFIX of the current focus path — clicking any
 * segment JUMPS to that level, never appending. The segment for the
 * current leaf isn't a link.
 *
 * Pure component: `segments` comes in already-resolved (the consumer
 * maps `focusPath: ElementId[]` through the session tree to get the
 * display labels). `onNavigateToDepth(depth)` is called with the
 * zero-based index the user wants to navigate to — depth 0 is the
 * session root. Matches the store's `navigateFocusToDepth` contract
 * added in Phase A2.
 *
 * "Breadcrumb — hierarchical, not cyclic".
 */
import type { CSSProperties } from 'react';

export interface BreadcrumbSegment {
  /** ElementId of the node this segment represents. */
  id: string;
  /** Display label (name from the tree). */
  label: string;
}

export interface BreadcrumbProps {
  /**
   * Ordered segments from root → leaf. Empty = the session root is
   * focused; the bar renders only the Home chip.
   */
  segments: readonly BreadcrumbSegment[];
  /**
   * Called with a zero-based depth when the user clicks a segment or
   * the Home icon. Depth 0 means "the session root" and clears the
   * focus path; depth N means "keep the first N segments".
   */
  onNavigateToDepth: (depth: number) => void;
  /** data-testid prefix (default `breadcrumb`). */
  testIdPrefix?: string;
}

const SEGMENT_STYLE: CSSProperties = {
  border: 'none',
  background: 'transparent',
  padding: 0,
  cursor: 'pointer',
  color: 'var(--on-surface-variant)',
  font: 'inherit',
  fontSize: 11,
};

const LEAF_STYLE: CSSProperties = {
  color: 'var(--on-surface)',
  fontWeight: 600,
  fontSize: 11,
};

const SEPARATOR_STYLE: CSSProperties = {
  color: 'var(--outline)',
  fontSize: 12,
  userSelect: 'none',
};

export function Breadcrumb({
  segments,
  onNavigateToDepth,
  testIdPrefix = 'breadcrumb',
}: BreadcrumbProps) {
  return (
    <nav
      aria-label="breadcrumb"
      data-testid={testIdPrefix}
      data-depth={segments.length}
      className="flex items-center gap-1.5 px-3"
      style={{
        height: 28,
        borderBottom: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-lowest)',
      }}
    >
      <button
        type="button"
        data-testid={`${testIdPrefix}-home`}
        aria-label="Session root"
        onClick={() => onNavigateToDepth(0)}
        disabled={segments.length === 0}
        style={{
          ...SEGMENT_STYLE,
          color:
            segments.length === 0 ? 'var(--on-surface)' : 'var(--on-surface-variant)',
          fontWeight: segments.length === 0 ? 600 : 400,
          cursor: segments.length === 0 ? 'default' : 'pointer',
          display: 'inline-flex',
          alignItems: 'center',
        }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 14 }}
          aria-hidden="true"
        >
          home
        </span>
      </button>
      {segments.map((seg, idx) => {
        const isLeaf = idx === segments.length - 1;
        return (
          <span
            key={`${seg.id}-${idx}`}
            className="flex items-center gap-1.5"
          >
            <span aria-hidden="true" style={SEPARATOR_STYLE}>
              ›
            </span>
            {isLeaf ? (
              <span
                data-testid={`${testIdPrefix}-leaf`}
                data-segment-id={seg.id}
                aria-current="page"
                style={LEAF_STYLE}
              >
                {seg.label}
              </span>
            ) : (
              <button
                type="button"
                data-testid={`${testIdPrefix}-segment-${idx}`}
                data-segment-id={seg.id}
                onClick={() => onNavigateToDepth(idx + 1)}
                style={SEGMENT_STYLE}
              >
                {seg.label}
              </button>
            )}
          </span>
        );
      })}
    </nav>
  );
}
