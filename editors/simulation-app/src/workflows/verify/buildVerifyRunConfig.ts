/**
 * buildVerifyRunConfig — the single home for turning the Verify config
 * panel's state (suite / scope / selection / live session) into the
 * runner's `VerifyRunConfig` (ninebar Phase 4).
 *
 * Both the legacy two-column body and the ninebar five-slot body drive
 * the same run, so this resolution lives in one place rather than being
 * open-coded twice (the Verify tab tracks selection by element id, but
 * `sysml.verify` / `sysml.sessions.verify` look cases up by name — the
 * id→name resolution and the "empty selection" fallbacks are the subtle
 * bits worth not duplicating).
 */

import type { RunTargetSummary } from '@/features/run-targets/types';
import type { VerifyRunConfig, VerifySuiteKind } from '@/engine/types';
import type { VerifySuite } from './useVerifyConfig';

/** Map the UI's verbose suite id onto the runner's short kind. */
export function toRunnerKind(suite: VerifySuite): VerifySuiteKind | null {
  switch (suite) {
    case 'evaluate_verification_cases':
      return 'verification-cases';
    case 'evaluate_constraints':
      return 'constraints';
    case 'evaluate_calculations':
      // Not yet wired in the runner — Run is a no-op for this suite.
      return null;
    default:
      return null;
  }
}

/** Suites that run against the whole scope without a case selection. */
export function suiteRunsWithoutSelection(suite: VerifySuite): boolean {
  return suite === 'evaluate_constraints' || suite === 'evaluate_calculations';
}

export interface BuildRunConfigInput {
  suite: VerifySuite;
  hasSelection: boolean;
  selectedCaseIds: Set<string>;
  availableCases: RunTargetSummary[];
  loadedUris: string[];
  /** Live session id, or null for static evaluation. */
  activeSessionId: string | null;
}

/**
 * Resolve the run config, or `null` when the run can't proceed (suite not
 * wired, or no scope + no session).
 */
export function buildVerifyRunConfig(input: BuildRunConfigInput): VerifyRunConfig | null {
  const { suite, hasSelection, selectedCaseIds, availableCases, loadedUris, activeSessionId } = input;

  const kind = toRunnerKind(suite);
  if (!kind) return null;
  // Verify is workspace-scoped; `loadedUris` is only the "is a workspace
  // loaded at all" gate — the run itself never addresses individual files.
  if (!activeSessionId && loadedUris.length === 0) return null;

  // Selection is by element id; the per-case command addresses cases by
  // name. Anonymous (nameless) cases can't be addressed, so they drop.
  const selectedNames = Array.from(selectedCaseIds)
    .map((id) => availableCases.find((c) => c.id === id)?.name ?? null)
    .filter((n): n is string => !!n);

  // Live mode has no "ask the backend for every case" fallback — an empty
  // selection resolves to every known case name; static mode leaves it
  // undefined (the static command's own all-cases fallback).
  const caseIds = activeSessionId
    ? hasSelection
      ? selectedNames
      : availableCases.map((c) => c.name).filter((n): n is string => !!n)
    : hasSelection
      ? selectedNames
      : undefined;

  return { suite: kind, caseIds, sessionId: activeSessionId ?? undefined };
}
