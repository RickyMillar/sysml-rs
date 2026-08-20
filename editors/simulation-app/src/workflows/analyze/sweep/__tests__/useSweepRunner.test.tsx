/**
 * useSweepRunner — the client half of a sweep's execution.
 *
 * Two defects are pinned here, both found on `examples/radiation-cooling`.
 *
 * 1. Selected outcomes never reached the backend. `batch.create` was sent
 *    without them, so nothing measured what the study asked for.
 *
 * 2. The runner drove EVERY child at once (`Promise.all(childIds.map(...))`).
 *    Driving a child ends in `sessions.stop`, which archives the run, so
 *    unbounded fan-out made peak backend work the sum of all children. A
 *    25-point two-factor sweep took the service past the machine's RAM and it
 *    was OOM-killed — reported as "hung for a while, then crashed".
 *
 * The bound is the fix that belongs on this side. It is not a throughput
 * knob and must not be relaxed into one.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act, waitFor, cleanup } from '@testing-library/react';
import { useSweepRunner, DEFAULT_CHILD_CONCURRENCY } from '../useSweepRunner';
import type { SweepPoint } from '../cartesianProduct';

/**
 * A dispatch double that records call order and reports the high-water mark
 * of concurrently in-flight `sessions.step` calls.
 */
function makeDispatch(
  opts: { childCount: number; failStepAt?: number[]; budgetTicks?: number } = { childCount: 4 },
) {
  const { childCount, failStepAt = [], budgetTicks } = opts;
  const ids = Array.from({ length: childCount }, (_, i) => `child-${i}`);
  const calls: { command: string; params: Record<string, unknown> }[] = [];
  let inFlight = 0;
  let peakInFlight = 0;
  const settled = new Set<string>();

  const dispatch = async <T,>(command: string, params: Record<string, unknown>): Promise<T> => {
    calls.push({ command, params });
    if (command === 'sysml.batch.create') {
      return { batch_id: 'batch-1', child_session_ids: ids } as T;
    }
    if (command === 'sysml.batch.status') {
      return {
        batch: {
          id: 'batch-1',
          children: ids.map((sid, i) => ({
            session_id: sid,
            index: i,
            params: { a: i },
            status: { status: settled.has(sid) ? 'complete' : 'pending' },
            verdicts: [],
          })),
          status: { status: settled.size === ids.length ? 'complete' : 'running', completed: settled.size },
        },
      } as T;
    }
    if (command === 'sysml.sessions.step') {
      const sid = params.session_id as string;
      const idx = ids.indexOf(sid);
      inFlight += 1;
      peakInFlight = Math.max(peakInFlight, inFlight);
      // Yield so genuinely-parallel callers overlap inside this window.
      await new Promise((r) => setTimeout(r, 5));
      inFlight -= 1;
      if (failStepAt.includes(idx)) throw new Error(`child ${idx} exploded`);
      // The backend advances only as far as the child's model-time budget
      // allows, and reports that in `ticks_advanced` WITHOUT raising.
      const requested = params.ticks as number;
      const advanced = budgetTicks != null ? Math.min(requested, budgetTicks) : requested;
      return { ticks_advanced: advanced, completed: true, time_ms: advanced } as T;
    }
    if (command === 'sysml.sessions.stop') {
      settled.add(params.session_id as string);
      return {} as T;
    }
    return {} as T;
  };

  return {
    dispatch,
    calls,
    ids,
    peak: () => peakInFlight,
    countOf: (command: string) => calls.filter((c) => c.command === command).length,
  };
}

const POINTS: SweepPoint[] = Array.from({ length: 12 }, (_, i) => ({ a: i }));

// The runner re-attaches to a remembered batch on mount, so a leftover id
// would make an unrelated test issue a `batch.status` it never asked for.
beforeEach(() => {
  window.localStorage.clear();
});

// `globals: false` in vitest.config.ts means RTL's auto-cleanup never
// registers, so a hook rendered here stays mounted until the whole file's
// environment is torn down — and a poll still in flight at that moment
// resolves into a dead jsdom and throws "window is not defined" out of
// `setError`. Unmounting runs the hook's own cleanup, which is what stops
// the poller.
afterEach(cleanup);

