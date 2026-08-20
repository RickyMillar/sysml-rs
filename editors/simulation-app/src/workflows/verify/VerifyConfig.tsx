/**
 * VerifyConfig — left-side config panel for /verify.
 *
 * Renders:
 *   1. A verification case picker (multi-select checkbox list,
 *      populated from `useRunTargets` for the active workspace — the
 *      `verificationSuites` group).
 *   2. A suite selector (`VerifySuite` — evaluate_constraints /
 *      evaluate_calculations / evaluate_verification_cases).
 *   3. A "Run Verification" button that stays disabled until at least
 *      one case is selected. The button is a pure event-emitter: the
 *      parent workflow (VerifyWorkflow) owns what "run" actually does.
 *
 * Verify always runs against the whole workspace, so there is no scope
 * toggle.
 *
 * This component is UI-only — all selection / suite state is
 * owned by `useVerifyConfig`, passed in via the `config` prop. The
 * `availableCases` list is also passed in so this component stays
 * dumb (easier to test, and parallel with the RunTargets tool which
 * owns its own query).
 */

import { VERIFY_SUITES, type VerifyConfigState, type VerifySuite } from './useVerifyConfig';
import type { RunTargetSummary } from '@/features/run-targets/types';
import { groupByOwnerPath } from '@/features/run-targets/normalize';
import type { SessionSummary } from '@/features/sessions/types';

export interface VerifyConfigProps {
  /** Verification cases the user may choose from. */
  availableCases: RunTargetSummary[];
  /** The config state (from `useVerifyConfig`). */
  config: VerifyConfigState;
  /** True while a verify run is in flight — disables Run. */
  isRunning?: boolean;
  /** Whether a workspace is actually loaded — gates the panel. */
  hasWorkspace?: boolean;
  /** Loading state for the case list (React Query flag). */
  isLoadingCases?: boolean;
  /**
   * True when the active suite executes against the whole scope
   * regardless of case selection (constraints / calculations). The
   * case picker is hidden and Run is enabled as soon as a workspace
   * is loaded.
   */
  runsWithoutSelection?: boolean;
  /** Click handler for the Run Verification button. */
  onRun: () => void;
  /**
   * True when the active suite has a live-session counterpart
   * (`evaluate_verification_cases` only — constraints have no
   * `sessions.verify` equivalent). Gates whether the session picker
   * renders at all.
   */
  supportsLiveSession?: boolean;
  /** Active sessions the user may verify against instead of static eval. */
  sessions?: SessionSummary[];
  /** Currently picked session id, or null for static (default) mode. */
  selectedSessionId?: string | null;
  /** Called when the user picks a session (or clears back to static). */
  onSelectSession?: (sessionId: string | null) => void;
}

/**
 * Suite-specific copy for the case-list empty state. Before BUG 15 was
 * fixed, every suite saw the same "Add a VerificationCaseUsage..."
 * message, which is meaningless for the Constraints / Calculations
 * paths.
 */
function emptyTitleForSuite(suite: string): string {
  switch (suite) {
    case 'evaluate_constraints':
      return 'No constraints found';
    case 'evaluate_calculations':
      return 'No calculations found';
    case 'evaluate_verification_cases':
    default:
      return 'No verification cases';
  }
}
function emptyHintForSuite(suite: string): string {
  switch (suite) {
    case 'evaluate_constraints':
      return 'Add a `constraint def` or `assert constraint` to the loaded model.';
    case 'evaluate_calculations':
      return 'Add a `calc def :> GetDerivative` (or any `calc def`) to the model.';
    case 'evaluate_verification_cases':
    default:
      return 'Add a VerificationCaseUsage or VerificationCaseDefinition to the model.';
  }
}

