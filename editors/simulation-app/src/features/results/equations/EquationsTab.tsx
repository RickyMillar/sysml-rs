/**
 * EquationsTab — inspect expression-bearing model elements.
 *
 * Replaces the old compact equations card with a searchable workbench:
 * group by kind, select an expression, view rendered math/source/AST
 * symbols, and inspect latest live values for referenced variables.
 */

import { useEffect, useMemo, useState } from 'react';
import type { ExpressionAstNode, ExpressionAstResult } from '@sysml-rs/expression-view';
import { CardShell } from '@/components/cards/CardShell';
import { ExpressionViewReact } from '@/components/cards/ExpressionViewReact';
import { VerdictBadge } from '@/components/VerdictBadge';
import { EXPRESSION_VIEW_ENABLED } from '@/featureFlags';
import type { TimePoint } from '@/features/sessions/types';
import { useEvaluateExpression } from './useEvaluateExpression';

interface EquationsTabProps {
  results: ExpressionAstResult[];
  timeSeries: Record<string, TimePoint[]>;
  uri?: string | null;
  loading?: boolean;
  error?: string | null;
  selectedElementId?: string | null;
  expanded?: boolean;
  onHeaderClick?: () => void;
}

const GROUPS: Array<{ label: string; match: (kind: string) => boolean }> = [
  { label: 'Constraints', match: (k) => /Constraint/i.test(k) },
  { label: 'Calculations', match: (k) => /Calc/i.test(k) },
  { label: 'Requirements', match: (k) => /Requirement/i.test(k) },
  { label: 'Attributes', match: (k) => /Attribute/i.test(k) },
];

function groupFor(kind: string): string {
  for (const group of GROUPS) if (group.match(kind)) return group.label;
  return 'Other';
}

