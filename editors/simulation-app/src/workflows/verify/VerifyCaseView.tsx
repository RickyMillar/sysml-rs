/**
 * VerifyCaseView — the case-as-document surface (Verify design 1a).
 *
 * The ONE detail surface for a verification case (the right rail is dead in
 * Verify). Three registers keep the three layers (§1.2) from ever being
 * conflated:
 *
 *   · model    — the case as AUTHORED: the objective with its check
 *                occurrences nested UNDER it, recursive subrequirement
 *                failure chains, links to the verified Requirements rows.
 *                The structure is authored; per-check verdicts (ƒ) are
 *                computed.
 *   · computed — the tool's READ, not authored: the case verdict (the only
 *                place verdict colours appear), the evaluation-mode badge,
 *                and mode-appropriate evidence. Dashed edges keep it legible
 *                as "the tool's answer", never source.
 *   · process  — history & attestations recorded as PEOPLE: append-only,
 *                never in source. Attestations are signed human acts, never
 *                verdict-styled.
 *
 * Source of truth: one `VerificationCaseRow` from `useVerificationCases`
 * (the static `sysml.evaluate.verification_cases` read — §4.1). That read
 * preserves the nested `subrequirements` chain the model register renders;
 * the runner's flattened `Verdict[]` does not, which is why the case view
 * reads the raw rows.
 *
 * Binding constraints honoured (brief §5): verdict colours only on verdicts
 * (1); mode badge neutral geometry, external reads as ingested (2); checks
 * nest under the objective, never a flat peer list (4); two labeled run
 * affordances (5); honest empty/teaching states (6); staleness whenever the
 * server says so (7); requirement links deep-link to the Requirements row (8).
 */

import { useNavigate } from 'react-router-dom';
import { useMemo } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { VerdictBadge, normalizeVerdict, type VerdictKind } from '@/components/VerdictBadge';
import {
  DeclaredComputedPair,
  EvaluationModeBadge,
  normalizeEvaluationMode,
  type EvaluationMode,
} from '@/components/EvaluationModeBadge';
import { ActorGate } from '@/features/workflow/ActorGate';
import { useSetApproval, useWorkflowState } from '@/features/workflow/queries';
// The ONE approval control (crib sheet: approval is a Sans stepper, never a
// pill). Imported from its current home in the Requirements lane; if a third
// consumer appears, hoist it to features/workflow beside ActorGate.
import { ApprovalStepper } from '@/workflows/requirements/RequirementChips';
import { VerdictTimelinePanel } from './VerdictTimelinePanel';
import {
  isBareObjectiveRow,
  normalizeCaseVerdict,
  type VerificationCaseRequirement,
  type VerificationCaseRow,
} from './useVerificationCases';
import { latestByCase, relativeAgePhrase, useLatestStatus } from './useExecutionHistory';

export interface VerifyCaseViewProps {
  /** The resolved case row, or null when none is selected / not found. */
  caseRow: VerificationCaseRow | null;
  /** Current-model digest for the `@ <digest7>` chip; absent ⇒ nothing. */
  modelDigest?: string;
  /** True while the static read is in flight. */
  isLoading?: boolean;
  /**
   * Evaluation mode of the row the case was opened from, when there was one.
   * Used to mark which evidence line the reader clicked, and to say plainly
   * that nothing is stored when they came in on a trajectory row and no run
   * exists. Null when they browsed in from the Cases list.
   */
  entryMode?: EvaluationMode | null;
  /** `esc` / "back to cases" — return to the originating sub-view. */
  onBack: () => void;
  /** `= Evaluate (static)` — recompute the static verdict for this case. */
  onEvaluateStatic: () => void;
  /** `∿ Run with simulation` — the trajectory (session-coupled) run. */
  onRunWithSimulation: () => void;
  /**
   * Whether the simulation-coupled run can proceed (a live session is
   * available). When false, the affordance is visibly gated with an honest
   * tooltip — never hidden, never faked (§2.1a(d), brief D).
   */
  canRunWithSimulation: boolean;
  /** True while a static evaluate is refetching. */
  isEvaluating?: boolean;
}

