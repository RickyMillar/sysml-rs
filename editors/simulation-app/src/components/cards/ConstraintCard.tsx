/**
 * ConstraintCard — 4-valued verdict pills for assert constraints.
 *
 * Each constraint renders as a `<VerdictBadge>` that flips in real-time.
 * The four states (Pass / Fail / Inconclusive / Error) come from the
 * backend `VerdictKind`; the legacy `pass: boolean` field is still
 * accepted for back-compat and is mapped to `pass` / `fail` here.
 *
 * Layout (R47): pills are GROUPED by verdict (failing/errored/inconclusive
 * first, passing last) so a run with many constraints reads as "what's
 * wrong right now" instead of an undifferentiated wall of red. A filter box
 * narrows by name or expression, and every pill carries a stable label —
 * constraints the backend leaves unnamed fall back to their expression (or a
 * positional id) rather than rendering as an anonymous "Fail".
 *
 * When the `expressionView` feature flag is enabled AND the caller supplies
 * an `expressionAsts` map keyed by constraint name, each pill renders the
 * constraint expression via `@sysml-rs/expression-view` (KaTeX). Missing
 * ASTs fall through to the existing plain-text renderer, and the flag-off
 * path is byte-identical to the pre-flag behaviour.
 */

import { useCallback, useMemo, useState } from 'react';
import type { ExpressionAstResult } from '@sysml-rs/expression-view';
import { CardShell } from './CardShell';
import type { ExportAction } from './CardShell';
import { ExpressionViewReact } from './ExpressionViewReact';
import { exportJSON } from '../../shared/export';
import { EXPRESSION_VIEW_ENABLED } from '../../featureFlags';
import { VerdictBadge, type VerdictKind } from '../VerdictBadge';

interface ConstraintInfo {
  name: string;
  expression: string;
  /**
   * Legacy 2-valued flag. Still accepted because the activity store
   * emits it today; prefer `verdict` when the backend surfaces the full
   * 4-valued kind.
   */
  pass: boolean;
  /**
   * 4-valued verdict from the backend. When present, wins over `pass`.
   */
  verdict?: VerdictKind;
  actualValue?: string;
  expectedValue?: string;
  /**
   * `metadata.error_reason` when `verdict === 'error'`, or a human
   * explanation when `verdict === 'inconclusive'`.
   */
  reason?: string;
}

interface ConstraintCardProps {
  constraints: ConstraintInfo[];
  running: boolean;
  /**
   * Optional map of constraint name -> expression AST (from
   * `sysml.expression.ast`). Only consumed when the feature flag is on.
   */
  expressionAsts?: Record<string, ExpressionAstResult>;
  expanded?: boolean;
  onHeaderClick?: () => void;
}

function resolveVerdict(c: ConstraintInfo): VerdictKind {
  if (c.verdict) return c.verdict;
  return c.pass ? 'pass' : 'fail';
}

/** Worst-first so failures surface at the top of the card. */
const VERDICT_ORDER: VerdictKind[] = ['fail', 'error', 'inconclusive', 'pass'];

const VERDICT_GROUP_LABEL: Record<VerdictKind, string> = {
  fail: 'Failing',
  error: 'Errored',
  inconclusive: 'Inconclusive',
  pass: 'Passing',
};

const VERDICT_COLOR: Record<VerdictKind, string> = {
  fail: 'var(--verdict-fail)',
  error: 'var(--verdict-error)',
  inconclusive: 'var(--verdict-inconclusive)',
  pass: 'var(--verdict-pass)',
};

/**
 * A display label for a constraint. The backend leaves many derived /
 * anonymous constraints unnamed; rather than render an anonymous "Fail"
 * pill, fall back to the (truncated) expression, then to a positional id.
 */
function constraintLabel(c: ConstraintInfo, index: number): string {
  const n = c.name?.trim();
  if (n) return n;
  const e = c.expression?.trim();
  if (e) return e.length > 48 ? `${e.slice(0, 47)}…` : e;
  return `constraint #${index + 1}`;
}