describe('useSweepRunner — selected outcomes reach the backend', () => {
  it('sends the requested outcomes, JSON-encoded like children_params', async () => {
    const d = makeDispatch({ childCount: 2 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));

    await act(async () => {
      await result.current.start({
        uri: 'file:///RadiationCooling.sysml',
        childrenParams: [{ ambientTemp: 250 }, { ambientTemp: 300 }],
        runMode: 'parallel',
        outcomes: ['temperature'],
      });
    });

    const create = d.calls.find((c) => c.command === 'sysml.batch.create');
    expect(create?.params.outcomes).toBe('["temperature"]');
  });

  it('omits outcomes entirely when the study selected none', async () => {
    const d = makeDispatch({ childCount: 1 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: [{ a: 1 }],
        runMode: 'parallel',
      });
    });
    const create = d.calls.find((c) => c.command === 'sysml.batch.create');
    expect(create?.params.outcomes).toBeUndefined();
  });
});

describe('useSweepRunner — bounded child execution', () => {
  it('never drives more than the concurrency bound at once', async () => {
    const d = makeDispatch({ childCount: 12 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));

    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: POINTS,
        runMode: 'parallel',
      });
    });
    await waitFor(() => expect(d.countOf('sysml.sessions.stop')).toBe(12), { timeout: 4000 });

    expect(d.peak()).toBeLessThanOrEqual(DEFAULT_CHILD_CONCURRENCY);
    // ...and the bound is a bound, not a serialisation: it does use it.
    expect(d.peak()).toBeGreaterThan(1);
  });

  it('honours an explicit concurrency override', async () => {
    const d = makeDispatch({ childCount: 12 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: POINTS,
        runMode: 'parallel',
        concurrency: 2,
      });
    });
    await waitFor(() => expect(d.countOf('sysml.sessions.stop')).toBe(12), { timeout: 4000 });
    expect(d.peak()).toBeLessThanOrEqual(2);
  });

  it('runs one at a time in sequential mode', async () => {
    const d = makeDispatch({ childCount: 6 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: POINTS.slice(0, 6),
        runMode: 'sequential',
      });
    });
    await waitFor(() => expect(d.countOf('sysml.sessions.stop')).toBe(6), { timeout: 4000 });
    expect(d.peak()).toBe(1);
  });

  it('loses no children — every one is stepped and stopped exactly once', async () => {
    // The bound must not be implemented by dropping work. All 25 points of
    // the two-factor repro have to run.
    const d = makeDispatch({ childCount: 25 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///RadiationCooling.sysml',
        childrenParams: Array.from({ length: 25 }, (_, i) => ({ ambientTemp: i, emissivity: i })),
        runMode: 'parallel',
      });
    });
    await waitFor(() => expect(d.countOf('sysml.sessions.stop')).toBe(25), { timeout: 6000 });

    const stepped = d.calls
      .filter((c) => c.command === 'sysml.sessions.step')
      .map((c) => c.params.session_id);
    expect(new Set(stepped).size).toBe(25);
    expect(stepped).toHaveLength(25);
  });
});

