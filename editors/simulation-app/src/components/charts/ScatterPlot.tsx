/**
 * Scatter plot — used by Study mode for Pareto multi-objective visualization.
 * Pure SVG. Highlighted points form the Pareto front.
 */

interface ScatterPoint {
  label: string;
  x: number;
  y: number;
  highlight?: boolean;
}

interface ScatterPlotProps {
  points: ScatterPoint[];
  xLabel?: string;
  yLabel?: string;
  width?: number;
  height?: number;
}

const COLORS = {
  point: 'var(--primary-container)',
  pointDimmed: 'var(--surface-container-highest)',
  front: 'var(--primary)',
  label: 'var(--on-surface)',
  labelDimmed: 'var(--outline)',
  axis: 'var(--outline-variant)',
  grid: 'var(--outline-variant)',
};

export function ScatterPlot({ points, xLabel, yLabel, width = 600, height = 200 }: ScatterPlotProps) {
  if (points.length === 0) return null;

  const pad = { top: 12, right: 20, bottom: 28, left: 50 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;

  const xs = points.map((p) => p.x);
  const ys = points.map((p) => p.y);
  let xMin = Math.min(...xs);
  let xMax = Math.max(...xs);
  let yMin = Math.min(...ys);
  let yMax = Math.max(...ys);
  const xRange = xMax - xMin || 1;
  const yRange = yMax - yMin || 1;
  xMin -= xRange * 0.05;
  xMax += xRange * 0.05;
  yMin -= yRange * 0.05;
  yMax += yRange * 0.05;

  const scaleX = (v: number) => pad.left + ((v - xMin) / (xMax - xMin)) * plotW;
  const scaleY = (v: number) => pad.top + plotH - ((v - yMin) / (yMax - yMin)) * plotH;

  // Pareto front line (connect highlighted points sorted by X)
  const frontPoints = points.filter((p) => p.highlight).sort((a, b) => a.x - b.x);
  const frontLine = frontPoints.map((p) => `${scaleX(p.x).toFixed(1)},${scaleY(p.y).toFixed(1)}`).join(' ');

  return (
    <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="xMidYMid meet">
      {/* Grid */}
      {[0.25, 0.5, 0.75].map((frac) => (
        <g key={frac}>
          <line
            x1={pad.left} y1={scaleY(yMin + frac * (yMax - yMin))}
            x2={pad.left + plotW} y2={scaleY(yMin + frac * (yMax - yMin))}
            stroke={COLORS.grid} strokeWidth={0.5} opacity={0.15} strokeDasharray="3,3"
          />
          <line
            x1={scaleX(xMin + frac * (xMax - xMin))} y1={pad.top}
            x2={scaleX(xMin + frac * (xMax - xMin))} y2={pad.top + plotH}
            stroke={COLORS.grid} strokeWidth={0.5} opacity={0.15} strokeDasharray="3,3"
          />
        </g>
      ))}

      {/* Axes */}
      <line x1={pad.left} y1={pad.top} x2={pad.left} y2={pad.top + plotH} stroke={COLORS.axis} strokeWidth={0.5} />
      <line x1={pad.left} y1={pad.top + plotH} x2={pad.left + plotW} y2={pad.top + plotH} stroke={COLORS.axis} strokeWidth={0.5} />

      {/* Axis labels */}
      {xLabel && (
        <text x={pad.left + plotW / 2} y={height - 4} textAnchor="middle" fill="var(--outline)" fontSize="9" fontFamily="var(--font-mono)">
          {xLabel}
        </text>
      )}
      {yLabel && (
        <text x={10} y={pad.top + plotH / 2} textAnchor="middle" fill="var(--outline)" fontSize="9" fontFamily="var(--font-mono)"
          transform={`rotate(-90, 10, ${pad.top + plotH / 2})`}
        >
          {yLabel}
        </text>
      )}

      {/* Axis tick values */}
      <text x={pad.left - 4} y={pad.top + 8} textAnchor="end" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">{yMax.toFixed(1)}</text>
      <text x={pad.left - 4} y={pad.top + plotH} textAnchor="end" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">{yMin.toFixed(1)}</text>
      <text x={pad.left} y={height - 14} textAnchor="middle" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">{xMin.toFixed(1)}</text>
      <text x={pad.left + plotW} y={height - 14} textAnchor="middle" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">{xMax.toFixed(1)}</text>

      {/* Pareto front line */}
      {frontLine && frontPoints.length >= 2 && (
        <polyline points={frontLine} fill="none" stroke={COLORS.front} strokeWidth={1.5} opacity={0.6} strokeDasharray="4,2" />
      )}

      {/* Points */}
      {points.map((p, i) => {
        const cx = scaleX(p.x);
        const cy = scaleY(p.y);
        const isHi = p.highlight;
        return (
          <g key={i}>
            <circle
              cx={cx} cy={cy} r={isHi ? 5 : 3.5}
              fill={isHi ? COLORS.point : COLORS.pointDimmed}
              opacity={isHi ? 1 : 0.5}
            >
              <title>{`${p.label}: (${p.x.toFixed(2)}, ${p.y.toFixed(2)})`}</title>
            </circle>
            <text
              x={cx + 7} y={cy + 3}
              fill={isHi ? COLORS.label : COLORS.labelDimmed}
              fontSize={isHi ? '9' : '8'}
              fontFamily="var(--font-mono)"
              fontWeight={isHi ? 600 : 400}
            >
              {p.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
