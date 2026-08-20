/**
 * EvaluationModeBadge — how a verdict was COMPUTED (B10 layer 2).
 *
 * `evaluation_mode` is a BINDING label on every verdict-bearing surface
 * (§2.1a ruling (d)): a static verdict on an ODE-backed case answers a
 * different question than a trajectory run, so the mode must be legible
 * next to the verdict — never buried in a tooltip suffix, never only shown
 * on disagreement. This is the canonical, self-contained primitive; Verify
 * owns the concept and every verdict surface (matrix, rail, report) plus
 * the Requirements rollup chip consumes it.
 *
 * ## GEOMETRY is the channel (Verify design loop, 1d)
 *
 * The three modes are told apart by SHAPE, not colour — "records are
 * square, verdicts are round", and a desk check has no record at all:
 *
 *   · static     — BARE muted mono text (`= static`), NO container. A desk
 *                  check against current values is weightless: no session,
 *                  no archive, nothing to point at but the model itself.
 *   · trajectory — SOLID border, SQUARE corners (radius 4). There is a
 *                  receipt: a session, a tick archive, B6 provenance. The
 *                  border is the container of record; ` · <recordRef>`
 *                  names the session when supplied.
 *   · external   — DASHED border, square corners, `↓ external` + the
 *                  producing tool. Recorded PROVENANCE, never a verdict the
 *                  tool computed — the dashed edge keeps that honest. A
 *                  `⚑ older model` marker (warning family, NOT a verdict
 *                  colour) rides the badge whenever `stale` is set, wherever
 *                  the badge appears.
 *
 * Chip-family discipline: modes are NEVER coloured with verdict colours
 * (that channel belongs to the verdict itself), and colour is never the
 * ONLY channel — the glyph (`= ∿ ↓`) and the geometry (bare / solid /
 * dashed) carry the difference for colour-blind users.
 *
 * Stable import path: `@/components/EvaluationModeBadge`.
 */
import type { CSSProperties, ReactNode } from 'react';

export type EvaluationMode = 'static' | 'trajectory' | 'external';

/** Geometry — the primary channel. `bare` = no container (static);
 *  `solid`/`dashed` = a square-cornered record (trajectory/external). */
type ModeGeometry = 'bare' | 'solid' | 'dashed';

interface ModeTokens {
  /** Short word shown on the badge. */
  label: string;
  /** Distinct glyph — the colour-blind-safe differentiator. */
  glyph: string;
  /** Neutral text/border colour (NEVER a verdict colour). */
  color: string;
  /** Shape channel: bare (no record) vs a solid/dashed square record. */
  geometry: ModeGeometry;
  /** Plain-language sentence: what the mode MEANS, not just the word. */
  tooltip: string;
  /** Shape name in the a11y tree, independent of colour. */
  shapeAriaName: string;
}

const MODE_TOKENS: Record<EvaluationMode, ModeTokens> = {
  static: {
    label: 'static',
    glyph: '=',
    // Neutral secondary ink — a computed desk check, the baseline mode.
    // Bare: no border/background, nothing to point at but the model.
    color: 'var(--text-secondary)',
    geometry: 'bare',
    tooltip:
      'Static evaluation — checked against the model’s current/default values, ' +
      'without running a simulation. Answers “do the numbers hold as written”, ' +
      'not “does it hold over a run”.',
    shapeAriaName: 'bare',
  },
  trajectory: {
    label: 'trajectory',
    glyph: '∿', // ∿ — a run's trace over time
    // Sim-accent tint (Ricky ruling 2026-07-19: session-backed verdicts pop
    // against static ones); the SOLID square border stays the primary channel
    // — geometry distinguishes the family, hue is secondary (never a verdict
    // colour).
    color: 'var(--sim-accent)',
    geometry: 'solid',
    tooltip:
      'Trajectory evaluation — checked against a live simulation run’s state. ' +
      'Reflects how the system actually behaved over time, not just the values as written.',
    shapeAriaName: 'solid-record',
  },
  external: {
    label: 'external',
    glyph: '↓', // ↓ — ingested from outside
    // Muted ink + dashed border: this reads as PROVENANCE, never as a
    // computed chip. The B10 hard line lives exactly here.
    color: 'var(--text-muted)',
    geometry: 'dashed',
    tooltip:
      'External evidence — this verdict was produced outside the tool ' +
      '(e.g. a CI run or test rig) and ingested. It is recorded provenance, ' +
      'not a verdict the tool computed.',
    shapeAriaName: 'dashed-provenance',
  },
};

/** Plain-language meaning of the staleness marker (kept out of verdict
 *  colours — it rides the badge in the warning family). */
const STALE_TOOLTIP =
  'Produced against an older model — the digest the producer claims it tested ' +
  'no longer matches the current model.';

