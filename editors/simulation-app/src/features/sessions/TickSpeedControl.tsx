/**
 * TickSpeedControl — compact button group for picking the autoplay tick rate.
 *
 * Bound to useSessionStore.stepsPerSecond. Default 10 sps (1×). Mappings:
 *   0.5× -> 5 sps, 1× -> 10 sps, 2× -> 20 sps, 5× -> 50 sps, 10× -> 100 sps
 *
 * useSessionController consumes stepsPerSecond as `interval = 1000 / sps`.
 */

import { useSessionStore } from './store';

const SPEED_OPTIONS: { label: string; sps: number }[] = [
  { label: '0.5x', sps: 5 },
  { label: '1x', sps: 10 },
  { label: '2x', sps: 20 },
  { label: '5x', sps: 50 },
  { label: '10x', sps: 100 },
];

export function TickSpeedControl() {
  const stepsPerSecond = useSessionStore((s) => s.stepsPerSecond);
  const setStepsPerSecond = useSessionStore((s) => s.setStepsPerSecond);

  return (
    <div
      data-testid="tick-speed-control"
      className="flex items-center"
      style={{
        background: 'var(--surface-container-high)',
        borderRadius: 4,
        padding: 2,
        gap: 1,
      }}
      title={`Tick rate: ${stepsPerSecond} steps/sec`}
    >
      {SPEED_OPTIONS.map((opt) => {
        const active = stepsPerSecond === opt.sps;
        return (
          <button
            key={opt.label}
            onClick={() => setStepsPerSecond(opt.sps)}
            data-testid={`tick-speed-${opt.label}`}
            className="px-2 py-0.5 rounded transition-all mono-text"
            style={{
              background: active ? 'var(--primary)' : 'transparent',
              color: active ? 'var(--on-primary)' : 'var(--on-surface)',
              border: 'none',
              cursor: 'pointer',
              fontSize: '11px',
              fontWeight: 600,
              minWidth: 30,
            }}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