export function ConstraintCard({ constraints, running, expressionAsts, expanded, onHeaderClick }: ConstraintCardProps) {
  const [filter, setFilter] = useState('');
  const useExpressionView = EXPRESSION_VIEW_ENABLED();
  const hasData = constraints.length > 0;

  const resolved = useMemo(
    () =>
      constraints.map((c, idx) => ({
        c,
        v: resolveVerdict(c),
        label: constraintLabel(c, idx),
        idx,
      })),
    [constraints],
  );

  const passCount = resolved.filter((r) => r.v === 'pass').length;
  const failCount = resolved.filter((r) => r.v === 'fail').length;
  const inconclusiveCount = resolved.filter((r) => r.v === 'inconclusive').length;
  const errorCount = resolved.filter((r) => r.v === 'error').length;

  const query = filter.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!query) return resolved;
    return resolved.filter(
      (r) =>
        r.label.toLowerCase().includes(query) ||
        (r.c.expression ?? '').toLowerCase().includes(query),
    );
  }, [resolved, query]);

  const groups = useMemo(() => {
    const m = new Map<VerdictKind, typeof filtered>();
    for (const r of filtered) {
      const arr = m.get(r.v);
      if (arr) arr.push(r);
      else m.set(r.v, [r]);
    }
    for (const arr of m.values()) arr.sort((a, b) => a.label.localeCompare(b.label));
    return m;
  }, [filtered]);

  const handleExportJSON = useCallback(() => {
    const data = constraints.map((c) => ({
      name: c.name,
      expression: c.expression,
      pass: c.pass,
      verdict: c.verdict ?? null,
      actualValue: c.actualValue ?? null,
    }));
    exportJSON(data, 'constraints.json');
  }, [constraints]);

  const exportActions: ExportAction[] = hasData
    ? [{ label: 'Export JSON', icon: 'data_object', onClick: handleExportJSON }]
    : [];

  return (
    <CardShell title="Constraints" icon="rule" accentColor="var(--text-secondary)" expanded={expanded} onHeaderClick={onHeaderClick} exportActions={exportActions}>
      {!hasData ? (
        <div style={{ fontSize: 'var(--text-xs)', color: 'var(--outline)' }}>
          {running ? 'Waiting for constraint evaluation...' : 'No constraints evaluated yet.'}
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {/* Summary — 4-valued */}
          <div className="flex items-center gap-3" style={{ fontSize: 'var(--text-xs)' }}>
            <span style={{ color: 'var(--verdict-pass)' }}>{passCount} pass</span>
            {failCount > 0 && <span style={{ color: 'var(--verdict-fail)' }}>{failCount} fail</span>}
            {inconclusiveCount > 0 && <span style={{ color: 'var(--verdict-inconclusive)' }}>{inconclusiveCount} inconclusive</span>}
            {errorCount > 0 && <span style={{ color: 'var(--verdict-error)' }}>{errorCount} error</span>}
          </div>

          {/* Filter — narrows by name or expression. Only worth showing once
              there are enough constraints to be hard to scan by eye. */}
          {resolved.length > 6 && (
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter constraints by name or expression…"
              data-testid="constraint-filter"
              style={{
                background: 'var(--surface-container)',
                color: 'var(--on-surface)',
                border: '1px solid var(--outline-variant)',
                borderRadius: 4,
                padding: '3px 8px',
                fontSize: 11,
              }}
            />
          )}

          {/* Grouped pills — failing/errored/inconclusive first, passing last. */}
          {VERDICT_ORDER.map((v) => {
            const items = groups.get(v);
            if (!items || items.length === 0) return null;
            return (
              <div key={v} data-testid={`constraint-group-${v}`} className="flex flex-col gap-1">
                <div
                  className="mono-text"
                  style={{
                    fontSize: 10,
                    fontWeight: 700,
                    textTransform: 'uppercase',
                    letterSpacing: '0.04em',
                    color: VERDICT_COLOR[v],
                  }}
                >
                  {VERDICT_GROUP_LABEL[v]} ({items.length})
                </div>
                <div className="flex flex-wrap gap-1.5" data-testid="constraint-pills">
                  {items.map((r) => {
                    const { c, idx } = r;
                    const ast = useExpressionView ? expressionAsts?.[c.name] : undefined;
                    const hasRenderableAst = !!(ast && ast.ast);
                    return (
                      <div
                        key={idx}
                        data-testid={`constraint-pill-${idx}`}
                        data-constraint-name={c.name}
                        data-verdict={r.v}
                        className="flex items-center gap-1.5"
                      >
                        <VerdictBadge
                          verdict={r.v}
                          name={r.label}
                          actual={c.actualValue ?? null}
                          expected={c.expectedValue ?? null}
                          reason={c.reason ?? null}
                          testId={`constraint-verdict-${idx}`}
                        />
                        <span className="mono-text" style={{ fontSize: '9px', color: 'var(--on-surface)' }}>
                          {r.label}
                        </span>
                        {hasRenderableAst ? (
                          <ExpressionViewReact
                            source={ast}
                            testId={`constraint-expr-${idx}`}
                            className="katex-host"
                          />
                        ) : (
                          c.actualValue && (
                            <span className="mono-text" style={{ fontSize: '8px', color: 'var(--outline)' }}>
                              {c.actualValue}
                            </span>
                          )
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })}

          {filtered.length === 0 && (
            <div data-testid="constraint-filter-empty" style={{ fontSize: 'var(--text-xs)', color: 'var(--outline)' }}>
              No constraints match “{filter.trim()}”.
            </div>
          )}
        </div>
      )}
    </CardShell>
  );
}
