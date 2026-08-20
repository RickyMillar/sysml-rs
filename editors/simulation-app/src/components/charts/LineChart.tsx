/**
 * Line chart — used by Study mode for ODE sweep characteristic curves.
 * Pure SVG. Supports linear and log Y scales.
 */

interface LinePoint {
  x: number;
  y: number;
}

interface LineSeries {
  name: string;
  points: LinePoint[];
  color?: string;
}

interface LineChartProps {
  series: LineSeries[];
  xLabel?: string;
  yLabel?: string;
  logY?: boolean;
  width?: number;
  height?: number;
  /** Optional vertical threshold markers. */
  markers?: Array<{ x: number; label: string; color?: string }>;
}

const COLORS = [
  'var(--chart-series-1)', 'var(--chart-series-2)', 'var(--chart-series-3)', 'var(--chart-series-4)',
  'var(--chart-series-5)', 'var(--chart-series-6)', 'var(--chart-series-7)', 'var(--chart-series-8)',
];

export function LineChart({
  series,
  xLabel,
  yLabel,
  logY = false,
  width = 600,
  height = 220,
  markers,
}: LineChartProps) {
  if (series.length === 0) return null;

  const pad = { top: 12, right: 20, bottom: 32, left: 60 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;

  // Collect all points across series for axis ranges.
  const allPts = series.flatMap((s) => s.points);
  if (allPts.length === 0) return null;

  let xMin = Math.min(...allPts.map((p) => p.x));
  let xMax = Math.max(...allPts.map((p) => p.x));
  const xRange = xMax - xMin || 1;
  xMin -= xRange * 0.02;
  xMax += xRange * 0.02;

  // Filter out non-positive Y values for log scale.
  const yVals = allPts.map((p) => p.y).filter((v) => !logY || v > 0);
  if (yVals.length === 0) return null;

  let yMin = Math.min(...yVals);
  let yMax = Math.max(...yVals);

  if (logY) {
    // Pad log range by ~half a decade.
    const logMin = Math.log10(Math.max(yMin, 1e-3));
    const logMax = Math.log10(Math.max(yMax, 1e-3));
    const logRange = logMax - logMin || 1;
    yMin = Math.pow(10, logMin - logRange * 0.05);
    yMax = Math.pow(10, logMax + logRange * 0.05);
  } else {
    const yRange = yMax - yMin || 1;
    yMin -= yRange * 0.05;
    yMax += yRange * 0.05;
  }

  const scaleX = (v: number) => pad.left + ((v - xMin) / (xMax - xMin)) * plotW;
  const scaleY = (v: number) => {
    if (logY) {
      const logVal = Math.log10(Math.max(v, 1e-6));
      const logMin = Math.log10(Math.max(yMin, 1e-6));
      const logMax = Math.log10(Math.max(yMax, 1e-6));
      return pad.top + plotH - ((logVal - logMin) / (logMax - logMin)) * plotH;
    }
    return pad.top + plotH - ((v - yMin) / (yMax - yMin)) * plotH;
  };

  // Y-axis grid ticks.
  const yTicks: number[] = [];
  if (logY) {
    const logMin = Math.floor(Math.log10(Math.max(yMin, 1e-6)));
    const logMax = Math.ceil(Math.log10(Math.max(yMax, 1e-6)));
    for (let exp = logMin; exp <= logMax; exp++) {
      yTicks.push(Math.pow(10, exp));
    }
  } else {
    const nTicks = 5;
    const step = (yMax - yMin) / nTicks;
    for (let i = 0; i <= nTicks; i++) {
      yTicks.push(yMin + i * step);
    }
  }

  // X-axis ticks.
  const nXTicks = 5;
  const xStep = (xMax - xMin) / nXTicks;
  const xTicks = Array.from({ length: nXTicks + 1 }, (_, i) => xMin + i * xStep);

  function formatY(v: number): string {
    if (logY) {
      if (v >= 1000) return `${(v / 1000).toFixed(0)}s`;
      if (v >= 1) return `${v.toFixed(0)}ms`;
      return v.toExponential(0);
    }
    if (Math.abs(v) >= 1000) return `${(v / 1000).toFixed(1)}s`;
    return v.toFixed(0);
  }

  return (
    <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="xMidYMid meet">
      {/* Grid */}
      {yTicks.map((v, i) => (
        <line
          key={`yg-${i}`}
          x1={pad.left} y1={scaleY(v)}
          x2={pad.left + plotW} y2={scaleY(v)}
          stroke="var(--outline-variant)" strokeWidth={0.5} opacity={0.15} strokeDasharray="3,3"
        />
      ))}
      {xTicks.map((v, i) => (
        <line
          key={`xg-${i}`}
          x1={scaleX(v)} y1={pad.top}
          x2={scaleX(v)} y2={pad.top + plotH}
          stroke="var(--outline-variant)" strokeWidth={0.5} opacity={0.15} strokeDasharray="3,3"
        />
      ))}

      {/* Axes */}
      <line x1={pad.left} y1={pad.top} x2={pad.left} y2={pad.top + plotH} stroke="var(--outline-variant)" strokeWidth={0.5} />
      <line x1={pad.left} y1={pad.top + plotH} x2={pad.left + plotW} y2={pad.top + plotH} stroke="var(--outline-variant)" strokeWidth={0.5} />

      {/* Y tick labels */}
      {yTicks.map((v, i) => (
        <text
          key={`yl-${i}`}
          x={pad.left - 4} y={scaleY(v) + 3}
          textAnchor="end" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)"
        >
          {formatY(v)}
        </text>
      ))}

      {/* X tick labels */}
      {xTicks.map((v, i) => (
        <text
          key={`xl-${i}`}
          x={scaleX(v)} y={pad.top + plotH + 14}
          textAnchor="middle" fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)"
        >
          {v.toFixed(1)}
        </text>
      ))}

      {/* Axis labels */}
      {xLabel && (
        <text x={pad.left + plotW / 2} y={height - 4} textAnchor="middle" fill="var(--outline)" fontSize="9" fontFamily="var(--font-mono)">
          {xLabel}
        </text>
      )}
      {yLabel && (
        <text x={12} y={pad.top + plotH / 2} textAnchor="middle" fill="var(--outline)" fontSize="9" fontFamily="var(--font-mono)"
          transform={`rotate(-90, 12, ${pad.top + plotH / 2})`}
        >
          {yLabel}
        </text>
      )}

      {/* Threshold markers */}
      {markers?.map((m, i) => (
        <g key={`m-${i}`}>
          <line
            x1={scaleX(m.x)} y1={pad.top}
            x2={scaleX(m.x)} y2={pad.top + plotH}
            stroke={m.color ?? 'var(--chart-annotation)'} strokeWidth={1} strokeDasharray="4,2" opacity={0.7}
          />
          <text
            x={scaleX(m.x) + 3} y={pad.top + 10}
            fill={m.color ?? 'var(--chart-annotation)'} fontSize="8" fontFamily="var(--font-mono)"
          >
            {m.label}
          </text>
        </g>
      ))}

      {/* Data series */}
      {series.map((s, si) => {
        const color = s.color ?? COLORS[si % COLORS.length];
        const validPts = logY ? s.points.filter((p) => p.y > 0) : s.points;
        if (validPts.length < 2) return null;

        const coords = validPts.map((p) => `${scaleX(p.x).toFixed(1)},${scaleY(p.y).toFixed(1)}`).join(' ');

        return (
          <g key={s.name}>
            <polyline points={coords} fill="none" stroke={color} strokeWidth={2} opacity={0.9} />
            {/* Data points */}
            {validPts.map((p, pi) => (
              <circle key={pi} cx={scaleX(p.x)} cy={scaleY(p.y)} r={2.5} fill={color} opacity={0.8}>
                <title>{`${s.name}: x=${p.x.toFixed(1)}, y=${p.y.toFixed(1)}`}</title>
              </circle>
            ))}
          </g>
        );
      })}

      {/* Legend */}
      {series.length > 1 && series.map((s, si) => {
        const color = s.color ?? COLORS[si % COLORS.length];
        return (
          <g key={`leg-${si}`}>
            <line x1={pad.left + 8} y1={pad.top + 8 + si * 14} x2={pad.left + 20} y2={pad.top + 8 + si * 14} stroke={color} strokeWidth={2} />
            <text x={pad.left + 24} y={pad.top + 11 + si * 14} fill="var(--on-surface)" fontSize="8" fontFamily="var(--font-mono)">
              {s.name}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
