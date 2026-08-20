/**
 * SensitivityResultsShell — right-side results panel for
 * /analyze/sensitivity (R7.4).
 *
 * Swaps the primary visualisation based on the requested method:
 *   - Morris → `<MorrisScatter>` (μ*-vs-σ scatter)
 *   - Sobol  → `<SobolBarChart>` (grouped S_i / S_Ti bars)
 *
 * Both methods also get a summary tornado-style ranked-importance list
 * alongside the primary chart, sorted by |μ*| (Morris) or S_Ti (Sobol).
 * That's the "tornado" viewpoint the task spec asks for — a focused
 * ranked bar list implemented inline rather than reusing
 * `<SweepTornadoViewer>` because the latter's input contract is over
 * `ChildDescriptor[]` and not a pre-computed indices list.
 */

import type {
  ChildDescriptor,
  SensitivityAnalyzeResult,
  SensitivityResult,
} from '@/engine/types';
import { MorrisScatter } from './MorrisScatter';
import { SobolBarChart } from './SobolBarChart';

export interface SensitivityResultsShellProps {
  /** Backend batch id once the runner has kicked off. */
  batchId: string | null;
  /** Child descriptors as they stream in (for progress + debugging). */
  children: ChildDescriptor[];
  /** Runner lifecycle state. */
  state:
    | 'idle'
    | 'creating'
    | 'running'
    | 'analyzing'
    | 'complete'
    | 'error';
  /** Analysis result (populated when `state === 'complete'`). */
  results: SensitivityAnalyzeResult | null;
  /** Latest error message, if any. */
  error?: string | null;
}

export function SensitivityResultsShell({
  batchId,
  children,
  state,
  results,
  error,
}: SensitivityResultsShellProps) {
  return (
    <div
      data-testid="sensitivity-results-shell"
      style={{
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
      {/* Header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '8px 12px',
          borderBottom: '1px solid var(--outline-variant)',
          fontSize: 12,
          color: 'var(--on-surface)',
          background: 'var(--surface-container-low)',
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
          analytics
        </span>
        <strong>Sensitivity</strong>
        <span style={{ color: 'var(--outline)' }}>·</span>
        <span data-testid="sensitivity-state" style={{ color: 'var(--outline)' }}>
          {describeState(state, children.length)}
        </span>
        {batchId && (
          <span
            className="mono-text"
            style={{ marginLeft: 'auto', fontSize: 10, color: 'var(--outline)' }}
          >
            batch {batchId.slice(0, 8)}
          </span>
        )}
      </div>

      {/* Error banner */}
      {error && (
        <div
          data-testid="sensitivity-error"
          style={{
            padding: '8px 12px',
            background: 'var(--verdict-fail)',
            color: 'var(--on-verdict)',
            fontSize: 12,
            borderBottom: '1px solid var(--outline-variant)',
          }}
        >
          {error}
        </div>
      )}

      {/* Body */}
      <div style={{ flex: 1, overflow: 'auto', padding: 16 }}>
        {state === 'idle' && (
          <EmptyCopy text="Configure parameters and click Run Analysis." />
        )}
        {(state === 'creating' || state === 'running') && (
          <EmptyCopy
            text={`Running ${children.length} children…`}
          />
        )}
        {state === 'analyzing' && (
          <EmptyCopy text="Computing sensitivity indices…" />
        )}
        {state === 'complete' && results && (
          <ResultsBody results={results} />
        )}
      </div>
    </div>
  );
}

function describeState(
  state: SensitivityResultsShellProps['state'],
  n: number,
): string {
  switch (state) {
    case 'idle':
      return 'idle';
    case 'creating':
      return 'creating batch…';
    case 'running':
      return `running (${n} children so far)`;
    case 'analyzing':
      return 'analyzing…';
    case 'complete':
      return 'complete';
    case 'error':
      return 'error';
  }
}

function EmptyCopy({ text }: { text: string }) {
  return (
    <div
      data-testid="sensitivity-empty-copy"
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        color: 'var(--outline)',
        fontSize: 12,
      }}
    >
      {text}
    </div>
  );
}

