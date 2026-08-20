/**
 * stream-status — proof-of-concept rail context (ninebar Phase 1, task #9).
 *
 * The one real registered context this phase ships: a small panel
 * showing the live stream phase, tick, and last error, sourced straight
 * from `sessionLiveStore`'s per-key selectors — active-session only, per
 * `railStore.ts`'s F15 doc comment. The remaining real contexts
 * (variables/breakpoints/diagnostics/causal-trace) are re-homed onto this
 * host by Phase 3.
 */
import { registerRailContext } from '../railRegistry';
import {
  useStreamError,
  useStreamPhase,
  useTick,
} from '@/features/sessions/sessionLiveStore';

function StreamStatusContext() {
  const phase = useStreamPhase();
  const tick = useTick();
  const error = useStreamError();

  return (
    <div
      data-testid="rail-context-stream-status"
      className="flex flex-col gap-2 px-3 py-3"
    >
      <StatusRow testId="stream-status-phase" label="phase" value={phase} />
      <StatusRow
        testId="stream-status-tick"
        label="tick"
        value={tick === null ? '—' : String(tick)}
      />
      <StatusRow
        testId="stream-status-error"
        label="last error"
        value={error ?? 'none'}
        valueColor={error ? 'var(--severity-error)' : undefined}
      />
    </div>
  );
}

function StatusRow({
  testId,
  label,
  value,
  valueColor,
}: {
  testId: string;
  label: string;
  value: string;
  valueColor?: string;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span
        style={{
          fontSize: 'var(--text-xs)',
          color: 'var(--text-muted)',
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
        }}
      >
        {label}
      </span>
      <span
        data-testid={testId}
        className="mono-text"
        style={{
          fontSize: 'var(--text-sm)',
          color: valueColor ?? 'var(--text-primary)',
        }}
      >
        {value}
      </span>
    </div>
  );
}

registerRailContext({
  id: 'stream-status',
  title: 'Stream status',
  icon: 'sensors',
  render: () => <StreamStatusContext />,
});
