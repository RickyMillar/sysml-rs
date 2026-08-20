/**
 * ConstraintDetail — rendered KaTeX expression + live verdict for a
 * focused constraint.
 *
 * Round 2 Task #144. Reuses:
 *   - `useExpressionAst(uri)` to pull the full AST bundle for the
 *     URI (cached 5 min by react-query — constraint ASTs only
 *     change when the source reloads).
 *   - `ExpressionViewReact` to render the AST via KaTeX.
 *   - `VerdictBadge` for the current tick's verdict.
 *
 * The live-operand overlay (identifier → current value inline in the
 * rendered math) is tracked as a follow-up because it needs the
 * backend to emit operand values alongside the verdict
 * (GAP-CONSTR-002). Until then we show the rendered expression and
 * the verdict side-by-side.
 */
import { useMemo } from 'react';
import { ExpressionViewReact } from '@/components/cards/ExpressionViewReact';
import { VerdictBadge } from '@/components/VerdictBadge';
import { useExpressionAst } from '@/features/results/useExpressionAst';
import type { ConstraintTreeNode } from '../types';
import { DetailMeta, DetailShell } from './common';

export function ConstraintDetail({
  node,
  testIdPrefix,
}: {
  node: ConstraintTreeNode;
  testIdPrefix: string;
}) {
  // The `sysml.expression.ast` command returns an array keyed by
  // element id; find the entry that matches this constraint node.
  const { data: asts, isLoading } = useExpressionAst(node.uri);
  const matchingAst = useMemo(
    // Match against the underlying element id — `node.id` may be a
    // dedupe-rewritten tree-position key when the constraint is
    // surfaced via typed-def inlining.
    () => (asts ?? []).find((r) => r.element_id === node.elementId) ?? null,
    [asts, node.elementId],
  );

  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="constraint">
      <DetailMeta node={node} />

      <div className="flex items-center gap-2">
        <span
          style={{
            fontSize: 14,
            fontWeight: 600,
            color: 'var(--on-surface)',
          }}
          data-testid={`${testIdPrefix}-constraint-name`}
        >
          {node.name}
        </span>
        {node.verdict && (
          <VerdictBadge
            verdict={node.verdict}
            name={node.name}
            size="compact"
            testId={`${testIdPrefix}-constraint-verdict`}
          />
        )}
      </div>

      <ExpressionBlock
        ast={matchingAst}
        isLoading={isLoading}
        fallbackSource={node.expression ?? matchingAst?.source ?? null}
        testIdPrefix={testIdPrefix}
      />

      <OperandOverlay
        operands={node.operands}
        verdict={node.verdict}
        testIdPrefix={testIdPrefix}
      />
    </DetailShell>
  );
}

/**
 * Live-operand overlay (GAP-CONSTR-002).
 *
 * Note on scope: the plan wanted `[value]` badges decorating KaTeX
 * identifier glyphs inline. That requires mutating the opaque KaTeX
 * DOM (`@sysml-rs/expression-view` renders imperatively via
 * `katex.render`), which would be fragile to KaTeX version bumps. We
 * ship the live values as a compact below-expression table instead —
 * same information, robust to KaTeX internals. Inline decoration
 * remains a natural follow-up once `@sysml-rs/expression-view` grows
 * a decorator hook.
 */
function OperandOverlay({
  operands,
  verdict,
  testIdPrefix,
}: {
  operands: Readonly<Record<string, number>> | undefined;
  verdict: ConstraintTreeNode['verdict'];
  testIdPrefix: string;
}) {
  const entries = useMemo(() => {
    if (!operands) return [];
    return Object.entries(operands).sort(([a], [b]) => a.localeCompare(b));
  }, [operands]);

  if (!operands) {
    return (
      <div
        data-testid={`${testIdPrefix}-constraint-operands-pending`}
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          fontStyle: 'italic',
        }}
      >
        Live operand values arrive once the constraint is evaluated.
      </div>
    );
  }

  if (entries.length === 0) {
    return (
      <div
        data-testid={`${testIdPrefix}-constraint-operands-empty`}
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          fontStyle: 'italic',
        }}
      >
        No numeric operands to overlay.
      </div>
    );
  }

  return (
    <div data-testid={`${testIdPrefix}-constraint-operands`}>
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--outline)',
          marginBottom: 4,
        }}
      >
        Live operand values
        {verdict ? ` · at ${verdict}` : ''}
      </div>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'auto 1fr',
          columnGap: 12,
          rowGap: 3,
          fontSize: 11,
          fontFamily: 'var(--font-mono, ui-monospace)',
        }}
      >
        {entries.map(([name, value]) => (
          <OperandRow
            key={name}
            name={name}
            value={value}
            testId={`${testIdPrefix}-constraint-operand-${name}`}
          />
        ))}
      </div>
    </div>
  );
}

function OperandRow({
  name,
  value,
  testId,
}: {
  name: string;
  value: number;
  testId: string;
}) {
  return (
    <>
      <span
        data-testid={testId}
        style={{
          color: 'var(--on-surface-variant)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
        title={name}
      >
        {name}
      </span>
      <span
        data-testid={`${testId}-value`}
        style={{
          color: 'var(--on-surface)',
          fontVariantNumeric: 'tabular-nums',
          textAlign: 'right',
          fontWeight: 600,
        }}
      >
        {formatOperand(value)}
      </span>
    </>
  );
}

function formatOperand(v: number): string {
  if (!Number.isFinite(v)) return '—';
  if (v === 0) return '0';
  const abs = Math.abs(v);
  if (abs < 1e-3 || abs >= 1e6) return v.toExponential(3);
  return v.toPrecision(5).replace(/\.?0+$/, '');
}

/**
 * AST present → KaTeX.
 * AST absent but we have raw source text (from ConstraintView.expression
 * or ExpressionAstResult.source) → render the source in a monospace
 * code block.
 * Neither → explain that the backend didn't retain the expression.
 */
function ExpressionBlock({
  ast,
  isLoading,
  fallbackSource,
  testIdPrefix,
}: {
  ast: { ast: unknown; source: string | null } | null;
  isLoading: boolean;
  fallbackSource: string | null;
  testIdPrefix: string;
}) {
  if (isLoading) {
    return (
      <div
        data-testid={`${testIdPrefix}-constraint-expression-loading`}
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          padding: 8,
          border: '1px dashed var(--outline-variant)',
          borderRadius: 4,
        }}
      >
        Loading expression…
      </div>
    );
  }
  if (ast && ast.ast) {
    return (
      <div
        data-testid={`${testIdPrefix}-constraint-expression-katex`}
        style={{
          padding: '10px 12px',
          background: 'var(--surface-container)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          minHeight: 36,
        }}
      >
        <ExpressionViewReact
          source={ast as Parameters<typeof ExpressionViewReact>[0]['source']}
          displayMode
        />
      </div>
    );
  }
  if (fallbackSource) {
    return (
      <div
        data-testid={`${testIdPrefix}-constraint-expression-source`}
        className="mono-text"
        style={{
          padding: 8,
          borderRadius: 4,
          background: 'var(--surface-container)',
          border: '1px solid var(--outline-variant)',
          fontSize: 11,
          color: 'var(--on-surface-variant)',
          whiteSpace: 'pre-wrap',
        }}
      >
        {fallbackSource}
      </div>
    );
  }
  return (
    <div
      data-testid={`${testIdPrefix}-constraint-expression-absent`}
      style={{
        fontSize: 11,
        color: 'var(--outline)',
        padding: 8,
        border: '1px dashed var(--outline-variant)',
        borderRadius: 4,
      }}
    >
      No expression text available for this constraint.
    </div>
  );
}
