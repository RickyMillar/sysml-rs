/**
 * SweepFilterBar — predicate builder + active-filter chip (R5.4).
 *
 * A small controlled UI used on both sides of the sweep workflow:
 *
 *  - **Pre-run** (before "Run sweep"): builds a `ParamPredicate` that
 *    `applyPreRunFilter` uses to drop cartesian-product points that
 *    would violate the predicate. Shown in the config panel.
 *  - **Post-hoc** (results shell): builds the same `ParamPredicate` but
 *    feeds it to `useSweepSlice`, which calls `sysml.batch.slice` to
 *    narrow the streamed / completed children without re-running.
 *
 * The component is intentionally dumb-and-controlled — callers own the
 * predicate state and react to `onApply` / `onClear`. Chip rendering is
 * local (it's a pure function of the predicate), which means the chip
 * updates synchronously with an apply, without waiting for the backend.
 *
 * Parametrised by the list of available parameter names (so it can be
 * mounted with a nested-range expansion's visible params, not every
 * param the model knows about).
 */

import { useState, type ReactNode } from 'react';
import type { CompareOp, ParamPredicate } from '@/engine/types';

/** Operator options. Spellings match the engine's lowercase enum. */
const OPERATORS: Array<{ value: CompareOp; label: string }> = [
  { value: 'lt', label: '<' },
  { value: 'le', label: '≤' },
  { value: 'gt', label: '>' },
  { value: 'ge', label: '≥' },
  { value: 'eq', label: '=' },
  { value: 'ne', label: '≠' },
];

/** Human-readable operator symbol. Exported for the chip / tests. */
export function operatorLabel(op: CompareOp): string {
  return OPERATORS.find((o) => o.value === op)?.label ?? op;
}

/**
 * Props for the filter bar. All visual state lives in the caller; the
 * bar only manages the in-progress draft (before "Apply").
 */
export interface SweepFilterBarProps {
  /** Available parameter names — the dropdown populates from these. */
  params: string[];
  /** Currently-applied predicate, or `null` when no filter is active. */
  predicate: ParamPredicate | null;
  /** Fires when the user clicks "Apply". */
  onApply: (predicate: ParamPredicate) => void;
  /** Fires when the user clicks the chip's "clear". */
  onClear: () => void;
  /**
   * Label for the bar — visually distinguishes "pre-run" vs "post-hoc"
   * mounts. Defaults to "Filter".
   */
  title?: string;
  /** Extra slot rendered after the predicate builder (buttons etc.). */
  rightSlot?: ReactNode;
}

/** Default draft predicate when nothing is applied yet. */
function defaultDraft(params: string[]): ParamPredicate {
  return {
    param: params[0] ?? '',
    op: 'gt',
    value: 0,
  };
}

export function SweepFilterBar(props: SweepFilterBarProps) {
  const {
    params,
    predicate,
    onApply,
    onClear,
    title = 'Filter',
    rightSlot,
  } = props;

  const [draft, setDraft] = useState<ParamPredicate>(
    predicate ?? defaultDraft(params),
  );

  const canApply =
    params.length > 0 &&
    draft.param !== '' &&
    Number.isFinite(draft.value);

  return (
    <div
      className="sweep-filter-bar"
      data-testid="sweep-filter-bar"
      role="group"
      aria-label={title}
    >
      <span className="sweep-filter-bar__title">{title}</span>

      <label className="sweep-filter-bar__field">
        <span className="sr-only">parameter</span>
        <select
          data-testid="sweep-filter-bar-param"
          value={draft.param}
          onChange={(e) => setDraft({ ...draft, param: e.target.value })}
        >
          {params.length === 0 ? (
            <option value="" disabled>
              (no params)
            </option>
          ) : (
            params.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))
          )}
        </select>
      </label>

      <label className="sweep-filter-bar__field">
        <span className="sr-only">operator</span>
        <select
          data-testid="sweep-filter-bar-op"
          value={draft.op}
          onChange={(e) =>
            setDraft({ ...draft, op: e.target.value as CompareOp })
          }
        >
          {OPERATORS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>

      <label className="sweep-filter-bar__field">
        <span className="sr-only">value</span>
        <input
          data-testid="sweep-filter-bar-value"
          type="number"
          value={Number.isFinite(draft.value) ? String(draft.value) : ''}
          onChange={(e) => {
            const n = Number(e.target.value);
            setDraft({ ...draft, value: Number.isFinite(n) ? n : 0 });
          }}
        />
      </label>

      <button
        type="button"
        data-testid="sweep-filter-bar-apply"
        disabled={!canApply}
        onClick={() => {
          if (canApply) onApply({ ...draft });
        }}
      >
        Apply
      </button>

      {predicate ? (
        <ActiveFilterChip predicate={predicate} onClear={onClear} />
      ) : null}

      {rightSlot}
    </div>
  );
}

// ── Chip ────────────────────────────────────────────────────────────

interface ActiveFilterChipProps {
  predicate: ParamPredicate;
  onClear: () => void;
}

/**
 * Active-filter chip. Exported so the results shell can render the chip
 * standalone (e.g. in a summary header) without the builder.
 */
export function ActiveFilterChip(props: ActiveFilterChipProps) {
  const { predicate, onClear } = props;
  return (
    <span
      className="sweep-filter-bar__chip"
      data-testid="sweep-filter-bar-chip"
      role="status"
    >
      <span className="sweep-filter-bar__chip-text">
        {predicate.param} {operatorLabel(predicate.op)} {predicate.value}
      </span>
      <button
        type="button"
        data-testid="sweep-filter-bar-chip-clear"
        aria-label="clear filter"
        onClick={onClear}
      >
        ×
      </button>
    </span>
  );
}
