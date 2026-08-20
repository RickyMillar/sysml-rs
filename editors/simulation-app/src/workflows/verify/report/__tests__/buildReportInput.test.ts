/**
 * buildReportInput — assembles a ReportInput from live Verify run state
 * (ninebar Phase 4). Covers case grouping, worst-wins overall, and the
 * §6.2 provenance block (what the frontend fills vs what waits on the
 * backend billet).
 */

import { describe, it, expect } from 'vitest';
import type { Verdict, VerifyRunResult } from '@/engine/types';
import { buildReportInput } from '../buildReportInput';

const TS = new Date(Date.UTC(2026, 6, 14, 12, 0, 0));

function v(caseName: string, verdict: Verdict['verdict']): Verdict {
  return {
    verdict,
    actual: null,
    expected: null,
    margin: null,
    error: null,
    sensitivity: null,
    evidence: null,
    metadata: { case_name: caseName },
  };
}

describe('buildReportInput', () => {
  it('groups verdicts into cases by case name and computes worst-wins overall', () => {
    const verdicts = [v('CaseA', 'pass'), v('CaseA', 'fail'), v('CaseB', 'pass')];
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      verdicts,
      suiteLabel: 'Verification Cases',
      selectedCaseNames: ['CaseA', 'CaseB'],
      sessionId: null,
      runTimestamp: TS,
    });

    expect(input.cases.map((c) => c.id).sort()).toEqual(['CaseA', 'CaseB']);
    const caseA = input.cases.find((c) => c.id === 'CaseA')!;
    expect(caseA.summary.overall).toBe('fail'); // worst-wins over pass+fail
    // Every verdict got a stable id so grouping is deterministic.
    expect(input.verdicts.every((x) => typeof x.id === 'string' && x.id.length > 0)).toBe(true);
  });

  it('prefers the runner result summary + duration when present', () => {
    const result: VerifyRunResult = {
      verdicts: [v('C', 'error')],
      durationMs: 1234,
      summary: { pass: 0, fail: 0, inconclusive: 0, error: 1 },
    };
    const input = buildReportInput({
      workspaceName: 'ws',
      result,
      verdicts: [],
      suiteLabel: 'S',
      selectedCaseNames: [],
      sessionId: 'sess-9',
      runTimestamp: TS,
    });
    expect(input.durationMs).toBe(1234);
    expect(input.summary.overall).toBe('error');
    expect(input.summary.error).toBe(1);
  });

  it('fills the provenance it knows and leaves model/archive for the backend billet', () => {
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      verdicts: [v('C', 'pass')],
      suiteLabel: 'S',
      selectedCaseNames: ['C'],
      sessionId: 'sess-1',
      runTimestamp: TS,
    });
    expect(input.provenance?.sessionId).toBe('sess-1');
    expect(input.provenance?.selectedCases).toEqual(['C']);
    // No session provenance supplied (session predating B6 capture, or a
    // session-free static verify) — undefined here, rendered as "—".
    expect(input.provenance?.model).toBeUndefined();
    expect(input.provenance?.archiveId).toBeUndefined();
  });

  it('derives distinct evaluation modes from the verdicts, computed-first (B10, §2.1a(d))', () => {
    const withMode = (caseName: string, mode: string): Verdict => ({
      ...v(caseName, 'pass'),
      metadata: { case_name: caseName, evaluation_mode: mode },
    });
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      // deliberately out of display order + a duplicate, to prove dedup + ordering
      verdicts: [withMode('A', 'external'), withMode('B', 'static'), withMode('C', 'static')],
      suiteLabel: 'S',
      selectedCaseNames: [],
      sessionId: null,
      runTimestamp: TS,
    });
    expect(input.provenance?.evaluationModes).toEqual(['static', 'external']);
  });

  it('leaves evaluationModes undefined when no verdict carries a mode', () => {
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      verdicts: [v('C', 'pass')],
      suiteLabel: 'S',
      selectedCaseNames: [],
      sessionId: null,
      runTimestamp: TS,
    });
    expect(input.provenance?.evaluationModes).toBeUndefined();
  });

  it('fills the model revision from the session provenance block (B6)', () => {
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      verdicts: [v('C', 'pass')],
      suiteLabel: 'S',
      selectedCaseNames: ['C'],
      sessionId: 'sess-1',
      sessionProvenance: {
        model_digest: 'abc123',
        git: { sha: 'deadbeef', dirty: true, branch: 'main' },
        workspace_root: '/ws/project',
      },
      runTimestamp: TS,
    });
    expect(input.provenance?.model?.manifestHash).toBe('abc123');
    expect(input.provenance?.model?.workspaceRoot).toBe('/ws/project');
    // Absent per-file manifest stays `undefined` (report renders `—`) —
    // never fabricated from the digest.
    expect(input.provenance?.model?.files).toBeUndefined();
  });

  it('maps the per-file manifest into model.files (§6.2)', () => {
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      verdicts: [v('C', 'pass')],
      suiteLabel: 'S',
      selectedCaseNames: ['C'],
      sessionId: 'sess-1',
      sessionProvenance: {
        model_digest: 'abc123',
        workspace_root: '/ws/project',
        file_manifest: [
          { path: 'parts/Engine.sysml', content_hash: 'aaaa' },
          { path: 'parts/Wheel.sysml', content_hash: 'bbbb' },
        ],
      },
      runTimestamp: TS,
    });
    expect(input.provenance?.model?.files).toEqual([
      { uri: 'parts/Engine.sysml', hash: 'aaaa' },
      { uri: 'parts/Wheel.sysml', hash: 'bbbb' },
    ]);
  });

  it('treats an empty per-file manifest as absent (renders —, not an empty list)', () => {
    const input = buildReportInput({
      workspaceName: 'ws',
      result: null,
      verdicts: [v('C', 'pass')],
      suiteLabel: 'S',
      selectedCaseNames: ['C'],
      sessionId: 'sess-1',
      sessionProvenance: { model_digest: 'abc123', file_manifest: [] },
      runTimestamp: TS,
    });
    expect(input.provenance?.model?.files).toBeUndefined();
  });
});
