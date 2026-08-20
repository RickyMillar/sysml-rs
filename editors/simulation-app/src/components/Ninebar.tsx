/**
 * <Ninebar/> — the ninebar meter module.
 *
 * The logo's geometry is the origin of the product's rhythm system: nine
 * segments whose height curve peaks at position 6. It is the ONE glyph for a
 * live or pending measure — running sessions, pending verdicts, coverage,
 * confidence, pass-rate, loading a live value. Do not draw a new meter, and
 * do not use this decoratively: if it isn't a live/pending measure, it isn't
 * a ninebar. (Same governance as "amber is never decorative".)
 *
 * Modes
 * - determinate:  `value` in [0,1] — segments fill left→right (coverage,
 *   pass-rate, confidence, progress).
 * - indeterminate: no `value` — all segments render and pulse (running /
 *   pending). Pulse is a staggered `nb-pulse` (keyframes in global.css);
 *   `prefers-reduced-motion` freezes it to a static glyph via the
 *   `.nb-meter [data-nb-seg]` media rule.
 *
 * `compact` renders segments 4/6/8 only (the 3-bar mark used inside table
 * cells and pills) — same curve, same meaning, less room.
 */

// Height fractions of the nine segments: the curve peaks at position 6.
const CURVE = [0.32, 0.44, 0.58, 0.72, 0.86, 1.0, 0.82, 0.6, 0.4] as const;
const COMPACT_INDICES = [3, 5, 7] as const; // segments 4, 6, 8 (0-based)

export interface NinebarProps {
  /** Determinate fill in [0,1]. Omit for indeterminate (pulsing) mode. */
  value?: number;
  /** 3-segment variant for table cells / pills. */
  compact?: boolean;
  /** Overall height in px (segment heights scale to the curve). Default 12. */
  size?: number;
  /** CSS color for lit segments. Default: the accent. */
  color?: string;
  /** CSS color for unlit segments in determinate mode. */
  trackColor?: string;
  /** Accessible label, e.g. "session running", "coverage 72%". */
  label?: string;
}

export function Ninebar({
  value,
  compact = false,
  size = 12,
  color = 'var(--accent-fg)',
  trackColor = 'var(--border-default)',
  label,
}: NinebarProps) {
  const indeterminate = value === undefined;
  const indices: readonly number[] = compact
    ? COMPACT_INDICES
    : CURVE.map((_, i) => i);
  const litCount = indeterminate
    ? indices.length
    : Math.round(Math.max(0, Math.min(1, value)) * indices.length);
  const segWidth = Math.max(2, Math.round(size / 6));
  const gap = Math.max(1, Math.round(segWidth / 2));

  return (
    <span
      className="nb-meter"
      role={indeterminate ? 'status' : 'meter'}
      aria-label={label ?? (indeterminate ? 'in progress' : undefined)}
      {...(indeterminate
        ? {}
        : { 'aria-valuemin': 0, 'aria-valuemax': 1, 'aria-valuenow': value })}
      style={{
        display: 'inline-flex',
        alignItems: 'flex-end',
        gap: `${gap}px`,
        height: `${size}px`,
      }}
    >
      {indices.map((curveIdx, i) => {
        const lit = indeterminate || i < litCount;
        return (
          <span
            key={curveIdx}
            data-nb-seg
            style={{
              width: `${segWidth}px`,
              height: `${Math.max(2, Math.round(CURVE[curveIdx] * size))}px`,
              background: lit ? color : trackColor,
              borderRadius: '1px',
              ...(indeterminate
                ? {
                    animation: `nb-pulse 1.2s ease-in-out ${i * 0.1}s infinite`,
                  }
                : {}),
            }}
          />
        );
      })}
    </span>
  );
}
