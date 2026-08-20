/**
 * DrillStatusToast — inline notice shown when a user clicks a verdict
 * that lacks evidence (R3.5 degraded path).
 *
 * Visual aesthetic follows `VerdictBadge` (inconclusive amber family,
 * color-mix tokens, rounded pill, distinct glyph). Kept intentionally
 * friendly — the missing evidence is a backend wiring gap (Agent R),
 * not a user error.
 *
 * The toast is rendered as a fixed overlay in the bottom-right of the
 * viewport so it doesn't shift layout of the host workflow. An
 * auto-dismiss timer is owned by `DrillProvider` — this component is
 * presentational.
 */

import type { CSSProperties } from 'react';

export interface DrillStatusToastProps {
  /** Message to display — typically `DRILL_NO_EVIDENCE_MESSAGE`. */
  message: string;
  /** Optional click-to-dismiss handler. */
  onDismiss?: () => void;
  /** test id passthrough */
  testId?: string;
}

/**
 * Canonical copy for the "evidence missing" path. Exported so tests and
 * the provider can stay in sync without duplicating the string literal.
 *
 * The message is deliberately friendly and directs future attention to
 * the R3.5 backend task rather than blaming the user.
 */
export const DRILL_NO_EVIDENCE_MESSAGE =
  'No evidence attached to this verdict — backend must populate `Verdict.evidence` (see R3.5 backend task).';

// ── Design tokens (mirrored from VerdictBadge inconclusive family) ──

const INCONCLUSIVE_COLOR = 'var(--verdict-inconclusive)';

const containerStyle: CSSProperties = {
  position: 'fixed',
  right: 16,
  bottom: 16,
  zIndex: 60,
  display: 'inline-flex',
  alignItems: 'center',
  gap: 8,
  maxWidth: 420,
  padding: '10px 12px',
  borderRadius: 8,
  background: 'color-mix(in srgb, var(--verdict-inconclusive) 15%, transparent)',
  border: `1px solid color-mix(in srgb, ${INCONCLUSIVE_COLOR} 35%, transparent)`,
  color: INCONCLUSIVE_COLOR,
  fontSize: 12,
  lineHeight: 1.4,
  boxShadow:
    '0 2px 4px rgba(0,0,0,0.12), 0 4px 16px rgba(0,0,0,0.18)',
  whiteSpace: 'normal',
};

const glyphStyle: CSSProperties = {
  fontSize: 14,
  fontWeight: 600,
  lineHeight: 1,
  flexShrink: 0,
  width: 14,
  textAlign: 'center',
};

const dismissButtonStyle: CSSProperties = {
  appearance: 'none',
  background: 'transparent',
  border: 'none',
  color: 'inherit',
  cursor: 'pointer',
  fontSize: 12,
  fontWeight: 600,
  padding: '2px 6px',
  borderRadius: 4,
  flexShrink: 0,
};

/**
 * Render the drill-status toast. Presentational only — owner controls
 * mount/unmount and auto-dismiss timing.
 */
export function DrillStatusToast(props: DrillStatusToastProps) {
  const { message, onDismiss, testId = 'drill-status-toast' } = props;
  return (
    <div
      role="status"
      aria-live="polite"
      data-testid={testId}
      style={containerStyle}
    >
      <span aria-hidden="true" style={glyphStyle}>
        {'?'}
      </span>
      <span style={{ flex: 1 }}>{message}</span>
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss"
          data-testid={`${testId}-dismiss`}
          style={dismissButtonStyle}
        >
          {'\u2715'}
        </button>
      )}
    </div>
  );
}
