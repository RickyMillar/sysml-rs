/**
 * Hook tests for useMonteCarloRunner.
 *
 * The hook calls the shared /api/command dispatcher with two commands:
 *
 *   1. `sysml.batch.create { kind: 'monte_carlo', children_params }`
 *      → `{ batch_id }`
 *   2. `sysml.batch.status { batch_id }` → `{ status, completed, total, children }`
 *
 * We inject a fake poster so the poll loop is deterministic. Covers:
 *   - idle → creating → running → complete transitions
 *   - payload includes deterministically-generated `children_params`
 *   - error surfaced when the backend doesn't return a batch_id
 *   - cancel() drops state back to idle
 */

import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useMonteCarloRunner, type HttpPoster } from '../useMonteCarloRunner';
import {
  generateChildrenParams,
  type DistributionMap,
} from '../sampleDistribution';

const DISTS: DistributionMap = {
  voltage: { kind: 'normal', mean: 12, sigma: 1 },
  current: { kind: 'uniform', min: 0, max: 10 },
};

/**
 * Build a fake poster that sequences create → status(running) → status(complete).
 */
function makePoster() {
  const calls: Array<{ command: string; params: Record<string, unknown> }> = [];
  let statusIdx = 0;
  // Backend wire shapes:
  //  - `sysml.batch.create` → `{batch_id, child_session_ids}` (no
  //    BatchSession payload — runner must wait for first poll).
  //  - `sysml.batch.status`  → `{batch: {status, children}}` where
  //    `BatchStatus` and `ChildStatus` are serde tagged enums using
  //    `status` as the tag field (NOT `kind`).
  const CHILD_IDS = ['c0', 'c1', 'c2', 'c3', 'c4'];
  const mkChildren = (tags: string[]) =>
    CHILD_IDS.map((id, i) => ({ session_id: id, status: { status: tags[i] } }));
  const statusScript = [
    {
      batch: {
        id: 'batch-7',
        status: { status: 'running', running: 4, completed: 1 },
        children: mkChildren(['complete', 'running', 'pending', 'pending', 'pending']),
      },
    },
    {
      batch: {
        id: 'batch-7',
        status: { status: 'running', running: 2, completed: 3 },
        children: mkChildren(['complete', 'complete', 'complete', 'running', 'pending']),
      },
    },
    {
      batch: {
        id: 'batch-7',
        status: { status: 'complete' },
        children: mkChildren(['complete', 'complete', 'complete', 'complete', 'complete']),
      },
    },
  ];

  const poster: HttpPoster = vi.fn(async (_path, body) => {
    const call = body as { command: string; params: Record<string, unknown> };
    calls.push(call);
    if (call.command === 'sysml.batch.create') {
      return {
        batch_id: 'batch-7',
        child_session_ids: [...CHILD_IDS],
      } as never;
    }
    if (call.command === 'sysml.batch.status') {
      const next = statusScript[Math.min(statusIdx, statusScript.length - 1)];
      statusIdx++;
      return next as never;
    }
    if (call.command === 'sysml.sessions.step') {
      // Reach the terminal state immediately so the drive loop exits
      // after one tick per child.
      return { completed: true } as never;
    }
    if (call.command === 'sysml.sessions.stop') {
      return {} as never;
    }
    throw new Error(`unexpected command ${call.command}`);
  }) as HttpPoster;
  return { poster, calls };
}

