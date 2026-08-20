/**
 * svgPaths — pure SVG path-building helpers for the Phase 6 diff
 * canvas. Framework-free; unit-tested in node.
 *
 * All helpers work over the rectangular `samples[s][t]` matrices from
 * `seriesMath.buildSampleMatrix` (NaN = no data at that tick) and a
 * shared linear scale: x = tick → [0, width], y = value → [height, 0]
 * over the variable's finite min/max across every picked session.
 */

import type { SamplesBySession } from '../selectors';

export interface Scale {
  width: number;
  height: number;
  maxTick: number;
  yMin: number;
  yMax: number;
}

/** Finite min/max across the whole matrix. Null when no finite value. */
export function valueDomain(
  samples: SamplesBySession,
): { yMin: number; yMax: number } | null {
  let mn = Infinity;
  let mx = -Infinity;
  for (const row of samples) {
    for (const v of row) {
      if (Number.isFinite(v)) {
        if (v < mn) mn = v;
        if (v > mx) mx = v;
      }
    }
  }
  if (!Number.isFinite(mn)) return null;
  if (mn === mx) {
    // Flat signal — pad so the line sits mid-band instead of on an
    // edge (and the scale never divides by zero).
    const pad = Math.abs(mn) > 1e-12 ? Math.abs(mn) * 0.05 : 0.5;
    return { yMin: mn - pad, yMax: mx + pad };
  }
  return { yMin: mn, yMax: mx };
}

export function xFor(tick: number, s: Scale): number {
  if (s.maxTick <= 0) return 0;
  return (tick / s.maxTick) * s.width;
}

export function yFor(value: number, s: Scale): number {
  const span = s.yMax - s.yMin;
  if (span <= 0) return s.height / 2;
  return s.height - ((value - s.yMin) / span) * s.height;
}

/**
 * Polyline path for one session's row. NaN gaps BREAK the line (a new
 * `M` segment) — missing data is never drawn through, per the
 * missing/NaN-dimming decision.
 */
export function linePath(row: number[], s: Scale): string {
  let d = '';
  let pen = false;
  for (let t = 0; t < row.length; t++) {
    const v = row[t];
    if (!Number.isFinite(v)) {
      pen = false;
      continue;
    }
    const cmd = pen ? 'L' : 'M';
    d += `${cmd}${xFor(t, s).toFixed(2)},${yFor(v, s).toFixed(2)}`;
    pen = true;
  }
  return d;
}

/**
 * Closed area path between the cross-session min and max at each tick
 * — the FILL that carries the diff signal (channel reclamation: the
 * curves desaturate, the spread between them is the layer that speaks).
 * Only ticks where ≥2 sessions have finite values contribute; runs of
 * contributing ticks become separate closed regions.
 */
export function envelopePath(samples: SamplesBySession, s: Scale): string {
  const T = samples[0]?.length ?? 0;
  let d = '';
  let upper: string[] = [];
  let lower: string[] = [];

  const flush = () => {
    if (upper.length >= 2) {
      d += `M${upper.join('L')}L${lower.reverse().join('L')}Z`;
    }
    upper = [];
    lower = [];
  };

  for (let t = 0; t < T; t++) {
    let mn = Infinity;
    let mx = -Infinity;
    let present = 0;
    for (const row of samples) {
      const v = row[t];
      if (Number.isFinite(v)) {
        if (v < mn) mn = v;
        if (v > mx) mx = v;
        present++;
      }
    }
    if (present < 2) {
      flush();
      continue;
    }
    const x = xFor(t, s).toFixed(2);
    upper.push(`${x},${yFor(mx, s).toFixed(2)}`);
    lower.push(`${x},${yFor(mn, s).toFixed(2)}`);
  }
  flush();
  return d;
}
