import { describe, it, expect } from 'vitest';
import {
  diagnosticTooltip,
  diagnosticsForNode,
  severityGlyph,
  verdictGlyph,
  verdictPillStyle,
  worstState,
} from '../overlay';
import type {
  DiagnosticOverlay,
  ElementDiagnostics,
  ElementOverlay,
  ElementVerdict,
} from '../viewmodel-types';

const verdict = (v: ElementVerdict['verdict']): ElementVerdict => ({ verdict: v, value: null });
const activeOverlay: ElementOverlay = { activity: 'active', value: null };
const diag = (severity: ElementDiagnostics['severity']): ElementDiagnostics => ({
  severity,
  items: [{ severity, message: 'm' }],
});

describe('verdict pill style (brief §3.5)', () => {
  it('pass/fail are solid hue pills with ✓/✗ glyphs', () => {
    expect(verdictPillStyle(verdict('Pass'))).toEqual({
      token: 'pass', glyph: '✓', solid: true, dashed: false, hatched: false,
    });
    expect(verdictPillStyle(verdict('Fail'))).toEqual({
      token: 'fail', glyph: '✗', solid: true, dashed: false, hatched: false,
    });
  });

  it('inconclusive is a dashed outline; error a hatched fill — the redundant non-colour encodings', () => {
    expect(verdictPillStyle(verdict('Inconclusive'))).toMatchObject({ token: 'inconclusive', dashed: true, solid: false });
    expect(verdictPillStyle(verdict('Error'))).toMatchObject({ token: 'error', hatched: true, solid: false });
  });

  it('no verdict → no pill', () => {
    expect(verdictPillStyle(null)).toBeNull();
    expect(verdictPillStyle({ verdict: null, value: 3 })).toBeNull();
  });

  it('error and inconclusive glyphs are distinct (⨯ vs ?)', () => {
    expect(verdictGlyph(verdict('Error'))).toBe('⨯');
    expect(verdictGlyph(verdict('Inconclusive'))).toBe('?');
  });
});

describe('diagnostics overlay join + badge', () => {
  const overlay: DiagnosticOverlay = {
    elements: {
      'e-1': {
        severity: 'error',
        items: [
          { severity: 'warning', message: 'suspicious unit', code: 'PH001' },
          { severity: 'error', message: 'type mismatch', code: 'S002' },
        ],
      },
    },
  };

  it('joins by element id; absent id → null', () => {
    expect(diagnosticsForNode(overlay, 'e-1')?.severity).toBe('error');
    expect(diagnosticsForNode(overlay, 'e-2')).toBeNull();
    expect(diagnosticsForNode(null, 'e-1')).toBeNull();
  });

  it('tooltip lists every diagnostic, one [code] message line each', () => {
    expect(diagnosticTooltip(overlay.elements['e-1'])).toBe(
      '[PH001] suspicious unit\n[S002] type mismatch',
    );
    expect(diagnosticTooltip({ severity: 'info', items: [{ severity: 'info', message: 'note' }] })).toBe('note');
  });

  it('severity glyphs are distinct (colour is never the only encoding)', () => {
    expect(severityGlyph('error')).toBe('✕');
    expect(severityGlyph('warning')).toBe('!');
    expect(severityGlyph('info')).toBe('i');
  });
});

describe('worst-state dot (brief §4 glyph LOD: "one worst-state dot")', () => {
  it('failing verdict outranks everything', () => {
    expect(worstState(activeOverlay, verdict('Fail'), diag('error'))).toBe('fail');
  });

  it('diagnostic error > warning > live-active', () => {
    expect(worstState(activeOverlay, verdict('Pass'), diag('error'))).toBe('error');
    expect(worstState(activeOverlay, null, diag('warning'))).toBe('warning');
    expect(worstState(activeOverlay, null, diag('info'))).toBe('active');
  });

  it('nothing dot-worthy → null (info alone does not earn a dot)', () => {
    expect(worstState(null, null, null)).toBeNull();
    expect(worstState({ activity: 'completed', value: null }, verdict('Pass'), diag('info'))).toBeNull();
  });
});
