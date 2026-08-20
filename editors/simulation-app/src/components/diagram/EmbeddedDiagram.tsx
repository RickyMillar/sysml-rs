/**
 * EmbeddedDiagram — Phase 6.
 *
 * Thin sidebar wrapper around `DiagramHost` for the non-Run workflow
 * tabs (Verify, Sweep, MonteCarlo). Gives the user spatial context
 * for whatever case / batch they're configuring without making them
 * jump back to /run.
 *
 * Reuses the same workspace-store-driven `DiagramHost` as the main
 * canvas, so the file picker, view selector, and URL sync introduced
 * in Phase 4 keep working — opening the Verify tab from a deep-linked
 * `/run?uri=…&view=…` URL leaves the focused diagram untouched.
 *
 * Width is fixed at 360 px — wide enough to read part labels, narrow
 * enough to leave the workflow's primary results pane usable on
 * 1280-wide displays.
 */

import { DiagramHost } from './DiagramHost';

export interface EmbeddedDiagramProps {
  /**
   * Heading text shown above the canvas. Pick something that names
   * the surface for the workflow it's mounted in (e.g. "Subject" for
   * Verify, "Model" for Analyze).
   */
  label?: string;
  /** Override the default 360 px sidebar width when needed. */
  widthPx?: number;
}

export function EmbeddedDiagram({
  label = 'Model',
  widthPx = 360,
}: EmbeddedDiagramProps = {}) {
  return (
    <aside
      data-testid="embedded-diagram"
      className="flex flex-col shrink-0 overflow-hidden"
      style={{
        width: widthPx,
        borderLeft: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-lowest)',
      }}
    >
      <header
        className="flex items-center px-3 py-1 shrink-0"
        style={{
          borderBottom: '1px solid var(--outline-variant)',
          fontSize: 10,
          fontWeight: 800,
          color: 'var(--outline)',
          letterSpacing: '0.04em',
          textTransform: 'uppercase',
        }}
      >
        {label}
      </header>
      <div className="flex-1 min-h-0 overflow-hidden">
        <DiagramHost />
      </div>
    </aside>
  );
}
