import { describe, it, expect } from 'vitest';
import { generateMarkdownReport } from '../generateMarkdownReport';
import { SAMPLE_INPUT } from './fixtures';

describe('generateMarkdownReport', () => {
  it('produces GFM with heading, verdict-count summary, and environment list', () => {
    const md = generateMarkdownReport(SAMPLE_INPUT);
    expect(md).toContain('# Espresso Production Cell — Verification Report');
    expect(md).toContain('**Overall:**');
    expect(md).toContain('1 pass · 1 fail · 0 inconclusive · 2 error');
    expect(md).toContain('## Environment');
    expect(md).toContain('- **Backend:** sysml-runtime 0.3.1');
    expect(md).toContain('- **Suite:** Espresso cell acceptance suite');
    expect(md).toContain('- **Seeds:** 42, 1729');
  });

  it('emits the case table with pipe-separated columns', () => {
    const md = generateMarkdownReport(SAMPLE_INPUT);
    expect(md).toContain('| Case | Verdict | Actual | Expected | Margin | Runtime |');
    expect(md).toContain('|---|---|---|---|---|---|');
    expect(md).toMatch(/Brew quality — station 1 \|[^|]+\|/);
    expect(md).toContain('Pressure safety');
  });

  it('includes a requirement-detail section for non-passing cases', () => {
    const md = generateMarkdownReport(SAMPLE_INPUT);
    expect(md).toContain('## Requirement detail');
    expect(md).toContain('Station reaches Ready within 5 s');
    expect(md).toContain('[Open in Run](/run?session=sess-001&tick=137&element=station1.readyTime)');
    // Error verdict without evidence renders the italic stub.
    expect(md).toContain('_no evidence_');
  });

  it('skips requirement detail when every case passes', () => {
    const passSummary = { pass: 1, fail: 0, inconclusive: 0, error: 0, overall: 'pass' as const };
    const md = generateMarkdownReport({
      ...SAMPLE_INPUT,
      verdicts: [{ id: 'r1', verdict: 'pass', actual: 1 }],
      cases: [{ id: 'c1', label: 'C1', requirementIds: ['r1'], summary: passSummary }],
      summary: passSummary,
    });
    expect(md).not.toContain('## Requirement detail');
  });

  it('renders the evaluation-mode provenance line when modes are present (B10, §2.1a(d))', () => {
    const md = generateMarkdownReport({
      ...SAMPLE_INPUT,
      provenance: { ...SAMPLE_INPUT.provenance, evaluationModes: ['trajectory'] },
    });
    expect(md).toContain('- **Evaluation:** trajectory');
  });

  it('whole-document snapshot (stable)', () => {
    const md = generateMarkdownReport(SAMPLE_INPUT);
    expect(md).toMatchSnapshot();
  });
});
