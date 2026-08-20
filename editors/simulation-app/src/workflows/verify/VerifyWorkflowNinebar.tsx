/**
 * VerifyWorkflowNinebar — the flag-on Verify surface (design 1a).
 *
 * "Verify, re-composed": the verdict matrix is the primary surface, and the
 * **case view is the ONE detail surface** — the right rail is DEAD in Verify
 * (design 1a, the fork resolved 2026-07-19). A sub-view toggle
 * (Matrix · Cases · History · Report) swaps the primary surface; opening a
 * case (Cases list click, or matrix row double-click) shows the
 * case-as-document `VerifyCaseView`, and `esc` returns to the originating
 * sub-view. The retired Aggregate sub-view's rollup moves into the suite
 * header here.
 *
 * The legacy two-column body (`VerifyWorkflow` flag-off) is untouched —
 * this component only renders under the (default-on) `ninebar` flag.
 */

import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { LeftRailContent, BottomStripContent } from '@/app/slots';
import { Ninebar } from '@/components/Ninebar';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useSessionStore } from '@/features/sessions/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useRunTargets } from '@/features/run-targets/queries';
import { groupByOwnerPath } from '@/features/run-targets/normalize';
import { useSessionList } from '@/features/sessions/queries';
import type { RunTargetSummary } from '@/features/run-targets/types';
import type { SessionSummary } from '@/features/sessions/types';
import type { Verdict, VerifyRunConfig } from '@/engine/types';

import { useVerifyConfig, VERIFY_SUITES, type VerifySuite } from './useVerifyConfig';
import { useVerifyRunner } from './runner/useVerifyRunner';
import { buildVerifyRunConfig, suiteRunsWithoutSelection } from './buildVerifyRunConfig';
import { VerdictMatrix, caseNameOf } from './VerdictMatrix';
import type { EvaluationMode } from '@/components/EvaluationModeBadge';
import { VerifyHistoryView } from './VerifyHistoryView';
import { VerifyReportView } from './VerifyReportView';
import { VerifyCaseView } from './VerifyCaseView';
import { VerifyCasesList } from './VerifyCasesList';
import { useApprovalStates } from '@/features/workflow/queries';
import { useVerificationCases, findCase, isBareObjectiveRow, normalizeCaseVerdict, type VerificationCaseRow } from './useVerificationCases';
import { useVerifySelectionStore } from './verifySelectionStore';
import { DrillProvider } from './useDrillFromVerdict';

type SubView = 'matrix' | 'cases' | 'history' | 'report';

export function VerifyWorkflowNinebar() {
  return (
    <DrillProvider>
      <VerifyNinebarBody />
    </DrillProvider>
  );
}