describe('useMonteCarloRunner', () => {
  it('drives idle → creating → running → complete', async () => {
    const { poster, calls } = makePoster();
    const { result } = renderHook(() =>
      useMonteCarloRunner({ poster, pollIntervalMs: 1 }),
    );

    expect(result.current.state).toBe('idle');

    await act(async () => {
      const id = await result.current.run({
        workspaceRoot: '/ws',
        distributions: DISTS,
        count: 5,
        seed: 42,
      });
      expect(id).toBe('batch-7');
    });

    await waitFor(() => expect(result.current.state).toBe('complete'));

    expect(result.current.batchId).toBe('batch-7');
    expect(result.current.children).toHaveLength(5);
    expect(result.current.completed).toBe(5);
    expect(result.current.total).toBe(5);

    // The first call should be sysml.batch.create with deterministic children_params.
    const createCall = calls.find((c) => c.command === 'sysml.batch.create');
    expect(createCall).toBeDefined();
    expect(createCall?.params.kind).toBe('monte_carlo');
    // `workspace_root` / `seed` are no longer on the wire — backend
    // signature only accepts { kind, uri, subsystem_name?, children_params,
    // label? }. Seed is baked into `children_params` before send.
    // Backend contract (R5.0 AA2): `children_params` is a JSON-encoded
    // string, not an array. Parse at the assertion boundary.
    const shippedRaw = createCall?.params.children_params as string;
    expect(typeof shippedRaw).toBe('string');
    const shipped = JSON.parse(shippedRaw) as Array<Record<string, number>>;
    expect(shipped).toHaveLength(5);
    // Deterministic given seed: regenerate locally and compare.
    const expected = generateChildrenParams(DISTS, 5, 42);
    expect(shipped).toEqual(expected);

    // And at least one status poll occurred.
    const statusCalls = calls.filter((c) => c.command === 'sysml.batch.status');
    expect(statusCalls.length).toBeGreaterThan(0);
  });

  it('surfaces an error when the backend omits batch_id', async () => {
    const poster: HttpPoster = vi.fn(async () => ({} as never));
    const { result } = renderHook(() =>
      useMonteCarloRunner({ poster, pollIntervalMs: 1 }),
    );

    await act(async () => {
      await result.current.run({
        distributions: DISTS,
        count: 3,
        seed: 1,
      });
    });

    await waitFor(() => expect(result.current.state).toBe('error'));
    expect(result.current.error).toMatch(/batch_id/i);
  });

  it('surfaces an error when generate fails on invalid distributions', async () => {
    const poster: HttpPoster = vi.fn(async () => ({ batch_id: 'x' } as never));
    const { result } = renderHook(() =>
      useMonteCarloRunner({ poster, pollIntervalMs: 1 }),
    );

    await act(async () => {
      await result.current.run({
        distributions: { bad: { kind: 'uniform', min: 5, max: 1 } },
        count: 3,
        seed: 1,
      });
    });

    await waitFor(() => expect(result.current.state).toBe('error'));
    // generate threw before any HTTP fired.
    expect(poster).not.toHaveBeenCalled();
  });

  it('cancel() drops the in-flight run back to idle', async () => {
    // Keep the status calls "running" forever so cancel matters.
    const poster: HttpPoster = vi.fn(async (_path, body) => {
      const call = body as { command: string };
      if (call.command === 'sysml.batch.create') {
        return { batch_id: 'loop', child_session_ids: [] } as never;
      }
      if (call.command === 'sysml.sessions.step') return { completed: false } as never;
      if (call.command === 'sysml.sessions.stop') return {} as never;
      return {
        batch: {
          id: 'loop',
          status: { status: 'running', running: 3, completed: 0 },
          children: [],
        },
      } as never;
    });
    const { result } = renderHook(() =>
      useMonteCarloRunner({ poster, pollIntervalMs: 1 }),
    );

    // Fire and forget — don't await, we want to cancel while it's polling.
    let runPromise: Promise<unknown>;
    await act(async () => {
      runPromise = result.current.run({
        distributions: DISTS,
        count: 3,
        seed: 1,
      });
    });
    await waitFor(() => expect(result.current.state).toBe('running'));

    act(() => result.current.cancel());
    await waitFor(() => expect(result.current.state).toBe('idle'));
    // Drain the runPromise so it doesn't leak.
    await act(async () => {
      await runPromise!;
    });
  });

  it('reset() clears batchId / children / error', async () => {
    const { poster } = makePoster();
    const { result } = renderHook(() =>
      useMonteCarloRunner({ poster, pollIntervalMs: 1 }),
    );
    await act(async () => {
      await result.current.run({
        distributions: DISTS,
        count: 5,
        seed: 42,
      });
    });
    await waitFor(() => expect(result.current.state).toBe('complete'));
    act(() => result.current.reset());
    expect(result.current.state).toBe('idle');
    expect(result.current.batchId).toBeNull();
    expect(result.current.children).toEqual([]);
    expect(result.current.error).toBeNull();
  });
});
