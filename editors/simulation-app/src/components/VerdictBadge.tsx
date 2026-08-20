/**
 * VerdictBadge — canonical 4-valued verdict indicator.
 *
 * Backend `VerdictKind` is `Pass | Fail | Inconclusive | Error`. Every
 * verdict-rendering site in the UI must use this component so the Error
 * case is visually distinct from Fail (pairing color + distinct glyph
 * shape for color-blind accessibility).
 *
 * This component is the canonical import path — future workflows (Verify,
 * Compare) and the Variables pane should all consume it.
 *
 * Stable import path: `@/components/VerdictBadge`
 *   (or `../components/VerdictBadge` from features).
 */
import type { CSSProperties, ReactNode } from 'react';

export type VerdictKind = 'pass' | 'fail' | 'inconclusive' | 'error';

export interface VerdictBadgeProps {
  /** The 4-valued verdict. Case-insensitive string from the backend is accepted via helper below. */
  verdict: VerdictKind;
  /** Optional actual value (shown in tooltip). */
  actual?: string | null;
  /** Optional expected value (shown in tooltip when verdict is fail). */
  expected?: string | null;
  /**
   * Backend evaluation error. When set, the badge's tooltip leads with
   * `Error: <msg>` regardless of the verdict kind — distinct from the
   * `reason` string which carries narrative context for inconclusive /
   * error verdicts that DID complete (just without a value).
   */
  error?: string | null;
  /**
   * Verdict-specific message. Required semantically for `error` (the
   * `metadata.error_reason` from the backend Verdict struct) and
   * strongly recommended for `inconclusive`.
   */
  reason?: string | null;
  /**
   * Compact = tight row height (Variables pane). Standard = card pills.
   * Bare = the calm-pass form: a coloured glyph on the bare ground — no
   * pill, no background, no border. The verdict reads by COLOUR + glyph
   * shape; the container is dropped because it is decoration, not meaning
   * (calm-pass brief P2). `showLabel` still controls the word.
   */
  size?: 'compact' | 'standard' | 'bare';
  /** When `false` (or 'compact'/'bare' size), only the glyph renders; hover shows full detail. */
  showLabel?: boolean;
  /** Optional stable element name for tooltip prefix ("foo: Pass — ..."). */
  name?: string;
  /** Override for the tooltip — rarely needed. */
  titleOverride?: string;
  /** test id passthrough */
  testId?: string;
}

interface Tokens {
  color: string;
  bg: string;
  border: string;
  glyph: string;
  /** Distinct SHAPE, independent of color, so color-blind users can distinguish. */
  shapeAriaName: string;
  label: string;
}

const PASS_COLOR = 'var(--verdict-pass)';
const FAIL_COLOR = 'var(--verdict-fail)';
// Inconclusive is a hollow/neutral state, never amber — amber is reserved
// for selection/active/running.
const INCONCLUSIVE_COLOR = 'var(--verdict-inconclusive)';
const ERROR_COLOR = 'var(--verdict-error)';

const TOKENS: Record<VerdictKind, Tokens> = {
  pass: {
    color: PASS_COLOR,
    bg: 'color-mix(in srgb, var(--verdict-pass) 15%, transparent)',
    border: 'color-mix(in srgb, var(--verdict-pass) 35%, transparent)',
    glyph: '\u2713', // ✓
    shapeAriaName: 'check',
    label: 'Pass',
  },
  fail: {
    color: FAIL_COLOR,
    bg: 'color-mix(in srgb, var(--verdict-fail) 15%, transparent)',
    border: 'color-mix(in srgb, var(--verdict-fail) 35%, transparent)',
    glyph: '\u2717', // ✗
    shapeAriaName: 'cross',
    label: 'Fail',
  },
  inconclusive: {
    color: INCONCLUSIVE_COLOR,
    bg: 'color-mix(in srgb, var(--verdict-inconclusive) 15%, transparent)',
    border: 'color-mix(in srgb, var(--verdict-inconclusive) 35%, transparent)',
    glyph: '?',
    shapeAriaName: 'question',
    label: 'Inconclusive',
  },
  error: {
    color: ERROR_COLOR,
    bg: 'color-mix(in srgb, var(--verdict-error) 15%, transparent)',
    border: 'color-mix(in srgb, var(--verdict-error) 45%, transparent)',
    glyph: '\u26A0', // ⚠ triangular warning — distinct shape from the circular/X group
    shapeAriaName: 'triangle',
    label: 'Error',
  },
};

const INCONCLUSIVE_FALLBACK = 'Expression evaluated non-boolean';
const ERROR_FALLBACK = 'Constraint could not be evaluated';

/**
 * Build the tooltip text. Kept exported so tests and adjacent components
 * can stay in sync with the visual spec.
 */