/**
 * Normalize a possibly-string / possibly-absent evaluation mode from the
 * backend. Returns `null` for absent or unknown values so callers render
 * NOTHING rather than fabricate a mode — an unlabelled verdict is honest;
 * a wrong label is not.
 */
export function normalizeEvaluationMode(
  raw: string | EvaluationMode | undefined | null,
): EvaluationMode | null {
  if (!raw) return null;
  const lower = String(raw).toLowerCase();
  if (lower === 'static' || lower === 'trajectory' || lower === 'external') {
    return lower;
  }
  return null;
}

/** Plain-language sentence for a mode — exported so tooltip-only call
 *  sites (e.g. a chip that can't nest a badge) can reuse the ONE wording. */
export function evaluationModeTooltip(mode: EvaluationMode): string {
  return MODE_TOKENS[mode].tooltip;
}

export interface EvaluationModeBadgeProps {
  /** The mode. Pass the normalized union, or a raw string via the helper. */
  mode: EvaluationMode;
  /**
   * Compact = a dense bare-text form (glyph + word, no container/record) for
   * matrix rows and the Requirements rollup chip; standard = the full
   * geometry family (bare static / solid trajectory / dashed external).
   *
   * Mark = the calm-pass form: the geometry channel ALONE — a small
   * solid/dashed square (record / provenance) and nothing for static (a
   * desk check has no mark). The mode WORD and glyph move up to the column
   * header, said once; the cell keeps only what varies (calm-pass brief P3).
   * The `⚑` stale flag still rides the mark. `recordRef` is ignored — the
   * caller renders the ref as its own quiet text beside the mark.
   */
  size?: 'compact' | 'standard' | 'mark';
  /**
   * The record this verdict points at — a session id (trajectory) or the
   * producing tool (external). Rendered as ` · <recordRef>` on the record
   * chip in the standard size. No-op for static (which has no record) and
   * for the compact form.
   */
  recordRef?: string;
  /**
   * Staleness — server-computed `matches_current_model === false`. Renders a
   * `⚑ older model` marker in the WARNING family (never a verdict colour)
   * riding the badge, wherever the badge appears.
   */
  stale?: boolean;
  /** test id passthrough */
  testId?: string;
}

export function EvaluationModeBadge({
  mode,
  size = 'standard',
  recordRef,
  stale = false,
  testId,
}: EvaluationModeBadgeProps) {
  const t = MODE_TOKENS[mode];

  // Mark = the calm geometry channel alone — a small solid/dashed square
  // (record / provenance) and NOTHING else. Static has no record, so no
  // mark at all (its absence IS the "desk check" signal, matching the
  // empty column head). The word + glyph live in the column header now;
  // the verdict, the record ref, and any staleness flag are rendered by
  // the caller as its own trailing text so the square leads cleanly and
  // the verdict sits next to it (calm-pass P3). `stale` only enriches the
  // tooltip/a11y here — it draws no marker of its own in this size.
  if (size === 'mark') {
    if (t.geometry === 'bare') return null;
    const markAria =
      `Evaluation mode: ${t.label} — ${t.tooltip}` + (stale ? ` ${STALE_TOOLTIP}` : '');
    return (
      <span
        data-testid={testId ?? `evaluation-mode-badge-${mode}`}
        data-evaluation-mode={mode}
        data-mode-shape={t.shapeAriaName}
        data-mode-stale={stale || undefined}
        aria-hidden="true"
        title={stale ? `${t.tooltip} · ${STALE_TOOLTIP}` : t.tooltip}
        aria-label={markAria}
        style={{
          display: 'inline-block',
          width: 9,
          height: 9,
          flex: 'none',
          boxSizing: 'border-box',
          borderRadius: 2,
          border: `1.5px ${t.geometry} ${t.color}`,
          verticalAlign: 'middle',
        }}
      />
    );
  }

  const isCompact = size === 'compact';
  // Static is always bare; compact flattens every mode to bare text for
  // matrix/rollup density (the glyph still carries the channel).
  const geometry: ModeGeometry = isCompact ? 'bare' : t.geometry;
  const isRecord = geometry !== 'bare';

  const style: CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    gap: 4,
    height: isCompact ? 16 : 18,
    // A record carries padding + a square border; the bare form is naked text.
    padding: isRecord ? '0 7px' : 0,
    borderRadius: isRecord ? 4 : 0, // square records
    background: 'transparent',
    border: isRecord ? `1px ${geometry} ${t.color}` : 'none',
    color: t.color,
    fontFamily: 'var(--font-mono)',
    fontSize: isCompact ? 9 : 10,
    lineHeight: 1,
    letterSpacing: '0.02em',
    whiteSpace: 'nowrap',
    verticalAlign: 'middle',
  };

  // `recordRef` names the record (session / tool) on the standard chip.
  const showRef = !isCompact && isRecord && !!recordRef;
  const ariaLabel =
    `Evaluation mode: ${t.label}` +
    (showRef ? ` · ${recordRef}` : '') +
    ` — ${t.tooltip}` +
    (stale ? ` ${STALE_TOOLTIP}` : '');

  return (
    <span
      data-testid={testId ?? `evaluation-mode-badge-${mode}`}
      data-evaluation-mode={mode}
      data-mode-shape={t.shapeAriaName}
      data-mode-stale={stale || undefined}
      title={stale ? `${t.tooltip} · ${STALE_TOOLTIP}` : t.tooltip}
      aria-label={ariaLabel}
      style={style}
    >
      <span aria-hidden="true" style={{ fontWeight: 600 }}>
        {t.glyph}
      </span>
      <span>{t.label}</span>
      {showRef ? (
        <span style={{ color: 'var(--text-muted)' }}>· {recordRef}</span>
      ) : null}
      {stale ? <StaleMarker /> : null}
    </span>
  );
}

