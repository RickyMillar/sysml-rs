/**
 * Swimlane timeline (Gantt-style) — shows subsystem states over time.
 * Ported from editors/diagram/src/ui/sim-charts.ts createMiniTimeline().
 * Pure SVG, one horizontal lane per subsystem.
 *
 * Combined-fragment overlays (loop collapse, alt/opt/break boundaries,
 * trigger annotations) are rendered when the corresponding props are supplied.
 */

import type { CollapsedLoop, FragmentBoundary, TransitionTrigger } from '../../features/results/fragmentDetection';

const STATE_COLORS = [
  'var(--chart-series-1)', 'var(--chart-series-2)', 'var(--chart-series-3)', 'var(--chart-series-4)',
  'var(--chart-series-5)', 'var(--chart-series-6)', 'var(--chart-series-7)', 'var(--chart-series-8)',
];

/** Matches a canonical element-id UUID appearing ANYWHERE in the string —
 *  action/occurrence lanes emit states like `<uuid>_initial` / `<uuid>_final`,
 *  so the test must be unanchored to catch the suffixed forms. Used to drop
 *  those lanes (and suppress any stray id label); real state names are
 *  identifiers without an embedded UUID, so they never match. */
const UUID_RE = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function hashColor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = ((hash << 5) - hash + name.charCodeAt(i)) | 0;
  }
  return STATE_COLORS[Math.abs(hash) % STATE_COLORS.length];
}

/** Fragment kind -> subtle boundary color. */
const FRAGMENT_COLORS: Record<string, string> = {
  alt: 'var(--chart-series-6)',
  opt: 'var(--chart-series-3)',
  // "break" signals interrupted/exceptional flow — reuse the severity ladder.
  break: 'var(--severity-error)',
};

export interface TimelineEntry {
  tick: number;
  timeMs: number;
  subsystems: Record<string, string>;
  /** Per-subsystem deferred event counts at this tick (absent = 0). */
  deferredCounts?: Record<string, number>;
}

interface SwimlaneTimelineProps {
  entries: TimelineEntry[];
  currentTick?: number;
  width?: number;
  laneHeight?: number;
  successions?: Array<{ fromSub: string; toSub: string; tick: number; label?: string }>;
  clockRates?: Record<string, number>;
  /** Collapsed loop regions to render as single segments with iteration badges. */
  loops?: CollapsedLoop[];
  /** Fragment boundary markers (alt/opt/break). */
  fragments?: FragmentBoundary[];
  /** Trigger annotations shown on hover at transition points. */
  triggers?: TransitionTrigger[];
  /**
   * Click handler for state segments (ninebar Phase 3 W3-D — the
   * guard-diagnosis drill). When set, segments get a pointer cursor and
   * report their lane + state + tick range on click.
   */
  onSegmentClick?: (info: {
    subsystem: string;
    state: string;
    startTick: number;
    endTick: number;
  }) => void;
}

