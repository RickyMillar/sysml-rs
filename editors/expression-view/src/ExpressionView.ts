import katex from 'katex';
import type { ExpressionAstNode, ExpressionAstResult } from './types';
import { astToKatex } from './astToKatex';

export type RenderOptions = {
  /** Render in display mode (centered, larger). Default false (inline). */
  displayMode?: boolean;
  /** If true, swallow KaTeX errors and show red-highlighted source instead. */
  throwOnError?: boolean;
  /** Extra KaTeX options forwarded through. */
  macros?: Record<string, string>;
};

/**
 * Render an AST node (or full result) into the given host element.
 * Replaces the host's contents.
 *
 * Consumers that want the string can use {@link astToKatex} directly and
 * pass it to `katex.render` / `katex.renderToString`.
 */
export function renderExpression(
  host: HTMLElement,
  source: ExpressionAstNode | ExpressionAstResult,
  options: RenderOptions = {},
): void {
  const node = 'ast' in source ? source.ast : source;
  if (!node) {
    host.textContent = '';
    return;
  }

  const tex = astToKatex(node);
  katex.render(tex, host, {
    displayMode: options.displayMode ?? false,
    throwOnError: options.throwOnError ?? false,
    macros: options.macros,
  });
}

/** Convert an AST node to a KaTeX-rendered HTML string. */
export function renderExpressionToString(
  source: ExpressionAstNode | ExpressionAstResult,
  options: RenderOptions = {},
): string {
  const node = 'ast' in source ? source.ast : source;
  if (!node) return '';
  const tex = astToKatex(node);
  return katex.renderToString(tex, {
    displayMode: options.displayMode ?? false,
    throwOnError: options.throwOnError ?? false,
    macros: options.macros,
  });
}
