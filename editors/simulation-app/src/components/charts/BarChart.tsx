/**
 * Horizontal bar chart — used by Study mode for alternative scores.
 * Pure SVG, responsive to container width.
 */

interface BarChartItem {
  label: string;
  value: number;
  highlight?: boolean;
}

interface BarChartProps {
  items: BarChartItem[];
  width?: number;
  barHeight?: number;
  gap?: number;
}

export function BarChart({ items, width = 600, barHeight = 24, gap = 6 }: BarChartProps) {
  if (items.length === 0) return null;

  const maxVal = Math.max(...items.map((d) => Math.abs(d.value)), 0.001);
  const labelWidth = 100;
  const valueWidth = 60;
  const plotWidth = width - labelWidth - valueWidth - 16;
  const totalHeight = items.length * (barHeight + gap) - gap + 8;

  return (
    <svg width="100%" height={totalHeight} viewBox={`0 0 ${width} ${totalHeight}`} preserveAspectRatio="xMidYMid meet">
      {items.map((item, i) => {
        const y = i * (barHeight + gap) + 4;
        const barW = Math.max((Math.abs(item.value) / maxVal) * plotWidth, 2);
        const isHi = item.highlight;

        return (
          <g key={item.label}>
            {/* Label */}
            <text
              x={labelWidth - 8}
              y={y + barHeight / 2 + 4}
              textAnchor="end"
              fill="var(--on-surface-variant)"
              fontSize="11"
              fontFamily="var(--font-mono)"
            >
              {item.label}
            </text>

            {/* Bar background */}
            <rect
              x={labelWidth}
              y={y}
              width={plotWidth}
              height={barHeight}
              rx={3}
              fill="var(--surface-dim)"
            />

            {/* Bar fill */}
            <rect
              x={labelWidth}
              y={y}
              width={barW}
              height={barHeight}
              rx={3}
              fill={isHi ? 'url(#bar-gradient)' : 'var(--surface-container-highest)'}
            >
              <animate attributeName="width" from="0" to={barW} dur="0.5s" fill="freeze" />
            </rect>

            {/* Value label */}
            <text
              x={labelWidth + plotWidth + 8}
              y={y + barHeight / 2 + 4}
              textAnchor="start"
              fill={isHi ? 'var(--primary)' : 'var(--outline)'}
              fontSize="11"
              fontFamily="var(--font-mono)"
              fontWeight={isHi ? 700 : 400}
            >
              {item.value.toFixed(3)}
            </text>
          </g>
        );
      })}

      {/* Gradient definition */}
      <defs>
        <linearGradient id="bar-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor="var(--primary-container)" />
          <stop offset="100%" stopColor="var(--primary)" />
        </linearGradient>
      </defs>
    </svg>
  );
}