export function VerifyCaseView(props: VerifyCaseViewProps) {
  const {
    caseRow,
    modelDigest,
    isLoading = false,
    entryMode = null,
    onBack,
    onEvaluateStatic,
    onRunWithSimulation,
    canRunWithSimulation,
    isEvaluating = false,
  } = props;

  // Per-mode latest executions (design turn 3, 3c) — a mode key the
  // server omits renders NO line; absence of a line is the information.
  const latestQuery = useLatestStatus();
  const latestMap = useMemo(() => latestByCase(latestQuery.data ?? []), [latestQuery.data]);

  if (isLoading && !caseRow) {
    return (
      <div data-testid="verify-case-view-loading" style={centeredStyle}>
        Loading verification case…
      </div>
    );
  }

  if (!caseRow) {
    return (
      <div data-testid="verify-case-view-empty" style={centeredStyle}>
        <div style={{ fontWeight: 600, marginBottom: 4 }}>No case selected</div>
        <div style={{ opacity: 0.75 }}>
          Pick a case from the list, or double-click a matrix cell to open it.
        </div>
      </div>
    );
  }

  const caseName = caseRow.case_name ?? caseRow.case_id ?? 'Verification case';
  const caseElementId = caseRow.element_id ?? caseRow.case_id ?? null;
  const caseId = caseRow.case_id ?? caseRow.element_id ?? caseName;
  const bare = isBareObjectiveRow(caseRow);
  const methods = (caseRow.methods ?? []).filter((m) => typeof m === 'string' && m.length > 0);
  const mode = normalizeEvaluationMode(caseRow.evaluation_mode);
  const requirements = caseRow.requirements ?? [];
  // A case with no checks mints no verdict at all (1e) — not even inconclusive.
  const verdict: VerdictKind | null = bare ? null : normalizeCaseVerdict(caseRow.verdict);
  const latestEntry =
    latestMap.get(caseName) ?? (caseRow.case_id ? latestMap.get(caseRow.case_id) : undefined);
  const latestTrajectory = latestEntry?.latest?.trajectory ?? null;
  const latestExternal = latestEntry?.latest?.external ?? null;

  /** True when the reader arrived from a row whose computed mode was a run. */
  const enteredOnTrajectory = entryMode === 'trajectory';
  /**
   * Whether the recorded run leads the computed register.
   *
   * A run leads whenever one exists — not only when the reader arrived from a
   * trajectory row. Two reasons: a trajectory verdict is evidence about an
   * execution that actually happened, which outranks a recomputation of the
   * authored model; and ordering the same case differently depending on which
   * row you clicked would make the document itself feel unstable. The entry
   * mode is used to MARK the line the reader came in on, not to reorder.
   */
  const runLeads = latestTrajectory !== null;

  return (
    <div
      data-testid="verify-case-view"
      data-case-id={caseId}
      className="flex flex-col h-full min-h-0"
      style={{ color: 'var(--text-primary)' }}
    >
      <div className="flex-1 min-h-0 overflow-auto">
        {/* ── Header (model register) ─────────────────────────────── */}
        <header style={{ padding: '20px 28px 0' }} data-testid="verify-case-header">
          <div className="flex items-baseline gap-3" style={{ flexWrap: 'wrap' }}>
            <span className="mono-text" style={{ fontSize: 20, fontWeight: 500 }}>
              {caseName}
            </span>
            {/* Source span is not carried on the verify wire — render just the
                kind, never a fabricated file/line (§6, brief scope A). */}
            <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>verification case</span>
            <div style={{ flex: 1 }} />
            <RunAffordance
              testId="verify-case-evaluate-static"
              glyph="="
              label="Evaluate (static)"
              onClick={onEvaluateStatic}
              disabled={isEvaluating}
              busy={isEvaluating}
              title="Recompute this case's verdict against the model's current values — a desk check, recomputed on every edit."
            />
            <RunAffordance
              testId="verify-case-run-simulation"
              glyph="∿"
              label="Run with simulation"
              onClick={onRunWithSimulation}
              disabled={!canRunWithSimulation}
              title={
                canRunWithSimulation
                  ? 'Run this case against a live simulation run (trajectory evidence).'
                  : 'Simulation-coupled verification runs from a live session — none is active. Start or select a session in the Matrix live-session picker.'
              }
            />
          </div>

          {/* Subject — the unit under test. Only the name is on the wire. */}
          <div className="flex items-center gap-6" style={{ height: 24, fontSize: 12, marginTop: 10, flexWrap: 'wrap' }}>
            {caseRow.subject ? (
              <span className="flex items-baseline gap-2" data-testid="verify-case-subject">
                <span style={{ color: 'var(--text-muted)', fontSize: 10.5 }}>subject</span>
                <span className="mono-text flex items-center gap-1.5" style={{ fontSize: 11.5 }}>
                  <span
                    aria-hidden="true"
                    style={{ width: 7, height: 7, border: '1.5px solid var(--sim-accent)', display: 'inline-block' }}
                  />
                  {caseRow.subject}
                </span>
              </span>
            ) : null}
          </div>

          {/* DECLARED (layer 1) × COMPUTED (layer 2) — the pairing rule (1d).
              Empty methods render the honest "no @VerificationMethod declared"
              placeholder, never a defaulted chip (§5.6). */}
          <div style={{ marginTop: 10 }} data-testid="verify-case-declared-computed">
            <DeclaredComputedPair
              methods={methods}
              mode={bare ? null : caseRow.evaluation_mode}
              testId="verify-case-pair"
            />
          </div>
        </header>

        {/* ── model register ──────────────────────────────────────── */}
        <section style={{ margin: '14px 28px 0' }} data-testid="verify-case-model-register">
          <RegisterLabel>model — the case as authored · checks nest under the objective</RegisterLabel>
          <div style={objectiveBoxStyle}>
            <div className="flex items-center gap-2.5" style={{ minHeight: 32, padding: '0 14px', borderBottom: '1px solid var(--border-hairline)' }}>
              <span style={{ fontSize: 10.5, color: 'var(--text-muted)' }}>objective</span>
              <span style={{ fontSize: 12.5, color: 'var(--text-primary)' }}>
                the case objective is satisfied by {caseRow.subject ?? 'the subject'}
              </span>
            </div>
            {bare ? (
              <BareObjectiveTeaching />
            ) : requirements.length === 0 ? (
              <BareObjectiveTeaching />
            ) : (
              <div style={{ padding: '6px 14px 10px' }} data-testid="verify-case-checks">
                {requirements.map((req, idx) => (
                  <CheckOccurrence key={`${req.requirement_id ?? req.requirement_element_id ?? idx}`} requirement={req} depth={0} />
                ))}
              </div>
            )}
          </div>
          <FootnoteLine>
            {requirements.length === 1 ? 'one check occurrence' : `${requirements.length} check occurrences`} ·
            per-check verdicts ƒ are computed, the structure is authored
          </FootnoteLine>
        </section>

        {/* ── computed register ───────────────────────────────────── */}
        <section style={computedRegisterStyle} data-testid="verify-case-computed-register">
          <div className="flex items-center gap-2" style={{ minHeight: 24, fontSize: 10.5, color: 'var(--text-muted)' }}>
            <span className="mono-text" style={{ fontStyle: 'italic' }}>ƒ</span>
            computed — the tool’s read · not authored
          </div>
          {bare || !verdict ? (
            <div className="flex items-center gap-3" style={{ padding: '2px 0 12px', fontSize: 11.5, color: 'var(--text-muted)' }} data-testid="verify-case-computed-noverdict">
              no verdict is minted for a case with no checks — not even inconclusive
            </div>
          ) : (
            <>
              {/* ── the run, when there is one ────────────────────────
                  A trajectory verdict is a claim about a specific execution,
                  so it leads and carries that execution's identity: session,
                  tick, and when it was recorded. Without those, "latest run:
                  PASS" is a claim a reader cannot check. */}
              {latestTrajectory ? (
                <div
                  className="flex flex-col gap-1"
                  style={{ padding: '2px 0 8px' }}
                  data-testid="verify-case-latest-run"
                >
                  <div className="flex items-center gap-3.5" style={{ flexWrap: 'wrap' }}>
                    <LatestLineLabel>{runLeads ? 'this run' : 'latest run'}</LatestLineLabel>
                    <EvaluationModeBadge mode="trajectory" size="mark" />
                    <VerdictBadge
                      verdict={normalizeVerdict(latestTrajectory.verdict)}
                      size="bare"
                      showLabel
                      name={caseName}
                      testId="verify-case-latest-run-verdict"
                    />
                    {enteredOnTrajectory ? (
                      <span
                        style={{ fontSize: 10.5, color: 'var(--text-muted)' }}
                        data-testid="verify-case-entry-marker"
                        title="You opened this case from this row"
                      >
                        ← the row you opened
                      </span>
                    ) : null}
                    {latestTrajectory.case_changed_since === true ? (
                      <span style={{ fontSize: 10.5, color: 'var(--severity-warning)' }} data-testid="verify-case-latest-run-changed">
                        Δ case edited since this run
                      </span>
                    ) : null}
                  </div>
                  {/* The run record itself. Session and tick come from the
                      archived evidence; a record minted before B10 carries
                      neither, and says so rather than showing blanks. */}
                  <div
                    className="flex items-center gap-3 mono-text"
                    style={{ fontSize: 10.5, color: 'var(--text-muted)', flexWrap: 'wrap', paddingLeft: 2 }}
                    data-testid="verify-case-run-record"
                  >
                    {latestTrajectory.evidence ? (
                      <>
                        <span title={`Session ${latestTrajectory.evidence.session_id}`}>
                          session {latestTrajectory.evidence.session_id.slice(0, 8)}
                        </span>
                        <span data-testid="verify-case-run-tick">
                          tick {latestTrajectory.evidence.tick}
                        </span>
                        {/* The model's own clock. Shown only when the record
                            carries it — a tick is not a time, and deriving one
                            from dt would be a guess dressed as provenance. */}
                        {typeof latestTrajectory.evidence.time_ms === 'number' ? (
                          <span
                            data-testid="verify-case-run-time"
                            title={`Simulated time ${latestTrajectory.evidence.time_ms} ms (model clock, not wall clock)`}
                          >
                            t = {formatSimTime(latestTrajectory.evidence.time_ms)}
                          </span>
                        ) : null}
                      </>
                    ) : latestTrajectory.evidence === null ? (
                      /* The server looked and there is genuinely none. */
                      <span data-testid="verify-case-run-record-absent">
                        no session or tick recorded for this run — it predates evidence capture
                      </span>
                    ) : (
                      /* The key never arrived, so the server is older than
                         this projection. Do NOT call the run legacy data on
                         that basis — the record may well carry a session and
                         tick that this server cannot report. */
                      <span
                        data-testid="verify-case-run-record-unreported"
                        title="The API response carried no evidence field at all — this server predates it. Rebuild and restart the backend to see the run's session and tick."
                      >
                        run provenance not reported by this server
                      </span>
                    )}
                    <span title={`Execution ${latestTrajectory.execution_id}`}>
                      run {latestTrajectory.execution_id.slice(0, 8)}
                    </span>
                    <span title={new Date(latestTrajectory.timestamp).toLocaleString()}>
                      {relativeAgePhrase(latestTrajectory.timestamp)}
                    </span>
                    {latestTrajectory.model_digest ? (
                      <span title={`Model digest ${latestTrajectory.model_digest}`}>
                        @ {latestTrajectory.model_digest.slice(0, 7)}
                      </span>
                    ) : null}
                  </div>
                </div>
              ) : enteredOnTrajectory ? (
                /* Opened from a trajectory row, but nothing is stored. Say
                   that plainly instead of letting the desk check stand in for
                   the run the reader came to see. */
                <div
                  className="flex items-center gap-3"
                  style={{ padding: '2px 0 8px', fontSize: 11.5, color: 'var(--text-muted)' }}
                  data-testid="verify-case-no-stored-run"
                >
                  no stored run for this case — the verdict below is a static desk check, not a trajectory result
                </div>
              ) : null}

              {/* The static desk check. When a recorded run exists it is
                  DEMOTED below that run and relabelled: a reader who opened
                  this case from a trajectory row must never read the desk
                  check as the verdict for the run they clicked (J5). It stays
                  visible — it answers a different question (does the authored
                  model check out as written?) and hiding it would lose that. */}
              <div
                className="flex items-center gap-3.5"
                style={
                  runLeads
                    ? {
                        padding: '6px 0 8px',
                        flexWrap: 'wrap',
                        borderTop: '1px dashed var(--border-hairline)',
                        opacity: 0.85,
                      }
                    : { padding: '2px 0 8px', flexWrap: 'wrap' }
                }
                data-testid="verify-case-static-line"
              >
                <LatestLineLabel>{runLeads ? 'static desk check' : 'static read'}</LatestLineLabel>
                {/* Calm form (P2): the geometry mark leads (null for a static
                    desk check — no record), then the bare coloured verdict. */}
                {mode ? <EvaluationModeBadge mode={mode} size="mark" testId="verify-case-mode" /> : null}
                <VerdictBadge
                  verdict={verdict}
                  size="bare"
                  showLabel
                  name={caseName}
                  reason={firstFailMessage(requirements)}
                  testId="verify-case-verdict"
                />
                {/* The display string earns its place only when it ADDS to the
                    chip ("FAIL (1/1 failed)"); a bare "INCONCLUSIVE" beside the
                    "? Inconclusive" chip says the same word twice (live-caught). */}
                {caseRow.display && caseRow.display.toLowerCase() !== verdict ? (
                  <span className="mono-text" style={{ fontSize: 11, color: 'var(--text-secondary)' }}>{caseRow.display}</span>
                ) : null}
                <ComputedEvidence mode={mode} modelDigest={modelDigest} navigateHref={caseElementId} />
              </div>

              {latestExternal ? (
                <div
                  className="flex items-center gap-3.5"
                  style={{ padding: '6px 0', borderTop: '1px dashed var(--border-hairline)', flexWrap: 'wrap' }}
                  data-testid="verify-case-latest-external"
                >
                  <LatestLineLabel>latest external</LatestLineLabel>
                  <EvaluationModeBadge
                    mode="external"
                    size="mark"
                    stale={latestExternal.matches_current_model === false}
                  />
                  <VerdictBadge
                    verdict={normalizeVerdict(latestExternal.verdict)}
                    size="bare"
                    showLabel
                    name={caseName}
                    testId="verify-case-latest-external-verdict"
                  />
                  {latestExternal.tool ? (
                    <span className="mono-text" style={{ fontSize: 10.5, color: 'var(--text-muted)' }}>
                      {latestExternal.tool}
                    </span>
                  ) : null}
                  <span className="mono-text" style={{ fontSize: 10.5, color: 'var(--text-muted)' }} title={new Date(latestExternal.timestamp).toLocaleString()}>
                    {relativeAgePhrase(latestExternal.timestamp)}
                  </span>
                  {latestExternal.matches_current_model === false ? (
                    <span
                      style={{ fontSize: 10.5, color: 'var(--severity-warning)' }}
                      title="Produced against an older model — the digest the producer claims it tested no longer matches the current model."
                      data-testid="verify-case-latest-external-stale"
                    >
                      ⚑ older model
                    </span>
                  ) : null}
                  {latestExternal.case_changed_since === true ? (
                    <span style={{ fontSize: 10.5, color: 'var(--severity-warning)' }} data-testid="verify-case-latest-external-changed">
                      Δ case edited since this run
                    </span>
                  ) : null}
                </div>
              ) : null}
            </>
          )}
        </section>

        {/* ── process register ────────────────────────────────────── */}
        <section style={{ margin: '14px 28px 20px' }} data-testid="verify-case-process-register">
          <RegisterLabel tone="secondary">
            process — approval, history &amp; attestations · recorded as people, append-only, never in source
          </RegisterLabel>
          {caseElementId ? <CaseApprovalBlock elementId={caseElementId} /> : null}
          <div style={{ marginTop: 6 }}>
            {/* Archive lanes key by case NAME (ArchivedVerdict.case_id is the
                name), while this row's case_id is an ElementId — filtering by
                the UUID matched nothing and the timeline rendered empty
                (live-caught). Pass both id families; unmatched ids are inert. */}
            <VerdictTimelinePanel
              caseIds={caseName !== caseId ? [caseName, caseId] : [caseId]}
              caseElementIds={caseElementId ? [caseElementId] : []}
              testId="verify-case-timeline"
            />
          </div>
          {mode === 'static' ? (
            <div style={{ marginTop: 6, fontSize: 10.5, color: 'var(--text-muted)', lineHeight: 1.5 }} data-testid="verify-case-static-history-note">
              the current verdict is a static desk check — static verdicts are
              not archived, so they carry no session and never appear in the
              process log above.
            </div>
          ) : null}
        </section>
      </div>

      {/* ── footer: source + esc-back ───────────────────────────────── */}
      <div
        className="flex items-center gap-3 shrink-0"
        style={{ height: 26, padding: '0 28px', borderTop: '1px solid var(--border-hairline)', fontSize: 11, color: 'var(--text-muted)' }}
        data-testid="verify-case-footer"
      >
        <div style={{ flex: 1 }} />
        <button
          type="button"
          data-testid="verify-case-back"
          onClick={onBack}
          style={{ display: 'inline-flex', alignItems: 'center', gap: 6, background: 'transparent', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: 11 }}
        >
          <span className="mono-text" style={{ color: 'var(--text-muted)' }}>esc</span>
          back to cases
        </button>
      </div>
    </div>
  );
}

