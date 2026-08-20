/**
 * Unit tests for verdictMapping — raw backend payloads → Verdict.
 *
 * Covers the edge cases listed in the R3.2 brief:
 *   - missing reason
 *   - both actual + expected
 *   - neither actual nor expected
 * plus the three shape families (constraint, verification-case,
 * verify-per-case) and the summary helper.
 */

import { describe, expect, it } from 'vitest';
import {
  emptyVerdict,
  expandRequirementVerdicts,
  mapConstraintResult,
  mapEvaluateVerificationCaseResult,
  mapVerifyResult,
  normalizeVerdictKind,
  summarize,
} from '../verdictMapping';

describe('normalizeVerdictKind', () => {
  it('accepts lower-case spec strings', () => {
    expect(normalizeVerdictKind('pass')).toBe('pass');
    expect(normalizeVerdictKind('fail')).toBe('fail');
    expect(normalizeVerdictKind('inconclusive')).toBe('inconclusive');
    expect(normalizeVerdictKind('error')).toBe('error');
  });

  it('accepts Rust debug capitalisation', () => {
    expect(normalizeVerdictKind('Pass')).toBe('pass');
    expect(normalizeVerdictKind('FAIL')).toBe('fail');
    expect(normalizeVerdictKind('Error')).toBe('error');
  });

  it('falls back to inconclusive for unknown / missing values', () => {
    expect(normalizeVerdictKind(null)).toBe('inconclusive');
    expect(normalizeVerdictKind(undefined)).toBe('inconclusive');
    expect(normalizeVerdictKind('bogus')).toBe('inconclusive');
    expect(normalizeVerdictKind(42)).toBe('inconclusive');
  });
});

describe('emptyVerdict', () => {
  it('zeroes every optional field to null / empty', () => {
    const v = emptyVerdict('pass');
    expect(v.verdict).toBe('pass');
    expect(v.actual).toBeNull();
    expect(v.expected).toBeNull();
    expect(v.margin).toBeNull();
    expect(v.sensitivity).toBeNull();
    expect(v.evidence).toBeNull();
    expect(v.metadata).toEqual({});
  });
});

describe('mapConstraintResult', () => {
  it('maps satisfied=true → pass and carries detail as reason', () => {
    const v = mapConstraintResult({
      element_id: 'c1',
      satisfied: true,
      detail: '= 3.14',
    });
    expect(v.verdict).toBe('pass');
    expect(v.metadata!.element_id).toBe('c1');
    expect(v.metadata!.reason).toBe('= 3.14');
    expect(v.metadata!.source).toBe('constraint');
    // evidence always None in R3.2
    expect(v.evidence).toBeNull();
  });

  it('maps satisfied=false → fail', () => {
    const v = mapConstraintResult({
      element_id: 'c2',
      satisfied: false,
      detail: 'x > 10 failed',
    });
    expect(v.verdict).toBe('fail');
    expect(v.metadata!.reason).toBe('x > 10 failed');
  });

  it('missing reason: detail absent → no metadata.reason key', () => {
    const v = mapConstraintResult({ element_id: 'c3', satisfied: true });
    expect(v.verdict).toBe('pass');
    expect(v.metadata!.reason).toBeUndefined();
  });

  it('empty-string detail is treated as no-reason', () => {
    const v = mapConstraintResult({ element_id: 'c3', satisfied: true, detail: '' });
    expect(v.metadata!.reason).toBeUndefined();
  });

  it('missing satisfied AND missing nested verdict → inconclusive', () => {
    const v = mapConstraintResult({ element_id: 'c4' });
    expect(v.verdict).toBe('inconclusive');
  });

  it('nested verdict supplies actual, expected, margin, sensitivity', () => {
    const v = mapConstraintResult({
      element_id: 'c5',
      satisfied: false,
      verdict: {
        verdict: 'Fail',
        actual: 12,
        expected: 10,
        margin: 2,
        sensitivity: { rho: 0.4 },
        metadata: { note: 'threshold exceeded' },
      },
    });
    expect(v.verdict).toBe('fail');
    expect(v.actual).toBe(12);
    expect(v.expected).toBe(10);
    expect(v.margin).toBe(2);
    expect(v.sensitivity).toEqual({ rho: 0.4 });
    expect(v.metadata!.note).toBe('threshold exceeded');
    expect(v.metadata!.element_id).toBe('c5');
  });

  it('nested verdict "Error" overrides satisfied flag', () => {
    const v = mapConstraintResult({
      element_id: 'c6',
      satisfied: false,
      verdict: { verdict: 'Error' },
    });
    expect(v.verdict).toBe('error');
  });

  it('neither actual nor expected → both null', () => {
    const v = mapConstraintResult({
      element_id: 'c7',
      satisfied: true,
      verdict: { verdict: 'Pass' },
    });
    expect(v.actual).toBeNull();
    expect(v.expected).toBeNull();
  });

  it('both actual and expected preserved when provided', () => {
    const v = mapConstraintResult({
      element_id: 'c8',
      satisfied: false,
      verdict: { verdict: 'Fail', actual: 7, expected: 5 },
    });
    expect(v.actual).toBe(7);
    expect(v.expected).toBe(5);
  });

  it('evidence is always None in R3.2', () => {
    const v = mapConstraintResult({
      element_id: 'c9',
      satisfied: true,
      // Even if the backend one day includes something here, R3.2 drops it.
      verdict: { verdict: 'Pass' },
    });
    expect(v.evidence).toBeNull();
  });
});

