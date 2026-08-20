/**
 * Unit tests for the structured fork-at-tick error consumption (F8).
 */
import { describe, expect, it } from 'vitest';
import {
  describeForkAtTickError,
  nearestValidTicks,
  parseForkAtTickError,
} from '../forkErrors';

describe('parseForkAtTickError', () => {
  it('parses a bare SnapshotMissing payload', () => {
    const e = parseForkAtTickError(
      '{"kind":"SnapshotMissing","tick":7,"earliest_available":4,"valid_ticks":[4,6,8]}',
    );
    expect(e).toEqual({
      kind: 'SnapshotMissing',
      tick: 7,
      earliest_available: 4,
      valid_ticks: [4, 6, 8],
    });
  });

  it('parses the payload embedded in a transport wrapper string', () => {
    const e = parseForkAtTickError(
      'API 500 /api/command: {"kind":"FutureTick","tick":99,"current":10}',
    );
    expect(e).toEqual({ kind: 'FutureTick', tick: 99, current: 10 });
  });

  it('returns null for plain-string errors (no guessing)', () => {
    expect(parseForkAtTickError('no session: abc')).toBeNull();
    expect(parseForkAtTickError('{"kind":"SomethingElse"}')).toBeNull();
    expect(parseForkAtTickError('{not json')).toBeNull();
  });

  it('tolerates a missing earliest_available (archive empty)', () => {
    const e = parseForkAtTickError(
      '{"kind":"SnapshotMissing","tick":3,"earliest_available":null,"valid_ticks":[]}',
    );
    expect(e).toEqual({
      kind: 'SnapshotMissing',
      tick: 3,
      earliest_available: null,
      valid_ticks: [],
    });
  });
});

describe('describeForkAtTickError', () => {
  it('names the exact nearby options for SnapshotMissing — never a clamp', () => {
    const msg = describeForkAtTickError({
      kind: 'SnapshotMissing',
      tick: 7,
      earliest_available: 0,
      valid_ticks: [0, 2, 4, 6, 8, 10, 12],
    });
    expect(msg).toContain('tick 7 is not archived');
    expect(msg).toContain('4, 6, 8');
  });

  it('explains FutureTick against the session clock', () => {
    expect(
      describeForkAtTickError({ kind: 'FutureTick', tick: 50, current: 10 }),
    ).toContain('ahead of this session');
  });

  it('is honest about an empty archive', () => {
    expect(
      describeForkAtTickError({
        kind: 'SnapshotMissing',
        tick: 3,
        earliest_available: null,
        valid_ticks: [],
      }),
    ).toContain('archive is empty');
  });
});

describe('nearestValidTicks', () => {
  it('returns the closest ticks, ascending', () => {
    expect(nearestValidTicks([0, 10, 20, 30, 40], 22, 3)).toEqual([10, 20, 30]);
  });
});
