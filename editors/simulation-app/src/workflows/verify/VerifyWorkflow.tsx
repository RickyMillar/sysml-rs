/**
 * VerifyWorkflow — route /verify.
 *
 * Two-column layout: config panel on the left, results shell on the right.
 *
 * Case picker is suite-aware:
 *   - evaluate_verification_cases → verificationSuites group
 *   - evaluate_constraints /
 *     evaluate_calculations       → no picker; Run executes against scope
 *
 * Run button is wired through `useVerifyRunner`. When `suite = constraints`
 * or `verification-cases`, the runner drives the real backend; calculations
 * fall back to a no-op until the runner extends to cover it.
 *
 * Live-session mode (UX closeout #3): when `evaluate_verification_cases` is
 * active, the user may pick a RUNNING session instead of running static
 * evaluation. Static eval has no runtime values for simulation-produced
 * derived attributes (e.g. hybrid `tripped`/`trip_time`) — those only resolve
 * against a live session's slot store via `sysml.sessions.verify`. No
 * selection defaults to every known case (there's no live equivalent of the
 * static "ask the backend for every case" fallback — see VerifyCaseRunner).
 */

import { useCallback, useMemo, useState } from 'react';
import { VerifyConfig } from './VerifyConfig';
import { VerifyResultsShell } from './VerifyResultsShell';
import { VerifyWorkflowNinebar } from './VerifyWorkflowNinebar';
import { useVerifyConfig } from './useVerifyConfig';
import { useVerifyRunner } from './runner/useVerifyRunner';
import { buildVerifyRunConfig, suiteRunsWithoutSelection } from './buildVerifyRunConfig';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useRunTargets } from '@/features/run-targets/queries';
import { useSessionList } from '@/features/sessions/queries';
import { EmbeddedDiagram } from '@/components/diagram/EmbeddedDiagram';
import { isFlagEnabled } from '@/featureFlags';

/**
 * Route entry for /verify. Under `?flag=ninebar` the surface is the
 * re-composed five-slot body (`VerifyWorkflowNinebar`); flag-off keeps
 * the legacy two-column body verbatim (deleted in Phase 8 per F17).
 */
export function VerifyWorkflow() {
  if (isFlagEnabled('ninebar')) return <VerifyWorkflowNinebar />;
  return <VerifyWorkflowLegacy />;
}

function VerifyWorkflowLegacy() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: groups, isLoading } = useRunTargets(workspaceRoot, loadedUris);
  const { data: sessions } = useSessionList();

  const config = useVerifyConfig();

  // Live-session verify only applies to the verification-cases suite —
  // constraints/calculations have no `sessions.verify` counterpart.
  const suiteSupportsLive = config.suite === 'evaluate_verification_cases';
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const activeSessionId = suiteSupportsLive ? selectedSessionId : null;

  // Pick the right group for the current suite. Constraints /
  // Calculations show no picker at all.
  const availableCases = useMemo(() => {
    if (!groups) return [];
    if (
      config.suite === 'evaluate_constraints' ||
      config.suite === 'evaluate_calculations'
    ) {
      return [];
    }
    const group = groups.find((g) => g.kind === 'verificationSuites');
    return group?.targets ?? [];
  }, [groups, config.suite]);

  const runner = useVerifyRunner();

  const runnerState = runner.state;
  const isRunning = runnerState === 'running';

  const handleRun = useCallback(() => {
    const runConfig = buildVerifyRunConfig({
      suite: config.suite,
      hasSelection: config.hasSelection,
      selectedCaseIds: config.selectedCaseIds,
      availableCases,
      loadedUris,
      activeSessionId,
    });
    if (!runConfig) return;
    void runner.run(runConfig);
  }, [config.suite, config.hasSelection, config.selectedCaseIds, availableCases, loadedUris, activeSessionId, runner]);

  const error = runner.error?.message ?? null;

  return (
    <div
      data-testid="verify-workflow"
      className="flex flex-row h-full w-full overflow-hidden"
    >
      <VerifyConfig
        availableCases={availableCases}
        config={config}
        isRunning={isRunning}
        hasWorkspace={!!workspaceRoot}
        isLoadingCases={isLoading}
        runsWithoutSelection={suiteRunsWithoutSelection(config.suite)}
        onRun={handleRun}
        supportsLiveSession={suiteSupportsLive}
        sessions={sessions ?? []}
        selectedSessionId={selectedSessionId}
        onSelectSession={setSelectedSessionId}
      />
      <main
        data-testid="verify-results"
        className="flex-1 overflow-hidden"
        style={{ background: 'var(--surface)' }}
      >
        <VerifyResultsShell
          verdicts={runner.verdicts}
          isRunning={isRunning}
          error={error}
        />
      </main>
      {/* Phase 6 — diagram on every workflow tab. The user sees what
          they're verifying without leaving the tab. */}
      <EmbeddedDiagram label="Subject" />
    </div>
  );
}