describe('mapEvaluateVerificationCaseResult', () => {
  it('maps the aggregate row and stashes counts in metadata', () => {
    const v = mapEvaluateVerificationCaseResult({
      element_id: 'case-1',
      case_id: 'vc-1',
      case_name: 'VoltageLimit',
      subject: 'Vehicle',
      methods: ['test', 'demo'],
      verdict: 'Pass',
      total_requirements: 3,
      passed_requirements: 3,
      display: 'PASS (3/3)',
    });
    expect(v.verdict).toBe('pass');
    expect(v.metadata!.case_id).toBe('vc-1');
    expect(v.metadata!.case_name).toBe('VoltageLimit');
    expect(v.metadata!.subject).toBe('Vehicle');
    expect(v.metadata!.methods).toEqual(['test', 'demo']);
    expect(v.metadata!.element_id).toBe('case-1');
    expect(v.metadata!.total_requirements).toBe(3);
    expect(v.metadata!.passed_requirements).toBe(3);
    expect(v.metadata!.display).toBe('PASS (3/3)');
    expect(v.metadata!.source).toBe('verification-case');
    // No actual/expected for aggregate rows.
    expect(v.actual).toBeNull();
    expect(v.expected).toBeNull();
  });

  it('rolls enriched evaluate verification requirements into metadata', () => {
    const v = mapEvaluateVerificationCaseResult({
      case_name: 'ThermalCase',
      verdict: 'Fail',
      requirements: [
        { requirement_id: 'r1', verdict: 'Pass', actual: true, expected: true, message: 'ok' },
        {
          requirement_id: 'r2',
          requirement_text: 'Temperature shall remain below limit',
          verdict: 'Fail',
          actual: false,
          expected: true,
          message: 'failed: constraint[0]',
          constraints: [{ expression: 'temp < 90', satisfied: false }],
        },
      ],
    });
    const reqs = v.metadata!.requirements as Array<{ requirement_id: string; verdict: string; actual: unknown; constraints?: unknown[] }>;
    expect(reqs).toHaveLength(2);
    expect(reqs[1].requirement_id).toBe('r2');
    expect(reqs[1].verdict).toBe('fail');
    expect(reqs[1].actual).toBe(false);
    expect(reqs[1].constraints).toEqual([{ expression: 'temp < 90', satisfied: false }]);
    expect(v.metadata!.reason).toBe('failed: constraint[0]');
  });

  it('falls back to inconclusive when verdict missing', () => {
    const v = mapEvaluateVerificationCaseResult({ case_name: 'Mystery' });
    expect(v.verdict).toBe('inconclusive');
  });

  it('carries evaluation_mode into metadata (B10 layer 2)', () => {
    const v = mapEvaluateVerificationCaseResult({
      case_name: 'VoltageLimit',
      verdict: 'Pass',
      evaluation_mode: 'static',
    });
    expect(v.metadata!.evaluation_mode).toBe('static');
  });

  it('omits evaluation_mode when the backend did not send one', () => {
    const v = mapEvaluateVerificationCaseResult({ case_name: 'VoltageLimit', verdict: 'Pass' });
    expect(v.metadata!.evaluation_mode).toBeUndefined();
  });
});