function ResultsBody({ results }: { results: SensitivityAnalyzeResult }) {
  const ranked = rankForSummary(results);

  return (
    <div
      data-testid="sensitivity-results-body"
      style={{ display: 'flex', flexDirection: 'column', gap: 24 }}
    >
      <section>
        <h3
          style={{
            margin: 0,
            marginBottom: 8,
            fontSize: 13,
            fontWeight: 600,
            color: 'var(--on-surface)',
          }}
        >
          {results.method === 'morris'
            ? 'μ-vs-σ scatter'
            : 'Variance decomposition'}
        </h3>
        {results.method === 'morris' ? (
          <MorrisScatter results={results.parameters} />
        ) : (
          <SobolBarChart results={results.parameters} />
        )}
      </section>

      <section>
        <h3
          style={{
            margin: 0,
            marginBottom: 8,
            fontSize: 13,
            fontWeight: 600,
            color: 'var(--on-surface)',
          }}
        >
          Ranked importance (tornado)
        </h3>
        <SensitivityTornado
          method={results.method}
          ranked={ranked}
        />
      </section>
    </div>
  );
}

// ── Tornado (ranked-importance bar list) ──────────────────────────

interface RankedRow {
  name: string;
  magnitude: number;
  display: string;
}

/** Sort parameters by |μ*| (Morris) or S_Ti (Sobol), descending. */
export function rankForSummary(
  results: SensitivityAnalyzeResult,
): RankedRow[] {
  const method = results.method;
  const rows = results.parameters.map<RankedRow>((r) => {
    if (method === 'morris') {
      const m = typeof r.mu === 'number' && Number.isFinite(r.mu) ? Math.abs(r.mu) : 0;
      return {
        name: r.name,
        magnitude: m,
        display: `μ*=${m.toFixed(3)} · σ=${
          typeof r.sigma === 'number' ? r.sigma.toFixed(3) : '—'
        }`,
      };
    }
    const st =
      typeof r.st === 'number' && Number.isFinite(r.st) ? Math.max(0, r.st) : 0;
    const s1 =
      typeof r.s1 === 'number' && Number.isFinite(r.s1) ? Math.max(0, r.s1) : 0;
    return {
      name: r.name,
      magnitude: st,
      display: `S_Ti=${st.toFixed(3)} · S_i=${s1.toFixed(3)}`,
    };
  });
  rows.sort((a, b) => b.magnitude - a.magnitude);
  return rows;
}

function SensitivityTornado({
  method,
  ranked,
}: {
  method: SensitivityAnalyzeResult['method'];
  ranked: RankedRow[];
}) {
  const max = ranked.reduce((acc, r) => Math.max(acc, r.magnitude), 0);
  if (ranked.length === 0) {
    return (
      <p
        data-testid="sensitivity-tornado-empty"
        style={{ margin: 0, color: 'var(--outline)', fontSize: 12 }}
      >
        No parameters to rank.
      </p>
    );
  }
  return (
    <ul
      data-testid="sensitivity-tornado"
      data-method={method}
      style={{
        listStyle: 'none',
        margin: 0,
        padding: 0,
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
      }}
    >
      {ranked.map((row) => {
        const pct = max > 0 ? (row.magnitude / max) * 100 : 0;
        return (
          <li
            key={row.name}
            data-testid={`sensitivity-tornado-row-${row.name}`}
            style={{
              display: 'grid',
              gridTemplateColumns: '140px 1fr 180px',
              alignItems: 'center',
              gap: 8,
              fontSize: 12,
              color: 'var(--on-surface)',
            }}
          >
            <span style={{ fontWeight: 500, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {row.name}
            </span>
            <div
              role="img"
              aria-label={`${row.name} magnitude ${row.magnitude.toFixed(3)}`}
              style={{
                height: 14,
                width: '100%',
                background: 'color-mix(in srgb, var(--outline-variant) 12%, transparent)',
                borderRadius: 4,
                overflow: 'hidden',
              }}
            >
              <div
                style={{
                  height: '100%',
                  width: `${Math.max(0, Math.min(100, pct))}%`,
                  borderRadius: 4,
                  background:
                    method === 'morris'
                      ? 'linear-gradient(90deg, var(--chart-series-2) 0%, var(--chart-series-3) 100%)'
                      : 'linear-gradient(90deg, var(--chart-series-4) 0%, var(--chart-series-3) 100%)',
                }}
              />
            </div>
            <span
              className="mono-text"
              style={{ fontSize: 11, color: 'var(--outline)' }}
            >
              {row.display}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
