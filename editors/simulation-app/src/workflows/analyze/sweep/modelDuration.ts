/**
 * Model-time formatting for the sweep's horizon.
 *
 * A horizon expressed in ticks does not tell you how much of a model's
 * behaviour a study covers — ticks and model time only coincide at a 1 ms
 * step. `examples/radiation-cooling` cools over ~2000 s; 1000 ticks reads as
 * a generous-looking number and is one second.
 *
 * This renders the product alongside the two inputs so the horizon can be
 * read in the units the model is written in.
 */

/** Format a model-time span in ms as a compact human-readable duration. */
export function formatModelDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—';
  if (ms === 0) return '0 s';
  if (ms < 1) return `${trim(ms)} ms`;
  if (ms < 1_000) return `${trim(ms)} ms`;
  const seconds = ms / 1_000;
  if (seconds < 90) return `${trim(seconds)} s`;
  const minutes = seconds / 60;
  if (minutes < 90) return `${trim(minutes)} min`;
  return `${trim(minutes / 60)} h`;
}

/** At most 3 significant decimals, with no trailing zeroes. */
function trim(value: number): string {
  return Number(value.toPrecision(4)).toString();
}
