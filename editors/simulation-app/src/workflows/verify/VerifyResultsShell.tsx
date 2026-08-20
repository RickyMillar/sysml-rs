/**
 * VerifyResultsShell — inspectable verification result workbench.
 *
 * Shows the shared verdict matrix plus a detail pane for selected
 * verdicts: actual/expected/margin, evidence, reason, metadata, and raw
 * payload. This keeps Verify useful without requiring users to inspect
 * JSON/logs.
 */

import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { Verdict, VerdictKind } from '@/engine/types';
import { useEvaluateExpression } from '@/features/results/equations/useEvaluateExpression';
import { passFailGridViewer, __internals as passFailGridInternals } from '@/shared/viewers/PassFailGridViewer';

interface RequirementDetail {
  requirement_id?: string;
  requirement_name?: string;
  requirement_text?: string | null;
  verdict?: VerdictKind | string;
  actual?: unknown;
  expected?: unknown;
  margin?: number | null;
  error?: string | null;
  message?: string;
  constraints?: Array<{ expression?: string; satisfied?: boolean | null; actual?: unknown; expected?: unknown; margin?: number | null; error?: string | null }>;
}

export interface VerifyResultsShellProps {
  verdicts: Verdict[];
  isRunning?: boolean;
  error?: string | null;
}

export function VerifyResultsShell({ verdicts, isRunning = false, error = null }: VerifyResultsShellProps) {
  const navigate = useNavigate();
  const [selected, setSelected] = useState<Verdict | null>(null);
  const counts = useMemo(() => countVerdicts(verdicts), [verdicts]);
  const active = selected && verdicts.includes(selected) ? selected : firstInterestingVerdict(verdicts);

  if (error) return <CenteredState testId="verify-results-error" icon="error" title="Verification failed" detail={error} tone="error" />;
  if (isRunning) return <CenteredState testId="verify-results-running" icon="progress_activity" title="Running verification…" detail="Collecting verdicts from the selected scope." />;
  if (verdicts.length === 0) {
    return (
      <CenteredState
        testId="verify-results-empty"
        icon="fact_check"
        title="Select cases and click Run"
        detail="Verdict matrix and per-check details will appear here once a run completes."
      />
    );
  }

  return (
    <div data-testid="verify-results-workbench" className="flex flex-col h-full w-full overflow-hidden" style={{ color: 'var(--on-surface)' }}>
      <header className="flex items-center gap-2 px-3 py-2 shrink-0" style={{ borderBottom: '1px solid var(--outline-variant)', background: 'var(--surface-container-low)' }}>
        <span className="material-symbols-outlined" style={{ fontSize: 16, color: 'var(--primary)' }}>fact_check</span>
        <span style={{ fontSize: 12, fontWeight: 800 }}>Verification Results</span>
        <span className="mono-text" style={{ fontSize: 10, color: 'var(--outline)' }}>{verdicts.length} verdicts</span>
        <div className="flex items-center gap-1" style={{ marginLeft: 'auto' }}>
          {(['pass', 'fail', 'inconclusive', 'error'] as const).map((kind) => (
            <VerdictCount key={kind} kind={kind} count={counts[kind]} />
          ))}
        </div>
      </header>

      <div className="grid flex-1 min-h-0" style={{ gridTemplateColumns: 'minmax(420px, 1.2fr) minmax(320px, 0.8fr)' }}>
        <section className="min-h-0 overflow-auto p-3" data-testid="verify-results-matrix">
          {passFailGridViewer.render({ kind: 'pass-fail-grid', verdicts, onVerdictSelect: setSelected }, {})}
        </section>
        <aside className="min-h-0 overflow-auto p-3" style={{ borderLeft: '1px solid var(--outline-variant)', background: 'var(--surface-container-low)' }}>
          {active ? (
            <VerdictDetail
              verdict={active}
              onDrill={() => {
                const url = drillUrl(active);
                if (url) navigate(url);
              }}
            />
          ) : null}
        </aside>
      </div>
    </div>
  );
}

