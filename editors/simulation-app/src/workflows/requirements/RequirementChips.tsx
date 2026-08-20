/**
 * RequirementChips — the two chip families the table renders per row,
 * plus the approval STEPPER (which is deliberately NOT a chip).
 *
 * Chip-family discipline (visual gate §5, binding):
 *   · Verified rollup is the ONLY place verdict colours appear.
 *   · Maturity is glyph-differentiated neutral (●/◐/○) — never colour.
 *   · Both chips carry a `title` — the three-state Verified ruling
 *     REQUIRES in-UI labeling (the demo left it unexplained).
 *
 * Register discipline (three-register redesign, crib sheet):
 *   · Maturity is MODEL — a `@StatusInfo` source field, lowercase mono
 *     chip on the model surface.
 *   · Approval is PROCESS — a Sans lifecycle stepper that exists only
 *     inside the process zone. Different kind of object, typography,
 *     and geography from maturity, so the two can never read as the
 *     same "status". Never render approval as a pill.
 */

import {
  maturityGlyph,
  verifiedChipModel,
} from '@/features/requirements/rollup';
import type { RequirementVerificationRollup } from '@/features/requirements/types';
import { EvaluationModeBadgeFromRaw } from '@/components/EvaluationModeBadge';

export function MaturityChip({ maturity }: { maturity: string | null }) {
  if (!maturity) return null;
  return (
    <span
      data-testid="req-maturity-chip"
      title={`Maturity (@StatusInfo): ${maturity}`}
      // Calm pass (P2): neutral lifecycle info, de-bordered — the glyph +
      // word carry it, the pill was decoration.
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        whiteSpace: 'nowrap',
        color: 'var(--text-muted)',
      }}
    >
      {maturityGlyph(maturity)} {maturity}
    </span>
  );
}

/**
 * DECLARED verification method(s) — model intent off the verifying cases'
 * `@VerificationMethod` annotations (B4). Neutral like maturity, never
 * verdict colours: the method says HOW verification is meant to be carried
 * out, not whether it passed — and it is NOT `evaluation_mode` (how the
 * tool computed the shown verdict); the two must stay visually distinct.
 */
export function MethodChip({ methods }: { methods: string[] }) {
  if (methods.length === 0) return null;
  return (
    <span
      data-testid="req-method-chip"
      title={`Declared verification method (@VerificationMethod): ${methods.join(
        ', ',
      )} — model intent, not how the shown verdict was computed`}
      // Calm pass (P2): neutral declared-intent info, de-bordered.
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        whiteSpace: 'nowrap',
        color: 'var(--text-muted)',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        maxWidth: '100%',
        display: 'inline-block',
        verticalAlign: 'middle',
        boxSizing: 'border-box',
      }}
    >
      {methods.join(' · ')}
    </span>
  );
}

/** The linear review path; `rejected` is an off-ramp, not a step. */
const STEPPER_PATH = ['draft', 'in_review', 'approved'] as const;

/**
 * ApprovalStepper — the process-register approval control (crib sheet,
 * binding: approval is a Sans lifecycle stepper, never a chip). Clicking
 * a step RECORDS a signed transition to that state — permanent, append-
 * only; there is no undo. `rejected` sits off the linear path as a
 * separate action / terminal marker.
 */