function VerifyNinebarBody() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: groups, isLoading } = useRunTargets(workspaceRoot, loadedUris);
  const { data: sessions } = useSessionList();

  const config = useVerifyConfig();
  const runner = useVerifyRunner();
  const selectVerdict = useVerifySelectionStore((s) => s.select);
  const clearVerdict = useVerifySelectionStore((s) => s.clear);
  const selectedVerdict = useVerifySelectionStore((s) => s.selectedVerdict);

  // The static verification-case read — source of truth for the Cases list,
  // the case document, and the suite header rollup (design 1a).
  const casesQuery = useVerificationCases();
  const cases = useMemo(() => casesQuery.data ?? [], [casesQuery.data]);
  // Approval states per case (P4 — the one workflow sidecar): the suite
  // rollup counts a passing-but-unapproved case in its own labeled bucket
  // (the ruled coverage principle), so it needs the states here.
  const caseElementIds = useMemo(
    () => cases.map((c) => c.element_id).filter((id): id is string => !!id),
    [cases],
  );
  const { states: approvals } = useApprovalStates(caseElementIds);
  // The frame-chip digest is sourced from `sysml.workspace.verify`'s
  // `model_digest` (steward ruling 2026-07-19), not this bare-array read.
  // Wiring that read here is deferred (see the case-view digest billet); the
  // chip renders nothing until then — a legitimate "not present" absence.
  const modelDigest: string | undefined = undefined;

  const [subView, setSubView] = useState<SubView>('matrix');
  // Case-as-document navigation: which case is open, and where `esc` returns.
  const [selectedCaseId, setSelectedCaseId] = useState<string | null>(null);
  const [caseOriginView, setCaseOriginView] = useState<SubView>('matrix');
  /** The evaluation mode of the row the reader opened the case from, when
   *  they arrived via a row that had one. Null = they browsed in. */
  const [caseEntryMode, setCaseEntryMode] = useState<EvaluationMode | null>(null);
  const selectedCase = useMemo(() => findCase(cases, selectedCaseId), [cases, selectedCaseId]);

  const suiteSupportsLive = config.suite === 'evaluate_verification_cases';

  // Verify used to keep its own session selection seeded to `null`, independent
  // of the session the rest of the app is on — so the header could read
  // "f7af4610 · orchestrator" while this rail read "Static (no session)" and the
  // matrix evaluated against nothing (finding 30, the same two-session-concepts
  // family as finding 8).
  //
  // `undefined` means "no explicit choice here, follow the app"; a real value
  // (including `null` for Static) is a deliberate override and sticks, so you
  // can still pin this surface to a different session — or to none — while the
  // rest of the app moves on.
  const appSessionId = useSessionStore((st) => st.activeSessionId);
  const [sessionOverride, setSessionOverride] = useState<string | null | undefined>(undefined);
  const selectedSessionId = sessionOverride === undefined ? appSessionId : sessionOverride;
  const setSelectedSessionId = setSessionOverride;
  const activeSessionId = suiteSupportsLive ? selectedSessionId : null;

  const availableCases = useMemo(() => {
    if (!groups) return [];
    if (suiteRunsWithoutSelection(config.suite)) return [];
    const group = groups.find((g) => g.kind === 'verificationSuites');
    return group?.targets ?? [];
  }, [groups, config.suite]);

  const isRunning = runner.state === 'running';

  // Cases the user selected that have not yet produced a verdict → the
  // matrix's not-run / running rows.
  const pendingCaseNames = useMemo(() => {
    if (suiteRunsWithoutSelection(config.suite)) return [];
    const produced = new Set(runner.verdicts.map((v) => caseNameOf(v)));
    return Array.from(config.selectedCaseIds)
      .map((id) => availableCases.find((c) => c.id === id)?.name ?? null)
      .filter((n): n is string => !!n && !produced.has(n));
  }, [config.suite, config.selectedCaseIds, availableCases, runner.verdicts]);

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
    clearVerdict();
    setSubView('matrix');
    void runner.run(runConfig);
  }, [config.suite, config.hasSelection, config.selectedCaseIds, availableCases, loadedUris, activeSessionId, runner, clearVerdict]);

  // Clear the selected-verdict handoff when leaving the surface.
  useEffect(() => () => clearVerdict(), [clearVerdict]);

  // Chip click SELECTS (the amber echo on the matrix); the right rail is dead
  // in Verify — detail lives on the case document (design 1a).
  const onSelectVerdict = useCallback(
    (verdict: Verdict) => {
      selectVerdict(verdict);
    },
    [selectVerdict],
  );

  // ── Case-as-document navigation ──────────────────────────────────────
  // Opening from the matrix remembers 'matrix' so `esc` returns there;
  // History rows remember 'history'; browsing the Cases list keeps the
  // origin as 'cases' (esc deselects).
  const openCaseFromMatrix = useCallback((caseId: string, mode: EvaluationMode | null) => {
    setCaseOriginView('matrix');
    // The mode of the row that was clicked. The case view marks the matching
    // evidence line so a reader who came in on a "trajectory" row can see
    // which line they came in on (J5) — it does not change what is shown.
    setCaseEntryMode(mode);
    setSelectedCaseId(caseId);
    setSubView('cases');
  }, []);

  const openCaseFromHistory = useCallback((caseId: string) => {
    setCaseOriginView('history');
    // History rows are execution records, so the reader always arrived via a
    // recorded run.
    setCaseEntryMode('trajectory');
    setSelectedCaseId(caseId);
    setSubView('cases');
  }, []);

  const handleCaseBack = useCallback(() => {
    setSelectedCaseId(null);
    if (caseOriginView !== 'cases') setSubView(caseOriginView);
  }, [caseOriginView]);

  const onSelectSubView = useCallback((view: SubView) => {
    setSubView(view);
    if (view === 'cases') {
      setCaseOriginView('cases');
      // Browsing the list is not an arrival via any one mode.
      setCaseEntryMode(null);
    }
    else setSelectedCaseId(null);
  }, []);

  // `esc` closes the open case (design 1a footer: "esc · back to cases").
  useEffect(() => {
    if (subView !== 'cases') return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') handleCaseBack();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [subView, handleCaseBack]);

  // Run affordances for the case view (§2.1a(d), design D — labeled, never
  // substituted). "Evaluate (static)" recomputes the static read; "Run with
  // simulation" drives the runner's live session-coupled path when a session
  // is active (else the affordance is visibly gated with an honest tooltip).
  const onEvaluateStatic = useCallback(() => {
    void casesQuery.refetch();
  }, [casesQuery]);

  const canRunWithSimulation = !!activeSessionId && !!selectedCase?.case_name;
  const onRunWithSimulation = useCallback(() => {
    if (!activeSessionId || !selectedCase?.case_name) return;
    const runConfig: VerifyRunConfig = {
      suite: 'verification-cases',
      caseIds: [selectedCase.case_name],
      sessionId: activeSessionId,
    };
    setSubView('matrix');
    setSelectedCaseId(null);
    void runner.run(runConfig);
  }, [activeSessionId, selectedCase, runner]);

  // Workspace-loaded gate for the History timeline. The timeline request
  // itself carries no uri — the backend archive's workspace_uri holds
  // run-scope values (`__workspace__` / target file uris), so the old
  // `loadedUris[0]` filter matched almost nothing and History rendered
  // empty for whole-workspace runs (scope-collapse plan, W7 follow-up).
  const hasWorkspace = loadedUris.length > 0 || !!workspaceRoot;

  const selectedCaseNames = useMemo(
    () =>
      Array.from(config.selectedCaseIds)
        .map((id) => availableCases.find((c) => c.id === id)?.name ?? null)
        .filter((n): n is string => !!n),
    [config.selectedCaseIds, availableCases],
  );
  const workspaceName = useMemo(() => {
    if (!workspaceRoot) return 'workspace';
    const segs = workspaceRoot.split(/[/\\]/).filter(Boolean);
    return segs[segs.length - 1] ?? workspaceRoot;
  }, [workspaceRoot]);

  return (
    <div data-testid="verify-workflow-ninebar" className="flex flex-col h-full w-full min-h-0" style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}>
      {/* ── Left rail: config (default) or the Cases list (Cases sub-view) ── */}
      <LeftRailContent>
        {subView === 'cases' ? (
          <VerifyCasesList
            cases={cases}
            isLoading={casesQuery.isLoading}
            isError={casesQuery.isError}
            hasWorkspace={!!workspaceRoot}
            selectedCaseId={selectedCaseId}
            onSelectCase={setSelectedCaseId}
            modelDigest={modelDigest}
          />
        ) : (
          <VerifyRailConfig
            config={config}
            availableCases={availableCases}
            hasWorkspace={!!workspaceRoot}
            isLoadingCases={isLoading}
            isRunning={isRunning}
            onRun={handleRun}
            supportsLiveSession={suiteSupportsLive}
            sessions={sessions ?? []}
            selectedSessionId={selectedSessionId}
            onSelectSession={setSelectedSessionId}
          />
        )}
      </LeftRailContent>

      {/* ── Sub-view nav: Matrix · Cases · History · Report + suite rollup ── */}
      <div className="flex items-center gap-1 px-3 shrink-0" style={{ height: 34, borderBottom: '1px solid var(--border-hairline)' }}>
        <SubViewTab id="matrix" label="Matrix" active={subView === 'matrix'} onClick={() => onSelectSubView('matrix')} />
        <SubViewTab id="cases" label="Cases" active={subView === 'cases'} onClick={() => onSelectSubView('cases')} />
        <SubViewTab id="history" label="History" active={subView === 'history'} onClick={() => onSelectSubView('history')} />
        <SubViewTab id="report" label="Report" active={subView === 'report'} onClick={() => onSelectSubView('report')} />
        {runner.state === 'error' ? (
          <span className="mono-text" style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--verdict-fail)' }}>
            {runner.error?.message ?? 'Verification failed'}
          </span>
        ) : (
          <SuiteHeaderRollup cases={cases} approvals={approvals} />
        )}
      </div>

      <div className="flex-1 min-h-0">
        {subView === 'matrix' && (
          <VerdictMatrix
            verdicts={runner.verdicts}
            pendingCaseNames={pendingCaseNames}
            runningCaseName={runner.progress?.caseId ?? null}
            isRunning={isRunning}
            selectedVerdict={selectedVerdict}
            onSelect={onSelectVerdict}
            onOpenCase={openCaseFromMatrix}
          />
        )}
        {subView === 'cases' && (
          <VerifyCaseView
            caseRow={selectedCase}
            modelDigest={modelDigest}
            isLoading={casesQuery.isLoading}
            entryMode={caseEntryMode}
            onBack={handleCaseBack}
            onEvaluateStatic={onEvaluateStatic}
            onRunWithSimulation={onRunWithSimulation}
            canRunWithSimulation={canRunWithSimulation}
            isEvaluating={casesQuery.isFetching}
          />
        )}
        {subView === 'history' && (
          <div className="h-full min-h-0" data-testid="verify-history">
            {hasWorkspace ? (
              // Design turn 3: latest-status table + executions list. The
              // global timeline is gone — the lane timeline survives only
              // case-scoped, in the case view's process register.
              <VerifyHistoryView onOpenCase={openCaseFromHistory} />
            ) : (
              <div className="p-3" style={{ fontSize: 12, color: 'var(--text-muted)' }}>Load a workspace to see verification history.</div>
            )}
          </div>
        )}
        {subView === 'report' && (
          <VerifyReportView
            result={runner.lastResult}
            verdicts={runner.verdicts}
            suiteLabel={config.suiteLabel}
            selectedCaseNames={selectedCaseNames}
            sessionId={activeSessionId}
            sessionProvenance={
              (activeSessionId && sessions?.find((s) => s.id === activeSessionId)?.provenance) ||
              null
            }
            workspaceName={workspaceName}
          />
        )}
      </div>

      {/* ── Bottom strip: rollup + report ──────────────────────────── */}
      <BottomStripContent>
        <VerifyStrip verdicts={runner.verdicts} isRunning={isRunning} suiteLabel={config.suiteLabel} />
      </BottomStripContent>
    </div>
  );
}