describe('useSweepRunner — failures are named, not swallowed', () => {
  it('reports which child failed, in which phase, at which parameters', async () => {
    const d = makeDispatch({ childCount: 4, failStepAt: [2] });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));

    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: [
          { ambientTemp: 250 },
          { ambientTemp: 275 },
          { ambientTemp: 300 },
          { ambientTemp: 325 },
        ],
        runMode: 'parallel',
      });
    });
    await waitFor(() => expect(result.current.failures).toHaveLength(1), { timeout: 4000 });

    expect(result.current.failures[0]).toMatchObject({
      index: 2,
      phase: 'step',
      params: { ambientTemp: 300 },
    });
    expect(result.current.failures[0].message).toContain('child 2 exploded');
    warn.mockRestore();
  });

  it('marks the failed child failed on the row every surface reads', async () => {
    // The backend archives and reports `complete` regardless — it cannot know
    // the step call never landed. Without the overlay the table would show a
    // successful row for a child that produced nothing.
    const d = makeDispatch({ childCount: 3, failStepAt: [1] });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));

    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: [{ a: 0 }, { a: 1 }, { a: 2 }],
        runMode: 'parallel',
      });
    });
    await waitFor(() => expect(result.current.progress.failed).toBe(1), { timeout: 4000 });

    const failed = result.current.children.find((c) => c.index === 1);
    expect(failed?.status).toBe('failed');
    expect(failed?.reason).toContain('exploded');
    // The other two keep their evidence.
    expect(result.current.progress.complete).toBe(2);
    warn.mockRestore();
  });

  it('keeps a failing child from wedging the rest of the batch', async () => {
    const d = makeDispatch({ childCount: 6, failStepAt: [0, 3] });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: POINTS.slice(0, 6),
        runMode: 'parallel',
      });
    });
    await waitFor(() => expect(d.countOf('sysml.sessions.stop')).toBe(6), { timeout: 4000 });
    // Every child still got stopped — no leaked sessions.
    expect(d.countOf('sysml.sessions.stop')).toBe(6);
    warn.mockRestore();
  });

  it('reports queued / running / complete / failed as a total-conserving split', async () => {
    const d = makeDispatch({ childCount: 5, failStepAt: [4] });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: POINTS.slice(0, 5),
        runMode: 'parallel',
      });
    });
    await waitFor(() => expect(result.current.progress.total).toBe(5), { timeout: 4000 });

    const p = result.current.progress;
    expect(p.queued + p.running + p.complete + p.failed).toBe(p.total);
    warn.mockRestore();
  });
});


describe('useSweepRunner — a run that stops short says so', () => {
  it('sends the study step size and a budget sized to the horizon', async () => {
    // Left unset the backend defaults to a 60 s budget, which silently
    // truncates any horizon longer than that. Ticks alone cannot express the
    // problem: they only equal model time at a 1 ms step.
    const d = makeDispatch({ childCount: 1 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///RadiationCooling.sysml',
        childrenParams: [{ ambientTemp: 300 }],
        runMode: 'parallel',
        horizonTicks: 20_000,
        dtMs: 100,
      });
    });
    const create = d.calls.find((c) => c.command === 'sysml.batch.create');
    expect(create?.params.dt_ms).toBe(100);
    // 20,000 ticks x 100 ms = 2,000,000 ms of model time.
    expect(create?.params.max_time_ms).toBe(2_000_000);
  });

  it('leaves the timing unset when the study did not choose one', async () => {
    const d = makeDispatch({ childCount: 1 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: [{ a: 1 }],
        runMode: 'parallel',
      });
    });
    const create = d.calls.find((c) => c.command === 'sysml.batch.create');
    expect(create?.params.dt_ms).toBeUndefined();
    expect(create?.params.max_time_ms).toBeUndefined();
  });

  it('reports a child that advanced fewer ticks than it was asked for', async () => {
    // The backend raises nothing here — the run simply stops moving — so this
    // is the only place the shortfall becomes visible.
    const d = makeDispatch({ childCount: 3, budgetTicks: 5_000 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///RadiationCooling.sysml',
        childrenParams: [{ ambientTemp: 250 }, { ambientTemp: 300 }, { ambientTemp: 350 }],
        runMode: 'parallel',
        horizonTicks: 20_000,
      });
    });
    await waitFor(() => expect(result.current.truncations).toHaveLength(3), { timeout: 4000 });

    expect(result.current.truncations[0]).toMatchObject({
      requestedTicks: 20_000,
      advancedTicks: 5_000,
    });
    // Truncation is NOT failure: the children ran and their evidence stands.
    expect(result.current.failures).toHaveLength(0);
    expect(result.current.progress.failed).toBe(0);
    // ...and the parameters are attached so the notice can name the points.
    expect(result.current.truncations.map((t) => t.params.ambientTemp)).toEqual([250, 300, 350]);
  });

  it('reports nothing when every child ran its full horizon', async () => {
    const d = makeDispatch({ childCount: 4 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: POINTS.slice(0, 4),
        runMode: 'parallel',
        horizonTicks: 1_000,
      });
    });
    await waitFor(() => expect(d.countOf('sysml.sessions.stop')).toBe(4), { timeout: 4000 });
    expect(result.current.truncations).toEqual([]);
  });

  it('does not carry a previous study\'s shortfall into the next run', async () => {
    // `start` resets the record. Without that, re-running a truncating study
    // would show its shortfall twice over, then three times, and the count in
    // the notice would drift away from the batch it describes.
    const d = makeDispatch({ childCount: 2, budgetTicks: 10 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));

    const run = async () => {
      await act(async () => {
        await result.current.start({
          uri: 'file:///m.sysml',
          childrenParams: POINTS.slice(0, 2),
          runMode: 'parallel',
          horizonTicks: 1_000,
        });
      });
      await waitFor(() => expect(result.current.truncations).toHaveLength(2), { timeout: 4000 });
    };

    await run();
    await run();
    // Two children truncated in the second study — not four.
    expect(result.current.truncations).toHaveLength(2);
  });
});

