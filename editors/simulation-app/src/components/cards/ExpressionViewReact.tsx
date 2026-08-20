/**
 * Thin React wrapper around `@sysml-rs/expression-view`.
 *
 * The shared package is pure DOM (it calls `katex.render` into a host
 * element). Simulation-app is React, so we wrap the imperative renderer
 * in a ref-driven component so cards can drop it inline as JSX.
 *
 * Consumers pass either a full `ExpressionAstResult` (what the
 * `sysml.expression.ast` service command returns) or a bare
 * `ExpressionAstNode`. When the AST is null/undefined we render nothing
 * and callers are expected to fall back to plain text themselves.
 */
import { useEffect, useRef } from 'react';
import {
  renderExpression,
  type ExpressionAstNode,
  type ExpressionAstResult,
} from '@sysml-rs/expression-view';

interface ExpressionViewReactProps {
  source: ExpressionAstNode | ExpressionAstResult | null | undefined;
  displayMode?: boolean;
  /** Optional data attribute so tests can target a specific constraint. */
  testId?: string;
  className?: string;
}

export function ExpressionViewReact({
  source,
  displayMode = false,
  testId,
  className,
}: ExpressionViewReactProps) {
  const hostRef = useRef<HTMLSpanElement | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    if (!source) {
      host.textContent = '';
      return;
    }
    try {
      renderExpression(host, source, { displayMode });
    } catch (err) {
      // KaTeX errors should never take down the card — degrade to source text.
      // `renderExpression` already has throwOnError: false, but guard anyway.
      const fallback =
        (typeof source === 'object' && source !== null && 'source' in source
          ? (source as ExpressionAstResult).source
          : null) ?? '';
      host.textContent = fallback || String((err as Error).message ?? 'render error');
    }
  }, [source, displayMode]);

  return (
    <span
      ref={hostRef}
      className={className}
      data-testid={testId}
      data-expression-view="1"
    />
  );
}