export function ApprovalStepper({
  current,
  disabled,
  onTransition,
}: {
  current: string;
  disabled?: boolean;
  onTransition: (to: string) => void;
}) {
  const currentIdx = STEPPER_PATH.indexOf(current as (typeof STEPPER_PATH)[number]);
  const stepButton = (state: (typeof STEPPER_PATH)[number], i: number) => {
    const isCurrent = state === current;
    const reached = currentIdx >= 0 && i <= currentIdx;
    return (
      <button
        key={state}
        type="button"
        data-testid={`workflow-approval-step-${state}`}
        data-current={isCurrent || undefined}
        disabled={disabled || isCurrent}
        onClick={() => onTransition(state)}
        title={
          isCurrent
            ? `Current approval state: ${state}`
            : `Record a signed transition to ${state} — permanent, append-only`
        }
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          border: 'none',
          background: 'transparent',
          padding: '0 0 2px',
          fontFamily: 'var(--font-body)',
          fontSize: 'var(--text-xs)',
          color: isCurrent
            ? 'var(--text-primary)'
            : reached
              ? 'var(--text-muted)'
              : 'var(--text-disabled)',
          borderBottom: isCurrent ? '2px solid var(--text-primary)' : '2px solid transparent',
          cursor: disabled || isCurrent ? 'default' : 'pointer',
          whiteSpace: 'nowrap',
        }}
      >
        <span
          aria-hidden
          style={{
            width: 6,
            height: 6,
            flex: 'none',
            borderRadius: '50%',
            background: reached ? 'currentColor' : 'transparent',
            border: reached ? 'none' : '1px solid currentColor',
          }}
        />
        {state.replace('_', ' ')}
      </button>
    );
  };
  return (
    <div
      data-testid="workflow-approval-stepper"
      style={{
        display: 'flex',
        alignItems: 'center',
        minHeight: 'var(--row-default)',
        fontFamily: 'var(--font-body)',
      }}
    >
      {STEPPER_PATH.map((state, i) => (
        // Steps joined by a short rule — one linear path, read at a glance.
        <span key={state} style={{ display: 'flex', alignItems: 'center' }}>
          {i > 0 && (
            <span
              aria-hidden
              style={{ width: 18, height: 1, background: 'var(--border-default)', margin: '0 6px' }}
            />
          )}
          {stepButton(state, i)}
        </span>
      ))}
      <span style={{ flex: 1 }} />
      {current === 'rejected' ? (
        <span
          data-testid="workflow-approval-rejected"
          title="Approval was rejected — record a transition on the path to resume review"
          style={{ fontSize: 'var(--text-xs)', color: 'var(--severity-error)' }}
        >
          rejected
        </span>
      ) : (
        <button
          type="button"
          data-testid="workflow-approval-reject"
          disabled={disabled}
          onClick={() => onTransition('rejected')}
          title="Record a signed transition to rejected — permanent, append-only"
          style={{
            border: 'none',
            background: 'transparent',
            padding: 0,
            fontFamily: 'var(--font-body)',
            fontSize: 'var(--text-xs)',
            color: 'var(--text-disabled)',
            cursor: disabled ? 'default' : 'pointer',
          }}
        >
          reject…
        </button>
      )}
    </div>
  );
}

export function VerifiedChip({
  rollup,
}: {
  rollup: RequirementVerificationRollup;
}) {
  const model = verifiedChipModel(rollup);
  if (model.variant === 'none') {
    return (
      <span
        data-testid="req-verified-chip"
        data-variant="none"
        title={model.title}
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          color: 'var(--text-disabled)',
        }}
      >
        {model.label}
      </span>
    );
  }
  // Calm pass (P2): the verdict rollup reads by COLOUR + glyph on the bare
  // ground — no filled pill, no outline box. `fail` gets a leading ✗ (the
  // pass label already carries ✓) so the state is colour-blind-safe;
  // `outline` (incomplete — not a verdict) stays muted, no verdict colour.
  const verdictColor =
    model.variant === 'pass'
      ? 'var(--verdict-pass)'
      : model.variant === 'fail'
        ? 'var(--verdict-fail)'
        : 'var(--text-muted)';
  return (
    // The mode badge (§2.1a(d), B10) carries the evaluation-mode signal as a
    // visible sibling. Wrapper keeps the verdict chip span's data shape
    // (testid/variant/title) so its tests are unchanged.
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
      <span
        data-testid="req-verified-chip"
        data-variant={model.variant}
        title={model.title}
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          fontWeight: 600,
          whiteSpace: 'nowrap',
          color: verdictColor,
        }}
      >
        {model.variant === 'fail' ? '✗ ' : ''}
        {model.label}
      </span>
      <EvaluationModeBadgeFromRaw
        mode={rollup.evaluation_mode}
        size="compact"
        testId="req-evaluation-mode"
      />
    </span>
  );
}