describe('useSweepRunner — results survive leaving the page', () => {
  // The reported failure: run a sweep, click another workflow, come back, and
  // the results are gone. They were held in this hook's React state, which the
  // router destroys on unmount — while the batch itself lives on the backend
  // for the life of the process. The `batch_id` was the only key to it, and
  // there is no `batch.list` to recover one, so losing it made live data
  // permanently unreachable.
  it('remembers the batch id as soon as the batch exists', async () => {
    const d = makeDispatch({ childCount: 2 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: [{ a: 1 }, { a: 2 }],
        runMode: 'parallel',
      });
    });
    expect(window.localStorage.getItem('sysml.sweep.lastBatchId')).toBe('batch-1');
  });

  it('reopens the last batch on mount', async () => {
    window.localStorage.setItem('sysml.sweep.lastBatchId', 'batch-1');
    const d = makeDispatch({ childCount: 3 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));

    await waitFor(() => expect(result.current.children).toHaveLength(3), { timeout: 4000 });
    expect(result.current.batchId).toBe('batch-1');
    // It READS the batch; it does not re-run it. The children were already
    // stepped, verified and stopped.
    expect(d.countOf('sysml.sessions.step')).toBe(0);
    expect(d.countOf('sysml.batch.create')).toBe(0);
  });

  it('does nothing when no batch was remembered', async () => {
    const d = makeDispatch({ childCount: 2 });
    const { result } = renderHook(() => useSweepRunner({ dispatch: d.dispatch, pollIntervalMs: 5 }));
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(result.current.batchId).toBeNull();
    expect(d.calls).toHaveLength(0);
  });

  it('forgets a batch the backend no longer has, without an error banner', async () => {
    // The backend restarted, or the id predates it. An empty workflow is the
    // honest state; a failure notice about a batch the user never asked to
    // reopen is noise.
    window.localStorage.setItem('sysml.sweep.lastBatchId', 'batch-gone');
    const dispatch = vi.fn(async (command: string) => {
      if (command === 'sysml.batch.status') throw new Error('no batch: batch-gone');
      return {} as never;
    });
    const { result } = renderHook(() =>
      useSweepRunner({ dispatch: dispatch as never, pollIntervalMs: 5 }),
    );
    await waitFor(
      () => expect(window.localStorage.getItem('sysml.sweep.lastBatchId')).toBeNull(),
      { timeout: 4000 },
    );
    expect(result.current.error).toBeNull();
    expect(result.current.status.kind).toBe('pending');
    expect(result.current.children).toEqual([]);
  });

  it('drops the remembered id when a new study starts', async () => {
    // Otherwise a study that fails to create would leave the PREVIOUS batch
    // stored, and coming back would silently reopen the wrong results.
    window.localStorage.setItem('sysml.sweep.lastBatchId', 'batch-old');
    const dispatch = vi.fn(async (command: string) => {
      if (command === 'sysml.batch.status') throw new Error('no batch');
      if (command === 'sysml.batch.create') throw new Error('create refused');
      return {} as never;
    });
    const { result } = renderHook(() =>
      useSweepRunner({ dispatch: dispatch as never, pollIntervalMs: 5 }),
    );
    await act(async () => {
      await result.current.start({
        uri: 'file:///m.sysml',
        childrenParams: [{ a: 1 }],
        runMode: 'parallel',
      });
    });
    expect(window.localStorage.getItem('sysml.sweep.lastBatchId')).toBeNull();
    expect(result.current.status.kind).toBe('failed');
  });
});
