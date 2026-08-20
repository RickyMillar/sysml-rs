import { DiagramHost } from './diagram/DiagramHost';

/**
 * Thin facade over `DiagramHost` so existing imports keep working.
 *
 * `DiagramHost` (in `./diagram/`) does the renderer dispatch — SvgCanvas
 * for graph-shaped views, TanStack Table for grid, native tree for
 * browser, SVG/Three.js for geometry. This indirection lets us keep
 * `DiagramView` as the stable import surface while the renderer
 * structure underneath evolves.
 */
export function DiagramView() {
  return <DiagramHost />;
}
