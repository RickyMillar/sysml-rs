/**
 * PromoteToCompareButton — R5.12 handoff from TradeStudyTableViewer to
 * CompareWorkflow.
 *
 * The user multi-selects 2..6 alternatives in the trade-study table
 * (via row checkboxes); clicking this button takes their `session_id`s,
 * pushes them into `useCompareStore.setPickedSessionIds`, and navigates
 * to `/compare`.
 *
 * Contracts:
 *   - Enabled window is 2..6 selections. Outside that window the button
 *     is disabled with an explanatory tooltip (via `title`).
 *   - Selected rows without a `session_id` (pending / failed-pre-kickoff)
 *     are silently filtered OUT before the handoff — compare cannot
 *     overlay a row that never ran. If that filter drops the count
 *     below 2, the button stays disabled even though `selectedChildren`
 *     is ≥ 2. Tests pin this behaviour.
 *   - Ordering is preserved exactly as caller supplies. Compare's
 *     session pills use that ordering for its colour palette, so
 *     stability matters.
 *
 * Styling: aligns with the shared viewer kit — no hover transitions
 * (`prefers-reduced-motion`), MaterialSymbols not required, card-style
 * border. Colours from the app's OKLCH palette via CSS vars.
 */
import type { CSSProperties } from 'react';
import { useNavigate } from 'react-router-dom';
import { useCompareStore } from '@/workflows/compare/useCompareStore';
import type { ChildDescriptorLike } from './tradeHelpers';

/** Min/max selection window (inclusive). Mirrors Compare's picker clamp. */
export const PROMOTE_MIN_SELECTED = 2;
export const PROMOTE_MAX_SELECTED = 6;

/**
 * Pure helper — returns the session_ids that would actually reach
 * Compare after the "has session_id" filter. Exported for testing.
 */
export function toPromotedSessionIds(
  selectedChildren: ChildDescriptorLike[],
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const child of selectedChildren) {
    const sid = child.session_id;
    if (typeof sid !== 'string' || sid.length === 0) continue;
    if (seen.has(sid)) continue;
    seen.add(sid);
    out.push(sid);
  }
  return out;
}

/**
 * Message for the button's `title` (tooltip) when it is disabled, so
 * users know *why* they can't promote yet. Exported so the test suite
 * can lock the exact copy (avoids accidental drift during polish).
 */
export function promoteDisabledReason(
  selected: number,
  promotable: number,
): string | null {
  if (selected < PROMOTE_MIN_SELECTED) {
    return `Select at least ${PROMOTE_MIN_SELECTED} alternatives to promote`;
  }
  if (selected > PROMOTE_MAX_SELECTED) {
    return `Compare supports at most ${PROMOTE_MAX_SELECTED} sessions`;
  }
  if (promotable < PROMOTE_MIN_SELECTED) {
    return 'Selected alternatives have no completed runs to compare';
  }
  return null;
}

export interface PromoteToCompareButtonProps {
  /** The user-selected rows from the trade-study table. */
  selectedChildren: ChildDescriptorLike[];
  /**
   * Optional override for the navigate target. Defaults to `/compare`.
   * Tests use this to pin the navigation argument without needing a
   * router mock for the app's real route.
   */
  navigateTo?: string;
  /** Optional extra className for the host panel's layout. */
  className?: string;
}

export function PromoteToCompareButton({
  selectedChildren,
  navigateTo = '/run/compare',
  className,
}: PromoteToCompareButtonProps) {
  const navigate = useNavigate();
  const setPickedSessionIds = useCompareStore(
    (s) => s.setPickedSessionIds,
  );

  const promotable = toPromotedSessionIds(selectedChildren);
  const disabledReason = promoteDisabledReason(
    selectedChildren.length,
    promotable.length,
  );
  const disabled = disabledReason !== null;

  const handleClick = () => {
    if (disabled) return;
    setPickedSessionIds(promotable);
    navigate(navigateTo);
  };

  const label = `Promote ${promotable.length} to comparison`;

  const buttonStyle: CSSProperties = {
    padding: '6px 12px',
    borderRadius: 6,
    border: '1px solid color-mix(in srgb, var(--outline-variant) 40%, transparent)',
    background: disabled
      ? 'color-mix(in srgb, var(--outline-variant) 8%, transparent)'
      : 'color-mix(in srgb, var(--primary) 12%, transparent)',
    color: disabled ? 'var(--on-surface)' : 'var(--primary)',
    fontSize: 12,
    fontWeight: 600,
    cursor: disabled ? 'not-allowed' : 'pointer',
    opacity: disabled ? 0.55 : 1,
    // Respect prefers-reduced-motion: no hover transitions.
    transition: 'none',
    userSelect: 'none',
  };

  return (
    <button
      type="button"
      data-testid="promote-to-compare-button"
      data-selected={selectedChildren.length}
      data-promotable={promotable.length}
      onClick={handleClick}
      disabled={disabled}
      title={disabledReason ?? `Promote ${promotable.length} alternatives to /compare`}
      aria-label={label}
      aria-disabled={disabled}
      className={className}
      style={buttonStyle}
    >
      {'\u2192 '}
      {label}
    </button>
  );
}
