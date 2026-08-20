import { useMemo } from 'react';

export interface SequenceMessage {
  tick: number;
  timeMs: number;
  from: string;
  to: string;
  label: string;
  kind?: 'transition' | 'event' | 'message';
  trigger?: 'auto' | 'event';
}

export interface SequenceViewProps {
  lifelines: string[];
  messages: SequenceMessage[];
  currentTick?: number;
  width?: number;
}

const HEADER_HEIGHT = 32;
const LIFELINE_SPACING = 120;
const ROW_HEIGHT = 24;
const PADDING = { left: 40, top: 8, right: 20 };
const SELF_LOOP_W = 16;
const ARROW_SIZE = 4;

interface CollapsedMessage extends SequenceMessage {
  count: number; // 1 = normal, >1 = collapsed self-loop
}

function collapseRepeats(msgs: SequenceMessage[]): CollapsedMessage[] {
  const result: CollapsedMessage[] = [];
  for (const m of msgs) {
    // Skip no-op steps (same state, no meaningful transition) — these are just ticks where nothing happened
    if (m.from === m.to && !m.label.includes('→')) {
      // It's a real named self-transition — keep it but collapse consecutive repeats
      const prev = result[result.length - 1];
      if (prev && prev.from === m.from && prev.to === m.to && prev.label === m.label) {
        prev.count++;
        prev.tick = m.tick;
        continue;
      }
      result.push({ ...m, count: 1 });
    } else if (m.from === m.to) {
      // No-op with arrow label like "heating → heating" — collapse into loop
      const prev = result[result.length - 1];
      if (prev && prev.from === m.from && prev.to === m.to) {
        prev.count++;
        prev.tick = m.tick;
        continue;
      }
      result.push({ ...m, count: 1 });
    } else {
      // Real state transition — always show
      result.push({ ...m, count: 1 });
    }
  }
  return result;
}

export function SequenceView({ lifelines, messages, currentTick, width: widthProp }: SequenceViewProps) {
  const collapsed = useMemo(() => collapseRepeats(messages), [messages]);
  const width = widthProp ?? PADDING.left + lifelines.length * LIFELINE_SPACING + PADDING.right;
  const bodyTop = PADDING.top + HEADER_HEIGHT + 12;
  const height = bodyTop + collapsed.length * ROW_HEIGHT + 24;

  const lifelineX = useMemo(
    () => lifelines.map((_, i) => PADDING.left + i * LIFELINE_SPACING + LIFELINE_SPACING / 2),
    [lifelines],
  );

  const idxOf = useMemo(() => {
    const m = new Map<string, number>();
    lifelines.forEach((n, i) => m.set(n, i));
    return m;
  }, [lifelines]);

  return (
    <svg width={width} height={height} style={{ display: 'block', minWidth: width }}>
      <defs>
        <marker id="seq-arrow" markerWidth={ARROW_SIZE} markerHeight={ARROW_SIZE}
          refX={ARROW_SIZE} refY={ARROW_SIZE / 2} orient="auto">
          <path d={`M0,0 L${ARROW_SIZE},${ARROW_SIZE / 2} L0,${ARROW_SIZE}`}
            fill="var(--on-surface-variant)" />
        </marker>
      </defs>

      {/* Lifeline headers */}
      {lifelines.map((name, i) => {
        const cx = lifelineX[i];
        const tw = Math.max(name.length * 7 + 16, 48);
        return (
          <g key={name}>
            <rect x={cx - tw / 2} y={PADDING.top} width={tw} height={HEADER_HEIGHT} rx={6}
              fill="var(--surface-container-high)" />
            <text x={cx} y={PADDING.top + HEADER_HEIGHT / 2} textAnchor="middle" dominantBaseline="central"
              fill="var(--on-surface)" fontSize="10" fontFamily="var(--font-mono)">
              {name}
            </text>
          </g>
        );
      })}

      {/* Lifeline dashed lines */}
      {lifelineX.map((cx, i) => (
        <line key={i} x1={cx} y1={PADDING.top + HEADER_HEIGHT} x2={cx} y2={height - 8}
          stroke="var(--outline-variant)" strokeDasharray="4 3" strokeWidth={1} />
      ))}

      {/* Current tick highlight */}
      {currentTick != null && collapsed.map((m, i) => {
        if (m.tick !== currentTick) return null;
        const y = bodyTop + i * ROW_HEIGHT;
        return (
          <rect key={`hl-${i}`} x={0} y={y - ROW_HEIGHT / 2 + 2} width={width} height={ROW_HEIGHT}
            fill="var(--sim-active)" opacity={0.10} rx={2} />
        );
      })}

      {/* Messages */}
      {collapsed.map((m, i) => {
        const y = bodyTop + i * ROW_HEIGHT;
        const fi = idxOf.get(m.from) ?? 0;
        const ti = idxOf.get(m.to) ?? 0;
        const isSelf = fi === ti;
        const x1 = lifelineX[fi];
        const x2 = lifelineX[ti];

        const strokeColor = m.kind === 'event'
          ? 'var(--sim-available)'
          : 'var(--on-surface-variant)';

        if (isSelf) {
          // Self-loop: collapsed into a single loop with count
          const lx = x1 + 2;
          const loopLabel = m.count > 1 ? `loop [${m.count}x]` : m.label;
          return (
            <g key={i}>
              {/* Loop fragment box when count > 1 */}
              {m.count > 1 && (
                <>
                  <rect x={lx - 4} y={y - 8} width={SELF_LOOP_W + 60} height={20}
                    fill="none" stroke="var(--outline-variant)" strokeWidth={0.5} strokeDasharray="3,2" rx={2} />
                  <text x={lx - 2} y={y - 10} fill="var(--outline)" fontSize="7" fontFamily="var(--font-mono)">
                    loop
                  </text>
                </>
              )}
              <path
                d={`M${lx},${y - 4} L${lx + SELF_LOOP_W},${y - 4} L${lx + SELF_LOOP_W},${y + 6} L${lx},${y + 6}`}
                fill="none" stroke={strokeColor} strokeWidth={1} markerEnd="url(#seq-arrow)" />
              <text x={lx + SELF_LOOP_W + 4} y={y + 2} fill="var(--on-surface)" fontSize="9"
                fontFamily="var(--font-mono)" dominantBaseline="central">
                {loopLabel}
              </text>
              {m.trigger && m.count === 1 && (
                <text x={lx + SELF_LOOP_W + 4} y={y + 12} fill={m.trigger === 'event' ? 'var(--sim-available)' : 'var(--outline)'}
                  fontSize="7" fontFamily="var(--font-mono)">
                  [{m.trigger}]
                </text>
              )}
            </g>
          );
        }

        const labelX = (x1 + x2) / 2;

        return (
          <g key={i}>
            <line x1={x1} y1={y} x2={x2} y2={y}
              stroke={strokeColor} strokeWidth={1} markerEnd="url(#seq-arrow)" />
            <text x={labelX} y={y - 5} textAnchor="middle" fill="var(--on-surface)" fontSize="9"
              fontFamily="var(--font-mono)">
              {m.label}
            </text>
            {m.trigger && (
              <text x={labelX} y={y - 5 + 10} textAnchor="middle" fontSize="7" fontFamily="var(--font-mono)"
                fill={m.trigger === 'event' ? 'var(--sim-available)' : 'var(--outline)'}>
                [{m.trigger}]
              </text>
            )}
            {/* Tick label in left gutter */}
            <text x={PADDING.left - 6} y={y} textAnchor="end" dominantBaseline="central"
              fill="var(--outline)" fontSize="8" fontFamily="var(--font-mono)">
              {m.tick}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
