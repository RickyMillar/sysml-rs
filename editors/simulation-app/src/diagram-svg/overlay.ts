/**
 * Simulation-overlay helpers for the SvgCanvas renderer (Bucket 3.5).
 *
 * The overlay (`sysml.diagram.sim_overlay`) is per-tick session state, joined to
 * the scene by `ElementId`. These pure helpers turn it into per-node visual
 * facts (active highlight, value badge); the polling + rendering live in
 * SvgCanvas.
 */

import type {
  DiagnosticOverlay,
  DiagnosticSeverity,
  ElementDiagnostics,
  ElementOverlay,
  ElementVerdict,
  OverlayValue,
  SimOverlay,
  VerdictOverlay,
} from './viewmodel-types';

/** The overlay delta for one scene node id, if any this tick. */
export function overlayForNode(overlay: SimOverlay | null, id: string): ElementOverlay | null {
  return overlay?.elements[id] ?? null;
}

/** Format a value badge: trim to a compact reading with its unit. */
export function formatOverlayValue(v: OverlayValue): string {
  const n = v.value;
  // Compact: integers bare, otherwise ~3 significant decimals without trailing zeros.
  const num = Number.isInteger(n) ? String(n) : Number(n.toPrecision(3)).toString();
  return v.unit ? `${num} ${v.unit}` : num;
}

/** Whether a tick reports this element as actively executing (vs completed/idle). */
export function isActive(o: ElementOverlay | null): boolean {
  return o?.activity === 'active';
}

export function isCompleted(o: ElementOverlay | null): boolean {
  return o?.activity === 'completed';
}

/** The verdict delta for one scene node id, if any this run. */
export function verdictForNode(overlay: VerdictOverlay | null, id: string): ElementVerdict | null {
  return overlay?.elements[id] ?? null;
}

/** The pass/fail/inconclusive/error glyph for a verdict, or '' when absent.
 *  Glyphs per brief §3.5 (colour is never the only encoding). */
export function verdictGlyph(v: ElementVerdict | null): string {
  switch (v?.verdict) {
    case 'Pass':
      return '✓';
    case 'Fail':
      return '✗';
    case 'Inconclusive':
      return '?';
    case 'Error':
      return '⨯';
    default:
      return '';
  }
}

/** How to draw the SW verdict pill for a verdict state (brief §3.5): which
 *  palette.verdict colour it takes, and its redundant non-colour encoding —
 *  pass/fail are solid, inconclusive is a dashed outline, error a hatched fill
 *  (error means *couldn't evaluate*, deliberately neutral-dark). */
export interface VerdictPillStyle {
  /** `palette.verdict` key. */
  token: 'pass' | 'fail' | 'inconclusive' | 'error';
  glyph: string;
  solid: boolean;
  dashed: boolean;
  hatched: boolean;
}

export function verdictPillStyle(v: ElementVerdict | null): VerdictPillStyle | null {
  switch (v?.verdict) {
    case 'Pass':
      return { token: 'pass', glyph: '✓', solid: true, dashed: false, hatched: false };
    case 'Fail':
      return { token: 'fail', glyph: '✗', solid: true, dashed: false, hatched: false };
    case 'Inconclusive':
      return { token: 'inconclusive', glyph: '?', solid: false, dashed: true, hatched: false };
    case 'Error':
      return { token: 'error', glyph: '⨯', solid: false, dashed: false, hatched: true };
    default:
      return null;
  }
}

/** The diagnostics for one scene node id, if any. */
export function diagnosticsForNode(
  overlay: DiagnosticOverlay | null,
  id: string,
): ElementDiagnostics | null {
  return overlay?.elements[id] ?? null;
}

/** Badge glyph per diagnostic severity (crib §3: ✕ on the error badge; a
 *  second, non-colour encoding per brief §4 precedence rule 4). */
export function severityGlyph(sev: DiagnosticSeverity): string {
  return sev === 'error' ? '✕' : sev === 'warning' ? '!' : 'i';
}

/** Multi-line tooltip body for a diagnostics badge: one `[code] message` line
 *  per diagnostic. */
export function diagnosticTooltip(d: ElementDiagnostics): string {
  return d.items.map((i) => (i.code ? `[${i.code}] ${i.message}` : i.message)).join('\n');
}

/** The single worst state a node carries, for the glyph-LOD "one worst-state
 *  dot" (brief §4 LOD table). Precedence: failing verdict > diagnostic error >
 *  diagnostic warning > live-active > null (nothing dot-worthy). Returns a
 *  palette path the renderer resolves to a colour. */
export type WorstState = 'fail' | 'error' | 'warning' | 'active';

export function worstState(
  overlay: ElementOverlay | null,
  verdict: ElementVerdict | null,
  diag: ElementDiagnostics | null,
): WorstState | null {
  if (verdict?.verdict === 'Fail') return 'fail';
  if (diag?.severity === 'error') return 'error';
  if (diag?.severity === 'warning') return 'warning';
  if (isActive(overlay)) return 'active';
  return null;
}
