/**
 * Time-series line chart — used by Simulate mode for ODE variables.
 * Pure SVG, one subplot per series, shared X axis.
 */

interface Series {
  name: string;
  points: Array<{ t: number; v: number }>;
  color?: string;
  isDiscrete?: boolean;  // render as staircase instead of smooth line
}

interface TimeSeriesChartProps {
  series: Series[];
  width?: number;
  subplotHeight?: number;
  /** Optional vertical event markers (e.g., state transitions). */
  markers?: Array<{ t: number; label: string }>;
  /** Optional zero-crossing markers — solid amber lines with ⚡ icon. */
  crossings?: Array<{ t: number; label: string; variable?: string }>;
}

const COLORS = [
  'var(--chart-series-1)', 'var(--chart-series-2)', 'var(--chart-series-3)', 'var(--chart-series-4)',
  'var(--chart-series-5)', 'var(--chart-series-6)', 'var(--chart-series-7)', 'var(--chart-series-8)',
];

function buildStaircasePath(
  points: Array<{ t: number; v: number }>,
  scaleX: (t: number) => number,
  scaleY: (v: number) => number,
): string {
  if (points.length === 0) return '';
  const parts = [`M ${scaleX(points[0].t).toFixed(1)} ${scaleY(points[0].v).toFixed(1)}`];
  for (let i = 1; i < points.length; i++) {
    // Horizontal line to next point's x at current y
    parts.push(`H ${scaleX(points[i].t).toFixed(1)}`);
    // Vertical line to next point's y
    parts.push(`V ${scaleY(points[i].v).toFixed(1)}`);
  }
  return parts.join(' ');
}