export function VerifyConfig({
  availableCases,
  config,
  isRunning = false,
  hasWorkspace = true,
  isLoadingCases = false,
  runsWithoutSelection = false,
  onRun,
  supportsLiveSession = false,
  sessions = [],
  selectedSessionId = null,
  onSelectSession,
}: VerifyConfigProps) {
  const {
    selectedCaseIds,
    suite,
    toggleCase,
    selectAll,
    clearSelection,
    setSelection,
    setSuite,
    hasSelection,
    selectedCount,
  } = config;

  const allIds = availableCases.map((c) => c.id);
  const allSelected = allIds.length > 0 && allIds.every((id) => selectedCaseIds.has(id));
  const canRun = hasWorkspace && (runsWithoutSelection || hasSelection);

  return (
    <aside
      data-testid="verify-config"
      className="flex flex-col shrink-0 h-full overflow-hidden"
      style={{
        width: 320,
        borderRight: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-low)',
      }}
    >
      {/* Header */}
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        style={{
          height: 36,
          borderBottom: '1px solid var(--outline-variant)',
        }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 16, color: 'var(--primary)' }}
        >
          verified
        </span>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--on-surface)' }}>
          Verify
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="verify-config-summary"
        >
          {selectedCount} selected
        </span>
      </div>

      {/* Suite selector */}
      <section
        data-testid="verify-config-suite"
        className="flex flex-col gap-1 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <label
          htmlFor="verify-suite-select"
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Suite
        </label>
        <select
          id="verify-suite-select"
          data-testid="verify-suite-select"
          value={suite}
          onChange={(e) => {
            const next = e.target.value as VerifySuite;
            setSuite(next);
          }}
          style={{
            height: 28,
            padding: '0 8px',
            background: 'var(--surface-container)',
            color: 'var(--on-surface)',
            border: '1px solid var(--outline-variant)',
            borderRadius: 6,
            fontSize: 12,
          }}
        >
          {VERIFY_SUITES.map((opt) => (
            <option key={opt.id} value={opt.id}>
              {opt.label}
            </option>
          ))}
        </select>
        <p
          style={{
            margin: 0,
            fontSize: 11,
            color: 'var(--outline)',
            lineHeight: 1.4,
          }}
        >
          {VERIFY_SUITES.find((s) => s.id === suite)?.description ?? ''}
        </p>
      </section>

      {/* Live session picker — only for suites with a `sessions.verify`
          counterpart. Static eval has no runtime values for
          simulation-produced attributes (e.g. hybrid `tripped`); picking a
          running session verifies against its live final-tick state
          instead. */}
      {supportsLiveSession && (
        <section
          data-testid="verify-config-live-session"
          className="flex flex-col gap-1 px-3 py-3"
          style={{ borderBottom: '1px solid var(--outline-variant)' }}
        >
          <label
            htmlFor="verify-live-session-select"
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: 'var(--outline)',
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}
          >
            Live Session
          </label>
          <select
            id="verify-live-session-select"
            data-testid="verify-live-session-select"
            value={selectedSessionId ?? ''}
            onChange={(e) => onSelectSession?.(e.target.value || null)}
            style={{
              height: 28,
              padding: '0 8px',
              background: 'var(--surface-container)',
              color: 'var(--on-surface)',
              border: '1px solid var(--outline-variant)',
              borderRadius: 6,
              fontSize: 12,
            }}
          >
            <option value="">Static (no session)</option>
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>
                {(s.label ?? s.subsystem_name ?? s.id)} · tick {s.tick}
              </option>
            ))}
          </select>
          <p
            style={{
              margin: 0,
              fontSize: 11,
              color: 'var(--outline)',
              lineHeight: 1.4,
            }}
          >
            {selectedSessionId
              ? 'Verifying against this running session’s live state — reads simulation-produced attributes.'
              : sessions.length === 0
                ? 'No active sessions. Start a run to verify against live state.'
                : 'Pick a running session to verify against its live state instead of static evaluation.'}
          </p>
        </section>
      )}

      {/* Case picker — hidden for suites that run against the whole scope. */}
      {runsWithoutSelection ? (
        <section
          data-testid="verify-config-no-picker"
          className="flex flex-col flex-1 overflow-y-auto px-3 py-4 gap-2"
          style={{ color: 'var(--outline)' }}
        >
          <span
            style={{
              fontSize: 11,
              fontWeight: 600,
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}
          >
            Scope-wide run
          </span>
          <p style={{ margin: 0, fontSize: 11, lineHeight: 1.4 }}>
            {suite === 'evaluate_constraints'
              ? 'Every constraint in the selected scope will be evaluated — no case selection needed.'
              : 'Every calculation in the selected scope will be evaluated — no case selection needed.'}
          </p>
        </section>
      ) : (
      <section
        data-testid="verify-config-cases"
        className="flex flex-col flex-1 overflow-hidden"
      >
        <div
          className="flex items-center gap-2 px-3 py-2 shrink-0"
          style={{ borderBottom: '1px solid var(--outline-variant)' }}
        >
          <span
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: 'var(--outline)',
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}
          >
            Cases
          </span>
          <span
            className="mono-text"
            style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          >
            {availableCases.length}
          </span>
          {availableCases.length > 0 && (
            <>
              <button
                type="button"
                data-testid="verify-select-all"
                onClick={() => (allSelected ? clearSelection() : selectAll(allIds))}
                style={{
                  background: 'transparent',
                  border: 'none',
                  color: 'var(--primary)',
                  fontSize: 11,
                  cursor: 'pointer',
                  padding: '2px 4px',
                }}
              >
                {allSelected ? 'Clear' : 'All'}
              </button>
            </>
          )}
        </div>

        <div className="flex-1 overflow-y-auto">
          {!hasWorkspace ? (
            <CaseListEmpty
              icon="folder_open"
              title="No workspace loaded"
              hint="Load a workspace to list verification cases."
              testId="verify-cases-no-workspace"
            />
          ) : isLoadingCases ? (
            <CaseListEmpty
              icon="progress_activity"
              title="Discovering cases…"
              hint="Scanning the workspace for verification cases."
              spinning
              testId="verify-cases-loading"
            />
          ) : availableCases.length === 0 ? (
            <CaseListEmpty
              icon="search_off"
              title={emptyTitleForSuite(suite)}
              hint={emptyHintForSuite(suite)}
              testId="verify-cases-empty"
            />
          ) : (
            <ul
              data-testid="verify-case-list"
              style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}
            >
              {groupByOwnerPath(availableCases, (c) => c.qualifiedName).map(
                (group) => (
                  <SuiteBand
                    key={group.ownerPath ?? '(ungrouped)'}
                    ownerPath={group.ownerPath}
                    targets={group.items}
                    selectedCaseIds={selectedCaseIds}
                    onToggleCase={toggleCase}
                    onSelectSuite={selectAll}
                    onClearSuite={(ids) =>
                      setSelection(
                        [...selectedCaseIds].filter((id) => !ids.includes(id)),
                      )
                    }
                  />
                ),
              )}
            </ul>
          )}
        </div>
      </section>
      )}

      {/* Run button + running summary */}
      <section
        className="flex flex-col gap-2 px-3 py-3 shrink-0"
        style={{
          borderTop: '1px solid var(--outline-variant)',
          background: 'var(--surface-container)',
        }}
      >
        <div
          data-testid="verify-running-summary"
          style={{ fontSize: 11, color: 'var(--outline)', lineHeight: 1.4 }}
        >
          {runsWithoutSelection
            ? 'Scope-wide run'
            : `${selectedCount} ${selectedCount === 1 ? 'case' : 'cases'} selected`}
          ,{' '}
          <span style={{ color: 'var(--on-surface-variant)' }}>
            {VERIFY_SUITES.find((s) => s.id === suite)?.label}
          </span>
          {' '}suite ·{' '}
          {selectedSessionId ? 'live session' : 'workspace'}
        </div>
        <button
          type="button"
          data-testid="verify-run"
          disabled={!canRun || isRunning}
          onClick={onRun}
          style={{
            height: 32,
            background: canRun && !isRunning
              ? 'var(--primary)'
              : 'var(--surface-container-high)',
            color: canRun && !isRunning
              ? 'var(--on-primary)'
              : 'var(--outline)',
            border: 'none',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor: canRun && !isRunning ? 'pointer' : 'not-allowed',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 6,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            play_arrow
          </span>
          {isRunning ? 'Running…' : 'Run Verification'}
        </button>
      </section>
    </aside>
  );
}

