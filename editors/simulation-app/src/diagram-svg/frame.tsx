/**
 * Framed-view layer (spec §8.2.3.26, notation contract §C, ViewModel §F-10).
 *
 * A diagram IS a framed-view: an outer frame around the content, a heading
 * tab at the frame's top-left whose name compartment reads
 * `«view» Name (: kind)`, and up to three optional corner info compartments
 * (top-right / bottom-left / bottom-right) carrying Expose / Filter /
 * annotation summaries. The data comes straight from `vm.frame` — this layer
 * only draws it.
 *
 * NOTE on the heading suffix: the contract's optional slot is the *view
 * definition name*; the ViewModel carries the render `view_kind`
 * (General / Interconnection / StateTransition / …), which is the closest
 * honest value we have. Whether the slot should instead name a standard
 * ViewDefinition is queued as a spec-research item in the review register.
 */
import type { ViewFrame } from './viewmodel-types';

/** Space between the laid-out content bounds and the frame border. */
export const FRAME_PAD = 24;
/** Nominal content box for an EMPTY declared view: the frame still draws
 *  around this region, which carries the empty-state note (D-B3 — an empty
 *  view renders as an honest empty framed view, never a fallback surface). */
export const EMPTY_VIEW_CONTENT = { width: 340, height: 140 };
/** Heading tab height (the `«view» …` name compartment). */
export const FRAME_HEADING_H = 22;
/** Extra strip inside the frame bottom when bottom info slots are present. */
export const FRAME_SLOT_H = 18;

export interface FrameExtents {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** Frame bounds around a content box of (0,0)-(width,height). The frame is
 *  at least wide enough for the heading tab plus the top-right info slot —
 *  narrow content must not leave the heading overlapping the slot text. */
export function frameExtents(
  content: { width: number; height: number },
  frame: ViewFrame,
): FrameExtents {
  const hasBottom = Boolean(frame.bottom_left || frame.bottom_right);
  const topRightW = frame.top_right ? frame.top_right.text.length * 6.6 + 16 : 0;
  const minW = headingTabWidth(frame) + topRightW + 24;
  return {
    x0: -FRAME_PAD,
    y0: -(FRAME_PAD + FRAME_HEADING_H),
    x1: Math.max(content.width + FRAME_PAD, -FRAME_PAD + minW),
    y1: content.height + FRAME_PAD + (hasBottom ? FRAME_SLOT_H : 0),
  };
}

/** The heading's main text (`«view» Name`) and muted type suffix.
 *  Per spec §8.2.3.26 (R7) the suffix is the view's LITERALLY-declared
 *  immediate type/supertype (`frame.type_name`) — e.g. `«view» DriveModesView :
 *  StateTransition`. A view that declares no type gets no suffix (never the
 *  internal render-word `view_kind`, which is a rendering concern, not spec
 *  notation). */
export function frameHeadingParts(frame: ViewFrame): { main: string; suffix: string } {
  return { main: `«view» ${frame.name}`, suffix: frame.type_name ? ` : ${frame.type_name}` : '' };
}

/** Heading tab width from approximate char advances (600-weight main ≈7.2px,
 *  regular suffix ≈6.6px) + padding + the 45° corner cut. */
export function headingTabWidth(frame: ViewFrame): number {
  const { main, suffix } = frameHeadingParts(frame);
  return Math.ceil(main.length * 7.2 + suffix.length * 6.6 + 18);
}

export function ViewFrameLayer({
  frame,
  width,
  height,
  palette,
}: {
  frame: ViewFrame;
  /** Laid-out content bounds — the frame wraps (0,0)-(width,height). */
  width: number;
  height: number;
  palette: { bg: string; muted: string; text: string };
}) {
  const ext = frameExtents({ width, height }, frame);
  const { main, suffix } = frameHeadingParts(frame);
  const headW = headingTabWidth(frame);
  const cut = 8;
  return (
    <g data-testid="svgc-view-frame" style={{ pointerEvents: 'none' }}>
      <rect
        data-testid="svgc-view-frame-border"
        x={ext.x0}
        y={ext.y0}
        width={ext.x1 - ext.x0}
        height={ext.y1 - ext.y0}
        fill="none"
        stroke={palette.muted}
        strokeWidth={1.25}
      />
      {/* Heading tab: rectangle with the bottom-right corner cut at 45° (the
          UML/SysML frame-heading pentagon), flush with the frame's top-left. */}
      <path
        d={`M${ext.x0},${ext.y0} h${headW} v${FRAME_HEADING_H - cut} l${-cut},${cut} H${ext.x0} z`}
        fill={palette.bg}
        stroke={palette.muted}
        strokeWidth={1.25}
      />
      <text x={ext.x0 + 8} y={ext.y0 + 15} fontSize={12} fontWeight={600} fill={palette.text}>
        {main}
        <tspan fontWeight={400} fill={palette.muted}>
          {suffix}
        </tspan>
      </text>
      {frame.top_right && (
        <text x={ext.x1 - 8} y={ext.y0 + 15} fontSize={11} textAnchor="end" fill={palette.muted}>
          {frame.top_right.text}
        </text>
      )}
      {frame.bottom_left && (
        <text x={ext.x0 + 8} y={ext.y1 - 6} fontSize={11} fill={palette.muted}>
          {frame.bottom_left.text}
        </text>
      )}
      {frame.bottom_right && (
        <text x={ext.x1 - 8} y={ext.y1 - 6} fontSize={11} textAnchor="end" fill={palette.muted}>
          {frame.bottom_right.text}
        </text>
      )}
    </g>
  );
}