function VerdictDetail({ verdict, onDrill }: { verdict: Verdict; onDrill: () => void }) {
  const navigate = useNavigate();
  const evaluate = useEvaluateExpression();
  const hasEvidence = !!verdict.evidence?.session_id;
  const bareObjective = isBareObjective(verdict);
  const elementId = detailElementId(verdict);
  const expressionId = expressionElementId(verdict);
  const showInTreeUrl = elementId ? runUrl({ verdict, elementId }) : null;
  const equationUrl = expressionId ? runUrl({ verdict, elementId: expressionId, extra: { result_tab: 'equations', equation: expressionId } }) : null;
  // `sysml.evaluate.expression` is workspace-scoped and addresses the
  // expression by element id alone — an expression id is the whole gate.
  const canEvaluate = !!expressionId;
  return (
    <div data-testid="verify-verdict-detail" className="flex flex-col gap-3">
      <section className="rounded-lg" style={panelStyle}>
        <div className="flex items-start justify-between gap-2">
          <div>
            <div style={{ fontSize: 13, fontWeight: 800 }}>{verdict.label ?? verdict.id ?? passFailGridInternals.requirementOf(verdict)}</div>
            <div className="mono-text" style={{ fontSize: 10, color: 'var(--outline)' }}>{passFailGridInternals.caseNameOf(verdict)}</div>
            {declaredMethods(verdict).length > 0 && (
              <div
                data-testid="verify-verdict-method"
                className="mono-text"
                title="Declared verification method (@VerificationMethod on the case) — model intent, not how this verdict was computed"
                style={{ fontSize: 10, color: 'var(--on-surface-variant)', marginTop: 2 }}
              >
                declared: {declaredMethods(verdict).join(' · ')}
              </div>
            )}
          </div>
          {/* A bare objective mints no verdict (1e) — never a fabricated pill. */}
          {!bareObjective && <VerdictPill verdict={verdict.verdict} />}
        </div>
      </section>

      {bareObjective && (
        <section data-testid="verify-verdict-bare-objective" className="rounded-lg" style={panelStyle}>
          <div style={sectionTitleStyle}>Objective</div>
          <div style={{ fontSize: 11, color: 'var(--on-surface-variant)', lineHeight: 1.5 }}>
            A bare objective verifies nothing — add{' '}
            <span className="mono-text">verify &lt;req&gt;;</span> to bind requirements. No verdict is
            minted for a case with no checks.
          </div>
        </section>
      )}

      <section className="rounded-lg" style={panelStyle}>
        <div style={sectionTitleStyle}>Values</div>
        <dl className="grid gap-1" style={{ gridTemplateColumns: 'auto 1fr', fontSize: 11 }}>
          <Dt label="Actual" value={formatUnknown(verdict.actual)} />
          <Dt label="Expected" value={formatUnknown(verdict.expected)} />
          <Dt label="Margin" value={verdict.margin == null ? '—' : formatNumber(verdict.margin)} />
          <Dt label="Runtime" value={verdict.runtimeMs == null ? '—' : `${formatNumber(verdict.runtimeMs)} ms`} />
        </dl>
      </section>

      {(verdict.reason || metadataMessage(verdict)) && (
        <section className="rounded-lg" style={panelStyle}>
          <div style={sectionTitleStyle}>Reason</div>
          <div style={{ fontSize: 11, color: 'var(--on-surface-variant)', lineHeight: 1.5 }}>
            {verdict.reason ?? metadataMessage(verdict)}
          </div>
        </section>
      )}

      {requirementsOf(verdict).length > 0 && (
        <section className="rounded-lg" style={panelStyle}>
          <div style={sectionTitleStyle}>Requirements</div>
          <div className="flex flex-col gap-2">
            {requirementsOf(verdict).map((requirement, index) => (
              <div key={`${requirement.requirement_id ?? index}`} className="rounded-md" style={{ border: '1px solid var(--outline-variant)', padding: 8 }}>
                <div className="flex items-center gap-2">
                  <VerdictPill verdict={normalizeVerdict(requirement.verdict)} />
                  <span className="mono-text" style={{ fontSize: 11 }}>{requirement.requirement_name ?? requirement.requirement_id ?? `Requirement ${index + 1}`}</span>
                </div>
                {requirement.requirement_text && <div style={preStyle}>{requirement.requirement_text}</div>}
                {requirement.message && <div style={{ marginTop: 6, fontSize: 11, color: 'var(--on-surface-variant)' }}>{requirement.message}</div>}
                {Array.isArray(requirement.constraints) && requirement.constraints.length > 0 && (
                  <div className="flex flex-col gap-1" style={{ marginTop: 6 }}>
                    {requirement.constraints.map((constraint, constraintIndex) => (
                      <div key={constraintIndex} className="mono-text" style={{ fontSize: 10, color: constraint.satisfied === false ? 'var(--error)' : 'var(--outline)' }}>
                        {constraint.satisfied === false ? '✗' : '✓'} {constraint.expression ?? `constraint[${constraintIndex}]`}
                        {(constraint.actual !== undefined || constraint.expected !== undefined || typeof constraint.margin === 'number') && (
                          <span style={{ color: 'var(--outline)' }}>
                            {' '}actual={formatUnknown(constraint.actual)} expected={formatUnknown(constraint.expected)} margin={constraint.margin == null ? '—' : formatNumber(constraint.margin)}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        </section>
      )}

      {(showInTreeUrl || equationUrl || canEvaluate) && (
        <section className="rounded-lg" style={panelStyle}>
          <div style={sectionTitleStyle}>Actions</div>
          <div className="flex flex-wrap gap-2">
            {showInTreeUrl && (
              <button type="button" data-testid="verify-action-show-tree" onClick={() => navigate(showInTreeUrl)} style={buttonStyle}>
                <span className="material-symbols-outlined" style={{ fontSize: 13 }}>account_tree</span>
                Show in tree
              </button>
            )}
            {equationUrl && (
              <button type="button" data-testid="verify-action-open-equation" onClick={() => navigate(equationUrl)} style={buttonStyle}>
                <span className="material-symbols-outlined" style={{ fontSize: 13 }}>function</span>
                Open equation
              </button>
            )}
            {canEvaluate && (
              <button
                type="button"
                data-testid="verify-action-evaluate-expression"
                onClick={() => evaluate.mutate({ elementId: expressionId, overrides: {} })}
                disabled={evaluate.isPending}
                style={buttonStyle}
              >
                <span className="material-symbols-outlined" style={{ fontSize: 13 }}>{evaluate.isPending ? 'progress_activity' : 'play_arrow'}</span>
                Evaluate expression
              </button>
            )}
          </div>
          {evaluate.data && (
            <div data-testid="verify-action-evaluate-result" className="mono-text" style={{ marginTop: 8, fontSize: 11 }}>
              {evaluate.data.display ?? formatUnknown(evaluate.data.value)} · {evaluate.data.verdict}
            </div>
          )}
          {evaluate.error && <div style={{ marginTop: 8, fontSize: 11, color: 'var(--error)' }}>{String(evaluate.error)}</div>}
        </section>
      )}

      <section className="rounded-lg" style={panelStyle}>
        <div style={sectionTitleStyle}>Evidence</div>
        {hasEvidence ? (
          <div className="flex flex-col gap-2">
            <dl className="grid gap-1" style={{ gridTemplateColumns: 'auto 1fr', fontSize: 11 }}>
              <Dt label="Session" value={verdict.evidence!.session_id} />
              <Dt label="Tick" value={String(verdict.evidence!.tick)} />
              <Dt label="Element" value={verdict.evidence!.element_id ?? '—'} />
            </dl>
            <button type="button" data-testid="verify-verdict-drill" onClick={onDrill} style={buttonStyle}>
              <span className="material-symbols-outlined" style={{ fontSize: 13 }}>open_in_new</span>
              Drill to run evidence
            </button>
          </div>
        ) : (
          <div style={{ fontSize: 11, color: 'var(--outline)' }}>No runtime evidence was attached to this verdict.</div>
        )}
      </section>

      {verdict.sensitivity && Object.keys(verdict.sensitivity).length > 0 && (
        <section className="rounded-lg" style={panelStyle}>
          <div style={sectionTitleStyle}>Sensitivity</div>
          {Object.entries(verdict.sensitivity).map(([name, value]) => (
            <div key={name} className="flex justify-between gap-3 mono-text" style={{ fontSize: 11 }}>
              <span>{name}</span>
              <span>{formatNumber(value)}</span>
            </div>
          ))}
        </section>
      )}

      <details>
        <summary style={{ cursor: 'pointer', fontSize: 11, color: 'var(--outline)' }}>Raw verdict JSON</summary>
        <pre data-testid="verify-verdict-raw" style={preStyle}>{JSON.stringify(verdict, null, 2)}</pre>
      </details>
    </div>
  );
}

function CenteredState({ testId, icon, title, detail, tone }: { testId: string; icon: string; title: string; detail: string; tone?: 'error' }) {
  return (
    <div data-testid={testId} className="flex flex-col items-center justify-center h-full w-full gap-2" style={{ color: tone === 'error' ? 'var(--error)' : 'var(--outline)' }}>
      <span className="material-symbols-outlined" style={{ fontSize: 32, opacity: 0.85 }}>{icon}</span>
      <span style={{ fontSize: 13, fontWeight: 600 }}>{title}</span>
      <span style={{ fontSize: 11, maxWidth: 420, textAlign: 'center' }}>{detail}</span>
    </div>
  );
}

function VerdictCount({ kind, count }: { kind: VerdictKind; count: number }) {
  return <span className="mono-text" style={{ fontSize: 10, color: colorForVerdict(kind), border: '1px solid var(--outline-variant)', borderRadius: 999, padding: '2px 7px' }}>{kind}: {count}</span>;
}

function VerdictPill({ verdict }: { verdict: VerdictKind }) {
  return <span data-testid={`verify-verdict-pill-${verdict}`} className="mono-text" style={{ fontSize: 10, color: colorForVerdict(verdict), border: '1px solid var(--outline-variant)', borderRadius: 999, padding: '2px 7px' }}>{verdict}</span>;
}

function Dt({ label, value }: { label: string; value: string }) {
  return <><dt style={{ color: 'var(--outline)' }}>{label}</dt><dd className="mono-text" style={{ margin: 0, minWidth: 0, overflowWrap: 'anywhere' }}>{value}</dd></>;
}

function countVerdicts(verdicts: Verdict[]): Record<VerdictKind, number> {
  return verdicts.reduce<Record<VerdictKind, number>>((acc, verdict) => {
    acc[verdict.verdict] += 1;
    return acc;
  }, { pass: 0, fail: 0, inconclusive: 0, error: 0 });
}

function firstInterestingVerdict(verdicts: Verdict[]): Verdict | null {
  return verdicts.find((v) => v.verdict === 'fail' || v.verdict === 'error') ?? verdicts[0] ?? null;
}

function drillUrl(verdict: Verdict): string | null {
  if (!verdict.evidence?.session_id) return null;
  const params = new URLSearchParams({ session: verdict.evidence.session_id, tick: String(verdict.evidence.tick) });
  if (verdict.evidence.element_id) params.set('element', verdict.evidence.element_id);
  return `/run?${params.toString()}`;
}

function runUrl({ verdict, elementId, extra }: { verdict: Verdict; elementId: string; extra?: Record<string, string> }): string {
  const params = new URLSearchParams();
  if (verdict.evidence?.session_id) params.set('session', verdict.evidence.session_id);
  if (verdict.evidence?.tick != null) params.set('tick', String(verdict.evidence.tick));
  params.set('element', elementId);
  for (const [key, value] of Object.entries(extra ?? {})) params.set(key, value);
  return `/run?${params.toString()}`;
}

function detailElementId(verdict: Verdict): string | null {
  return metadataStringValue(verdict, 'element_id')
    ?? metadataStringValue(verdict, 'requirement_id')
    ?? verdict.evidence?.element_id
    ?? verdict.id
    ?? null;
}

function expressionElementId(verdict: Verdict): string | null {
  return metadataStringValue(verdict, 'constraint_id')
    ?? metadataStringValue(verdict, 'expression_id')
    ?? metadataStringValue(verdict, 'element_id')
    ?? verdict.evidence?.element_id
    ?? null;
}

function metadataStringValue(verdict: Verdict, key: string): string | null {
  const value = verdict.metadata?.[key];
  if (typeof value === 'string' && value.length > 0) return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return null;
}

/** DECLARED @VerificationMethod kinds off the case row (B4) — model
 *  intent; distinct from how the shown verdict was computed. */
function declaredMethods(verdict: Verdict): string[] {
  const value = verdict.metadata?.methods;
  if (!Array.isArray(value)) return [];
  return value.filter((v): v is string => typeof v === 'string' && v.length > 0);
}

function metadataMessage(verdict: Verdict): string | null {
  const message = verdict.metadata?.message ?? verdict.metadata?.error_reason ?? verdict.metadata?.reason;
  return typeof message === 'string' ? message : null;
}

/** A case whose objective binds no checks (`total_requirements === 0`).
 *  The wire returns Inconclusive; the FE mints no verdict (1e). */
function isBareObjective(verdict: Verdict): boolean {
  const total = verdict.metadata?.total_requirements;
  return typeof total === 'number' && total === 0;
}

function requirementsOf(verdict: Verdict): RequirementDetail[] {
  const requirements = verdict.metadata?.requirements;
  return Array.isArray(requirements) ? requirements.filter(isRequirementDetail) : [];
}

function isRequirementDetail(value: unknown): value is RequirementDetail {
  return !!value && typeof value === 'object';
}

function normalizeVerdict(value: unknown): VerdictKind {
  if (value === 'pass' || value === 'fail' || value === 'inconclusive' || value === 'error') return value;
  const lower = typeof value === 'string' ? value.toLowerCase() : '';
  if (lower === 'pass' || lower === 'fail' || lower === 'inconclusive' || lower === 'error') return lower;
  return 'inconclusive';
}

function formatUnknown(value: unknown): string {
  if (value == null) return '—';
  if (typeof value === 'number') return formatNumber(value);
  if (typeof value === 'string' || typeof value === 'boolean') return String(value);
  return JSON.stringify(value);
}

function formatNumber(value: number): string {
  if (!Number.isFinite(value)) return String(value);
  if (Math.abs(value) >= 1000 || Math.abs(value) < 0.001 && value !== 0) return value.toExponential(3);
  return Number(value.toPrecision(6)).toString();
}

function colorForVerdict(verdict: VerdictKind): string {
  switch (verdict) {
    case 'pass': return 'var(--verdict-pass)';
    case 'fail': return 'var(--verdict-fail)';
    case 'error': return 'var(--verdict-error)';
    case 'inconclusive': return 'var(--verdict-inconclusive)';
  }
}

const panelStyle = { border: '1px solid var(--outline-variant)', background: 'var(--surface-container)', padding: 10 } as const;
const sectionTitleStyle = { fontSize: 10, color: 'var(--outline)', textTransform: 'uppercase' as const, letterSpacing: '0.06em', marginBottom: 6 };
const preStyle = { marginTop: 8, whiteSpace: 'pre-wrap' as const, fontSize: 11, color: 'var(--on-surface-variant)' };
const buttonStyle = { display: 'inline-flex', alignItems: 'center', gap: 4, border: '1px solid var(--outline-variant)', background: 'var(--primary-container)', color: 'var(--on-primary-container)', borderRadius: 4, padding: '4px 8px', fontSize: 11, fontWeight: 700, cursor: 'pointer' } as const;
