/**
 * A sweep survives leaving the page.
 *
 * Reported: run a sweep, click another workflow, come back — the results are
 * gone. They lived in `useSweepRunner`'s React state, which the router
 * destroys on unmount, while the batch itself sat on the backend for the life
 * of the process, reachable only by an id that died with the component. There
 * is no `batch.list` to recover one.
 *
 * `useSweepRunner` remembers the id and re-attaches (see its own suite). This
 * pins the other half: the study DEFINITION. Restoring results without the
 * study that produced them is worse than restoring neither — the left rail
 * reports "0 factors, 0 combinations" beside a full table of results, and the
 * screen contradicts itself.
 */

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

const STUDY_KEY = 'sysml.sweep.study';

/** Re-import the module so its restore-on-load path runs against the current storage. */
async function freshStore() {
  vi.resetModules();
  return await import('../useSweepStudyStore');
}

beforeEach(() => {
  window.localStorage.clear();
});
afterEach(() => {
  vi.restoreAllMocks();
});

const RANGE = {
  parameterId: 'emissivity',
  spec: { kind: 'grid' as const, min: 0.5, max: 0.9, step: 0.1 },
};

describe('sweep study persistence', () => {
  it('starts from the documented defaults with nothing stored', async () => {
    const { useSweepStudyStore, DEFAULT_HORIZON_TICKS, DEFAULT_DT_MS } = await freshStore();
    const s = useSweepStudyStore.getState();
    expect(s.ranges).toEqual([]);
    expect(s.selectedMetricIds).toEqual([]);
    expect(s.horizonTicks).toBe(DEFAULT_HORIZON_TICKS);
    expect(s.dtMs).toBe(DEFAULT_DT_MS);
  });

  it('writes the study as soon as it changes', async () => {
    const { useSweepStudyStore } = await freshStore();
    useSweepStudyStore.getState().addRange(RANGE);
    useSweepStudyStore.getState().toggleMetric('temperature');
    useSweepStudyStore.getState().setDtMs(100);
    useSweepStudyStore.getState().setHorizonTicks(20_000);

    const stored = JSON.parse(window.localStorage.getItem(STUDY_KEY) ?? '{}');
    expect(stored.ranges).toHaveLength(1);
    expect(stored.selectedMetricIds).toEqual(['temperature']);
    expect(stored.dtMs).toBe(100);
    expect(stored.horizonTicks).toBe(20_000);
  });

  it('restores the whole study on reload', async () => {
    const first = await freshStore();
    first.useSweepStudyStore.getState().addRange(RANGE);
    first.useSweepStudyStore.getState().toggleMetric('temperature');
    first.useSweepStudyStore.getState().setDtMs(100);
    first.useSweepStudyStore.getState().setHorizonTicks(20_000);
    first.useSweepStudyStore.getState().setRunMode('sequential');

    const second = await freshStore();
    const s = second.useSweepStudyStore.getState();
    expect(s.ranges).toEqual([RANGE]);
    expect(s.selectedMetricIds).toEqual(['temperature']);
    expect(s.dtMs).toBe(100);
    expect(s.horizonTicks).toBe(20_000);
    expect(s.runMode).toBe('sequential');
    // The restored study expands to the same children it did before.
    expect(second.expandStudyChildren(s.ranges)).toHaveLength(5);
  });

  it('persists a removal, not just an addition', async () => {
    const first = await freshStore();
    first.useSweepStudyStore.getState().addRange(RANGE);
    first.useSweepStudyStore.getState().removeRange('emissivity');

    const second = await freshStore();
    expect(second.useSweepStudyStore.getState().ranges).toEqual([]);
  });

  it('discards a stored shape it does not recognise', async () => {
    // A blob from an older build must not resurrect as a half-valid study.
    window.localStorage.setItem(STUDY_KEY, JSON.stringify({ ranges: 'not-an-array', dtMs: 'fast' }));
    const { useSweepStudyStore, DEFAULT_DT_MS } = await freshStore();
    const s = useSweepStudyStore.getState();
    expect(s.ranges).toEqual([]);
    expect(s.dtMs).toBe(DEFAULT_DT_MS);
  });

  it('survives unparseable storage', async () => {
    window.localStorage.setItem(STUDY_KEY, '{ this is not json');
    const { useSweepStudyStore } = await freshStore();
    expect(useSweepStudyStore.getState().ranges).toEqual([]);
  });

  it('survives storage being unavailable entirely', async () => {
    // Private browsing / quota exceeded must degrade to an in-memory study,
    // not take the workflow down.
    const setItem = vi
      .spyOn(Storage.prototype, 'setItem')
      .mockImplementation(() => {
        throw new Error('QuotaExceededError');
      });
    const { useSweepStudyStore } = await freshStore();
    expect(() => useSweepStudyStore.getState().addRange(RANGE)).not.toThrow();
    expect(useSweepStudyStore.getState().ranges).toHaveLength(1);
    setItem.mockRestore();
  });
});

describe('last batch id', () => {
  it('round-trips through storage', async () => {
    const { readLastBatchId, writeLastBatchId } = await freshStore();
    expect(readLastBatchId()).toBeNull();
    writeLastBatchId('batch-7');
    expect(readLastBatchId()).toBe('batch-7');
  });

  it('clears rather than storing an empty id', async () => {
    const { readLastBatchId, writeLastBatchId } = await freshStore();
    writeLastBatchId('batch-7');
    writeLastBatchId(null);
    expect(readLastBatchId()).toBeNull();
    writeLastBatchId('');
    expect(readLastBatchId()).toBeNull();
  });
});
