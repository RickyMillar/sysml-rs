import type { ExpressionAstNode, ExpressionAstResult } from './types';
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
export declare function renderExpression(host: HTMLElement, source: ExpressionAstNode | ExpressionAstResult, options?: RenderOptions): void;
/** Convert an AST node to a KaTeX-rendered HTML string. */
export declare function renderExpressionToString(source: ExpressionAstNode | ExpressionAstResult, options?: RenderOptions): string;
//# sourceMappingURL=ExpressionView.d.ts.map