// ── Sub-components ──────────────────────────────────────────────────

/**
 * One structural suite band: the owning scope's qualified path as a
 * quiet header (with a whole-suite All/Clear toggle), its cases
 * underneath. The `ownerPath: null` bucket (root-namespace / unnamed
 * chains) renders its rows WITHOUT a header — an "(ungrouped)" label
 * would be an invented container. Completes the Phase-4 coordination
 * item "structural compliance-suite grouping" now that run targets
 * carry `qualifiedName` (backend ElementSummary projection).
 */
function SuiteBand({
  ownerPath,
  targets,
  selectedCaseIds,
  onToggleCase,
  onSelectSuite,
  onClearSuite,
}: {
  ownerPath: string | null;
  targets: RunTargetSummary[];
  selectedCaseIds: ReadonlySet<string>;
  onToggleCase: (id: string) => void;
  onSelectSuite: (ids: readonly string[]) => void;
  onClearSuite: (ids: readonly string[]) => void;
}) {
  const ids = targets.map((t) => t.id);
  const allSelected = ids.length > 0 && ids.every((id) => selectedCaseIds.has(id));
  return (
    <>
      {ownerPath !== null && (
        <li
          data-testid={`verify-suite-band-${ownerPath}`}
          className="flex items-center gap-2 px-3"
          style={{
            minHeight: 22,
            fontSize: 10,
            color: 'var(--text-muted)',
            borderBottom: '1px solid var(--border-hairline)',
            marginTop: 4,
          }}
        >
          <span className="mono-text truncate" title={ownerPath}>
            {ownerPath}
          </span>
          <span style={{ color: 'var(--outline)' }}>{targets.length}</span>
          <button
            type="button"
            data-testid={`verify-suite-band-toggle-${ownerPath}`}
            onClick={() => (allSelected ? onClearSuite(ids) : onSelectSuite(ids))}
            style={{
              marginLeft: 'auto',
              background: 'none',
              border: 'none',
              color: 'var(--primary)',
              fontSize: 10,
              cursor: 'pointer',
              padding: '1px 2px',
            }}
          >
            {allSelected ? 'Clear' : 'All'}
          </button>
        </li>
      )}
      {targets.map((c) => (
        <CaseRow
          key={c.id}
          target={c}
          checked={selectedCaseIds.has(c.id)}
          onToggle={() => onToggleCase(c.id)}
        />
      ))}
    </>
  );
}

