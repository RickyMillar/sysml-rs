import { useEffect, useMemo, useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { Link, useLocation } from 'react-router-dom';
import { httpPost } from '@/shared/api/http';
import { useWorkspaceStore, type Capabilities } from '@/store/workspace';
import { VerdictBadge, normalizeVerdict } from '@/components/VerdictBadge';
import { EvaluationModeBadge } from '@/components/EvaluationModeBadge';

/** One requirement line inside the objective verdict — the `VerifyResult`
 *  wire's `requirements` entries (shaped exactly like Verify's). */
export interface ObjectiveVerdictRequirement {
  requirement_id: string;
  requirement_text?: string | null;
  verdict: string;
  message?: string;
  margin?: number | null;
}

/**
 * The analysis case's own objective, judged against the computed result
 * (spec §7.23.2) — a real `VerifyResult`: verdict colours are ALLOWED on
 * it, and its mode is `static` (a desk check over the result; the solver
 * being iterative never makes it a trajectory — session-backed is the
 * only trajectory).
 */
export interface ObjectiveVerdict {
  verdict: string;
  evaluation_mode?: string;
  summary?: { pass: number; fail: number; inconclusive: number; error: number; overall: string };
  requirements?: ObjectiveVerdictRequirement[];
}

export interface AnalysisResult {
  case_name: string;
  tool_name?: string | null;
  input_parameters?: Array<Record<string, unknown>>;
  outputs?: Record<string, string>;
  converged?: boolean;
  iterations?: number | null;
  /** ABSENT when the case declares no verify'd objective — render
   *  nothing, never a null-state chip (design turn 2, 2b). */
  objective_verdict?: ObjectiveVerdict | null;
}

export interface AnalysisCaseSummary {
  element_id: string;
  case_name: string;
  display?: string;
  subject?: string | null;
  objective?: string | null;
  tool_name?: string | null;
  tool_uri?: string | null;
  parameters?: Array<Record<string, unknown>>;
  constraints?: Array<Record<string, unknown>>;
  result_expression?: string | null;
  diagnostics?: string[];
}

/** Stdlib base features the evaluate read leaks — vocabulary, never user
 *  cases (same wire bug the Verify lane filters; filed to fix on the wire).
 *  Without this the list led with "analysisCases"/"self" rows and the
 *  default selection landed on stdlib noise (visual QA, 2026-07-20). */
const STDLIB_ANALYSIS_CASE_NAMES = new Set([
  'analysisCases',
  'subAnalysisCases',
  'self',
  'AnalysisCase',
]);

async function fetchAnalysisCases(): Promise<AnalysisCaseSummary[]> {
  // `sysml.evaluate.analysis_cases` is workspace-scoped (scope-collapse W2
  // dropped its uri param): ONE call covers every loaded file. The rows
  // carry no source-file provenance — earlier versions called this once per
  // loaded uri and stamped each duplicate result set with the uri it was
  // "requested for", fabricating a per-file distinction the backend never
  // produced (N loaded files = every case listed N times).
  const rows = await httpPost<AnalysisCaseSummary[]>('/api/command', {
    command: 'sysml.evaluate.analysis_cases',
    params: {},
  });
  if (!Array.isArray(rows)) return [];
  return rows.filter((row) => !STDLIB_ANALYSIS_CASE_NAMES.has(row.case_name ?? ''));
}

export function AnalysisCasesLanding() {
  const loadedFiles = useWorkspaceStore((s) => s.loadedFiles);
  const loadedUris = useMemo(() => Array.from(loadedFiles.keys()), [loadedFiles]);
  const location = useLocation();
  const selectedId = new URLSearchParams(location.search).get('case_id');
  const query = useQuery({
    // loadedUris stays in the key purely for cache invalidation on
    // workspace load/reload — the fetch itself is one workspace call.
    queryKey: ['analysis-cases', loadedUris],
    enabled: loadedUris.length > 0,
    queryFn: fetchAnalysisCases,
  });
  const cases = query.data ?? [];
  const selected = useMemo(
    () => cases.find((item) => item.element_id === selectedId) ?? cases[0] ?? null,
    [cases, selectedId],
  );

  if (loadedUris.length === 0) {
    return <Centered icon="analytics" title="Load a workspace" detail="AnalysisCase definitions and usages will appear here after the workspace is loaded." />;
  }
  if (query.isLoading) {
    return <Centered icon="progress_activity" title="Discovering AnalysisCases…" detail="Reading compiled analysis metadata from loaded model files." />;
  }
  if (query.error) {
    return <Centered icon="error" title="Could not load AnalysisCases" detail={query.error instanceof Error ? query.error.message : String(query.error)} tone="error" />;
  }
  if (cases.length === 0) {
    return <AnalyzeGuidedLanding />;
  }

  return (
    <div data-testid="analysis-cases-landing" className="grid h-full min-h-0" style={{ gridTemplateColumns: 'minmax(280px, 0.8fr) minmax(420px, 1.2fr)', color: 'var(--on-surface)' }}>
      <aside className="min-h-0 overflow-auto p-3" style={{ borderRight: '1px solid var(--outline-variant)', background: 'var(--surface-container-low)' }}>
        <div style={eyebrow}>AnalysisCases</div>
        <div className="flex flex-col gap-2">
          {cases.map((item) => (
            <Link
              key={item.element_id}
              to={{ pathname: '/analyze', search: new URLSearchParams({ case_id: item.element_id }).toString() }}
              data-testid={`analysis-case-${item.element_id}`}
              className="rounded-lg"
              style={{
                textDecoration: 'none',
                border: '1px solid var(--outline-variant)',
                background: selected?.element_id === item.element_id ? 'var(--primary-container)' : 'var(--surface)',
                color: selected?.element_id === item.element_id ? 'var(--on-primary-container)' : 'var(--on-surface)',
                padding: 10,
              }}
            >
              <div style={{ fontSize: 12, fontWeight: 800 }}>{item.case_name}</div>
              {/* Selected cards are filled with `--primary-container`, so their
                  secondary lines must take the on-primary tier too — on the
                  surface tier they measured 1.31:1 against that fill. */}
              {item.subject && <div className="mono-text" style={{ fontSize: 10, color: selected?.element_id === item.element_id ? 'var(--on-primary-container)' : 'var(--outline)', overflowWrap: 'anywhere' }}>{item.subject}</div>}
              <div style={{ marginTop: 6, fontSize: 11, color: selected?.element_id === item.element_id ? 'var(--on-primary-container)' : 'var(--on-surface-variant)' }}>{item.display ?? 'Ready'}</div>
            </Link>
          ))}
        </div>
      </aside>
      <main className="min-h-0 overflow-auto p-3">
        {selected && <AnalysisCaseDetail item={selected} />}
      </main>
    </div>
  );
}

function AnalysisCaseDetail({ item }: { item: AnalysisCaseSummary }) {
  const parameters = item.parameters ?? [];
  const overridableParameters = parameters.filter((param) => parameterDirection(param) !== 'out');
  const [overrides, setOverrides] = useState<Record<string, string>>({});

  useEffect(() => {
    setOverrides(Object.fromEntries(overridableParameters.map((param) => [parameterName(param), parameterDefault(param)]).filter(([name]) => name)));
  }, [item.element_id]);

  const runAnalysis = useMutation({
    mutationFn: () => httpPost<AnalysisResult>('/api/command', {
      command: 'sysml.analysis.run',
      params: {
        case_name: item.case_name,
        overrides: Object.entries(overrides).filter(([, value]) => value.trim().length > 0),
      },
    }),
  });

  return (
    <div className="flex flex-col gap-3" data-testid="analysis-case-detail">
      <section className="rounded-lg" style={panelStyle}>
        <div className="flex items-start justify-between gap-3">
          <div>
            <div style={{ fontSize: 16, fontWeight: 850 }}>{item.case_name}</div>
            <div className="mono-text" style={{ fontSize: 10, color: 'var(--outline)' }}>{item.element_id}</div>
          </div>
          <button
            type="button"
            data-testid="analysis-case-run"
            onClick={() => runAnalysis.mutate()}
            disabled={runAnalysis.isPending}
            className="inline-flex items-center gap-1 rounded"
            style={{ border: '1px solid var(--outline-variant)', background: 'var(--primary-container)', color: 'var(--on-primary-container)', padding: '4px 8px', fontSize: 11, fontWeight: 800, cursor: runAnalysis.isPending ? 'not-allowed' : 'pointer' }}
          >
            <span className="material-symbols-outlined" style={{ fontSize: 13 }}>{runAnalysis.isPending ? 'progress_activity' : 'play_arrow'}</span>
            Run analysis
          </button>
        </div>
        {item.objective && <p style={{ fontSize: 12, lineHeight: 1.5, color: 'var(--on-surface-variant)' }}>{item.objective}</p>}
        {runAnalysis.error && <div style={{ marginTop: 8, fontSize: 11, color: 'var(--error)' }}>{runAnalysis.error instanceof Error ? runAnalysis.error.message : String(runAnalysis.error)}</div>}
      </section>

      <section className="rounded-lg" style={panelStyle}>
        <div style={eyebrow}>Execution metadata</div>
        <dl className="grid gap-1" style={{ gridTemplateColumns: 'auto 1fr', fontSize: 11 }}>
          <Dt label="Subject" value={item.subject ?? '—'} />
          <Dt label="Tool" value={item.tool_name ?? 'default solver'} />
          <Dt label="Tool URI" value={item.tool_uri ?? '—'} />
          <Dt label="Result" value={item.result_expression ?? '—'} />
        </dl>
      </section>

      <section className="rounded-lg" style={panelStyle}>
        <div style={eyebrow}>Parameters</div>
        <JsonList empty="No parameters declared." rows={parameters} />
        {overridableParameters.length > 0 && (
          <div className="flex flex-col gap-2" style={{ marginTop: 10 }} data-testid="analysis-parameter-overrides">
            <div style={{ fontSize: 10, color: 'var(--outline)', fontWeight: 800 }}>Run overrides</div>
            {overridableParameters.map((param) => {
              const name = parameterName(param);
              if (!name) return null;
              return (
                <label key={name} className="grid gap-1" style={{ fontSize: 11 }}>
                  <span className="mono-text" style={{ color: 'var(--outline)' }}>{name}</span>
                  <input
                    data-testid={`analysis-override-${name}`}
                    value={overrides[name] ?? ''}
                    onChange={(event) => setOverrides((current) => ({ ...current, [name]: event.target.value }))}
                    placeholder={parameterDefault(param) || 'override value'}
                    style={inputStyle}
                  />
                </label>
              );
            })}
          </div>
        )}
      </section>

      <section className="rounded-lg" style={panelStyle}>
        <div style={eyebrow}>Constraints</div>
        <JsonList empty="No constraints declared." rows={item.constraints ?? []} primary="expression" />
      </section>

      {runAnalysis.data && (
        <section className="rounded-lg" style={panelStyle} data-testid="analysis-run-result">
          <div style={eyebrow}>Run result</div>
          <ObjectiveVerdictSlot objective={runAnalysis.data.objective_verdict} />
          <dl className="grid gap-1" style={{ gridTemplateColumns: 'auto 1fr', fontSize: 11 }}>
            <Dt label="Converged" value={runAnalysis.data.converged === undefined ? '—' : String(runAnalysis.data.converged)} />
            <Dt label="Iterations" value={runAnalysis.data.iterations == null ? '—' : String(runAnalysis.data.iterations)} />
          </dl>
          <JsonList
            empty="No outputs returned."
            rows={Object.entries(runAnalysis.data.outputs ?? {}).map(([name, value]) => ({ name, value }))}
          />
        </section>
      )}

      {item.diagnostics && item.diagnostics.length > 0 && (
        <section className="rounded-lg" style={panelStyle}>
          <div style={eyebrow}>Diagnostics</div>
          {item.diagnostics.map((diag, index) => <div key={index} style={{ fontSize: 11, color: 'var(--error)' }}>{diag}</div>)}
        </section>
      )}
    </div>
  );
}

/**
 * The objective-verdict slot on the analysis result (design turn 2, 2b —
 * reconciled). Renders ONLY when the run carries one; a case whose
 * objective binds no `verify` is the common, honest shape (numbers are
 * the whole result) and gets no header, no empty chip, no placeholder.
 *
 * A failed objective does not fail the analysis — the computed result
 * stands beneath; the case's own bar was not met.
 */
function ObjectiveVerdictSlot({ objective }: { objective: ObjectiveVerdict | null | undefined }) {
  if (!objective) return null;
  const verdict = normalizeVerdict(objective.verdict);
  const summary = objective.summary;
  const display = summary
    ? verdict === 'pass'
      ? `PASS (${summary.pass}/${summary.pass + summary.fail + summary.inconclusive + summary.error})`
      : verdict === 'fail'
        ? `FAIL (${summary.fail}/${summary.pass + summary.fail + summary.inconclusive + summary.error} failed)`
        : verdict.toUpperCase()
    : null;
  const requirements = objective.requirements ?? [];
  return (
    <div
      data-testid="analysis-objective-verdict"
      style={{
        border: '1px dashed var(--outline-variant)',
        borderRadius: 8,
        marginBottom: 10,
        overflow: 'hidden',
      }}
    >
      <div
        className="flex items-center gap-2"
        style={{ minHeight: 22, padding: '2px 10px', fontSize: 10, color: 'var(--outline)' }}
      >
        <span className="mono-text" style={{ fontStyle: 'italic' }}>ƒ</span>
        objective verdict — the case’s own objective, judged against this result
      </div>
      {/* This strip sits on the primary-container fill, so its text takes
          `--on-primary-container`. On `--on-surface-variant` (a light neutral
          meant for the surface tier) the display chip measured 1.87:1 — the
          worst contrast anywhere in the app (finding 22). */}
      <div className="flex items-center gap-3" style={{ padding: '0 10px 8px', flexWrap: 'wrap' }}>
        <VerdictBadge verdict={verdict} size="standard" name="objective" testId="analysis-objective-verdict-badge" />
        {display ? (
          <span className="mono-text" style={{ fontSize: 11, color: 'var(--on-primary-container)' }}>{display}</span>
        ) : null}
        <EvaluationModeBadge mode="static" size="standard" testId="analysis-objective-mode" />
      </div>
      {requirements.length > 0 && (
        <div style={{ borderTop: '1px dashed var(--outline-variant)', padding: '4px 10px 8px' }}>
          {requirements.map((req) => (
            <div key={req.requirement_id} className="flex items-center gap-2" style={{ minHeight: 22, fontSize: 11 }}>
              <span className="mono-text" style={{ fontSize: 10, color: 'var(--outline)' }}>verify</span>
              <span className="mono-text" style={{ fontSize: 11 }}>{req.requirement_id}</span>
              {req.message ? (
                <span className="truncate" style={{ flex: 1, minWidth: 0, color: 'var(--on-surface-variant)' }} title={req.message}>
                  {req.message}
                </span>
              ) : (
                <span style={{ flex: 1 }} />
              )}
              <VerdictBadge verdict={normalizeVerdict(req.verdict)} size="compact" name={req.requirement_id} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * AnalyzeGuidedLanding — the guided activity surface shown when the loaded
 * workspace declares no AnalysisCases (Phase 5 residual: kind-availability
 * counts in the create-view-card posture).
 *
 * Two jobs, both honest:
 *  1. teach how AnalysisCases are authored (source is gospel — a snippet,
 *     never an in-app creation wizard), mirroring `RequirementsEmptyState`;
 *  2. route the user to the Analyze method that IS available right now,
 *     with per-method availability derived from the backend-owned
 *     `sysml.workspace.capabilities` profile (counts shown when the
 *     capability carries names; absence stated plainly, never faked).
 *
 * Design turn 2 (reconciled): the cards group under EVALUATES-OVER band
 * headers — TEACHING, never routing. The headers are static text (no
 * filter, no switcher, no navigation target); routes and availability are
 * byte-identical to the Phase-5 behavior. Straddling methods (Sweep,
 * Monte Carlo) appear ONCE, in the "the target decides" band, carrying
 * both mode badges — there is no pure-trajectory method, so the straddle
 * itself is the second band.
 */
function AnalyzeGuidedLanding() {
  const capabilities = useWorkspaceStore((s) => s.capabilities);
  const location = useLocation();
  const methods = methodAvailability(capabilities);
  const bands: Array<{
    key: string;
    header: string;
    teach: string;
    straddles: boolean;
    methods: MethodAvailability[];
  }> = [
    {
      key: 'static',
      header: 'OVER CURRENT VALUES',
      teach:
        'Every point is a desk check against the model as written — no session, no archive, nothing to point at but the values themselves.',
      straddles: false,
      methods: methods.filter((m) => m.evaluatesOver === 'static'),
    },
    {
      key: 'both',
      header: 'OVER VALUES OR RUNS — THE TARGET DECIDES',
      teach:
        'Sweeping a calculation is a static ensemble — many desk checks at different points. Sweeping an ODE or a batch of sessions is a trajectory ensemble — every point a real, archived run. The method is the same; what it evaluates over follows the target you point it at.',
      straddles: true,
      methods: methods.filter((m) => m.evaluatesOver === 'both'),
    },
  ];
  return (
    <div data-testid="analyze-guided-landing" className="h-full min-h-0 overflow-auto">
      <div style={{ maxWidth: 640, margin: '0 auto', padding: '40px 24px' }}>
        <div style={{ fontSize: 16, fontWeight: 700, color: 'var(--on-surface)' }}>
          This workspace declares no AnalysisCases yet
        </div>
        <p style={{ fontSize: 12, lineHeight: 1.6, color: 'var(--on-surface-variant)', marginTop: 8 }}>
          Analysis cases are authored in the source — declare one in any loaded
          <span className="mono-text"> .sysml </span>
          file and it appears here with its parameters, constraints, and a run affordance.
        </p>
        <pre
          style={{
            border: '1px dashed var(--outline-variant)',
            borderRadius: 8,
            padding: '12px 14px',
            marginTop: 12,
            fontSize: 11,
            lineHeight: 1.6,
            color: 'var(--on-surface-variant)',
            overflowX: 'auto',
          }}
        >
          {`analysis def ThermalMargin {
    subject board : Circuit;
    objective {
        doc /* Compute the worst-case junction temperature margin. */
    }
}
analysis thermal : ThermalMargin;`}
        </pre>

        <div style={{ ...eyebrow, marginTop: 24 }}>Available now in this workspace</div>
        {bands.map((band) => (
          <div key={band.key} data-testid={`analyze-evaluates-band-${band.key}`} style={{ marginTop: band.key === 'static' ? 0 : 18 }}>
            {/* Band header: static TEXT — teaching, never routing. */}
            <div className="flex items-baseline gap-2">
              <span className="mono-text" style={{ fontSize: 12, color: 'var(--outline)' }}>=</span>
              {band.straddles ? (
                <span className="mono-text" style={{ fontSize: 12, color: 'var(--sim-accent)' }}>∿</span>
              ) : null}
              <span style={{ fontSize: 10, letterSpacing: '0.06em', color: 'var(--outline)' }}>{band.header}</span>
            </div>
            <div style={{ fontSize: 10.5, lineHeight: 1.5, color: 'var(--outline)', margin: '2px 0 8px', maxWidth: 560 }}>
              {band.teach}
            </div>
            <div className="grid gap-2" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))' }}>
              {band.methods.map((m) => (
                <Link
                  key={m.id}
                  to={{ pathname: m.path, search: location.search }}
                  data-testid={`analyze-method-card-${m.id}`}
                  data-available={m.available}
                  className="rounded-lg"
                  style={{
                    textDecoration: 'none',
                    border: '1px solid var(--outline-variant)',
                    background: 'var(--surface-container)',
                    padding: 12,
                    opacity: m.available ? 1 : 0.65,
                  }}
                >
                  <div className="flex items-baseline gap-2">
                    <div style={{ fontSize: 12, fontWeight: 800, color: 'var(--on-surface)' }}>{m.label}</div>
                    <span style={{ flex: 1 }} />
                    <EvaluationModeBadge mode="static" size="compact" />
                    {m.evaluatesOver === 'both' ? <EvaluationModeBadge mode="trajectory" size="standard" /> : null}
                  </div>
                  <div style={{ fontSize: 11, lineHeight: 1.5, color: 'var(--on-surface-variant)', marginTop: 4 }}>{m.blurb}</div>
                  <div
                    className="mono-text"
                    style={{ fontSize: 10, marginTop: 8, color: m.available ? 'var(--primary)' : 'var(--outline)' }}
                  >
                    {m.availability}
                  </div>
                </Link>
              ))}
            </div>
          </div>
        ))}
        <div style={{ fontSize: 10, color: 'var(--outline)', marginTop: 14, lineHeight: 1.5 }}>
          the group headers teach what each method evaluates over — clicking a card opens the method
          exactly as before; there is no mode switcher.
        </div>
      </div>
    </div>
  );
}

interface MethodAvailability {
  id: string;
  path: string;
  label: string;
  blurb: string;
  /** Whether the workspace's content makes this method useful right now. */
  available: boolean;
  /** Honest availability line — counts when known, absence stated plainly. */
  availability: string;
  /** What the method evaluates over (ratified hybrid, turn 2): `static`
   *  = desk-check ensembles only; `both` = the target decides (a
   *  straddler — shown once, with both badges, never duplicated). */
  evaluatesOver: 'static' | 'both';
}

function methodAvailability(caps: Capabilities): MethodAvailability[] {
  const smCount = caps.stateMachineNames.length;
  const tradeCount = caps.tradeStudyNames.length;
  return [
    {
      id: 'sweep',
      path: '/analyze/sweep',
      label: 'Sweep',
      blurb: 'Step one parameter across a range and chart the response.',
      available: caps.hasOdeDynamics,
      availability: caps.hasOdeDynamics
        ? 'ODE dynamics detected'
        : 'No ODE dynamics in this workspace',
      evaluatesOver: 'both',
    },
    {
      id: 'montecarlo',
      path: '/analyze/montecarlo',
      label: 'Monte Carlo',
      blurb: 'Sample parameter distributions over repeated runs.',
      available: caps.hasOdeDynamics || smCount > 0,
      availability:
        smCount > 0
          ? `${smCount} state machine${smCount === 1 ? '' : 's'}${caps.hasOdeDynamics ? ' · ODE dynamics' : ''}`
          : caps.hasOdeDynamics
            ? 'ODE dynamics detected'
            : 'Nothing to simulate yet',
      evaluatesOver: 'both',
    },
    {
      id: 'trade-study',
      path: '/analyze/trade-study',
      label: 'Trade Study',
      blurb: 'Score alternatives against weighted criteria.',
      available: tradeCount > 0,
      availability:
        tradeCount > 0
          ? `${tradeCount} analysis case${tradeCount === 1 ? '' : 's'}`
          : 'No analysis cases declared',
      evaluatesOver: 'static',
    },
    {
      id: 'sensitivity',
      path: '/analyze/sensitivity',
      label: 'Sensitivity',
      blurb: 'Rank which parameters move the outputs most.',
      available: caps.hasConstraints,
      availability: caps.hasConstraints
        ? 'Constraints present'
        : 'No constraints in this workspace',
      evaluatesOver: 'static',
    },
  ];
}

function JsonList({ rows, empty, primary }: { rows: Array<Record<string, unknown>>; empty: string; primary?: string }) {
  if (rows.length === 0) return <div style={{ fontSize: 11, color: 'var(--outline)' }}>{empty}</div>;
  return (
    <div className="flex flex-col gap-2">
      {rows.map((row, index) => (
        <div key={index} className="rounded-md" style={{ border: '1px solid var(--outline-variant)', padding: 8 }}>
          {primary && typeof row[primary] === 'string' && <div className="mono-text" style={{ fontSize: 11, marginBottom: 4 }}>{String(row[primary])}</div>}
          <pre style={preStyle}>{JSON.stringify(row, null, 2)}</pre>
        </div>
      ))}
    </div>
  );
}

function Centered({ icon, title, detail, tone }: { icon: string; title: string; detail: string; tone?: 'error' }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-2" data-testid="analysis-cases-empty" style={{ color: tone === 'error' ? 'var(--error)' : 'var(--outline)' }}>
      <span className="material-symbols-outlined" style={{ fontSize: 34 }}>{icon}</span>
      <div style={{ fontSize: 14, fontWeight: 800 }}>{title}</div>
      <div style={{ fontSize: 11, maxWidth: 460, textAlign: 'center' }}>{detail}</div>
    </div>
  );
}

function Dt({ label, value }: { label: string; value: string }) {
  return <><dt style={{ color: 'var(--outline)' }}>{label}</dt><dd className="mono-text" style={{ margin: 0, overflowWrap: 'anywhere' }}>{value}</dd></>;
}

function parameterName(param: Record<string, unknown>): string {
  return typeof param.name === 'string' ? param.name : '';
}

function parameterDefault(param: Record<string, unknown>): string {
  const value = param.default_value ?? param.default ?? param.value;
  if (value == null) return '';
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') return String(value);
  return JSON.stringify(value);
}

function parameterDirection(param: Record<string, unknown>): string {
  return typeof param.direction === 'string' ? param.direction.toLowerCase() : 'in';
}

const panelStyle = { border: '1px solid var(--outline-variant)', background: 'var(--surface-container)', padding: 12 };
const eyebrow = { fontSize: 10, color: 'var(--outline)', textTransform: 'uppercase' as const, letterSpacing: '0.06em', fontWeight: 800, marginBottom: 8 };
const preStyle = { margin: 0, whiteSpace: 'pre-wrap' as const, fontSize: 10, color: 'var(--on-surface-variant)' };
const inputStyle = { width: '100%', border: '1px solid var(--outline-variant)', borderRadius: 6, background: 'var(--surface)', color: 'var(--on-surface)', padding: '6px 8px', fontSize: 11 };
