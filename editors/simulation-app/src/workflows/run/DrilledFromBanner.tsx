/**
 * DrilledFromBanner — sticky header strip shown when the user landed
 * on RunWorkflow via a drill-from-evidence click (R3.5; multi-hop trail
 * added in ninebar Phase 1, audit F15).
 *
 * Reads the current hop from `useInvestigationTrail` (the multi-hop
 * breadcrumb store that replaced `useSessionStore.drilledFrom`) and
 * renders its context (source session, tick, element). Earlier hops in
 * the trail — if any — render as a small clickable breadcrumb ahead of
 * the current hop's label; clicking one jumps straight to it via
 * `popTo(index)`. The "Back" button steps to the previous hop
 * (`popTo(cursor - 1)`); when the trail only has one hop this hides the
 * banner entirely, matching the old "dismiss" behaviour. When the trail
 * is empty the component renders nothing.
 *
 * Visual treatment uses the inconclusive-amber tokens from the shared
 * verdict palette so the banner reads as "informational, pay
 * attention" rather than "error" — you're here because of a failing
 * verdict, but the banner itself is not the failure.
 */

import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';

function originLabel(origin: string): string {
  return origin.charAt(0).toUpperCase() + origin.slice(1);
}

export function DrilledFromBanner() {
  const hops = useInvestigationTrail((s) => s.hops);
  const cursor = useInvestigationTrail((s) => s.cursor);
  const popTo = useInvestigationTrail((s) => s.popTo);

  const current = cursor >= 0 && cursor < hops.length ? hops[cursor] : null;
  if (!current) return null;

  const { tick, elementId, fromSessionId, origin } = current;
  const hasTick = tick !== undefined;
  const hasElement = !!elementId;
  const hasSession = fromSessionId.length > 0;
  // Earlier hops the user can jump straight back to — everything up to
  // (but not including) the current one.
  const priorHops = hops.slice(0, cursor);

  return (
    <div
      role="status"
      data-testid="drilled-from-banner"
      className="flex items-center gap-3 px-4 py-2 border-b"
      style={{
        background: 'color-mix(in srgb, var(--verdict-inconclusive) 10%, transparent)',
        borderColor: 'color-mix(in srgb, var(--verdict-inconclusive) 30%, transparent)',
        fontSize: 'var(--text-xs)',
        color: 'var(--on-surface)',
      }}
    >
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{ fontSize: '16px', color: 'var(--verdict-inconclusive)', flexShrink: 0 }}
      >
        my_location
      </span>

      <div className="flex items-center gap-2 flex-1 min-w-0">
        {priorHops.length > 0 && (
          <span
            data-testid="drilled-from-breadcrumb"
            className="flex items-center gap-1 min-w-0"
            style={{ color: 'var(--on-surface-variant)' }}
          >
            {priorHops.map((hop, i) => (
              <span key={`${hop.label}-${i}`} className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => popTo(i)}
                  data-testid={`drilled-from-crumb-${i}`}
                  className="truncate"
                  style={{
                    color: 'var(--on-surface-variant)',
                    background: 'transparent',
                    border: 'none',
                    padding: 0,
                    cursor: 'pointer',
                    textDecoration: 'underline',
                  }}
                >
                  {hop.label}
                </button>
                <span aria-hidden="true">›</span>
              </span>
            ))}
          </span>
        )}
        <span style={{ fontWeight: 600 }} data-testid="drilled-from-label">
          Drilled from {originLabel(origin)}
        </span>
        {hasTick && (
          <>
            <span style={{ color: 'var(--on-surface-variant)' }}>·</span>
            <span className="mono-text" data-testid="drilled-from-tick">
              tick {tick}
            </span>
          </>
        )}
        {hasElement && (
          <>
            <span style={{ color: 'var(--on-surface-variant)' }}>·</span>
            <span
              className="mono-text truncate"
              data-testid="drilled-from-element"
              title={elementId}
            >
              {elementId}
            </span>
          </>
        )}
        {hasSession && (
          <>
            <span style={{ color: 'var(--on-surface-variant)', flexShrink: 0 }}>
              ·
            </span>
            <span
              className="mono-text truncate"
              data-testid="drilled-from-session"
              title={fromSessionId}
              style={{ color: 'var(--on-surface-variant)', flexShrink: 0 }}
            >
              session {fromSessionId.slice(0, 8)}
            </span>
          </>
        )}
      </div>

      <button
        type="button"
        onClick={() => popTo(cursor - 1)}
        data-testid="drilled-from-back"
        aria-label="Back to previous hop"
        className="flex items-center gap-1 px-2 py-1 rounded hover:bg-black/10"
        style={{
          color: 'var(--on-surface-variant)',
          fontSize: 'var(--text-xs)',
          background: 'transparent',
          border: '1px solid color-mix(in srgb, var(--verdict-inconclusive) 25%, transparent)',
          cursor: 'pointer',
          flexShrink: 0,
        }}
      >
        Back
      </button>
    </div>
  );
}
