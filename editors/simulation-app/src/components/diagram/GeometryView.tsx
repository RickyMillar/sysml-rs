import { useMemo } from 'react';
import { useWorkspaceStore } from '@/store/workspace';
import type { GeometryPrimitive, Viewport } from '@/shared/api/model';

/**
 * Spatial / floor-plan view. Rendered when the store holds a `geometryModel`
 * payload (DiagramHost dispatches by payload shape).
 *
 * Reads the typed `GeometryModel` payload (`primitives` + optional
 * `viewport`) from the workspace store and renders rectangles in SVG.
 *
 * Currently supports `shape: 'rect'` only — sufficient for floor plans,
 * DIN rails, modular GroupHead layouts, and panel layouts. The empty state
 * carries a hint so users know the spec affordance exists, but no
 * model in the workspace has spatial attributes yet (Phase 5b adds
 * them to the espresso-production-cell).
 */
function fitViewport(viewport: Viewport, padding = 32): {
  viewBox: string;
} {
  const width = viewport.maxX - viewport.minX;
  const height = viewport.maxY - viewport.minY;
  const x = viewport.minX - padding;
  const y = viewport.minY - padding;
  const w = width + padding * 2;
  const h = height + padding * 2;
  return { viewBox: `${x} ${y} ${w} ${h}` };
}

function GeometryRect({ primitive }: { primitive: GeometryPrimitive }) {
  if (primitive.shape !== 'rect') return null;
  return (
    <g
      data-testid={`geometry-${primitive.id}`}
      data-element-id={primitive.elementId ?? undefined}
      className={primitive.cssClasses?.join(' ')}
    >
      <rect
        x={primitive.x}
        y={primitive.y}
        width={primitive.width}
        height={primitive.height}
        fill="var(--surface-container-high)"
        stroke="var(--outline)"
        strokeWidth={1}
        rx={2}
      />
      {primitive.label && (
        <text
          x={primitive.x + primitive.width / 2}
          y={primitive.y + primitive.height / 2}
          fill="var(--on-surface)"
          fontSize={Math.min(12, Math.max(8, primitive.height * 0.25))}
          textAnchor="middle"
          dominantBaseline="middle"
          style={{ pointerEvents: 'none', userSelect: 'none' }}
        >
          {primitive.label}
        </text>
      )}
    </g>
  );
}

export function GeometryView() {
  const geometryModel = useWorkspaceStore((s) => s.geometryModel);

  const fit = useMemo(() => {
    if (!geometryModel?.viewport) return null;
    return fitViewport(geometryModel.viewport);
  }, [geometryModel?.viewport]);

  const isEmpty = !geometryModel || geometryModel.primitives.length === 0;

  return (
    <div
      data-testid="geometry-view-root"
      style={{
        width: '100%',
        height: '100%',
        position: 'relative',
        background: 'var(--surface-dim)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'stretch',
        justifyContent: isEmpty ? 'center' : 'flex-start',
      }}
    >
      {geometryModel?.title && !isEmpty && (
        <div
          data-testid="geometry-view-title"
          style={{
            padding: '12px 16px 8px',
            fontSize: 13,
            fontWeight: 600,
            color: 'var(--on-surface)',
            borderBottom: '1px solid var(--outline-variant)',
          }}
        >
          {geometryModel.title}
        </div>
      )}

      {isEmpty ? (
        <>
          <svg
            width="100%"
            height="100%"
            style={{ position: 'absolute', inset: 0, opacity: 0.06 }}
            aria-hidden="true"
          >
            <defs>
              <pattern id="grid" width="32" height="32" patternUnits="userSpaceOnUse">
                <path
                  d="M 32 0 L 0 0 0 32"
                  fill="none"
                  stroke="var(--on-surface)"
                  strokeWidth="0.5"
                />
              </pattern>
            </defs>
            <rect width="100%" height="100%" fill="url(#grid)" />
          </svg>

          <div
            data-testid="geometry-view-empty"
            style={{
              maxWidth: 420,
              padding: 24,
              margin: 'auto',
              textAlign: 'center',
              color: 'var(--on-surface)',
              background: 'var(--surface-container-high)',
              border: '1px solid var(--outline-variant)',
              borderRadius: 6,
              zIndex: 1,
            }}
          >
            <div style={{ fontWeight: 600, fontSize: 13, marginBottom: 8 }}>
              Geometry view — not populated
            </div>
            <div style={{ fontSize: 12, color: 'var(--outline)', lineHeight: 1.5 }}>
              No element in the model carries <code>x</code>/<code>y</code> +
              <code> width</code>/<code>height</code> attributes. A spatial
              layer for the espresso-production-cell is planned (Phase 5b of the
              diagram-UX plan).
            </div>
          </div>
        </>
      ) : (
        <div
          data-testid="geometry-view-canvas"
          style={{ flex: 1, overflow: 'hidden' }}
        >
          <svg
            width="100%"
            height="100%"
            viewBox={fit?.viewBox}
            preserveAspectRatio="xMidYMid meet"
            style={{ display: 'block' }}
          >
            {geometryModel.primitives.map((p) => (
              <GeometryRect key={p.id} primitive={p} />
            ))}
          </svg>
        </div>
      )}
    </div>
  );
}