describe('mapVerifyResult', () => {
  it('rolls per-requirement verdicts into metadata.requirements', () => {
    const v = mapVerifyResult('ThermalCase', {
      verdict: 'Fail',
      requirements: [
        { requirement_id: 'r1', verdict: 'Pass', message: 'ok' },
        {
          requirement_id: 'r2',
          requirement_text: 'Temperature shall remain below limit',
          verdict: 'Fail',
          actual: 94,
          expected: 90,
          margin: 4,
          constraints: [{ expression: 'temp <= 90', satisfied: false, actual: 94, expected: 90, margin: 4 }],
          message: 'temperature too high',
        },
      ],
      diagnostics: [],
    });
    expect(v.verdict).toBe('fail');
    expect(v.metadata!.case_id).toBe('ThermalCase');
    const reqs = v.metadata!.requirements as Array<{ requirement_id: string; verdict: string; actual?: unknown; expected?: unknown; margin?: number; constraints?: unknown[] }>;
    expect(reqs).toHaveLength(2);
    expect(reqs[1].verdict).toBe('fail');
    expect(reqs[1].actual).toBe(94);
    expect(reqs[1].expected).toBe(90);
    expect(reqs[1].margin).toBe(4);
    expect(reqs[1].constraints).toEqual([{ expression: 'temp <= 90', satisfied: false, actual: 94, expected: 90, margin: 4 }]);
    // The non-pass requirement message becomes the tooltip reason.
    expect(v.metadata!.reason).toBe('temperature too high');
  });

  it('uses diagnostics as reason when no failing requirements present', () => {
    const v = mapVerifyResult('BrokenCase', {
      verdict: 'Error',
      requirements: [],
      diagnostics: ['compile failed: unresolved reference Foo'],
    });
    expect(v.verdict).toBe('error');
    expect(v.metadata!.reason).toBe('compile failed: unresolved reference Foo');
  });

  it('missing reason / message fields leave metadata.reason undefined', () => {
    const v = mapVerifyResult('CleanPass', {
      verdict: 'Pass',
      requirements: [{ requirement_id: 'r1', verdict: 'Pass' }],
      diagnostics: [],
    });
    expect(v.verdict).toBe('pass');
    expect(v.metadata!.reason).toBeUndefined();
  });

  it('missing requirements array is tolerated', () => {
    const v = mapVerifyResult('BareCase', { verdict: 'Pass' });
    expect(v.verdict).toBe('pass');
    expect(v.metadata!.requirements).toBeUndefined();
  });

  it('carries evaluation_mode into metadata — trajectory for a live run (B10)', () => {
    const v = mapVerifyResult('LiveCase', { verdict: 'Pass', evaluation_mode: 'trajectory' });
    expect(v.metadata!.evaluation_mode).toBe('trajectory');
  });
});


describe('expandRequirementVerdicts', () => {
  it('promotes nested requirements into first-class grid rows', () => {
    const aggregate = mapEvaluateVerificationCaseResult({
      case_id: 'vc-1',
      case_name: 'ThermalCase',
      verdict: 'Fail',
      requirements: [
        {
          requirement_id: 'r1',
          requirement_name: 'Temp max',
          requirement_element_id: 'req-el-1',
          verdict: 'Fail',
          actual: 94,
          expected: 90,
          margin: 4,
          message: 'too hot',
          constraints: [{ constraint_id: 'constraint-el-1', expression: 'temp <= 90' }],
        },
        { requirement_id: 'r2', verdict: 'Pass', actual: true, expected: true, message: 'ok' },
      ],
    });
    aggregate.metadata = { ...aggregate.metadata, uri: '/vehicle.sysml' };

    const rows = expandRequirementVerdicts(aggregate);

    expect(rows).toHaveLength(2);
    expect(rows[0].id).toBe('vc-1:r1');
    expect(rows[0].label).toBe('Temp max');
    expect(rows[0].verdict).toBe('fail');
    expect(rows[0].actual).toBe(94);
    expect(rows[0].expected).toBe(90);
    expect(rows[0].margin).toBe(4);
    expect(rows[0].reason).toBe('too hot');
    expect(rows[0].metadata!.case_name).toBe('ThermalCase');
    expect(rows[0].metadata!.requirement_id).toBe('r1');
    expect(rows[0].metadata!.requirement_element_id).toBe('req-el-1');
    expect(rows[0].metadata!.constraint_id).toBe('constraint-el-1');
    expect(rows[0].metadata!.element_id).toBe('constraint-el-1');
    expect(rows[0].metadata!.requirements).toBeUndefined();
    expect(rows[0].metadata!.uri).toBe('/vehicle.sysml');
    expect(rows[1].metadata!.requirement_id).toBe('r2');
  });

  it('leaves aggregate verdicts alone when no requirement breakdown exists', () => {
    const aggregate = mapEvaluateVerificationCaseResult({ case_name: 'BareCase', verdict: 'Pass' });
    expect(expandRequirementVerdicts(aggregate)).toEqual([aggregate]);
  });
});


describe('summarize', () => {
  it('counts by kind', () => {
    const s = summarize([
      emptyVerdict('pass'),
      emptyVerdict('pass'),
      emptyVerdict('fail'),
      emptyVerdict('inconclusive'),
      emptyVerdict('error'),
    ]);
    expect(s).toEqual({ pass: 2, fail: 1, inconclusive: 1, error: 1 });
  });

  it('returns zeroes for empty input', () => {
    expect(summarize([])).toEqual({ pass: 0, fail: 0, inconclusive: 0, error: 0 });
  });
});