/** `⚑ older model` — warning family, rides the badge. Never a verdict
 *  colour; the flag glyph carries it independent of colour. */
function StaleMarker() {
  return (
    <span
      data-testid="evaluation-mode-stale"
      title={STALE_TOOLTIP}
      style={{
        color: 'var(--severity-warning)',
        fontFamily: 'var(--font-sans, inherit)',
        marginLeft: 2,
      }}
    >
      <span aria-hidden="true">⚑ </span>older model
    </span>
  );
}

/**
 * Convenience: accept a raw backend string, render nothing when the mode
 * is absent/unknown. The common call shape at verdict surfaces.
 */
export function EvaluationModeBadgeFromRaw(
  props: Omit<EvaluationModeBadgeProps, 'mode'> & {
    mode: string | null | undefined;
  },
) {
  const normalized = normalizeEvaluationMode(props.mode);
  if (!normalized) return null;
  const { mode: _raw, ...rest } = props;
  return <EvaluationModeBadge {...rest} mode={normalized} />;
}

// ── Declared / computed pairing (1d pairing rule) ──────────────────────

/**
 * The pairing rule: a DECLARED method chip (layer 1 — a promise on the
 * model register) and the COMPUTED mode badge (layer 2 — how THIS verdict
 * was made) answer different questions, so when they appear together each
 * carries a tiny uppercase overline naming its register. A case declared
 * `test` holding a `= static` verdict is an honest, legible state — the
 * overlines are what keep the two from reading as one claim.
 *
 * `methods` is the DECLARED @VerificationMethod vocabulary (plural, joined
 * `·`); an empty array renders the honest "no @VerificationMethod declared"
 * placeholder rather than a fabricated default (§5.6).
 */
export interface DeclaredComputedPairProps {
  /** Declared @VerificationMethod kinds (layer 1). Empty ⇒ honest placeholder. */
  methods: string[];
  /** The computed mode for the shown verdict (layer 2). Absent ⇒ no badge. */
  mode: EvaluationMode | string | null | undefined;
  /** Record ref for the mode badge (session / tool). */
  recordRef?: string;
  /** Staleness flag for the mode badge. */
  stale?: boolean;
  testId?: string;
}

export function DeclaredComputedPair({
  methods,
  mode,
  recordRef,
  stale,
  testId,
}: DeclaredComputedPairProps) {
  const normalized = normalizeEvaluationMode(mode);
  return (
    <div
      data-testid={testId ?? 'declared-computed-pair'}
      style={{ display: 'flex', gap: 24, alignItems: 'flex-start' }}
    >
      <RegisterColumn label="DECLARED">
        {methods.length > 0 ? (
          <span
            className="mono-text"
            data-testid="declared-methods"
            style={{ fontSize: 11, color: 'var(--text-primary)' }}
          >
            {methods.join(' · ')}
          </span>
        ) : (
          <span
            data-testid="declared-methods-empty"
            style={{ fontSize: 11, color: 'var(--text-muted)', fontStyle: 'italic' }}
          >
            no @VerificationMethod declared
          </span>
        )}
      </RegisterColumn>
      <RegisterColumn label="COMPUTED ƒ">
        {normalized ? (
          <EvaluationModeBadge
            mode={normalized}
            size="standard"
            recordRef={recordRef}
            stale={stale}
            testId="computed-mode"
          />
        ) : (
          <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>—</span>
        )}
      </RegisterColumn>
    </div>
  );
}

function RegisterColumn({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <span
        style={{
          fontSize: 10,
          color: 'var(--text-muted)',
          letterSpacing: '0.04em',
        }}
      >
        {label}
      </span>
      {children}
    </span>
  );
}
