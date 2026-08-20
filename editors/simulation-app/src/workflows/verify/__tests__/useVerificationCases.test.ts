/**
 * Tests for useVerificationCases pure helpers — the static case read that
 * feeds the Cases list, the case document, and the suite header rollup.
 *
 * Covers the array|envelope response normalisation (incl. the defensive
 * model_digest read and the stdlib-noise filter), the suite rollup (bare
 * objectives drop out), and the case-lookup helpers.
 */

import { describe, it, expect } from 'vitest';
import {
  normalizeVerificationCasesResponse,
  suiteRollup,
  findCase,
  caseIdOf,
  isBareObjectiveRow,
  normalizeCaseVerdict,
  type VerificationCaseRow,
} from '../useVerificationCases';

const clauseFour: VerificationCaseRow = {
  case_id: 'c-clause4',
  element_id: 'e-clause4',
  case_name: 'ClauseFourReview',
  subject: 'bench',
  methods: [],
  evaluation_mode: 'static',
  verdict: 'Fail',
  display: 'FAIL (1/1 failed)',
  passed_requirements: 0,
  total_requirements: 1,
  requirements: [
    {
      requirement_id: 'protectionSpec',
      requirement_name: 'protectionSpec',
      requirement_element_id: 'r-protection',
      requirement_text: 'Protection requirements for the GroupHead family.',
      verdict: 'fail',
      message: 'sub-requirements not satisfied: tripTime, sensing, emcCompliance',
      subrequirements: [
        { requirement_id: 'tripTime', verdict: 'inconclusive', message: 'no modeled pass criteria' },
        { requirement_id: 'sensing', verdict: 'fail', message: 'accuracy bound violated' },
        {
          requirement_id: 'emcCompliance',
          verdict: 'fail',
          message: 'fails via referenced obligation',
          subrequirements: [
            { requirement_id: 'iecEmc.radiatedLimit', verdict: 'fail', message: 'referenced obligation not satisfied' },
          ],
        },
      ],
    },
  ],
};

describe('normalizeVerificationCasesResponse', () => {
  it('accepts the bare array and preserves the recursive subrequirement chain', () => {
    const cases = normalizeVerificationCasesResponse([clauseFour]);
    expect(cases).toHaveLength(1);
    const subs = cases[0].requirements?.[0].subrequirements ?? [];
    expect(subs.map((s) => s.requirement_id)).toEqual(['tripTime', 'sensing', 'emcCompliance']);
    // Three levels deep survives verbatim.
    expect(subs[2].subrequirements?.[0].requirement_id).toBe('iecEmc.radiatedLimit');
  });

  it('filters stdlib base-feature noise by name', () => {
    const cases = normalizeVerificationCasesResponse([
      clauseFour,
      { case_name: 'verificationCases', verdict: 'Pass' },
      { case_name: 'self', verdict: 'Pass' },
      { case_name: 'VerificationCase', verdict: 'Pass' },
    ]);
    expect(cases.map((c) => c.case_name)).toEqual(['ClauseFourReview']);
  });

  it('returns an empty list for a non-array (malformed / rejected-envelope) response', () => {
    // The read stays a bare array (steward ruling) — an object response is
    // not a shape we build for; it yields empty rather than a throw.
    expect(normalizeVerificationCasesResponse(null)).toEqual([]);
    expect(normalizeVerificationCasesResponse({ cases: [clauseFour] })).toEqual([]);
  });
});

describe('suiteRollup', () => {
  it('counts case verdicts and drops bare objectives (no verdict minted, 1e)', () => {
    const roll = suiteRollup([
      { case_name: 'A', verdict: 'Pass', total_requirements: 1 },
      { case_name: 'B', verdict: 'Fail', total_requirements: 2 },
      { case_name: 'C', verdict: 'Inconclusive', total_requirements: 1 },
      { case_name: 'Bare', verdict: 'Inconclusive', total_requirements: 0 },
    ]);
    expect(roll).toEqual({ pass: 1, fail: 1, inconclusive: 1, error: 0, total: 3 });
  });
});

describe('findCase / caseIdOf', () => {
  it('resolves by case_id, element_id, then case_name', () => {
    const cases = [clauseFour];
    expect(findCase(cases, 'c-clause4')?.case_name).toBe('ClauseFourReview');
    expect(findCase(cases, 'e-clause4')?.case_name).toBe('ClauseFourReview');
    expect(findCase(cases, 'ClauseFourReview')?.case_name).toBe('ClauseFourReview');
    expect(findCase(cases, 'nope')).toBeNull();
    expect(findCase(cases, null)).toBeNull();
  });

  it('caseIdOf prefers case_id', () => {
    expect(caseIdOf(clauseFour)).toBe('c-clause4');
    expect(caseIdOf({ element_id: 'e1' })).toBe('e1');
    expect(caseIdOf({ case_name: 'N' })).toBe('N');
  });
});

describe('isBareObjectiveRow / normalizeCaseVerdict', () => {
  it('flags total_requirements === 0 as bare', () => {
    expect(isBareObjectiveRow({ total_requirements: 0 })).toBe(true);
    expect(isBareObjectiveRow({ total_requirements: 1 })).toBe(false);
    expect(isBareObjectiveRow({})).toBe(false);
  });

  it('normalises backend casing to the four-valued ladder', () => {
    expect(normalizeCaseVerdict('Pass')).toBe('pass');
    expect(normalizeCaseVerdict('FAIL')).toBe('fail');
    expect(normalizeCaseVerdict('Error')).toBe('error');
    expect(normalizeCaseVerdict('weird')).toBe('inconclusive');
    expect(normalizeCaseVerdict(undefined)).toBe('inconclusive');
  });
});