// ── process register: case approval (P4 — the ONE approval sidecar) ──

/**
 * The case's definition lifecycle — the same `draft → in_review →
 * approved` (+ `rejected` off-ramp) requirements have, on the same
 * workflow sidecar, with the same stepper control. "Is the test case
 * complete?" is this stepper's question — distinct from every verdict;
 * a passing execution of an unapproved case reads qualified on coverage
 * surfaces (the ruled coverage principle).
 *
 * Own component so its hooks never race the parent's early returns.
 */
function CaseApprovalBlock({ elementId }: { elementId: string }) {
  const state = useWorkflowState(elementId);
  const setApproval = useSetApproval();
  const current = state.data?.approval?.[0] ?? 'draft';
  return (
    <div
      data-testid="verify-case-approval"
      style={{ marginTop: 6, padding: '6px 12px', border: '1px solid var(--border-hairline)', borderRadius: 6 }}
    >
      <ActorGate prompt="Approval transitions are signed">
        {(actor) => (
          <ApprovalStepper
            current={current}
            disabled={setApproval.isPending || state.data === undefined}
            onTransition={(to) => setApproval.mutate({ elementId, to, actor })}
          />
        )}
      </ActorGate>
    </div>
  );
}

// ── model register: recursive check occurrences ──────────────────────

