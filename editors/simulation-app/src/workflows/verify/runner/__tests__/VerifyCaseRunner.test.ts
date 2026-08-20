/**
 * Unit tests for VerifyCaseRunner — dispatch + progress + mapping.
 *
 * All backend calls are mocked via an injected poster so no HTTP is
 * issued. The clock is also injected so durationMs is deterministic.
 */

import { describe, expect, it, vi } from 'vitest';
import type { VerifyProgressEvent } from '../VerifyCaseRunner';
import { VerifyCaseRunner } from '../VerifyCaseRunner';
import type { VerifyRunConfig } from '@/engine/types';

// ── Helpers ──────────────────────────────────────────────────────────

interface MockPosterCall {
  path: string;
  command: string;
  params: Record<string, unknown>;
}

type MockResponder = unknown | ((params: Record<string, unknown>) => unknown);

function makeMockPoster(responses: Map<string, MockResponder>) {
  const calls: MockPosterCall[] = [];
  const poster = async <T>(path: string, body?: unknown): Promise<T> => {
    const envelope = body as { command: string; params: Record<string, unknown> };
    calls.push({ path, command: envelope.command, params: envelope.params });
    const response = responses.get(envelope.command);
    if (response === undefined) {
      throw new Error(`no mock response for command ${envelope.command}`);
    }
    if (typeof response === 'function') {
      return (response as (p: Record<string, unknown>) => unknown)(
        envelope.params,
      ) as T;
    }
    return response as T;
  };
  return { poster, calls };
}

function makeClock(start = 1_000): { clock: { now: () => number }; advance: (ms: number) => void } {
  let t = start;
  return {
    clock: { now: () => t },
    advance: (ms) => {
      t += ms;
    },
  };
}

// ── Tests ────────────────────────────────────────────────────────────

describe('VerifyCaseRunner — constraints suite', () => {
  it('calls sysml.evaluate.constraints and maps rows into Verdicts', async () => {
    const { poster, calls } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          [
            {
              element_id: 'c1',
              satisfied: true,
              detail: 'ok',
              verdict: { verdict: 'Pass', actual: 3 },
            },
            {
              element_id: 'c2',
              satisfied: false,
              detail: 'too high',
              verdict: { verdict: 'Fail', actual: 11, expected: 10 },
            },
          ],
        ],
      ]),
    );
    const { clock } = makeClock(1_000);

    const runner = new VerifyCaseRunner({ poster, clock });
    const config: VerifyRunConfig = {
      suite: 'constraints',
    };

    const progress: VerifyProgressEvent[] = [];
    runner.onProgress((ev) => progress.push(ev));

    const result = await runner.run(config);

    expect(calls).toEqual([
      { path: '/api/command', command: 'sysml.evaluate.constraints', params: {} },
    ]);
    expect(result.verdicts).toHaveLength(2);
    expect(result.verdicts[0].verdict).toBe('pass');
    expect(result.verdicts[1].verdict).toBe('fail');
    expect(result.verdicts[1].actual).toBe(11);
    expect(result.verdicts[1].expected).toBe(10);
    expect(result.summary).toEqual({ pass: 1, fail: 1, inconclusive: 0, error: 0 });
    // progress fires once per constraint.
    expect(progress).toHaveLength(2);
    expect(progress[0].total).toBe(2);
    expect(progress[0].index).toBe(0);
    expect(progress[1].index).toBe(1);
  });

  it('filters by caseIds when provided', async () => {
    const { poster } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          [
            { element_id: 'c1', satisfied: true },
            { element_id: 'c2', satisfied: false },
            { element_id: 'c3', satisfied: true },
          ],
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({
      suite: 'constraints',
      caseIds: ['c1', 'c3'],
    });
    expect(result.verdicts).toHaveLength(2);
    expect(result.verdicts.map((v) => v.metadata!.element_id)).toEqual(['c1', 'c3']);
  });

  it('issues exactly one workspace call — no per-file fan-out, no fabricated uri provenance', async () => {
    // Regression pin (scope-collapse follow-up, 2026-07-17): the runner
    // used to call the workspace-scoped command once per loaded uri and
    // stamp each duplicated result set with `metadata.uri` — provenance
    // the backend never produced.
    const { poster, calls } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          [
            { element_id: 'c1', satisfied: true },
            { element_id: 'c2', satisfied: false },
          ],
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({ suite: 'constraints' });
    expect(calls).toHaveLength(1);
    expect(result.verdicts).toHaveLength(2);
    expect(result.verdicts.every((v) => !('uri' in (v.metadata ?? {})))).toBe(true);
    expect(result.summary.pass).toBe(1);
    expect(result.summary.fail).toBe(1);
  });

  it('converts a backend error into an error-kind Verdict', async () => {
    const { poster } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          () => {
            throw new Error('backend blew up');
          },
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({
      suite: 'constraints',
    });
    expect(result.verdicts).toHaveLength(1);
    expect(result.verdicts[0]!.verdict).toBe('error');
    expect(result.verdicts[0]!.metadata!.reason).toBe('backend blew up');
  });
});