// ── Suite header rollup (absorbs the retired Aggregate sub-view) ─────

/**
 * The suite-level verdict rollup, right-aligned in the sub-view nav
 * ("suite 2 pass · 1 fail · …"). Computed from the static case read — the
 * whole suite's standing, not a single run. Cases that bind no checks mint
 * no verdict (1e) and drop out of the counts. Verdict colours are used here
 * because these ARE verdict counts (§5.1).
 *
 * The ruled coverage principle (P4): a PASSING case whose definition is
 * not approved is counted in its own labeled bucket ("N pass (unapproved)",
 * warning family) — never silently inside "pass". Cases whose approval
 * state hasn't loaded yet count as plain pass (no guessed qualification).
 */
function SuiteHeaderRollup({
  cases,
  approvals,
}: {
  cases: VerificationCaseRow[];
  approvals: Map<string, string>;
}) {
  const roll = useMemo(() => {
    const out = { pass: 0, passUnapproved: 0, fail: 0, inconclusive: 0, error: 0, total: 0 };
    for (const c of cases) {
      if (isBareObjectiveRow(c)) continue;
      const kind = normalizeCaseVerdict(c.verdict);
      if (kind === 'pass') {
        const approval = c.element_id ? approvals.get(c.element_id) : undefined;
        if (approval !== undefined && approval !== 'approved') out.passUnapproved += 1;
        else out.pass += 1;
      } else {
        out[kind] += 1;
      }
      out.total += 1;
    }
    return out;
  }, [cases, approvals]);
  if (roll.total === 0) return <span style={{ marginLeft: 'auto' }} />;
  // Interleaved separators — a bucket list must never LEAD with a `·`
  // (live-caught: "suite · 4 inconclusive" when the pass bucket was 0).
  const parts: React.ReactNode[] = [];
  if (roll.pass > 0) parts.push(<span key="p" style={{ color: 'var(--verdict-pass)' }}>{roll.pass} pass</span>);
  if (roll.passUnapproved > 0)
    parts.push(
      <span
        key="q"
        style={{ color: 'var(--severity-warning)' }}
        title="Passing, but the case definition is not approved — qualified, never counted as plain coverage"
        data-testid="verify-suite-rollup-qualified"
      >
        {roll.passUnapproved} pass (unapproved)
      </span>,
    );
  if (roll.fail > 0) parts.push(<span key="f" style={{ color: 'var(--verdict-fail)' }}>{roll.fail} fail</span>);
  if (roll.inconclusive > 0)
    parts.push(<span key="i" style={{ color: 'var(--verdict-inconclusive)' }}>{roll.inconclusive} inconclusive</span>);
  if (roll.error > 0) parts.push(<span key="e" style={{ color: 'var(--verdict-error)' }}>{roll.error} error</span>);
  return (
    <div
      data-testid="verify-suite-rollup"
      className="mono-text flex items-center gap-2"
      style={{ marginLeft: 'auto', fontSize: 11 }}
    >
      <span style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-sans, inherit)' }}>suite</span>
      {parts.flatMap((part, i) =>
        i > 0
          ? [<span key={`sep-${i}`} style={{ color: 'var(--text-muted)' }}>·</span>, part]
          : [part],
      )}
    </div>
  );
}