export function SwimlaneTimeline({ entries, currentTick, width = 600, laneHeight = 22, successions, clockRates, loops, fragments, triggers, onSegmentClick }: SwimlaneTimelineProps) {
  if (entries.length === 0) return null;

  // Discover subsystem names in order
  const subsystemNames: string[] = [];
  for (const entry of entries) {
    for (const name of Object.keys(entry.subsystems)) {
      if (!subsystemNames.includes(name)) subsystemNames.push(name);
    }
  }

  // Filter to SM subsystems. Skip:
  //  - ODE lanes (numeric states), and
  //  - action / occurrence lanes whose "state" is a raw element UUID rather
  //    than a human state name (e.g. exit / destroyStep / ownedPerform). Those
  //    leaked into the timeline as unreadable UUID bars; the swimlane is a
  //    STATE-MACHINE view, so they don't belong here.
  const smSubs = subsystemNames.filter((name) => {
    const states = entries
      .map((e) => e.subsystems[name])
      .filter((s): s is string => !!s)
      .map((s) => s.trim());
    if (states.length === 0) return false;
    // ODE lanes carry numeric states — not a state machine.
    if (!isNaN(parseFloat(states[0]))) return false;
    // Action / occurrence lanes carry element UUIDs as their "state". Drop a
    // lane whose states are predominantly UUIDs (a backstop also suppresses
    // any stray UUID block label below).
    const uuidCount = states.filter((s) => UUID_RE.test(s)).length;
    return uuidCount * 2 < states.length;
  });

  if (smSubs.length === 0) return null;

  const effectiveLaneHeight = clockRates ? Math.max(laneHeight, 28) : laneHeight;
  const pad = { top: 4, left: 80, right: 8, bottom: 4 };
  const plotW = width - pad.left - pad.right;
  const totalHeight = smSubs.length * effectiveLaneHeight + pad.top + pad.bottom;
  // ── One coordinate system: REAL TICKS ──────────────────────────────
  //
  // This axis used to be entry-INDEX space (`plotW / entries.length`) while
  // triggers, successions, fragments and the playhead were all positioned
  // with real `tick` values against that same scale. Entries are sampled
  // (`useStateTimelineIngest` appends one per poll in which the state map
  // changed), so index equals tick only for a run stepped exactly one tick
  // per poll — never during a bulk step. Everything else on the axis was
  // therefore drawn in the wrong place, and the block tooltip reported an
  // index as a "tick": a relief that the runtime records at tick 3819 read
  // as "tick 7" because it was the 7th sampled entry (J3 timeline finding).
  //
  // Ticks are the only coordinate every producer already agrees on, so the
  // axis is now tick-space and the block builder converts its indices once,
  // at the boundary.
  const firstTick = entries[0]?.tick ?? 0;
  const lastTick = entries[entries.length - 1]?.tick ?? firstTick;
  const tickSpan = Math.max(lastTick - firstTick, 1);
  const pxPerTick = plotW / tickSpan;
  /** Tick → x. The single place tick-space becomes pixel-space. */
  const xOfTick = (t: number) => pad.left + (t - firstTick) * pxPerTick;

  return (
    <svg width="100%" height={totalHeight} viewBox={`0 0 ${width} ${totalHeight}`} preserveAspectRatio="xMidYMid meet">
      <defs>
        <marker id="succession-arrow" markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto">
          <path d="M 0 0 L 6 3 L 0 6" fill="none" stroke="var(--sim-active)" strokeWidth="1" />
        </marker>
      </defs>

      {smSubs.map((name, lane) => {
        const y = pad.top + lane * effectiveLaneHeight;

        // Build state blocks
        // State changes are found over entry indices (that is how the samples
        // are ordered), then immediately converted to the tick and time the
        // runtime actually reported for those samples. Nothing downstream
        // sees an index.
        const blocks: Array<{
          startTick: number;
          endTick: number;
          startMs: number;
          endMs: number;
          state: string;
          /** No later sample has closed this state — it is still current. */
          open: boolean;
        }> = [];
        let blockStart = 0;
        let curState = entries[0]?.subsystems[name] ?? '';

        const pushBlock = (startIdx: number, endIdx: number, state: string) => {
          const first = entries[startIdx];
          if (!first) return;
          // A state block runs until the sample that CHANGED it (`endIdx`), not
          // until its own last sample. Sampled state is a step function: all we
          // know is that the subsystem was in `state` at the sample that opened
          // the block and no longer was at the one that closed it, so the block
          // covers the whole gap between them.
          //
          // Ending a block on its own last sample made every state observed
          // exactly once collapse to a 2px sliver reporting "0ms" — which is
          // where the J3 report's "0 ms" came from. It also left visual gaps in
          // a lane whose subsystem is, by definition, always in some state.
          // The final block has no closing sample: the subsystem is still in
          // this state as far as we know. Its duration is UNKNOWN, not zero —
          // reporting 0ms for the state a run ended in is the same lie in the
          // other direction, so it is drawn to the axis end and labelled open.
          const closing = entries[endIdx];
          blocks.push({
            startTick: first.tick,
            endTick: (closing ?? entries[entries.length - 1]).tick,
            startMs: first.timeMs,
            endMs: (closing ?? entries[entries.length - 1]).timeMs,
            state,
            open: closing === undefined,
          });
        };

        for (let i = 1; i <= entries.length; i++) {
          const state = i < entries.length ? (entries[i].subsystems[name] ?? '') : '';
          if (state !== curState || i === entries.length) {
            if (curState) {
              pushBlock(blockStart, i, curState);
            }
            blockStart = i;
            curState = state;
          }
        }

        return (
          <g key={name}>
            {/* Lane label */}
            <text
              x={pad.left - 4}
              y={y + effectiveLaneHeight / 2 + (clockRates?.[name] ? -1 : 3)}
              textAnchor="end"
              fill="var(--on-surface-variant)"
              fontSize="9"
              fontFamily="var(--font-mono)"
            >
              {name.length > 12 ? name.substring(0, 12) + '..' : name}
            </text>

            {/* Clock rate label */}
            {clockRates?.[name] && (
              <text
                x={pad.left - 4}
                y={y + effectiveLaneHeight / 2 + 10}
                textAnchor="end"
                fill="var(--outline)"
                fontSize="7"
                fontFamily="var(--font-mono)"
                opacity={0.6}
              >
                {clockRates[name]}Hz
              </text>
            )}

            {/* State blocks — with loop collapse */}
            {(() => {
              // Find loops that apply to this subsystem
              const laneLoops = loops?.filter((l) => l.subsystem === name) ?? [];

              // Build a set of tick ranges covered by loops (for suppression)
              const suppressedRanges = laneLoops.map((l) => ({
                start: l.startTick,
                end: l.endTick,
              }));

              // `loops` carry REAL ticks (fragmentDetection reads `e.tick`),
              // so this comparison only ever made sense in tick-space — it was
              // being handed block indices.
              const isInsideLoop = (blockStart: number, blockEnd: number) =>
                suppressedRanges.some(
                  (r) => blockStart >= r.start && blockEnd <= r.end,
                );

              // Render normal blocks (those not inside a collapsed loop)
              const normalBlocks = blocks.filter(
                (block) => !isInsideLoop(block.startTick, block.endTick),
              );

              return (
                <>
                  {normalBlocks.map((block, bi) => {
                    const x = xOfTick(block.startTick);
                    // An open block runs to the right edge rather than
                    // collapsing to the minimum sliver at its own start.
                    const w = block.open
                      ? Math.max(plotW - (x - pad.left), 2)
                      : Math.max((block.endTick - block.startTick) * pxPerTick - 1, 2);
                    const color = hashColor(block.state);
                    const durationMs = (block.endMs - block.startMs).toFixed(0);
                    // Never surface a raw element UUID as a state label.
                    const stateLabel = UUID_RE.test(block.state) ? '' : block.state;
                    return (
                      <g key={`b-${bi}`}>
                        <rect
                          x={x}
                          y={y + 1}
                          width={w}
                          height={effectiveLaneHeight - 2}
                          fill={color}
                          rx={2}
                          opacity={0.85}
                          style={onSegmentClick ? { cursor: 'pointer' } : undefined}
                          onClick={
                            onSegmentClick
                              ? () =>
                                  onSegmentClick({
                                    subsystem: name,
                                    state: block.state,
                                    startTick: block.startTick,
                                    endTick: block.endTick,
                                  })
                              : undefined
                          }
                        >
                          {/* "sampled" is not decoration: these bounds are the
                              ticks at which the client OBSERVED the state open
                              and close, and the real transition lies somewhere
                              in the preceding gap. Saying so is the difference
                              between a timeline and a claim. */}
                          <title>
                            {block.open
                              ? `${name}: ${stateLabel || '(step)'} (from tick ${block.startTick}, still current)`
                              : `${name}: ${stateLabel || '(step)'} (tick ${block.startTick}\u2013${block.endTick}, ${durationMs}ms sampled)`}
                          </title>
                        </rect>
                        {w > 30 && stateLabel && (
                          <text
                            x={x + w / 2}
                            y={y + effectiveLaneHeight / 2 + (w > 50 ? -1 : 3)}
                            textAnchor="middle"
                            fill="#fff"
                            fontSize="8"
                            fontFamily="var(--font-mono)"
                            fontWeight={600}
                          >
                            {stateLabel.length > w / 6 ? stateLabel.substring(0, Math.floor(w / 6)) + '..' : stateLabel}
                          </text>
                        )}
                        {w > 50 && (
                          <text
                            x={x + w / 2}
                            y={y + effectiveLaneHeight / 2 + 10}
                            textAnchor="middle"
                            fill="rgba(255,255,255,0.6)"
                            fontSize="7"
                            fontFamily="var(--font-mono)"
                          >
                            {durationMs}ms
                          </text>
                        )}
                      </g>
                    );
                  })}

                  {/* Collapsed loop segments */}
                  {laneLoops.map((loop, li) => {
                    const lx = xOfTick(loop.startTick);
                    const lw = Math.max((loop.endTick - loop.startTick) * pxPerTick - 1, 2);
                    const color = hashColor(loop.pattern[0]);
                    const patternLabel = loop.pattern.join(' \u2192 ');
                    return (
                      <g key={`loop-${li}`}>
                        {/* Striped loop bar */}
                        <rect
                          x={lx} y={y + 1} width={lw} height={effectiveLaneHeight - 2}
                          fill={color} rx={2} opacity={0.65}
                          strokeDasharray="4,2" stroke="rgba(255,255,255,0.3)" strokeWidth={0.5}
                        >
                          <title>{`${name}: loop \u00d7${loop.iterations} [${patternLabel}] (tick ${loop.startTick}\u2013${loop.endTick})`}</title>
                        </rect>
                        {/* Iteration count badge */}
                        <rect
                          x={lx + lw - 20} y={y + 1}
                          width={19} height={11}
                          fill="rgba(0,0,0,0.55)" rx={3}
                        />
                        <text
                          x={lx + lw - 10.5} y={y + 9}
                          textAnchor="middle"
                          fill="#fff"
                          fontSize="7"
                          fontFamily="var(--font-mono)"
                          fontWeight={700}
                        >
                          {`\u00d7${loop.iterations}`}
                        </text>
                        {/* Pattern label if room */}
                        {lw > 50 && (
                          <text
                            x={lx + (lw - 20) / 2}
                            y={y + effectiveLaneHeight / 2 + 3}
                            textAnchor="middle"
                            fill="#fff"
                            fontSize="7"
                            fontFamily="var(--font-mono)"
                            fontWeight={600}
                            opacity={0.85}
                          >
                            {patternLabel.length > lw / 5 ? patternLabel.substring(0, Math.floor(lw / 5)) + '..' : patternLabel}
                          </text>
                        )}
                      </g>
                    );
                  })}
                </>
              );
            })()}

            {/* Trigger annotation markers (diamonds at transition points) */}
            {triggers
              ?.filter((t) => t.subsystem === name)
              .map((t, ti) => {
                const tx = xOfTick(t.tick);
                const ty = y + effectiveLaneHeight - 3;
                return (
                  <g key={`trig-${ti}`}>
                    <polygon
                      points={`${tx},${ty - 3} ${tx + 3},${ty} ${tx},${ty + 3} ${tx - 3},${ty}`}
                      fill="var(--sim-active)"
                      opacity={0.6}
                    >
                      <title>{`${t.event}`}</title>
                    </polygon>
                  </g>
                );
              })}

            {/* Deferred event replay markers — shown when deferred count drops (event consumed) */}
            {entries.map((entry, ei) => {
              if (ei === 0) return null;
              const prevCount = entries[ei - 1].deferredCounts?.[name] ?? 0;
              const curCount = entry.deferredCounts?.[name] ?? 0;
              // A drop in deferred count means one or more deferred events were replayed
              if (prevCount > 0 && curCount < prevCount) {
                // Was `ei * pxPerTick` — an entry index on a tick axis.
                const dx = xOfTick(entry.tick);
                const dy = y + 2;
                return (
                  <g key={`def-${ei}`}>
                    <polygon
                      points={`${dx},${dy} ${dx + 3},${dy + 4} ${dx - 3},${dy + 4}`}
                      fill="var(--chart-annotation)"
                      opacity={0.85}
                    >
                      <title>{`Deferred event replayed at tick ${entry.tick} (queue ${prevCount}\u2192${curCount})`}</title>
                    </polygon>
                  </g>
                );
              }
              return null;
            })}

            {/* Lane separator */}
            <line
              x1={pad.left}
              y1={y + effectiveLaneHeight}
              x2={width - pad.right}
              y2={y + effectiveLaneHeight}
              stroke="var(--outline-variant)"
              strokeWidth={0.5}
              opacity={0.3}
            />
          </g>
        );
      })}

      {/* HappensBefore succession arrows */}
      {successions?.map((s, si) => {
        const fromLane = smSubs.indexOf(s.fromSub);
        const toLane = smSubs.indexOf(s.toSub);
        if (fromLane < 0 || toLane < 0) return null;

        const x = xOfTick(s.tick) + pxPerTick / 2;
        const y1 = pad.top + fromLane * effectiveLaneHeight + effectiveLaneHeight / 2;
        const y2 = pad.top + toLane * effectiveLaneHeight + effectiveLaneHeight / 2;
        const midX = x + 15;

        return (
          <g key={si}>
            <path
              d={`M ${x} ${y1} Q ${midX} ${(y1 + y2) / 2} ${x} ${y2}`}
              fill="none"
              stroke="var(--sim-active)"
              strokeWidth={1}
              opacity={0.5}
              strokeDasharray="3,2"
              markerEnd="url(#succession-arrow)"
            />
            {s.label && (
              <text
                x={midX + 2}
                y={(y1 + y2) / 2 + 3}
                fill="var(--sim-active)"
                fontSize="7"
                fontFamily="var(--font-mono)"
                opacity={0.7}
              >
                {s.label}
              </text>
            )}
          </g>
        );
      })}

      {/* Fragment boundary markers (vertical lines spanning all lanes) */}
      {fragments?.map((frag, fi) => {
        const fx = xOfTick(frag.tick);
        const color = FRAGMENT_COLORS[frag.kind] ?? '#888';
        return (
          <g key={`frag-${fi}`}>
            <line
              x1={fx} y1={pad.top}
              x2={fx} y2={totalHeight - pad.bottom}
              stroke={color}
              strokeWidth={1}
              strokeDasharray="3,3"
              opacity={0.45}
            />
            <rect
              x={fx + 1} y={pad.top - 1}
              width={Math.min(frag.label.length * 5 + 6, 70)} height={10}
              fill={color} rx={2} opacity={0.2}
            />
            <text
              x={fx + 4} y={pad.top + 7}
              fill={color}
              fontSize="7"
              fontFamily="var(--font-mono)"
              fontWeight={600}
              opacity={0.8}
            >
              {frag.label.length > 12 ? frag.label.substring(0, 12) + '..' : frag.label}
            </text>
          </g>
        );
      })}

      {/* Playhead */}
      {currentTick !== undefined && entries.length > 1 && (
        <line
          x1={xOfTick(currentTick)}
          y1={pad.top}
          x2={xOfTick(currentTick)}
          y2={totalHeight - pad.bottom}
          stroke="var(--sim-active)"
          strokeWidth={1.5}
          opacity={0.8}
        />
      )}
    </svg>
  );
}
