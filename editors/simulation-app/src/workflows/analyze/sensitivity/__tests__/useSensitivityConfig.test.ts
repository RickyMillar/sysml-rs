import { describe, it, expect } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useSensitivityConfig } from '../useSensitivityConfig';

describe('useSensitivityConfig', () => {
  it('starts with morris + empty ranges and isValid=false', () => {
    const { result } = renderHook(() => useSensitivityConfig());
    expect(result.current.method).toBe('morris');
    expect(result.current.ranges).toHaveLength(0);
    expect(result.current.isValid).toBe(false);
  });

  it('adding a range + output metric makes the config valid', () => {
    const { result } = renderHook(() => useSensitivityConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'mass',
        label: 'mass',
        min: 0,
        max: 1,
      });
    });
    expect(result.current.isValid).toBe(false); // metric still missing
    act(() => {
      result.current.setOutputMetric('fail_count');
    });
    expect(result.current.isValid).toBe(true);
  });

  it('childCount mirrors r*(d+1) for morris and N*(d+2) for sobol', () => {
    const { result } = renderHook(() => useSensitivityConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'a',
        label: 'a',
        min: 0,
        max: 1,
      });
      result.current.addRange({
        parameterId: 'b',
        label: 'b',
        min: 0,
        max: 1,
      });
      result.current.setMorrisR(5);
      result.current.setMorrisP(4);
    });
    expect(result.current.childCount).toBe(5 * (2 + 1));

    act(() => {
      result.current.setMethod('sobol');
      result.current.setSobolN(64);
    });
    expect(result.current.childCount).toBe(64 * (2 + 2));
  });

  it('rejects a range where max <= min', () => {
    const { result } = renderHook(() => useSensitivityConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'bad',
        label: 'bad',
        min: 1,
        max: 1,
      });
      result.current.setOutputMetric('fail_count');
    });
    expect(result.current.isValid).toBe(false);
  });

  it('paramRanges maps labels to the name field for backend use', () => {
    const { result } = renderHook(() => useSensitivityConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'file::mass',
        label: 'mass',
        min: 1,
        max: 2,
      });
    });
    expect(result.current.paramRanges).toEqual([
      { name: 'mass', min: 1, max: 2 },
    ]);
  });
});
