import { describe, it, expect, vi } from 'vitest';
import { layoutWithTimeout, type ElkLike } from '../layout';
import type { ElkNode } from 'elkjs';

const GRAPH: ElkNode = { id: 'root', children: [{ id: 'a' }], edges: [] };

describe('layoutWithTimeout (elk wall-clock cap)', () => {
  it('passes a fast layout straight through', async () => {
    const elk: ElkLike = { layout: async (g) => ({ ...g, width: 10, height: 10 }) };
    const laid = await layoutWithTimeout(elk, GRAPH, 1000);
    expect(laid.width).toBe(10);
  });

  it('rejects a runaway layout at the cap, terminating the worker', async () => {
    vi.useFakeTimers();
    try {
      const terminate = vi.fn();
      const onTimeout = vi.fn();
      const hung: ElkLike = {
        layout: () => new Promise<ElkNode>(() => {}), // never resolves
        terminateWorker: terminate,
      };
      const p = layoutWithTimeout(hung, GRAPH, 20_000, onTimeout);
      const assertion = expect(p).rejects.toThrow(/timed out after 20s.*1 top-level nodes/);
      await vi.advanceTimersByTimeAsync(20_000);
      await assertion;
      expect(terminate).toHaveBeenCalledOnce();
      expect(onTimeout).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears the timer when the layout wins the race', async () => {
    vi.useFakeTimers();
    try {
      const terminate = vi.fn();
      const elk: ElkLike = {
        layout: async (g) => g,
        terminateWorker: terminate,
      };
      await layoutWithTimeout(elk, GRAPH, 20_000);
      await vi.advanceTimersByTimeAsync(30_000);
      expect(terminate).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