export function EquationsTab({ results, timeSeries, uri, loading, error, selectedElementId, expanded, onHeaderClick }: EquationsTabProps) {
  if (!EXPRESSION_VIEW_ENABLED()) return null;

  const [query, setQuery] = useState('');
  // Most of the ~900 expression-bearing elements in a real model are noise:
  // bare constants ("x = 40") and synthetic elements with hex-id names
  // ("71b9ccee"). Hide them by default so the list shows genuine equations;
  // the toggle restores the full set.
  const [hideNoise, setHideNoise] = useState(true);
  const renderable = useMemo(() => results.filter((r) => r.ast !== null), [results]);
  const meaningful = useMemo(() => renderable.filter(isMeaningfulEquation), [renderable]);
  const hiddenCount = renderable.length - meaningful.length;
  const base = hideNoise ? meaningful : renderable;
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return base;
    return base.filter((r) =>
      (r.element_name ?? '').toLowerCase().includes(q) ||
      r.element_kind.toLowerCase().includes(q) ||
      (r.source ?? '').toLowerCase().includes(q),
    );
  }, [query, base]);
  const [selectedId, setSelectedId] = useState<string | null>(selectedElementId ?? null);
  useEffect(() => {
    if (selectedElementId) setSelectedId(selectedElementId);
  }, [selectedElementId]);
  const selected = filtered.find((r) => r.element_id === selectedId) ?? filtered[0] ?? null;
  const grouped = useMemo(() => groupExpressions(filtered), [filtered]);

  return (
    <CardShell title="Equations" icon="function" accentColor="var(--text-secondary)" expanded={expanded} onHeaderClick={onHeaderClick}>
      {error ? (
        <div style={{ fontSize: 12, color: 'var(--error)' }}>Failed to load expression ASTs: {error}</div>
      ) : loading && renderable.length === 0 ? (
        <div style={{ fontSize: 12, color: 'var(--outline)' }}>Loading equations…</div>
      ) : renderable.length === 0 ? (
        <div data-testid="equations-empty" style={{ fontSize: 12, color: 'var(--outline)' }}>
          No rendered expressions available for this model.
        </div>
      ) : (
        <div data-testid="equations-tab" className="grid gap-2" style={{ gridTemplateColumns: 'minmax(260px, 0.8fr) minmax(360px, 1.2fr)', minHeight: 260 }}>
          <aside className="flex flex-col gap-2" style={{ minWidth: 0 }}>
            <input
              data-testid="equations-search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search equations…"
              style={inputStyle}
            />
            {hiddenCount > 0 && (
              <label
                data-testid="equations-hide-noise"
                style={{ fontSize: 11, color: 'var(--outline)', display: 'inline-flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}
              >
                <input
                  type="checkbox"
                  checked={hideNoise}
                  onChange={(e) => setHideNoise(e.target.checked)}
                />
                Hide constants &amp; unnamed ({hiddenCount})
              </label>
            )}
            {base.length === 0 && (
              <div data-testid="equations-all-hidden" style={{ fontSize: 11, color: 'var(--outline)' }}>
                All {renderable.length} expressions are constants or unnamed — untick above to show them.
              </div>
            )}
            <div className="flex flex-col gap-2" style={{ maxHeight: 520, overflow: 'auto' }}>
              {Array.from(grouped.entries()).map(([group, items]) => (
                <section key={group} className="flex flex-col gap-1">
                  <div style={groupHeaderStyle}>{group} ({items.length})</div>
                  {items.map((item) => (
                    <button
                      key={item.element_id}
                      type="button"
                      data-testid={`equation-select-${item.element_id}`}
                      onClick={() => setSelectedId(item.element_id)}
                      className="text-left rounded"
                      style={{
                        border: '1px solid var(--outline-variant)',
                        background: selected?.element_id === item.element_id ? 'var(--primary-container)' : 'var(--surface-container-low)',
                        color: selected?.element_id === item.element_id ? 'var(--on-primary-container)' : 'var(--on-surface)',
                        padding: '6px 8px',
                        cursor: 'pointer',
                      }}
                    >
                      <div style={{ fontSize: 11, fontWeight: 700 }}>{item.element_name ?? item.element_id.slice(0, 8)}</div>
                      <div className="mono-text" style={{ fontSize: 9, color: 'var(--outline)' }}>{item.element_kind}</div>
                    </button>
                  ))}
                </section>
              ))}
            </div>
          </aside>

          <main className="rounded-lg overflow-hidden" style={{ border: '1px solid var(--outline-variant)', background: 'var(--surface-container-low)' }}>
            {selected ? <EquationDetail result={selected} timeSeries={timeSeries} uri={uri ?? null} /> : null}
          </main>
        </div>
      )}
    </CardShell>
  );
}

function EquationDetail({ result, timeSeries, uri }: { result: ExpressionAstResult; timeSeries: Record<string, TimePoint[]>; uri: string | null }) {
  const symbols = useMemo(() => collectSymbols(result.ast), [result.ast]);
  const [overrides, setOverrides] = useState<Record<string, string>>({});
  const evaluate = useEvaluateExpression();
  const runEvaluate = () => {
    if (!uri) return;
    evaluate.mutate({ elementId: result.element_id, overrides });
  };
  return (
    <div data-testid="equation-detail" className="flex flex-col gap-3 p-3">
      <header className="flex items-start justify-between gap-3">
        <div>
          <div style={{ fontSize: 13, fontWeight: 800, color: 'var(--on-surface)' }}>{result.element_name ?? result.element_id}</div>
          <div className="mono-text" style={{ fontSize: 10, color: 'var(--outline)' }}>{result.element_kind} · {result.element_id}</div>
        </div>
      </header>

      <section className="rounded" style={panelStyle}>
        <div style={sectionTitleStyle}>Rendered equation</div>
        <div style={{ overflowX: 'auto', paddingTop: 6 }}>
          <ExpressionViewReact source={result} displayMode testId="equation-detail-rendered" />
        </div>
      </section>

      <section className="rounded" style={panelStyle}>
        <div style={sectionTitleStyle}>Source</div>
        <pre data-testid="equation-detail-source" style={preStyle}>{result.source ?? '—'}</pre>
      </section>

      <section className="rounded" style={panelStyle}>
        <div style={sectionTitleStyle}>Referenced symbols and latest values</div>
        {symbols.length === 0 ? (
          <div style={{ fontSize: 11, color: 'var(--outline)' }}>No named symbols discovered in this AST.</div>
        ) : (
          <div className="grid gap-1" style={{ gridTemplateColumns: '1fr auto auto' }}>
            {symbols.map((symbol) => {
              const exact = latestValue(timeSeries[symbol]);
              const suffix = exact === null ? latestValue(findBySuffix(timeSeries, symbol)) : null;
              const value = exact ?? suffix;
              return (
                <div key={symbol} className="contents">
                  <div className="mono-text" style={{ fontSize: 11 }}>{symbol}</div>
                  <div className="mono-text" style={{ fontSize: 11, color: value === null ? 'var(--outline)' : 'var(--on-surface)' }}>{value === null ? 'unresolved' : formatNumber(value)}</div>
                  <div style={{ fontSize: 10, color: 'var(--outline)' }}>{value === null ? '' : 'latest'}</div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section className="rounded" style={panelStyle}>
        <div className="flex items-center gap-2" style={{ marginBottom: 6 }}>
          <div style={sectionTitleStyle}>Evaluate with overrides</div>
          <button
            type="button"
            data-testid="equation-evaluate-run"
            onClick={runEvaluate}
            disabled={!uri || evaluate.isPending}
            style={buttonStyle}
          >
            {evaluate.isPending ? 'Evaluating…' : 'Evaluate'}
          </button>
        </div>
        {!uri ? (
          <div style={{ fontSize: 11, color: 'var(--outline)' }}>Start or select a model-backed session to evaluate this expression.</div>
        ) : symbols.length === 0 ? (
          <div style={{ fontSize: 11, color: 'var(--outline)' }}>This expression has no overridable symbols.</div>
        ) : (
          <div className="grid gap-1" style={{ gridTemplateColumns: '1fr 1fr' }}>
            {symbols.map((symbol) => (
              <label key={symbol} className="contents">
                <span className="mono-text" style={{ fontSize: 11, color: 'var(--on-surface-variant)' }}>{symbol}</span>
                <input
                  data-testid={`equation-override-${symbol}`}
                  value={overrides[symbol] ?? ''}
                  onChange={(e) => setOverrides((prev) => ({ ...prev, [symbol]: e.target.value }))}
                  placeholder="override value"
                  style={smallInputStyle}
                />
              </label>
            ))}
          </div>
        )}
        {evaluate.data && (
          <div data-testid="equation-evaluate-result" className="flex flex-col gap-2" style={{ marginTop: 10 }}>
            <div className="flex items-center gap-2">
              <VerdictBadge verdict={evaluate.data.verdict} size="compact" showLabel />
              <span className="mono-text" style={{ fontSize: 11 }}>{evaluate.data.display ?? formatUnknown(evaluate.data.value)}</span>
              {evaluate.data.value_type && <span style={{ fontSize: 10, color: 'var(--outline)' }}>{evaluate.data.value_type}</span>}
            </div>
            {evaluate.data.diagnostics && evaluate.data.diagnostics.length > 0 && (
              <div style={{ fontSize: 11, color: 'var(--error)' }}>{evaluate.data.diagnostics.join('; ')}</div>
            )}
            <details>
              <summary style={{ cursor: 'pointer', fontSize: 11, color: 'var(--outline)' }}>Evaluation context</summary>
              <pre style={preStyle}>{JSON.stringify(evaluate.data.context ?? {}, null, 2)}</pre>
            </details>
          </div>
        )}
        {evaluate.error && (
          <div style={{ marginTop: 8, fontSize: 11, color: 'var(--error)' }}>{String(evaluate.error)}</div>
        )}
      </section>

      <details>
        <summary style={{ cursor: 'pointer', fontSize: 11, color: 'var(--outline)' }}>AST JSON</summary>
        <pre data-testid="equation-detail-ast" style={preStyle}>{JSON.stringify(result.ast, null, 2)}</pre>
      </details>
    </div>
  );
}

function groupExpressions(items: ExpressionAstResult[]): Map<string, ExpressionAstResult[]> {
  const grouped = new Map<string, ExpressionAstResult[]>();
  for (const item of items) {
    const group = groupFor(item.element_kind);
    grouped.set(group, [...(grouped.get(group) ?? []), item]);
  }
  return grouped;
}

function collectSymbols(ast: ExpressionAstNode | null): string[] {
  const out = new Set<string>();
  const visit = (node: ExpressionAstNode | null) => {
    if (!node) return;
    if (node.name && !isLiteralKind(node.kind)) out.add(node.name);
    for (const child of node.children ?? []) visit(child);
  };
  visit(ast);
  return [...out].sort();
}

function isLiteralKind(kind: string): boolean {
  return /Literal|Number|Boolean|String/i.test(kind);
}

/** A name that is just a hex element id (e.g. "71b9ccee"), not a real name. */
const HEX_NAME_RE = /^[0-9a-f]{6,}$/i;

/**
 * Whether an expression is a genuine equation worth listing, vs. noise. A real
 * model carries ~900 expression-bearing elements; most are bare constants
 * ("x = 40" — AST root is a literal) or synthetic elements whose only "name"
 * is a hex id. Those drown out the actual calc/constraint equations.
 */
function isMeaningfulEquation(r: ExpressionAstResult): boolean {
  // Bare constant — the whole expression is a single literal.
  if (r.ast && isLiteralKind(r.ast.kind) && !(r.ast.children && r.ast.children.length > 0)) {
    return false;
  }
  // No human name — absent or a raw hex id.
  const name = (r.element_name ?? '').trim();
  if (!name || HEX_NAME_RE.test(name)) return false;
  return true;
}

function latestValue(points: TimePoint[] | undefined): number | null {
  if (!points || points.length === 0) return null;
  return points[points.length - 1]?.v ?? null;
}

function findBySuffix(timeSeries: Record<string, TimePoint[]>, symbol: string): TimePoint[] | undefined {
  return Object.entries(timeSeries).find(([name]) => name === symbol || name.endsWith(`.${symbol}`))?.[1];
}

function formatNumber(value: number): string {
  if (Math.abs(value) >= 1000 || Math.abs(value) < 0.001 && value !== 0) return value.toExponential(3);
  return Number(value.toPrecision(6)).toString();
}

function formatUnknown(value: unknown): string {
  if (value == null) return '—';
  if (typeof value === 'number') return formatNumber(value);
  if (typeof value === 'string' || typeof value === 'boolean') return String(value);
  return JSON.stringify(value);
}

const inputStyle = {
  background: 'var(--surface-container)',
  color: 'var(--on-surface)',
  border: '1px solid var(--outline-variant)',
  borderRadius: 4,
  padding: '6px 8px',
  fontSize: 11,
} as const;

const groupHeaderStyle = {
  fontSize: 9,
  color: 'var(--outline)',
  textTransform: 'uppercase' as const,
  letterSpacing: '0.06em',
};

const sectionTitleStyle = {
  fontSize: 10,
  color: 'var(--outline)',
  textTransform: 'uppercase' as const,
  letterSpacing: '0.06em',
};

const panelStyle = {
  border: '1px solid var(--outline-variant)',
  background: 'var(--surface-container)',
  padding: 10,
};

const smallInputStyle = {
  background: 'var(--surface-container-low)',
  color: 'var(--on-surface)',
  border: '1px solid var(--outline-variant)',
  borderRadius: 4,
  padding: '3px 6px',
  fontSize: 11,
} as const;

const buttonStyle = {
  marginLeft: 'auto',
  border: '1px solid var(--outline-variant)',
  background: 'var(--primary-container)',
  color: 'var(--on-primary-container)',
  borderRadius: 4,
  padding: '3px 8px',
  fontSize: 11,
  fontWeight: 700,
  cursor: 'pointer',
} as const;

const preStyle = {
  margin: 0,
  whiteSpace: 'pre-wrap' as const,
  fontSize: 11,
  color: 'var(--on-surface-variant)',
};
