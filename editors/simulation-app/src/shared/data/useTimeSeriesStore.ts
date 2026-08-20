/**
 * useTimeSeriesStore — Zustand store wrapping TimeSeriesBuffer (ADR-008).
 *
 * Step responses feed into the ring buffer via `pushPoint`. Components
 * read from `getTimeSeries()` which returns the legacy TimePoint[] format
 * for backward compat with PlotsTab / KpisTab.
 */

import { create } from 'zustand';
import { TimeSeriesBuffer } from './TimeSeriesBuffer';
import type { TimePoint } from '../../features/sessions/types';

interface TimeSeriesStoreState {
  /** The underlying ring buffer. */
  buffer: TimeSeriesBuffer;

  /** Monotonic bump counter so React picks up changes. */
  revision: number;

  /** Push a single step's worth of data into the ring buffer. */
  pushPoint: (timestamp: number, values: Record<string, number>) => void;

  /**
   * Read all buffered data as the legacy Record<string, TimePoint[]> format
   * consumed by PlotsTab and KpisTab.
   */
  getTimeSeries: () => Record<string, TimePoint[]>;

  /**
   * F2b: read a uniformly-decimated sample (bounded to `maxPoints` per
   * series, latest point always included) in O(maxPoints * seriesCount)
   * instead of `getTimeSeries()`'s O(totalStoredPoints). Feeds the live
   * chart's hot path — the viewer already decimates for display (F2), so
   * handing it a pre-decimated series is a no-op-below-budget, behavior-
   * preserving swap that just avoids the full-ring copy on every ~60Hz
   * flush. Do NOT use this for KPI aggregates or CSV export — both need
   * every stored sample; keep those on `getTimeSeries()`.
   */
  getDecimatedTimeSeries: (maxPoints: number) => Record<string, TimePoint[]>;

  /** Reset the buffer (e.g., when switching sessions). */
  reset: (estimatedSeriesCount?: number) => void;
}

export const useTimeSeriesStore = create<TimeSeriesStoreState>((set, get) => ({
  buffer: new TimeSeriesBuffer(),
  revision: 0,

  pushPoint: (timestamp, values) => {
    get().buffer.append(timestamp, values);
    set((s) => ({ revision: s.revision + 1 }));
  },

  getTimeSeries: () => {
    const { buffer } = get();
    if (buffer.length === 0) return {};

    const { timestamps, series } = buffer.getRange();
    const result: Record<string, TimePoint[]> = {};

    for (const [name, values] of Object.entries(series)) {
      const points: TimePoint[] = [];
      for (let i = 0; i < timestamps.length; i++) {
        const v = values[i];
        if (!Number.isNaN(v)) {
          points.push({ t: timestamps[i], v });
        }
      }
      result[name] = points;
    }

    return result;
  },

  getDecimatedTimeSeries: (maxPoints) => {
    const { buffer } = get();
    if (buffer.length === 0) return {};

    const { timestamps, series } = buffer.getDecimatedRange(maxPoints);
    const result: Record<string, TimePoint[]> = {};

    for (const [name, values] of Object.entries(series)) {
      const points: TimePoint[] = [];
      for (let i = 0; i < timestamps.length; i++) {
        const v = values[i];
        if (!Number.isNaN(v)) {
          points.push({ t: timestamps[i], v });
        }
      }
      result[name] = points;
    }

    return result;
  },

  reset: (estimatedSeriesCount = 10) => {
    set({
      buffer: new TimeSeriesBuffer(undefined, estimatedSeriesCount),
      revision: 0,
    });
  },
}));