/**
 * One check occurrence and its recursive subrequirement failure chain
 * (§4.1 `subrequirements`). Each level indents under the previous — the
 * whole-clause `ClauseFourReview` fails through a referenced obligation
 * three levels deep, and the chain is what makes the failure legible.
 */
function CheckOccurrence({
  requirement,
  depth,
}: {
  requirement: VerificationCaseRequirement;
  depth: number;
}) {
  const navigate = useNavigate();
  const verdict = normalizeVerdict(requirement.verdict);
  const name = requirement.requirement_name ?? requirement.requirement_id ?? 'requirement';
  const reqId = requirement.requirement_id;
  const elementId = requirement.requirement_element_id ?? requirement.element_id ?? null;
  const subs = requirement.subrequirements ?? [];
  const isTopCheck = depth === 0;

  return (
    <div data-testid={`verify-case-check-${name}`} data-check-depth={depth}>
      <div className="flex items-center gap-2.5" style={{ minHeight: 24, fontSize: 12 }}>
        {isTopCheck ? (
          <span className="mono-text" style={{ fontSize: 10.5, color: 'var(--text-muted)' }}>verify</span>
        ) : null}
        <RequirementLink
          name={name}
          reqId={reqId}
          elementId={elementId}
          onNavigate={(href) => navigate(href)}
        />
        {requirement.requirement_text ? (
          <span
            className="truncate"
            style={{ fontSize: 11.5, color: 'var(--text-secondary)', flex: 1, minWidth: 0 }}
            title={requirement.requirement_text}
          >
            {requirement.requirement_text}
          </span>
        ) : requirement.message ? (
          <span className="truncate" style={{ fontSize: 11, color: 'var(--text-muted)', flex: 1, minWidth: 0 }} title={requirement.message}>
            {requirement.message}
          </span>
        ) : (
          <span style={{ flex: 1 }} />
        )}
        {isTopCheck ? <span className="mono-text" style={{ fontSize: 10, fontStyle: 'italic', color: 'var(--text-muted)' }}>ƒ</span> : null}
        <VerdictBadge verdict={verdict} size="bare" name={name} reason={requirement.message ?? undefined} testId={`verify-case-check-verdict-${name}`} />
      </div>

      {/* Binding redefinitions declared on the check (`attribute limit = 5;`). */}
      {renderConstraints(requirement.constraints)}

      {/* The failure message that names the chain, then the chain itself. */}
      {(requirement.requirement_text && requirement.message) ? (
        <div style={{ marginLeft: 18, fontSize: 11, color: 'var(--verdict-inconclusive)', minHeight: 22, display: 'flex', alignItems: 'center' }}>
          {requirement.message}
        </div>
      ) : null}

      {subs.length > 0 ? (
        <div style={{ marginLeft: 18, borderLeft: '1px solid var(--border-hairline)', paddingLeft: 14 }} data-testid={`verify-case-subs-${name}`}>
          {subs.map((sub, i) => (
            <CheckOccurrence key={`${sub.requirement_id ?? sub.requirement_element_id ?? i}`} requirement={sub} depth={depth + 1} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function renderConstraints(constraints: unknown[] | undefined): ReactNode {
  if (!Array.isArray(constraints) || constraints.length === 0) return null;
  const lines = constraints
    .map((c) => (c && typeof c === 'object' ? (c as { expression?: string }).expression : undefined))
    .filter((e): e is string => typeof e === 'string' && e.length > 0);
  if (lines.length === 0) return null;
  return (
    <div style={{ marginLeft: 18 }} data-testid="verify-case-check-bindings">
      {lines.map((line, i) => (
        <div key={i} className="mono-text" style={{ fontSize: 11, color: 'var(--text-muted)', minHeight: 22, display: 'flex', alignItems: 'center' }}>
          {line}
        </div>
      ))}
    </div>
  );
}

/**
 * The verified requirement rendered as a link to its Requirements-workbench
 * row (§8). Navigates to `/requirements` keyed by the requirement's
 * ElementId (the join key === `RequirementRow.id`). Row-preselection off the
 * `?req=` param is a filed follow-up on the Requirements lane.
 */
function RequirementLink({
  name,
  reqId,
  elementId,
  onNavigate,
}: {
  name: string;
  reqId?: string;
  elementId: string | null;
  onNavigate: (href: string) => void;
}) {
  const label = reqId && reqId !== name ? `${name} · ${reqId}` : name;
  if (!elementId) {
    return <span className="mono-text" style={{ fontSize: 11.5, color: 'var(--text-secondary)' }}>{label}</span>;
  }
  return (
    <button
      type="button"
      data-testid={`verify-case-req-link-${name}`}
      data-requirement-element-id={elementId}
      onClick={() => onNavigate(`/requirements?req=${encodeURIComponent(elementId)}`)}
      title={`Open ${name} in the Requirements workbench`}
      className="mono-text"
      style={{
        fontSize: 11.5,
        color: 'var(--accent-fg)',
        background: 'transparent',
        border: 'none',
        padding: 0,
        cursor: 'pointer',
        textAlign: 'left',
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </button>
  );
}

// ── computed register: mode-appropriate evidence ─────────────────────

/**
 * The evidence line under the case verdict. Static desk checks have no
 * stored record (their evidence is the current model) — that's an honest
 * absence, not an error. Trajectory/external branches are guarded: the
 * static evaluate read never carries their evidence, so they render only if
 * that data is ever present.
 */
function ComputedEvidence({
  mode,
  modelDigest,
  navigateHref,
}: {
  mode: ReturnType<typeof normalizeEvaluationMode>;
  modelDigest?: string;
  navigateHref: string | null;
}) {
  void navigateHref;
  const digest7 = modelDigest ? modelDigest.slice(0, 7) : null;
  // Static is the only mode this read produces; render its honest evidence.
  if (mode === 'static' || mode == null) {
    return (
      <>
        <span style={{ fontSize: 11.5, color: 'var(--text-secondary)' }} data-testid="verify-case-evidence-static">
          computed against the current model
          {digest7 ? (
            <>
              {' '}
              <span className="mono-text" style={{ color: 'var(--text-muted)' }} title={modelDigest}>@ {digest7}</span>
            </>
          ) : null}
          {' '}· recomputed on every edit
        </span>
        <span style={{ flex: 1 }} />
        <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
          no stored evidence record — the evidence <em>is</em> the current model
        </span>
      </>
    );
  }
  // Non-static modes have no evidence on the static read — say so plainly.
  return (
    <span style={{ fontSize: 11, color: 'var(--text-muted)' }} data-testid="verify-case-evidence-other">
      evidence is recorded in this case’s history below
    </span>
  );
}

// ── small presentational bits ────────────────────────────────────────

function RunAffordance({
  testId,
  glyph,
  label,
  onClick,
  disabled,
  busy,
  title,
}: {
  testId: string;
  glyph: string;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  busy?: boolean;
  title: string;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      data-gated={disabled || undefined}
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="flex items-center gap-1.5"
      style={{
        fontSize: 12,
        color: disabled ? 'var(--text-muted)' : 'var(--text-primary)',
        background: 'var(--surface-raised)',
        border: '1px solid var(--border-strong)',
        borderRadius: 4,
        padding: '5px 12px',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.7 : 1,
      }}
    >
      <span className="mono-text" style={{ color: 'var(--text-muted)' }}>{glyph}</span>
      {busy ? `${label}…` : label}
    </button>
  );
}

/** The left-hand label of one computed-register line ("static read" /
 *  "latest run" / "latest external") — three lines, three questions. */
function LatestLineLabel({ children }: { children: ReactNode }) {
  return (
    <span style={{ fontSize: 10.5, color: 'var(--text-muted)', width: 96, flex: 'none' }}>
      {children}
    </span>
  );
}

function RegisterLabel({ children, tone }: { children: ReactNode; tone?: 'secondary' }) {
  return (
    <div
      className="flex items-center gap-2"
      style={{ minHeight: 24, fontSize: 10.5, color: tone === 'secondary' ? 'var(--text-secondary)' : 'var(--text-muted)' }}
    >
      {children}
    </div>
  );
}

function FootnoteLine({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center" style={{ minHeight: 24, fontSize: 10.5, color: 'var(--text-muted)' }}>
      {children}
    </div>
  );
}

function BareObjectiveTeaching() {
  return (
    <div style={{ padding: '10px 14px' }} data-testid="verify-case-bare-objective">
      <div style={{ fontSize: 11.5, color: 'var(--text-muted)', lineHeight: 1.55 }}>
        a bare objective verifies nothing — add{' '}
        <span className="mono-text" style={{ color: 'var(--text-secondary)' }}>verify &lt;req&gt;;</span>{' '}
        to bind requirements.
      </div>
    </div>
  );
}

/** First non-pass requirement message — the verdict tooltip's reason. */
function firstFailMessage(requirements: VerificationCaseRequirement[]): string | undefined {
  for (const r of requirements) {
    if (normalizeVerdict(r.verdict) !== 'pass' && r.message) return r.message;
  }
  return undefined;
}

// ── styles ───────────────────────────────────────────────────────────

const centeredStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  justifyContent: 'center',
  height: '100%',
  gap: 4,
  fontSize: 12,
  color: 'var(--text-muted)',
  textAlign: 'center',
  padding: 24,
};

const objectiveBoxStyle: CSSProperties = {
  border: '1px solid var(--border-hairline)',
  borderRadius: 8,
  background: 'var(--surface-panel)',
  overflow: 'hidden',
};

/**
 * Simulated time for display. Milliseconds below a second, seconds above —
 * "t = 5.00 s" reads as a model time, "t = 5001 ms" reads as a counter, and
 * the exact millisecond value is on the title attribute either way.
 */
export function formatSimTime(ms: number): string {
  if (!Number.isFinite(ms)) return '—';
  return Math.abs(ms) < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`;
}

// Dashed top+bottom edges — the computed register reads as "the tool's read".
const computedRegisterStyle: CSSProperties = {
  margin: '8px 28px 0',
  background: 'var(--surface-sunken)',
  borderTop: '1px dashed var(--border-strong)',
  borderBottom: '1px dashed var(--border-strong)',
};
