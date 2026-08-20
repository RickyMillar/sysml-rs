/**
 * TrailCrumb — frame chip surfacing the investigation drill trail
 * (ninebar Phase 1, plan §0 frame row / audit F7/F15).
 *
 * A compact companion to `DrilledFromBanner` (`src/workflows/run/`),
 * which owns the full in-workflow trail UI (prior-hop breadcrumb strip +
 * tick/element/session context). This chip is the frame-level presence
 * indicator: it renders only while `useInvestigationTrail` has a hop in
 * view, showing just the current hop's precomputed `label` plus a "back"
 * control that steps the cursor to the previous hop
 * (`popTo(cursor - 1)`), matching `DrilledFromBanner`'s back semantics
 * (forward hops are retained, not discarded — see the store's doc
 * comment).
 */
import { selectCurrentHop, useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';

export function TrailCrumb() {
  const hops = useInvestigationTrail((s) => s.hops);
  const cursor = useInvestigationTrail((s) => s.cursor);
  const popTo = useInvestigationTrail((s) => s.popTo);

  const current = selectCurrentHop({ hops, cursor });
  if (!current) return null;

  return (
    <div
      data-testid="trail-crumb"
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        height: 'var(--row-compact)',
        padding: '0 8px',
        maxWidth: 260,
        fontSize: 'var(--text-sm)',
        color: 'var(--text-secondary)',
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-sm)',
      }}
    >
      <span
        data-testid="trail-crumb-label"
        title={current.label}
        style={{
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {current.label}
      </span>
      <button
        type="button"
        data-testid="trail-crumb-back"
        aria-label="Back to previous investigation hop"
        onClick={() => popTo(cursor - 1)}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          width: 16,
          height: 16,
          flexShrink: 0,
          padding: 0,
          fontSize: 'var(--text-sm)',
          lineHeight: 1,
          color: 'var(--text-secondary)',
          background: 'transparent',
          border: 'none',
          borderRadius: 'var(--radius-sm)',
          cursor: 'pointer',
        }}
      >
        ‹
      </button>
    </div>
  );
}
