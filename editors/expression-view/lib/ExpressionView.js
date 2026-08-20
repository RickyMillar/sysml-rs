import katex from 'katex';
import { astToKatex } from './astToKatex';
/**
 * Render an AST node (or full result) into the given host element.
 * Replaces the host's contents.
 *
 * Consumers that want the string can use {@link astToKatex} directly and
 * pass it to `katex.render` / `katex.renderToString`.
 */
export function renderExpression(host, source, options = {}) {
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
export function renderExpressionToString(source, options = {}) {
    const node = 'ast' in source ? source.ast : source;
    if (!node)
        return '';
    const tex = astToKatex(node);
    return katex.renderToString(tex, {
        displayMode: options.displayMode ?? false,
        throwOnError: options.throwOnError ?? false,
        macros: options.macros,
    });
}
//# sourceMappingURL=ExpressionView.js.map