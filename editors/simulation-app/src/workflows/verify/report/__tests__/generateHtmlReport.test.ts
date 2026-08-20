import { describe, it, expect } from 'vitest';
import { generateHtmlReport } from '../generateHtmlReport';
import { SAMPLE_INPUT } from './fixtures';

describe('generateHtmlReport — validity', () => {
  it('produces a well-formed standalone HTML document', () => {
    const { html } = generateHtmlReport(SAMPLE_INPUT);
    expect(html.startsWith('<!DOCTYPE html>')).toBe(true);
    expect(html).toContain('<main class="report">');
    expect(html).toContain('</main>');
    expect(html.trimEnd().endsWith('</html>')).toBe(true);
    // Must embed its own stylesheet + script, not reference externals.
    expect(html).toContain('<style>');
    expect(html).toContain('<script>');
    expect(html).not.toMatch(/<link[^>]+rel=["']stylesheet["']/);
  });

  it('emits a filename matching the required convention', () => {
    const { filename } = generateHtmlReport(SAMPLE_INPUT);
    expect(filename).toMatch(/^verify-([a-z0-9-]+)-\d{8}-\d{6}\.html$/);
    expect(filename).toBe('verify-espresso-production-cell-20260419-140732.html');
  });

  it('slugifies workspace names with special characters', () => {
    const { filename } = generateHtmlReport({
      ...SAMPLE_INPUT,
      workspaceName: 'Acme / Widgets (v2)',
    });
    expect(filename).toMatch(/^verify-acme-widgets-v2-\d{8}-\d{6}\.html$/);
  });
});

describe('generateHtmlReport — content snapshot', () => {
  it('includes overall verdict badge, counts, environment, and both cases', () => {
    const { html } = generateHtmlReport(SAMPLE_INPUT);
    expect(html).toContain('Espresso Production Cell — Verification Report');
    expect(html).toContain('Espresso cell acceptance suite');
    expect(html).toContain('sysml-runtime 0.3.1');
    expect(html).toContain('Brew quality — station 1');
    expect(html).toContain('Pressure safety');
    // Evidence link for the failing case points to /run with the session+tick.
    expect(html).toContain('/run?session=sess-001&amp;tick=137&amp;element=station1.readyTime');
    // Error case has no evidence — stub rendered instead.
    expect(html).toContain('no evidence attached');
  });

  it('emits verdict badges with the expected shape markers', () => {
    const { html } = generateHtmlReport(SAMPLE_INPUT);
    expect(html).toContain('data-verdict="pass"');
    expect(html).toContain('data-verdict="fail"');
    expect(html).toContain('data-verdict="error"');
    expect(html).toContain('data-verdict-shape="check"');
    expect(html).toContain('data-verdict-shape="cross"');
    expect(html).toContain('data-verdict-shape="triangle"');
  });

  it('renders the evaluation-mode provenance row when modes are present (B10, §2.1a(d))', () => {
    const { html } = generateHtmlReport({
      ...SAMPLE_INPUT,
      provenance: { ...SAMPLE_INPUT.provenance, evaluationModes: ['static', 'external'] },
    });
    expect(html).toContain('Evaluation');
    expect(html).toContain('static, external');
  });

  it('whole-document snapshot (stable)', () => {
    const { html } = generateHtmlReport(SAMPLE_INPUT);
    expect(html).toMatchSnapshot();
  });
});

describe('generateHtmlReport — edge cases', () => {
  it('handles an empty verdict list gracefully', () => {
    const { html } = generateHtmlReport({
      ...SAMPLE_INPUT,
      verdicts: [],
      cases: [],
      summary: { pass: 0, fail: 0, inconclusive: 0, error: 0, overall: 'pass' },
    });
    expect(html).toContain('<!DOCTYPE html>');
    // Empty rollup → pass (matches Rust `VerdictRollup::overall`).
    expect(html).toContain('data-overall="pass"');
  });

  it('groups verdicts whose id is not mapped into an ungrouped bucket', () => {
    const { html } = generateHtmlReport({
      ...SAMPLE_INPUT,
      cases: [], // no case mappings — every verdict falls through
    });
    expect(html).toContain('Ungrouped verdicts');
  });
});
