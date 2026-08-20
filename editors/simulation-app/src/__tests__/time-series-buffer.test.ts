/**
 * Tests for TimeSeriesBuffer — Float64Array ring buffer (ADR-008).
 */

import { describe, it, expect } from 'vitest';
import { TimeSeriesBuffer } from '../shared/data/TimeSeriesBuffer';

describe('TimeSeriesBuffer', () => {
  it('appends and retrieves data', () => {
    const buf = new TimeSeriesBuffer(100);
    buf.append(0, { voltage: 1.0, current: 2.0 });
    buf.append(10, { voltage: 1.5, current: 2.5 });

    expect(buf.length).toBe(2);
    expect(buf.variableNames).toContain('voltage');
    expect(buf.variableNames).toContain('current');

    const { timestamps, series } = buf.getRange();
    expect([...timestamps]).toEqual([0, 10]);
    expect([...series['voltage']]).toEqual([1.0, 1.5]);
    expect([...series['current']]).toEqual([2.0, 2.5]);
  });

  it('wraps when full — oldest data is overwritten', () => {
    const buf = new TimeSeriesBuffer(4);
    buf.append(0, { x: 10 });
    buf.append(1, { x: 20 });
    buf.append(2, { x: 30 });
    buf.append(3, { x: 40 });
    // Buffer is now full (4/4)
    expect(buf.length).toBe(4);

    // This overwrites the oldest (t=0, x=10)
    buf.append(4, { x: 50 });
    expect(buf.length).toBe(4); // Still 4, not 5

    const { timestamps, series } = buf.getRange();
    expect([...timestamps]).toEqual([1, 2, 3, 4]);
    expect([...series['x']]).toEqual([20, 30, 40, 50]);
  });

  it('wraps multiple times correctly', () => {
    const buf = new TimeSeriesBuffer(3);
    for (let i = 0; i < 10; i++) {
      buf.append(i, { v: i * 10 });
    }
    expect(buf.length).toBe(3);
    const { timestamps, series } = buf.getRange();
    expect([...timestamps]).toEqual([7, 8, 9]);
    expect([...series['v']]).toEqual([70, 80, 90]);
  });

  it('reports accurate memory usage', () => {
    const buf = new TimeSeriesBuffer(1000);
    buf.append(0, { a: 1, b: 2, c: 3 });

    // 1 timestamp ring + 3 variable rings = 4 rings * 1000 * 8 bytes
    expect(buf.memoryUsageBytes()).toBe(4 * 1000 * 8);
  });

  it('derives maxPoints from memory budget', () => {
    // 80 bytes budget, 1 series: bytesPerPoint = 8 * (1+1) = 16
    // maxPoints = floor(80 / 16) = 5
    const buf = new TimeSeriesBuffer(undefined, 1, 80);
    expect(buf.capacity).toBe(5);
  });

  it('getRange with start/end filters data', () => {
    const buf = new TimeSeriesBuffer(100);
    for (let i = 0; i < 10; i++) {
      buf.append(i * 100, { temp: 20 + i });
    }

    const { timestamps, series } = buf.getRange(300, 600);
    expect([...timestamps]).toEqual([300, 400, 500, 600]);
    expect([...series['temp']]).toEqual([23, 24, 25, 26]);
  });

  it('getRange returns empty for out-of-range bounds', () => {
    const buf = new TimeSeriesBuffer(100);
    buf.append(10, { v: 1 });
    buf.append(20, { v: 2 });

    const { timestamps } = buf.getRange(100, 200);
    expect(timestamps.length).toBe(0);
  });

  it('handles late-arriving variable names gracefully', () => {
    const buf = new TimeSeriesBuffer(100);
    buf.append(0, { a: 1 });
    buf.append(1, { a: 2 });
    // Variable "b" appears for the first time at tick 2
    buf.append(2, { a: 3, b: 10 });

    const { series } = buf.getRange();
    expect([...series['a']]).toEqual([1, 2, 3]);
    // "b" should be NaN for the first two points, then 10
    expect(series['b'][0]).toBeNaN();
    expect(series['b'][1]).toBeNaN();
    expect(series['b'][2]).toBe(10);
  });

  it('fills NaN for missing variables in a tick', () => {
    const buf = new TimeSeriesBuffer(100);
    buf.append(0, { a: 1, b: 2 });
    buf.append(1, { a: 3 }); // "b" is missing

    const { series } = buf.getRange();
    expect([...series['a']]).toEqual([1, 3]);
    expect(series['b'][0]).toBe(2);
    expect(series['b'][1]).toBeNaN();
  });

  it('clear resets data but preserves capacity', () => {
    const buf = new TimeSeriesBuffer(50);
    buf.append(0, { x: 1 });
    buf.append(1, { x: 2 });
    expect(buf.length).toBe(2);

    buf.clear();
    expect(buf.length).toBe(0);
    expect(buf.capacity).toBe(50);
  });
});