describe('VerifyCaseRunner — verification-cases suite', () => {
  it('per-case mode: calls sysml.verify for each case id', async () => {
    const { poster, calls } = makeMockPoster(
      new Map([
        [
          'sysml.verify',
          (p: Record<string, unknown>) => ({
            verdict: p.case_name === 'CaseA' ? 'Pass' : 'Fail',
            requirements: [
              {
                requirement_id: 'r1',
                verdict: p.case_name === 'CaseA' ? 'Pass' : 'Fail',
                message: p.case_name === 'CaseA' ? 'ok' : 'broke',
              },
            ],
            diagnostics: [],
          }),
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({
      suite: 'verification-cases',
      caseIds: ['CaseA', 'CaseB'],
      overrides: { x: 5 },
    });
    expect(calls).toHaveLength(2);
    expect(calls[0].command).toBe('sysml.verify');
    expect(calls[0].params.case_name).toBe('CaseA');
    expect(calls[0].params.overrides).toEqual([['x', '5']]);
    expect(result.verdicts.map((v) => v.verdict)).toEqual(['pass', 'fail']);
    expect(result.verdicts[1]!.metadata!.reason).toBe('broke');
    expect(result.summary).toEqual({ pass: 1, fail: 1, inconclusive: 0, error: 0 });
  });

  it('aggregate mode: falls back to sysml.evaluate.verification_cases', async () => {
    const { poster, calls } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.verification_cases',
          [
            {
              element_id: 'case-1',
              case_name: 'VoltageLimit',
              verdict: 'Pass',
              total_requirements: 3,
              passed_requirements: 3,
              display: 'PASS (3/3)',
            },
            {
              element_id: 'case-2',
              case_name: 'CurrentLimit',
              verdict: 'Inconclusive',
              total_requirements: 2,
              passed_requirements: 1,
              display: 'INCONCLUSIVE',
            },
          ],
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({
      suite: 'verification-cases',
    });
    expect(calls).toHaveLength(1);
    expect(calls[0].command).toBe('sysml.evaluate.verification_cases');
    expect(result.verdicts).toHaveLength(2);
    expect(result.summary).toEqual({ pass: 1, fail: 0, inconclusive: 1, error: 0 });
  });
});

describe('VerifyCaseRunner — live session mode (sessionId set)', () => {
  it('calls sysml.sessions.info once, then sysml.sessions.verify per case with case_names: [name]', async () => {
    const { poster, calls } = makeMockPoster(
      new Map<string, MockResponder>([
        ['sysml.sessions.info', { summary: { tick: 42 } }],
        [
          'sysml.sessions.verify',
          (p: Record<string, unknown>) => {
            const names = p.case_names as string[];
            return [
              {
                verdict: names[0] === 'CaseA' ? 'Pass' : 'Fail',
                requirements: [
                  {
                    requirement_id: 'r1',
                    verdict: names[0] === 'CaseA' ? 'Pass' : 'Fail',
                    message: names[0] === 'CaseA' ? 'ok' : 'tripped early',
                  },
                ],
                diagnostics: [],
              },
            ];
          },
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({
      suite: 'verification-cases',
      caseIds: ['CaseA', 'CaseB'],
      sessionId: 'sess-1',
    });

    const infoCalls = calls.filter((c) => c.command === 'sysml.sessions.info');
    const verifyCalls = calls.filter((c) => c.command === 'sysml.sessions.verify');
    expect(infoCalls).toHaveLength(1);
    expect(infoCalls[0]!.params.session_id).toBe('sess-1');
    expect(verifyCalls).toHaveLength(2);
    expect(verifyCalls[0]!.params).toEqual({ session_id: 'sess-1', case_names: ['CaseA'] });
    expect(verifyCalls[1]!.params).toEqual({ session_id: 'sess-1', case_names: ['CaseB'] });

    // sysml.verify is NOT called — live mode bypasses static evaluation entirely.
    expect(calls.some((c) => c.command === 'sysml.verify')).toBe(false);

    expect(result.verdicts.map((v) => v.verdict)).toEqual(['pass', 'fail']);
    // Evidence is synthesized client-side from the picked session + its
    // snapshotted tick — `sessions.verify` itself carries no evidence field.
    expect(result.verdicts[0]!.evidence).toEqual({ session_id: 'sess-1', tick: 42 });
    expect(result.verdicts[1]!.evidence).toEqual({ session_id: 'sess-1', tick: 42 });
  });

  it('a case absent from the live session workspace maps to an inconclusive verdict, not a dropped row', async () => {
    const { poster } = makeMockPoster(
      new Map([
        ['sysml.sessions.info', { summary: { tick: 7 } }],
        ['sysml.sessions.verify', []],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const result = await runner.run({
      suite: 'verification-cases',
      caseIds: ['GhostCase'],
      sessionId: 'sess-1',
    });
    expect(result.verdicts).toHaveLength(1);
    expect(result.verdicts[0]!.verdict).toBe('inconclusive');
    expect(result.verdicts[0]!.reason).toMatch(/GhostCase/);
  });

  it('ignores sessionId for the constraints suite (no sessions.verify counterpart)', async () => {
    const { poster, calls } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          [{ element_id: 'c1', satisfied: true }],
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    await runner.run({
      suite: 'constraints',
      sessionId: 'sess-1',
    });
    expect(calls.every((c) => c.command === 'sysml.evaluate.constraints')).toBe(true);
    expect(calls.some((c) => c.command.startsWith('sysml.sessions'))).toBe(false);
  });
});

describe('VerifyCaseRunner — cancel + duration', () => {
  it('cancel() aborts an in-flight run (AbortError)', async () => {
    // A deferred-response poster so we can observe mid-flight abort.
    let reject!: (reason: unknown) => void;
    const pending = new Promise<unknown[]>((_resolve, rej) => {
      reject = rej;
    });
    const poster = async <T>(_path: string, _body?: unknown): Promise<T> => {
      return (await pending) as unknown as T;
    };
    const runner = new VerifyCaseRunner({ poster });
    const promise = runner.run({
      suite: 'constraints',
    });
    runner.cancel();
    reject(new DOMException('cancelled', 'AbortError'));
    await expect(promise).rejects.toMatchObject({ name: 'AbortError' });
  });

  it('durationMs uses the injected clock', async () => {
    const { poster } = makeMockPoster(
      new Map([['sysml.evaluate.constraints', []]]),
    );
    const { clock, advance } = makeClock(0);
    const runner = new VerifyCaseRunner({ poster, clock });
    // The mock poster runs synchronously; advance clock from inside a
    // progress callback to simulate the elapsed time.
    runner.onProgress(() => advance(42));
    const result = await runner.run({
      suite: 'constraints',
    });
    // No rows returned → no progress fires; advance manually to verify
    // the end-of-run clock read is the actual end time.
    advance(17);
    // Run a second pass with at least one row so advance fires.
    const { poster: poster2 } = makeMockPoster(
      new Map([
        ['sysml.evaluate.constraints', [{ element_id: 'c1', satisfied: true }]],
      ]),
    );
    const runner2 = new VerifyCaseRunner({ poster: poster2, clock });
    runner2.onProgress(() => advance(50));
    const result2 = await runner2.run({
      suite: 'constraints',
    });
    expect(result.durationMs).toBeGreaterThanOrEqual(0);
    expect(result2.durationMs).toBeGreaterThanOrEqual(50);
  });
});

describe('VerifyCaseRunner — onProgress subscribers', () => {
  it('onProgress subscribers receive each step, unsub stops further calls', async () => {
    const { poster } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          [
            { element_id: 'c1', satisfied: true },
            { element_id: 'c2', satisfied: false },
          ],
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    const events: VerifyProgressEvent[] = [];
    const unsub = runner.onProgress((ev) => events.push(ev));
    await runner.run({
      suite: 'constraints',
    });
    expect(events).toHaveLength(2);
    unsub();
    // A second run should not emit to the unsubbed handler.
    await runner.run({
      suite: 'constraints',
    });
    expect(events).toHaveLength(2);
  });

  it('a throwing subscriber does not break the run', async () => {
    const { poster } = makeMockPoster(
      new Map([
        [
          'sysml.evaluate.constraints',
          [{ element_id: 'c1', satisfied: true }],
        ],
      ]),
    );
    const runner = new VerifyCaseRunner({ poster });
    runner.onProgress(() => {
      throw new Error('bad subscriber');
    });
    // Silence the expected console.error.
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const result = await runner.run({
      suite: 'constraints',
    });
    expect(result.verdicts).toHaveLength(1);
    expect(spy).toHaveBeenCalled();
    spy.mockRestore();
  });
});
