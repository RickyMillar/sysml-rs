/**
 * forkErrors — structured consumption of `fork_with_overrides(at_tick)`
 * failures (audit F8: "consume structured FutureTick/SnapshotMissing
 * errors", never string-match).
 *
 * The backend serialises `ForkAtTickError` as a tagged JSON object
 * (`{"kind":"SnapshotMissing",…}`) and `ServiceError::ForkAtTick`'s
 * Display renders exactly that JSON, so transports that carry errors
 * as opaque strings (HTTP error bodies → `ApiError.message`) still
 * deliver the structured payload. The parser below extracts and
 * validates it; anything unrecognisable returns `null` and the caller
 * shows the raw message (an honest fallback, not a guess).
 */

import type { ForkAtTickError } from '@/features/sessions/types';

/** Extract the structured payload from an error's message string. */
export function parseForkAtTickError(message: string): ForkAtTickError | null {
  const start = message.indexOf('{');
  const end = message.lastIndexOf('}');
  if (start < 0 || end <= start) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(message.slice(start, end + 1));
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== 'object') return null;
  const obj = parsed as Record<string, unknown>;
  if (obj.kind === 'FutureTick') {
    if (typeof obj.tick === 'number' && typeof obj.current === 'number') {
      return { kind: 'FutureTick', tick: obj.tick, current: obj.current };
    }
    return null;
  }
  if (obj.kind === 'SnapshotMissing') {
    if (typeof obj.tick !== 'number') return null;
    const earliest =
      typeof obj.earliest_available === 'number' ? obj.earliest_available : null;
    const valid = Array.isArray(obj.valid_ticks)
      ? obj.valid_ticks.filter((t): t is number => typeof t === 'number')
      : [];
    return {
      kind: 'SnapshotMissing',
      tick: obj.tick,
      earliest_available: earliest,
      valid_ticks: valid,
    };
  }
  return null;
}

/**
 * Human copy for a structured fork error. `SnapshotMissing` names the
 * nearest valid ticks around the request — the caller's EXACT options
 * (the backend never clamps, so neither does the copy).
 */
export function describeForkAtTickError(e: ForkAtTickError): string {
  if (e.kind === 'FutureTick') {
    return `tick ${e.tick} is ahead of this session (currently at tick ${e.current})`;
  }
  if (e.valid_ticks.length === 0) {
    return `tick ${e.tick} is not archived and the archive is empty — step the session first`;
  }
  const near = nearestValidTicks(e.valid_ticks, e.tick, 5);
  return `tick ${e.tick} is not archived — forkable ticks near it: ${near.join(', ')}${
    e.valid_ticks.length > near.length ? ' …' : ''
  }`;
}

/** The `count` valid ticks closest to `target`, ascending. */
export function nearestValidTicks(
  validTicks: number[],
  target: number,
  count: number,
): number[] {
  return [...validTicks]
    .sort((a, b) => Math.abs(a - target) - Math.abs(b - target) || a - b)
    .slice(0, count)
    .sort((a, b) => a - b);
}