export function TimeSeriesChart({ series, width = 600, subplotHeight = 70, markers, crossings }: TimeSeriesChartProps) {
  if (series.length === 0) return null;

  const pad = { top: 4, right: 12, bottom: 18, left: 56 };
  const plotW = width - pad.left - pad.right;
  const totalHeight = series.length * subplotHeight + pad.bottom;

  // Global time range
  let tMin = Infinity;
  let tMax = -Infinity;
  for (const s of series) {
    for (const p of s.points) {
      if (p.t < tMin) tMin = p.t;
      if (p.t > tMax) tMax = p.t;
    }
  }
  const tRange = tMax - tMin || 1;
  const scaleX = (t: number) => pad.left + ((t - tMin) / tRange) * plotW;

  return (
    <svg width="100%" height={totalHeight} viewBox={`0 0 ${width} ${totalHeight}`} preserveAspectRatio="xMidYMid meet">
      {series.map((s, si) => {
        const color = s.color ?? COLORS[si % COLORS.length];
        const yOff = si * subplotHeight;
        const plotH = subplotHeight - pad.top - 4;

        // Per-series Y range
        let vMin = Infinity;
        let vMax = -Infinity;
        for (const p of s.points) {
          if (p.v < vMin) vMin = p.v;
          if (p.v > vMax) vMax = p.v;
        }
        const vRange = vMax - vMin || 1;
        vMin -= vRange * 0.08;
        vMax += vRange * 0.08;
        const scaleY = (v: number) => yOff + pad.top + plotH - ((v - vMin) / (vMax - vMin)) * plotH;

        // Polyline points
        const coords = s.points.map((p) => `${scaleX(p.t).toFixed(1)},${scaleY(p.v).toFixed(1)}`).join(' ');
        const last = s.points[s.points.length - 1];

        return (
          <g key={s.name}>
            {/* Background line */}
            <line
              x1={pad.left}
              y1={yOff + subplotHeight - 0.5}
              x2={pad.left + plotW}
              y2={yOff + subplotHeight - 0.5}
              stroke="var(--outline-variant)"
              strokeWidth={0.5}
              opacity={0.3}
            />

            {/* Grid midline */}
            <line
              x1={pad.left}
              y1={scaleY((vMin + vMax) / 2)}
              x2={pad.left + plotW}
              y2={scaleY((vMin + vMax) / 2)}
              stroke="var(--outline-variant)"
              strokeWidth={0.5}
              opacity={0.15}
              strokeDasharray="3,3"
            />

            {/* Y axis labels */}
            <text x={pad.left - 4} y={yOff + pad.top + 8} textAnchor="end" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">
              {vMax.toFixed(1)}
            </text>
            <text x={pad.left - 4} y={yOff + pad.top + plotH} textAnchor="end" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">
              {vMin.toFixed(1)}
            </text>

            {/* Series name */}
            <text x={4} y={yOff + subplotHeight / 2 + 3} fill={color} fontSize="9" fontFamily="var(--font-mono)" fontWeight={600}>
              {s.name}
            </text>
            {s.isDiscrete && (
              <text x={4} y={yOff + subplotHeight / 2 + 12} fill={color} fontSize="7" fontFamily="var(--font-mono)" opacity={0.6}>
                DISCRETE
              </text>
            )}

            {/* Left axis */}
            <line x1={pad.left} y1={yOff + pad.top} x2={pad.left} y2={yOff + pad.top + plotH} stroke="var(--outline-variant)" strokeWidth={0.5} />

            {/* Data line */}
            {s.points.length >= 2 && (
              s.isDiscrete ? (
                <path
                  d={buildStaircasePath(s.points, scaleX, scaleY)}
                  fill="none"
                  stroke={color}
                  strokeWidth={1.5}
                  opacity={0.9}
                />
              ) : (
                <polyline points={coords} fill="none" stroke={color} strokeWidth={1.5} opacity={0.9} />
              )
            )}

            {/* Discrete series sample dots */}
            {s.isDiscrete && s.points.map((p, pi) => (
              <circle key={pi} cx={scaleX(p.t)} cy={scaleY(p.v)} r={2} fill={color} opacity={0.7} />
            ))}

            {/* Current value dot + label */}
            {last && (
              <>
                <circle cx={scaleX(last.t)} cy={scaleY(last.v)} r={3} fill={color} />
                <text
                  x={scaleX(last.t) + 6}
                  y={scaleY(last.v) + 3}
                  fill={color}
                  fontSize="10"
                  fontFamily="var(--font-mono)"
                  fontWeight={700}
                >
                  {last.v.toFixed(2)}
                </text>
              </>
            )}

            {/* Event markers */}
            {markers?.map((m, mi) => (
              <g key={mi}>
                <line
                  x1={scaleX(m.t)}
                  y1={yOff + pad.top}
                  x2={scaleX(m.t)}
                  y2={yOff + pad.top + plotH}
                  stroke="var(--sim-active)"
                  strokeWidth={1}
                  strokeDasharray="2,2"
                  opacity={0.6}
                />
                {si === 0 && (
                  <text x={scaleX(m.t) + 2} y={yOff + pad.top + 8} fill="var(--sim-active)" fontSize="7" fontFamily="var(--font-mono)">
                    {m.label}
                  </text>
                )}
              </g>
            ))}

            {/* Zero-crossing markers */}
            {crossings
              ?.filter((c) => !c.variable || c.variable === s.name)
              .map((c, ci) => (
              <g key={`zc-${ci}`}>
                <line
                  x1={scaleX(c.t)}
                  y1={yOff + pad.top}
                  x2={scaleX(c.t)}
                  y2={yOff + pad.top + plotH}
                  stroke="var(--chart-annotation)"
                  strokeWidth={1.5}
                  opacity={0.8}
                />
                {si === 0 || c.variable === s.name ? (
                  <text x={scaleX(c.t) + 2} y={yOff + pad.top + 8} fill="var(--chart-annotation)" fontSize="7" fontFamily="var(--font-mono)">
                    {'\u26A1'} {c.label}
                  </text>
                ) : null}
              </g>
            ))}
          </g>
        );
      })}

      {/* Time axis on bottom */}
      {series.length > 0 && (() => {
        const yBottom = series.length * subplotHeight;
        const ticks = Math.min(5, series[0].points.length);
        return Array.from({ length: ticks + 1 }, (_, i) => {
          const t = tMin + (i / ticks) * tRange;
          return (
            <text
              key={i}
              x={scaleX(t)}
              y={yBottom + 12}
              textAnchor="middle"
              fill="var(--outline)"
              fontSize="8"
              fontFamily="var(--font-mono)"
            >
              {(t / 1000).toFixed(1)}s
            </text>
          );
        });
      })()}
    </svg>
  );
}