export function buildVerdictTooltip(
  verdict: VerdictKind,
  opts: { actual?: string | null; expected?: string | null; reason?: string | null; name?: string; error?: string | null } = {},
): string {
  const { actual, expected, reason, name, error } = opts;
  const prefix = name ? `${name}: ` : '';
  // Evaluation error short-circuits the verdict-specific tooltip — the
  // user needs to see the underlying error string, not "Pass — actual: …".
  if (typeof error === 'string' && error.length > 0) {
    return `${prefix}Error: ${error}`;
  }
  switch (verdict) {
    case 'pass':
      return `${prefix}Pass${actual != null ? ` — actual: ${actual}` : ''}`;
    case 'fail': {
      const parts: string[] = [];
      if (actual != null) parts.push(`actual: ${actual}`);
      if (expected != null) parts.push(`expected: ${expected}`);
      const tail = parts.length ? ` — ${parts.join(' — ')}` : '';
      return `${prefix}Fail${tail}`;
    }
    case 'inconclusive':
      return `${prefix}Inconclusive — ${reason || INCONCLUSIVE_FALLBACK}`;
    case 'error':
      return `${prefix}Error — ${reason || ERROR_FALLBACK}`;
  }
}

/**
 * Normalize a possibly-string verdict from the backend into the TS union.
 * Unknown strings collapse to `inconclusive` so the UI never silently
 * drops a result.
 */
export function normalizeVerdict(raw: string | VerdictKind | undefined | null): VerdictKind {
  if (!raw) return 'inconclusive';
  const lower = String(raw).toLowerCase();
  if (lower === 'pass' || lower === 'fail' || lower === 'error') return lower;
  if (lower === 'inconclusive') return 'inconclusive';
  return 'inconclusive';
}

export function VerdictBadge(props: VerdictBadgeProps) {
  const {
    verdict,
    actual,
    expected,
    reason,
    error,
    size = 'standard',
    showLabel = size === 'standard',
    name,
    titleOverride,
    testId,
  } = props;

  const hasError = typeof error === 'string' && error.length > 0;
  const tokens = TOKENS[verdict];
  const title = titleOverride ?? buildVerdictTooltip(verdict, { actual, expected, reason, name, error });
  // Surface the underlying evaluation error in the accessibility tree so
  // screen-reader users distinguish "error: <msg>" from inconclusive.
  const ariaLabel = `Verdict: ${tokens.label}${hasError ? ` — error: ${error}` : reason ? ` — ${reason}` : ''}${name ? ` (${name})` : ''}`;

  const isBare = size === 'bare';
  const isCompact = size === 'compact';
  const glyphSize = isBare ? 12 : isCompact ? 12 : 14;
  const rowHeight = isBare ? 'auto' : isCompact ? 16 : 22;
  const paddingX = isCompact ? 4 : 8;
  const fontSize = isBare || isCompact ? 10 : 11;

  const containerStyle: CSSProperties = isBare
    ? {
        // Calm form: coloured glyph on the bare ground, no container.
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        color: tokens.color,
        lineHeight: 1,
        fontSize,
        fontWeight: 600,
        whiteSpace: 'nowrap',
      }
    : {
        display: 'inline-flex',
        alignItems: 'center',
        gap: isCompact ? 3 : 5,
        height: rowHeight,
        padding: `0 ${paddingX}px`,
        borderRadius: 9999,
        background: tokens.bg,
        border: `1px solid ${tokens.border}`,
        color: tokens.color,
        lineHeight: 1,
        fontSize,
        whiteSpace: 'nowrap',
      };

  return (
    <span
      role="status"
      aria-live="off"
      aria-label={ariaLabel}
      data-verdict={verdict}
      data-verdict-shape={tokens.shapeAriaName}
      data-testid={testId ?? `verdict-badge-${verdict}`}
      title={title}
      style={containerStyle}
    >
      <span
        aria-hidden="true"
        data-testid={testId ? `${testId}-glyph` : undefined}
        style={{ fontSize: glyphSize, fontWeight: 600, display: 'inline-block', width: glyphSize, textAlign: 'center' }}
      >
        {tokens.glyph}
      </span>
      {showLabel ? (
        <span style={{ fontWeight: 500 }}>{tokens.label}</span>
      ) : null}
      {/* Hidden affix carrying the Error/Inconclusive signal into the
          accessibility tree for compact rows (so screen-readers pick it
          up even when the user never hovers for the tooltip). */}
      {(isCompact || isBare) && (verdict === 'error' || verdict === 'inconclusive') ? (
        <span className="sr-only" style={srOnlyStyle}>
          {verdict === 'error' ? '!' : '?'} {reason ?? (verdict === 'error' ? ERROR_FALLBACK : INCONCLUSIVE_FALLBACK)}
        </span>
      ) : null}
    </span>
  );
}

/**
 * Convenience wrapper: boolean `pass` flag → pass/fail badge.
 * Used at legacy call sites that pre-date the 4-valued verdict; prefer
 * the full `<VerdictBadge verdict=...>` API when possible.
 */
export function VerdictBadgeFromBool(
  props: Omit<VerdictBadgeProps, 'verdict'> & { pass: boolean },
): ReactNode {
  const { pass, ...rest } = props;
  return <VerdictBadge {...rest} verdict={pass ? 'pass' : 'fail'} />;
}

const srOnlyStyle: CSSProperties = {
  position: 'absolute',
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: 'hidden',
  clip: 'rect(0, 0, 0, 0)',
  whiteSpace: 'nowrap',
  border: 0,
};
