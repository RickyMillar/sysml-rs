import { describe, it, expect } from 'vitest';
import { renderExpressionToString, renderExpression } from '../ExpressionView';
const sampleResult = {
    element_id: 't1',
    element_name: 'tempBand',
    element_kind: 'ConstraintUsage',
    source: 'temp >= 90.0',
    ast: {
        kind: 'OperatorExpression',
        props: { operator: '>=' },
        children: [
            { kind: 'FeatureReferenceExpression', name: 'temp', props: {}, children: [] },
            { kind: 'LiteralRational', props: { value: 90.0 }, children: [] },
        ],
    },
};
describe('ExpressionView', () => {
    it('renderExpressionToString produces KaTeX HTML', () => {
        const html = renderExpressionToString(sampleResult);
        expect(html).toContain('katex');
        expect(html).toContain('≥'); // \geq rendered to entity
    });
    it('renderExpression writes into host element', () => {
        const host = document.createElement('div');
        renderExpression(host, sampleResult);
        expect(host.querySelector('.katex')).not.toBeNull();
        expect(host.textContent).toBeTruthy();
    });
    it('null ast renders empty', () => {
        const host = document.createElement('div');
        const nullResult = {
            element_id: 'n',
            element_name: null,
            element_kind: 'AttributeUsage',
            source: null,
            ast: null,
        };
        renderExpression(host, nullResult);
        expect(host.textContent).toBe('');
    });
});
//# sourceMappingURL=ExpressionView.test.js.map