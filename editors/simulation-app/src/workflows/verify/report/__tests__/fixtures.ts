/**
 * Shared fixtures for the report generator tests. One small run with
 * 3 cases covering the full verdict matrix (pass / fail / error) plus
 * a mix of evidence-present / evidence-absent verdicts so snapshots
 * cover every render branch.
 *
 * Synthetic espresso-production-cell verdict matrix — no product model,
 * calibration data, or proprietary source (retired-internal remediation, UI-REPORT).
 */

import type { Verdict } from '@/engine/types';
import type { ReportInput } from '../types';

export const SAMPLE_VERDICTS: Verdict[] = [
  {
    id: 'req.brew_temp.in_band',
    label: 'Group-head temperature within 90–96 °C',
    verdict: 'pass',
    actual: 92.4,
    expected: 96,
    margin: -3.6,
    error: null,
    runtimeMs: 42,
    evidence: { session_id: 'sess-001', tick: 482, element_id: 'station1.groupHeadTemp' },
  },
  {
    id: 'req.warmup.within_budget',
    label: 'Station reaches Ready within 5 s',
    verdict: 'fail',
    actual: 5.4,
    expected: 5,
    margin: 0.4,
    error: null,
    reason: 'warm-up exceeded budget by 0.4 s',
    runtimeMs: 58,
    evidence: { session_id: 'sess-001', tick: 137, element_id: 'station1.readyTime' },
  },
  {
    id: 'req.pressure.safe',
    label: 'Manifold pressure stays ≤ 20 bar',
    verdict: 'error',
    actual: null,
    expected: null,
    error: null,
    reason: 'solver failed to converge on the demand-surge scenario',
    runtimeMs: 12,
    // intentionally no evidence — the error happened before the run
    // produced a meaningful tick snapshot.
    evidence: null,
  },
  {
    // Exercises the new error-string path: the backend could not
    // evaluate the constraint at all (unresolved binding), distinct
    // from the inconclusive "no value, no error" case above.
    id: 'req.demand.resolved',
    label: 'Aggregate demand binding resolves',
    verdict: 'error',
    actual: null,
    expected: null,
    error: 'unresolved binding: q_total',
    runtimeMs: 1,
    evidence: null,
  },
];

export const SAMPLE_INPUT: ReportInput = {
  workspaceName: 'Espresso Production Cell',
  runTimestamp: new Date(Date.UTC(2026, 3, 19, 14, 7, 32)),
  durationMs: 4321,
  environment: {
    backendVersion: 'sysml-runtime 0.3.1 (rev abc1234)',
    simDt: 0.001,
    seeds: [42, 1729],
    suiteName: 'Espresso cell acceptance suite',
  },
  verdicts: SAMPLE_VERDICTS,
  cases: [
    {
      id: 'case.brew_quality',
      label: 'Brew quality — station 1',
      requirementIds: ['req.brew_temp.in_band', 'req.warmup.within_budget'],
      // Pre-computed by the backend's `VerifySummary`: 1 pass, 1 fail → fail.
      summary: { pass: 1, fail: 1, inconclusive: 0, error: 0, overall: 'fail' },
    },
    {
      id: 'case.pressure',
      label: 'Pressure safety',
      requirementIds: ['req.pressure.safe'],
      summary: { pass: 0, fail: 0, inconclusive: 0, error: 1, overall: 'error' },
    },
    {
      id: 'case.demand',
      label: 'Demand binding',
      requirementIds: ['req.demand.resolved'],
      summary: { pass: 0, fail: 0, inconclusive: 0, error: 1, overall: 'error' },
    },
  ],
  // Workspace-level rollup across all verdicts: 1 pass, 1 fail, 2 error → error.
  summary: { pass: 1, fail: 1, inconclusive: 0, error: 2, overall: 'error' },
  // Provenance (§6.2). Deterministic fixed values so the snapshot is
  // stable; `archiveId`/`origin` present, model manifest present — a
  // future partial provenance (no model revision yet) would render `—`.
  provenance: {
    model: {
      manifestHash: 'sha256:deadbeefcafe',
      workspaceRoot: '/models/espresso-production-cell',
      files: [
        { uri: 'ProductQuality.sysml', hash: 'sha256:aaaa1111' },
        { uri: 'ScenarioVerification.sysml', hash: 'sha256:bbbb2222' },
      ],
    },
    sessionId: 'sess-001',
    archiveId: 'arch-0042',
    origin: 'user',
    selectedCases: ['Brew quality — station 1', 'Pressure safety'],
    overrides: [['ambient_c', '40']],
    runConfig: { dt: 0.001, scenario: 'nominal', view: 'Interconnection' },
    startedAt: '2026-04-19T14:07:28.000Z',
    stoppedAt: '2026-04-19T14:07:32.000Z',
  },
};
