/**
 * Histogram chart — used by Monte Carlo mode for pass rate distribution.
 * Pure SVG. Labels inside tall bars, tooltip on hover for all bars.
 */

interface HistogramProps {
  bins: Array<{ value: number; label: string }>;
  /** Threshold above which bins are "pass" colored. Default 0.95. */
  passThreshold?: number;
  width?: number;
  height?: number;
}

export function Histogram({ bins, passThreshold = 0.95, width = 600, height = 160 }: HistogramProps) {
  if (bins.length === 0) return null;

  const maxVal = Math.max(...bins.map(b => b.value), 0.001);
  const pad = { top: 8, right: 8, bottom: 8, left: 8 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const gap = Math.min(3, plotW / bins.length * 0.1);
  const barW = Math.max((plotW - gap * (bins.length - 1)) / bins.length, 4);

  return (
    <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="xMidYMid meet">
      {bins.map((bin, i) => {
        const barH = Math.max((bin.value / maxVal) * plotH, 2);
        const x = pad.left + i * (barW + gap);
        const y = pad.top + plotH - barH;
        const color = bin.value >= passThreshold
          ? 'var(--success)'
          : bin.value >= 0.7
            ? 'var(--warning)'
            : 'var(--error)';

        const pctText = `${(bin.value * 100).toFixed(0)}%`;
        const showLabel = barH > 30; // only show label in bar if tall enough

        return (
          <g key={i} style={{ cursor: 'default' }}>
            <rect
              x={x} y={y} width={barW} height={barH}
              rx={2} fill={color} opacity={0.75}
            >
              <animate attributeName="height" from="0" to={barH} dur="0.3s" fill="freeze" />
              <animate attributeName="y" from={pad.top + plotH} to={y} dur="0.3s" fill="freeze" />
            </rect>
            {/* Hover tooltip */}
            <title>{`${bin.label}: ${pctText} pass rate`}</title>
            {/* Label inside bar if tall enough */}
            {showLabel && (
              <>
                <text
                  x={x + barW / 2} y={y + 14}
                  textAnchor="middle" fill="#000" fontSize="9"
                  fontFamily="var(--font-mono)" fontWeight={600}
                  opacity={0.8}
                >
                  {pctText}
                </text>
                <text
                  x={x + barW / 2} y={y + 24}
                  textAnchor="middle" fill="#000" fontSize="7"
                  fontFamily="var(--font-mono)"
                  opacity={0.5}
                >
                  {bin.label.length > barW / 5 ? bin.label.substring(0, Math.floor(barW / 5)) + '..' : bin.label}
                </text>
              </>
            )}
            {/* Percentage above bar if too short for internal label */}
            {!showLabel && (
              <text
                x={x + barW / 2} y={y - 3}
                textAnchor="middle" fill={color} fontSize="8"
                fontFamily="var(--font-mono)" fontWeight={600}
              >
                {pctText}
              </text>
            )}
          </g>
        );
      })}

      {/* Baseline */}
      <line
        x1={pad.left} y1={pad.top + plotH}
        x2={pad.left + plotW} y2={pad.top + plotH}
        stroke="var(--outline-variant)" strokeWidth={0.5} opacity={0.3}
      />
    </svg>
  );
}
