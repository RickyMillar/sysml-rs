/**
 * Node shape resolution + geometry for the SvgCanvas renderer.
 *
 * The `VisualKind → Shape` table is **emitted from Rust** (`DesignTokens.shapes`,
 * single-sourced from `VisualKind::shape()`), so the frontend never re-lists
 * which kinds are control nodes / diamonds / ellipses. We look it up and dispatch.
 */

/** The `Shape` variant names emitted by Rust (`visual_kind.rs::Shape`). */
export type ShapeName =
  | 'Rect'
  | 'RoundedRect'
  | 'Ellipse'
  | 'Diamond'
  | 'HBar'
  | 'FilledCircle'
  | 'BullseyeCircle'
  | 'Pentagon'
  | 'HourglassPentagon'
  | 'NoteRect'
  | 'DashedRect'
  | 'CrossCircle';

/** Resolve a node's shape from the Rust-emitted map; falls back to a card. */
export function shapeForVisualKind(
  shapes: Record<string, string>,
  visualKind: string,
): ShapeName {
  return (shapes[visualKind] as ShapeName | undefined) ?? 'Rect';
}

/** Card-like shapes render the header/compartment box (with corner/stroke
 *  variations); everything else is a standalone glyph sized to its box. */
export function isCardShape(shape: ShapeName): boolean {
  return (
    shape === 'Rect' ||
    shape === 'RoundedRect' ||
    shape === 'NoteRect' ||
    shape === 'DashedRect'
  );
}

/**
 * Preferred fixed size for a non-card (glyph) node, by shape. elkjs sizes nodes
 * from their text box, which is wrong for control glyphs (a fork bar is wide and
 * short; a decision diamond is squarish). Returns `null` for card shapes and for
 * shapes that should keep their text-derived box (Ellipse, Pentagon variants —
 * they carry a name).
 */
export function glyphSizeFor(shape: ShapeName): { w: number; h: number } | null {
  switch (shape) {
    case 'Diamond':
      return { w: 40, h: 32 };
    case 'HBar':
      return { w: 60, h: 10 };
    case 'FilledCircle':
    case 'BullseyeCircle':
    case 'CrossCircle':
      return { w: 22, h: 22 };
    default:
      return null;
  }
}