function CaseRow({
  target,
  checked,
  onToggle,
}: {
  target: RunTargetSummary;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <li>
      <label
        data-testid={`verify-case-row-${target.id}`}
        data-checked={checked}
        className="flex items-center gap-2 px-3 py-1.5 transition-colors"
        style={{
          cursor: 'pointer',
          fontSize: 12,
          color: 'var(--on-surface)',
        }}
      >
        <input
          type="checkbox"
          checked={checked}
          onChange={onToggle}
          data-testid={`verify-case-checkbox-${target.id}`}
          style={{ cursor: 'pointer' }}
        />
        <div className="flex-1 min-w-0">
          <div className="truncate" style={{ fontSize: 12 }}>
            {target.name ?? '(anonymous)'}
          </div>
          <div
            className="truncate mono-text"
            style={{ fontSize: 10, color: 'var(--outline)' }}
          >
            {target.metadata.elementKind}
          </div>
        </div>
      </label>
    </li>
  );
}

function CaseListEmpty({
  icon,
  title,
  hint,
  testId,
  spinning = false,
}: {
  icon: string;
  title: string;
  hint: string;
  testId: string;
  spinning?: boolean;
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center gap-2 px-4 py-8"
      style={{ color: 'var(--outline)' }}
    >
      <span
        className="material-symbols-outlined"
        style={{
          fontSize: 28,
          opacity: 0.8,
          animation: spinning ? 'spin 1s linear infinite' : undefined,
        }}
      >
        {icon}
      </span>
      <span style={{ fontSize: 12, fontWeight: 500 }}>{title}</span>
      <span
        style={{ fontSize: 11, maxWidth: 240, textAlign: 'center' }}
      >
        {hint}
      </span>
    </div>
  );
}