// ── Bottom strip ────────────────────────────────────────────────────

function VerifyStrip({ verdicts, isRunning, suiteLabel }: { verdicts: Verdict[]; isRunning: boolean; suiteLabel: string }) {
  const counts = useMemo(() => {
    const c = { pass: 0, fail: 0, inconclusive: 0, error: 0 };
    for (const v of verdicts) c[v.verdict] += 1;
    return c;
  }, [verdicts]);
  const total = verdicts.length;
  return (
    <div data-testid="verify-strip" className="flex items-center gap-4 px-3 h-full" style={{ fontSize: 11 }}>
      <span className="mono-text" style={{ color: 'var(--text-muted)' }}>{suiteLabel}</span>
      {isRunning ? (
        <span className="flex items-center gap-2" style={{ color: 'var(--accent-fg)' }}>
          <Ninebar size={14} label="verification running" /> running
        </span>
      ) : total === 0 ? (
        <span style={{ color: 'var(--text-muted)' }}>No verdicts yet</span>
      ) : (
        <span className="flex items-center gap-3 mono-text">
          <span style={{ color: 'var(--verdict-pass)' }}>✓ {counts.pass}</span>
          <span style={{ color: 'var(--verdict-fail)' }}>✗ {counts.fail}</span>
          <span style={{ color: 'var(--verdict-inconclusive)' }}>? {counts.inconclusive}</span>
          <span style={{ color: 'var(--verdict-error)' }}>⨯ {counts.error}</span>
          <span style={{ color: 'var(--text-muted)' }}>· {total} total</span>
        </span>
      )}
    </div>
  );
}

