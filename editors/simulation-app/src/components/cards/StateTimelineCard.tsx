/**
 * StateTimelineCard — horizontal swimlane timeline in the Results Strip.
 *
 * Shows state machine instances as horizontal bars with colored state segments.
 * Includes combined-fragment visualization: loop collapse with iteration
 * badges, alt/opt/break boundary markers, and trigger annotations on hover.
 */

import { useRef, useCallback, useMemo, useState, useEffect } from 'react';
import { CardShell } from './CardShell';
import type { ExportAction } from './CardShell';
import { SwimlaneTimeline } from '../charts/SwimlaneTimeline';
import { exportPNG } from '../../shared/export';
import type { TimelineEntry } from '../../features/sessions/types';
import { detectLoops, detectFragments, detectTriggers } from '../../features/results/fragmentDetection';

interface StateTimelineCardProps {
  entries: TimelineEntry[];
  currentStep: number;
  running: boolean;
  expanded?: boolean;
  onHeaderClick?: () => void;
}

export function StateTimelineCard({ entries, currentStep, running, expanded, onHeaderClick }: StateTimelineCardProps) {
  const hasData = entries.length > 0;
  const chartRef = useRef<HTMLDivElement>(null);

  // Track the available width so the swimlane fills the panel instead of
  // rendering at a fixed 380px viewBox letterboxed in the centre of a wide
  // results strip. ResizeObserver keeps it correct across panel resize /
  // expand. The SVG is width:100%, so observing the wrapper never feeds back.
  const [chartWidth, setChartWidth] = useState(380);
  useEffect(() => {
    const el = chartRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver((obs) => {
      const w = obs[0]?.contentRect.width ?? 0;
      if (w > 0) setChartWidth(Math.max(Math.floor(w), 200));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [hasData]);

  // Heuristic combined-fragment detection (memoized — only recomputes when entries change)
  const loops = useMemo(() => detectLoops(entries), [entries]);
  const fragments = useMemo(() => detectFragments(entries), [entries]);
  const triggers = useMemo(() => detectTriggers(entries), [entries]);

  const handleExportPNG = useCallback(() => {
    if (chartRef.current) exportPNG(chartRef.current, 'state-timeline.png');
  }, []);

  const exportActions: ExportAction[] = hasData
    ? [{ label: 'Export PNG', icon: 'image', onClick: handleExportPNG }]
    : [];

  return (
    <CardShell title="State Timeline" icon="timeline" accentColor="var(--text-secondary)" expanded={expanded} onHeaderClick={onHeaderClick} exportActions={exportActions}>
      {!hasData ? (
        <div style={{ fontSize: 'var(--text-xs)', color: 'var(--outline)' }}>
          {running ? 'Waiting for state transitions...' : 'No state machines active. Run a simulation to see the timeline.'}
        </div>
      ) : (
        <div ref={chartRef}>
          <SwimlaneTimeline
            entries={entries}
            currentTick={currentStep}
            width={chartWidth}
            laneHeight={22}
            loops={loops}
            fragments={fragments}
            triggers={triggers}
          />
        </div>
      )}
    </CardShell>
  );
}
