/**
 * channels — CHANNEL RECLAMATION for the Compare surface (plan Phase 6
 * layout decision: "element family hue desaturates, fill becomes the
 * diff signal").
 *
 * On Run/Analyze surfaces the `--chart-series-N` family carries
 * per-element identity at full saturation. On Compare that channel is
 * reclaimed: session curves keep a TRACE of their family hue (enough
 * to tell six apart next to the swatch chips) but are mixed heavily
 * toward the neutral ink, so the saturated layer that remains — the
 * `--diff-*` envelope fill and gutter — is unambiguously the diff
 * signal. Diff never borrows verdict tokens.
 */

/** Desaturated stroke for the picked session at `index` (0-based). */
export function sessionStroke(index: number): string {
  const family = (index % 8) + 1;
  return `color-mix(in oklch, var(--chart-series-${family}) 40%, var(--text-secondary) 60%)`;
}
