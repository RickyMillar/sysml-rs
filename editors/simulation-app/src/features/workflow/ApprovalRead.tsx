/**
 * ApprovalRead — the read-only, row-density form of the approval
 * lifecycle (design turn 3, 3d).
 *
 * Register discipline (crib sheet, binding): approval is PROCESS — Sans
 * typography, and it is NEVER a pill/chip (that shape belongs to the
 * model/verdict families). This is a compression of the ApprovalStepper:
 * a dot whose fill walks the path (dashed → hollow → filled for
 * draft → in_review → approved) beside the lowercase state word.
 * `rejected` is the one loud state — it borrows the error hue because it
 * is a recorded off-ramp, not a verdict.
 *
 * The stepper (`workflows/requirements/RequirementChips.tsx`) remains the
 * ONE control; this component only ever reads.
 */

import type { CSSProperties } from 'react';

const DOT_BASE: CSSProperties = {
  width: 6,
  height: 6,
  flex: 'none',
  borderRadius: '50%',
  display: 'inline-block',
  boxSizing: 'border-box',
};

interface ApprovalTokens {
  color: string;
  dot: CSSProperties;
}

function tokensFor(state: string): ApprovalTokens {
  switch (state) {
    case 'approved':
      return { color: 'var(--text-secondary)', dot: { background: 'currentColor' } };
    case 'in_review':
      return {
        color: 'var(--text-muted)',
        dot: { background: 'transparent', border: '1px solid currentColor' },
      };
    case 'rejected':
      return {
        color: 'var(--severity-error)',
        dot: { background: 'transparent', border: '1px solid currentColor' },
      };
    default: // draft — every element's initial state
      return {
        color: 'var(--text-disabled)',
        dot: { background: 'transparent', border: '1px dashed currentColor' },
      };
  }
}

export function ApprovalRead({ state, testId }: { state: string; testId?: string }) {
  const t = tokensFor(state);
  return (
    <span
      data-testid={testId ?? 'approval-read'}
      data-approval-state={state}
      title={`Approval state: ${state.replace('_', ' ')} — the definition lifecycle (same states requirements have), distinct from every verdict`}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 5,
        fontFamily: 'var(--font-body)',
        fontSize: 10.5,
        color: t.color,
        whiteSpace: 'nowrap',
      }}
    >
      <span aria-hidden style={{ ...DOT_BASE, ...t.dot }} />
      {state.replace('_', ' ')}
    </span>
  );
}
