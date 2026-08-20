/**
 * aggregateReadiness — pure aggregation of the three readiness inputs
 * (diagnostics, dependency status, capabilities) into a `ReadinessSummary`.
 *
 * Kept separate from the hook (`useModelReadiness.ts`) so the
 * ready/warnings/errors/unknown state machine is unit-testable without
 * mocking react-query — mirrors `features/diagnostics/filterDiagnostics.ts`,
 * the sibling pure-helper pattern in the Diagnostics panel.
 */

import type { DiagnosticEntry } from '@/features/diagnostics/types';
import type { DependencyStatusWire } from '@/features/packages/queries';
import type { Capabilities, ReadinessDrillEntry, ReadinessSummary } from './types';

export interface AggregateReadinessInput {
  /** `false` when no workspace root is loaded — short-circuits to `'unknown'`. */
  hasWorkspace: boolean;
  diagnostics: DiagnosticEntry[];
  dependencyStatus: DependencyStatusWire | undefined;
  capabilities: Capabilities | undefined;
}

const EMPTY_SUMMARY: ReadinessSummary = {
  level: 'unknown',
  counts: { errors: 0, warnings: 0 },
  unresolvedDeps: [],
  missingCapabilities: [],
  drill: [],
};

/** `Capabilities` boolean field -> the informational label surfaced when it's `false`. */
const CAPABILITY_LABELS: ReadonlyArray<readonly [keyof Capabilities, string]> = [
  ['hasStateMachines', 'stateMachines'],
  ['hasActionFlows', 'actionFlows'],
  ['hasOdeDynamics', 'odeDynamics'],
  ['hasPortFlows', 'portFlows'],
  ['hasMultipleSubsystems', 'multipleSubsystems'],
  ['hasConstraints', 'constraints'],
  ['hasRequirements', 'requirements'],
  ['hasTradeStudies', 'tradeStudies'],
];

function failedDependenciesOf(
  wire: DependencyStatusWire | undefined,
): Array<{ root: string; name: string; message: string }> {
  if (!wire?.roots) return [];
  const out: Array<{ root: string; name: string; message: string }> = [];
  for (const root of wire.roots) {
    if ('failed_dependencies' in root && Array.isArray(root.failed_dependencies)) {
      for (const failure of root.failed_dependencies) {
        out.push({ root: root.root, name: failure.name, message: failure.message });
      }
    }
  }
  return out;
}

function missingCapabilityLabels(capabilities: Capabilities | undefined): string[] {
  if (!capabilities) return [];
  return CAPABILITY_LABELS.filter(([key]) => !capabilities[key]).map(([, label]) => label);
}

function diagnosticToDrillEntry(entry: DiagnosticEntry): ReadinessDrillEntry {
  return {
    file: entry.diagnostic.span?.file ?? entry.uri,
    severity: entry.diagnostic.severity,
    message: entry.diagnostic.message,
    // No span-to-element resolution today — see the type doc comment.
    elementId: undefined,
  };
}

export function aggregateReadiness(input: AggregateReadinessInput): ReadinessSummary {
  if (!input.hasWorkspace) {
    return EMPTY_SUMMARY;
  }

  const errorEntries = input.diagnostics.filter((e) => e.diagnostic.severity === 'error');
  const warningEntries = input.diagnostics.filter((e) => e.diagnostic.severity === 'warning');
  const failedDeps = failedDependenciesOf(input.dependencyStatus);
  const unresolvedDeps = failedDeps.map((d) => d.name);

  // A dependency that failed to resolve means the model can't even
  // fully load — worse than a diagnostic warning, so it forces the
  // 'errors' level even when the (still-partial) diagnostics set has
  // none yet.
  const level: ReadinessSummary['level'] =
    errorEntries.length > 0 || unresolvedDeps.length > 0
      ? 'errors'
      : warningEntries.length > 0
        ? 'warnings'
        : 'ready';

  const drill: ReadinessDrillEntry[] = [
    ...errorEntries.map(diagnosticToDrillEntry),
    ...failedDeps.map(
      (d): ReadinessDrillEntry => ({
        file: d.root,
        severity: 'error',
        message: `Unresolved dependency "${d.name}": ${d.message}`,
        elementId: undefined,
      }),
    ),
    ...warningEntries.map(diagnosticToDrillEntry),
  ];

  return {
    level,
    counts: { errors: errorEntries.length, warnings: warningEntries.length },
    unresolvedDeps,
    missingCapabilities: missingCapabilityLabels(input.capabilities),
    drill,
  };
}