// ── Left-rail config (compact) ──────────────────────────────────────

interface VerifyRailConfigProps {
  config: ReturnType<typeof useVerifyConfig>;
  availableCases: RunTargetSummary[];
  hasWorkspace: boolean;
  isLoadingCases: boolean;
  isRunning: boolean;
  onRun: () => void;
  supportsLiveSession: boolean;
  sessions: SessionSummary[];
  selectedSessionId: string | null;
  onSelectSession: (id: string | null) => void;
}

function VerifyRailConfig({
  config,
  availableCases,
  hasWorkspace,
  isLoadingCases,
  isRunning,
  onRun,
  supportsLiveSession,
  sessions,
  selectedSessionId,
  onSelectSession,
}: VerifyRailConfigProps) {
  const runsWithoutSelection = suiteRunsWithoutSelection(config.suite);
  const allIds = availableCases.map((c) => c.id);
  const allSelected = allIds.length > 0 && allIds.every((id) => config.selectedCaseIds.has(id));
  const canRun = hasWorkspace && (runsWithoutSelection || config.hasSelection);

  return (
    <div data-testid="verify-rail-config" className="flex flex-col h-full min-h-0" style={{ color: 'var(--text-primary)' }}>
      <div className="flex items-center gap-2 px-3 shrink-0" style={{ height: 32, borderBottom: '1px solid var(--border-hairline)' }}>
        <span className="material-symbols-outlined" style={{ fontSize: 15, color: 'var(--accent-fg)' }}>verified</span>
        <span style={{ fontSize: 11, fontWeight: 600 }}>Verify</span>
        <span className="mono-text" style={{ fontSize: 10, color: 'var(--text-muted)', marginLeft: 'auto' }}>{config.selectedCount} selected</span>
      </div>

      <section className="flex flex-col gap-1 px-3 py-2" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
        <RailLabel>Suite</RailLabel>
        <select
          data-testid="verify-suite-select"
          value={config.suite}
          onChange={(e) => config.setSuite(e.target.value as VerifySuite)}
          style={selectStyle}
        >
          {VERIFY_SUITES.map((opt) => (
            <option key={opt.id} value={opt.id}>{opt.label}</option>
          ))}
        </select>
      </section>

      {supportsLiveSession && (
        <section className="flex flex-col gap-1 px-3 py-2" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
          <RailLabel>Live session</RailLabel>
          <select
            data-testid="verify-live-session-select"
            value={selectedSessionId ?? ''}
            onChange={(e) => onSelectSession(e.target.value || null)}
            style={selectStyle}
          >
            <option value="">Static (no session)</option>
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>{(s.label ?? s.subsystem_name ?? s.id)} · tick {s.tick}</option>
            ))}
          </select>
        </section>
      )}

      {!runsWithoutSelection && (
        <section className="flex flex-col flex-1 min-h-0 overflow-hidden">
          <div className="flex items-center gap-2 px-3 py-1.5 shrink-0" style={{ borderBottom: '1px solid var(--border-hairline)' }}>
            <RailLabel>Cases</RailLabel>
            <span className="mono-text" style={{ fontSize: 10, color: 'var(--text-muted)', marginLeft: 'auto' }}>{availableCases.length}</span>
            {availableCases.length > 0 && (
              <button
                type="button"
                data-testid="verify-select-all"
                onClick={() => (allSelected ? config.clearSelection() : config.selectAll(allIds))}
                style={{ background: 'transparent', border: 'none', color: 'var(--accent-fg)', fontSize: 11, cursor: 'pointer' }}
              >
                {allSelected ? 'Clear' : 'All'}
              </button>
            )}
          </div>
          <div className="flex-1 overflow-y-auto">
            {!hasWorkspace ? (
              <RailEmpty text="No workspace loaded" />
            ) : isLoadingCases ? (
              <RailEmpty text="Discovering cases…" />
            ) : availableCases.length === 0 ? (
              <RailEmpty text="No verification cases" />
            ) : (
              <ul data-testid="verify-case-list" style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
                {/* Structural compliance-suite grouping (Phase-4 coord item):
                    cases band by their owning scope's qualified path, with a
                    whole-suite All/Clear on each band. The null bucket
                    (root-namespace / unnamed chains) renders bare — an
                    "(ungrouped)" label would be an invented container. */}
                {groupByOwnerPath(availableCases, (c) => c.qualifiedName).map((group) => {
                  const ids = group.items.map((t) => t.id);
                  const suiteSelected = ids.length > 0 && ids.every((id) => config.selectedCaseIds.has(id));
                  return (
                    <Fragment key={group.ownerPath ?? '(ungrouped)'}>
                      {group.ownerPath !== null && (
                        <li
                          data-testid={`verify-suite-band-${group.ownerPath}`}
                          className="flex items-center gap-2 px-3"
                          style={{ minHeight: 20, fontSize: 10, color: 'var(--text-muted)', marginTop: 4 }}
                        >
                          <span className="mono-text truncate" title={group.ownerPath}>{group.ownerPath}</span>
                          <span>{group.items.length}</span>
                          <button
                            type="button"
                            data-testid={`verify-suite-band-toggle-${group.ownerPath}`}
                            onClick={() =>
                              suiteSelected
                                ? config.setSelection([...config.selectedCaseIds].filter((id) => !ids.includes(id)))
                                : config.selectAll(ids)
                            }
                            style={{ marginLeft: 'auto', background: 'transparent', border: 'none', color: 'var(--accent-fg)', fontSize: 10, cursor: 'pointer' }}
                          >
                            {suiteSelected ? 'Clear' : 'All'}
                          </button>
                        </li>
                      )}
                      {group.items.map((c) => (
                        <li key={c.id}>
                          <label
                            data-testid={`verify-case-row-${c.id}`}
                            className="flex items-center gap-2 px-3"
                            style={{ height: 'var(--row-dense)', cursor: 'pointer', fontSize: 11 }}
                          >
                            <input
                              type="checkbox"
                              checked={config.selectedCaseIds.has(c.id)}
                              onChange={() => config.toggleCase(c.id)}
                              data-testid={`verify-case-checkbox-${c.id}`}
                            />
                            <span className="truncate mono-text">{c.name ?? '(anonymous)'}</span>
                          </label>
                        </li>
                      ))}
                    </Fragment>
                  );
                })}
              </ul>
            )}
          </div>
        </section>
      )}

      <section className="px-3 py-2 shrink-0" style={{ borderTop: '1px solid var(--border-hairline)' }}>
        <button
          type="button"
          data-testid="verify-run"
          disabled={!canRun || isRunning}
          onClick={onRun}
          style={{
            width: '100%',
            height: 30,
            background: canRun && !isRunning ? 'var(--accent)' : 'var(--surface-raised)',
            color: canRun && !isRunning ? 'var(--text-inverse)' : 'var(--text-muted)',
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
          {isRunning ? <Ninebar compact size={12} color="var(--text-muted)" label="running" /> : <span className="material-symbols-outlined" style={{ fontSize: 15 }}>play_arrow</span>}
          {isRunning ? 'Running…' : 'Run Verification'}
        </button>
      </section>
    </div>
  );
}

// ── Small presentational bits ───────────────────────────────────────

function SubViewTab({ id, label, active, onClick }: { id: SubView; label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      data-testid={`verify-subview-${id}`}
      data-active={active}
      onClick={onClick}
      style={{
        padding: '3px 10px',
        borderRadius: 4,
        fontSize: 12,
        cursor: 'pointer',
        background: active ? 'var(--surface-raised)' : 'transparent',
        color: active ? 'var(--text-primary)' : 'var(--text-muted)',
        border: active ? '1px solid var(--border-hairline)' : '1px solid transparent',
      }}
    >
      {label}
    </button>
  );
}

function RailLabel({ children }: { children: React.ReactNode }) {
  return <span style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-muted)', letterSpacing: '0.04em', textTransform: 'uppercase' }}>{children}</span>;
}

function RailEmpty({ text }: { text: string }) {
  return <div className="px-3 py-4" style={{ fontSize: 11, color: 'var(--text-muted)' }}>{text}</div>;
}

const selectStyle: React.CSSProperties = {
  height: 26,
  padding: '0 8px',
  background: 'var(--surface-panel)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-hairline)',
  borderRadius: 6,
  fontSize: 12,
};
