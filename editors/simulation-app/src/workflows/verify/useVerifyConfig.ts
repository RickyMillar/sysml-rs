/**
 * useVerifyConfig — local UI state for the VerifyWorkflow config panel.
 *
 * Owns the two pieces of user intent that make up a verify run:
 *
 *   1. Which verification cases to include (multi-select by id).
 *   2. Which backend suite to invoke (evaluate_constraints /
 *      evaluate_calculations / evaluate_verification_cases).
 *
 * Verify always runs against the whole workspace (the app only ever
 * loads via `sysml.load_workspace`), so there is no scope toggle.
 *
 * Pure local state — lives for the lifetime of the VerifyWorkflow page
 * and nothing else. Deliberately NOT a Zustand store: no other surface
 * needs to observe this, and "Run Verification" is a one-shot
 * imperative call driven by this panel.
 *
 * Consumers: VerifyConfig (UI), VerifyWorkflow (derived summary + run
 * button wiring). Tested in `__tests__/useVerifyConfig.test.ts`.
 */

import { useCallback, useMemo, useState } from 'react';

// ── Suites ──────────────────────────────────────────────────────────

/**
 * The verification suite to invoke. Each corresponds to one backend
 * command; the enum is the workflow-neutral choice the user makes in
 * the UI. R3.2 (the runner agent) maps this to the actual
 * `sysml.*` command and fan-out semantics.
 */
export type VerifySuite =
  | 'evaluate_constraints'
  | 'evaluate_calculations'
  | 'evaluate_verification_cases';

export interface VerifySuiteOption {
  id: VerifySuite;
  /** Short label for the select. */
  label: string;
  /** One-line hint used for tooltips / helper text. */
  description: string;
}

/**
 * Canonical list of suites the UI exposes — ordering matters (shown in
 * the `<select>` in this order).
 */
export const VERIFY_SUITES: readonly VerifySuiteOption[] = [
  {
    id: 'evaluate_verification_cases',
    label: 'Verification Cases',
    description: 'Run every selected verification case end-to-end.',
  },
  {
    id: 'evaluate_constraints',
    label: 'Constraints',
    description: 'Evaluate every constraint in scope; report pass/fail per.',
  },
  {
    id: 'evaluate_calculations',
    label: 'Calculations',
    description: 'Evaluate every calculation in scope; report results.',
  },
] as const;

export const DEFAULT_SUITE: VerifySuite = 'evaluate_verification_cases';

export function isVerifySuite(s: string): s is VerifySuite {
  return VERIFY_SUITES.some((o) => o.id === s);
}

// ── Hook ────────────────────────────────────────────────────────────

export interface VerifyConfigState {
  /** Set of selected case element ids. */
  selectedCaseIds: Set<string>;
  /** Currently chosen suite. */
  suite: VerifySuite;

  // selection actions
  /** Toggle a single case id. */
  toggleCase: (id: string) => void;
  /** Select every id in `ids` (merges into current selection). */
  selectAll: (ids: readonly string[]) => void;
  /** Clear the current selection. */
  clearSelection: () => void;
  /** Replace the selection wholesale. */
  setSelection: (ids: readonly string[]) => void;

  // suite actions
  setSuite: (suite: VerifySuite) => void;

  // derived
  /** True when at least one case is selected. */
  hasSelection: boolean;
  /** Number of selected cases. */
  selectedCount: number;
  /** Human-readable label of the current suite. */
  suiteLabel: string;
}

export interface UseVerifyConfigOptions {
  initialSuite?: VerifySuite;
  initialSelection?: readonly string[];
}

export function useVerifyConfig(
  opts: UseVerifyConfigOptions = {},
): VerifyConfigState {
  const {
    initialSuite = DEFAULT_SUITE,
    initialSelection = [],
  } = opts;

  const [selectedCaseIds, setSelectedCaseIds] = useState<Set<string>>(
    () => new Set(initialSelection),
  );
  const [suite, setSuite] = useState<VerifySuite>(initialSuite);

  const toggleCase = useCallback((id: string) => {
    setSelectedCaseIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const selectAll = useCallback((ids: readonly string[]) => {
    setSelectedCaseIds((prev) => {
      const next = new Set(prev);
      for (const id of ids) next.add(id);
      return next;
    });
  }, []);

  const clearSelection = useCallback(() => {
    setSelectedCaseIds(() => new Set());
  }, []);

  const setSelection = useCallback((ids: readonly string[]) => {
    setSelectedCaseIds(() => new Set(ids));
  }, []);

  const suiteLabel = useMemo(
    () => VERIFY_SUITES.find((s) => s.id === suite)?.label ?? suite,
    [suite],
  );

  return {
    selectedCaseIds,
    suite,
    toggleCase,
    selectAll,
    clearSelection,
    setSelection,
    setSuite,
    hasSelection: selectedCaseIds.size > 0,
    selectedCount: selectedCaseIds.size,
    suiteLabel,
  };
}
