/**
 * OdeDetail — live view of an integrated ODE state variable.
 *
 * Replaces the GAP-ODE-002 placeholder with three concrete surfaces:
 *   1. Current integrated value + current dy/dt projected from the
 *      backend's `NormalizedSnapshot.derivatives` map (see
 *      snapshot_view.rs — GAP-ODE-002).
 *   2. Sparkline history of dy/dt over the last ~64 ticks, maintained
 *      via a per-component ring buffer. State-value history stays in
 *      `useTimeSeriesStore` and piggy-backs the existing scalar_vars
 *      bridge.
 *   3. A compact phase portrait when a sibling ODE state variable
 *      exists under the same parent part (y₁ vs y₂), again sampled
 *      from the time-series store.
 */
import { useEffect, useMemo, useRef } from 'react';
import { Sparkline } from '@/features/variables/Sparkline';
import { formatVariableValue } from '@/features/variables/VariableTree';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { useSessionLiveStore } from '../../sessionLiveStore';
import type { OdeTreeNode } from '../types';
import { DetailMeta, DetailShell } from './common';

/** Max samples kept in the dy/dt ring buffer per component instance. */
const DERIV_RING_SIZE = 64;

/** Max points drawn in the phase portrait trail. */
const PHASE_TRAIL_LEN = 80;

export function OdeDetail({
  node,
  testIdPrefix,
}: {
  node: OdeTreeNode;
  testIdPrefix: string;
}) {
  const fullName = node.ownerPath
    ? `${node.ownerPath}.${node.name}`
    : node.name;

  const dydt = useSessionLiveStore(
    (s) => s.snapshot?.derivatives?.[fullName],
  );
  const currentScalar = useSessionLiveStore(
    (s) => s.snapshot?.scalar_vars[fullName],
  );
  // Allow the value baked into the tree node to act as a fallback
  // pre-session (`useSessionModelTree` seeds it from the workspace).
  const displayValue = currentScalar ?? node.value;

  const derivativeSamples = useDerivativeRingBuffer(fullName, dydt);

  const sibling = useSiblingOdeState(node);

  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="ode">
      <DetailMeta node={node} extra={`integrator: ${node.status ?? 'unknown'}`} />

      <div
        className="flex items-baseline gap-3"
        data-testid={`${testIdPrefix}-ode-readout`}
      >
        <span
          className="mono-text"
          style={{
            fontSize: 22,
            color: 'var(--on-surface)',
            fontWeight: 600,
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {displayValue === undefined || displayValue === null
            ? '—'
            : formatVariableValue(displayValue ?? null, node.unit)}
        </span>
        <span
          data-testid={`${testIdPrefix}-ode-dydt`}
          className="mono-text"
          style={{
            fontSize: 12,
            color:
              dydt === undefined
                ? 'var(--outline)'
                : 'var(--on-surface-variant)',
            fontVariantNumeric: 'tabular-nums',
          }}
          title={
            dydt === undefined
              ? 'Awaiting first integrator step'
              : `Instantaneous dy/dt at tick boundary`
          }
        >
          d/dt = {dydt === undefined ? '—' : formatDydt(dydt, node.unit)}
        </span>
      </div>

      <DerivativeHistory
        samples={derivativeSamples}
        testId={`${testIdPrefix}-ode-deriv-spark`}
      />

      {sibling && (
        <PhasePortrait
          xName={fullName}
          yName={sibling.fullName}
          xLabel={node.name}
          yLabel={sibling.label}
          testId={`${testIdPrefix}-ode-phase`}
        />
      )}
    </DetailShell>
  );
}

// ─── Helpers ──────────────────────────────────────────────────────────

function formatDydt(v: number, unit?: string): string {
  if (!Number.isFinite(v)) return '—';
  const abs = Math.abs(v);
  const body =
    abs === 0
      ? '0'
      : abs < 1e-3 || abs >= 1e6
        ? v.toExponential(3)
        : v.toPrecision(4).replace(/\.?0+$/, '');
  return unit ? `${body} ${unit}/s` : `${body} /s`;
}

/**
 * Per-component ring buffer for dy/dt samples. Resets when the
 * observed name changes (user focuses a different ODE) so the
 * sparkline never mixes histories from two variables. Skips `null`
 * / `undefined` ticks so a transient pre-session render doesn't
 * contaminate the series.
 */
function useDerivativeRingBuffer(
  name: string,
  current: number | undefined,
): number[] {
  const ringRef = useRef<{ name: string; samples: number[] }>({
    name,
    samples: [],
  });

  useEffect(() => {
    if (ringRef.current.name !== name) {
      ringRef.current = { name, samples: [] };
    }
    if (current === undefined || !Number.isFinite(current)) return;
    const { samples } = ringRef.current;
    // Avoid pushing the same value repeatedly between non-tick
    // re-renders — dy/dt only legitimately repeats in the exact
    // steady-state case, and the sparkline is visually identical
    // either way so the cheap dedup is fine.
    if (samples.length > 0 && samples[samples.length - 1] === current) return;
    samples.push(current);
    if (samples.length > DERIV_RING_SIZE) {
      samples.splice(0, samples.length - DERIV_RING_SIZE);
    }
  }, [name, current]);

  return ringRef.current.samples;
}

/**
 * Find a peer ODE state variable under the same parent part. If the
 * focused ODE lives at `station1.groupHead.temperature`, a sibling is any
 * derivative key of the form `station1.groupHead.*` other than
 * `temperature`. We take the first match (alphabetically) for
 * determinism — a richer UX would let the user pick, but that's
 * beyond the current scope.
 */
function useSiblingOdeState(
  node: OdeTreeNode,
): { fullName: string; label: string } | null {
  // Select the derivatives map itself (stable reference across
  // re-renders when unchanged) rather than a new `Object.keys`
  // array per call — the latter pattern triggers zustand's
  // "should be cached" warning and can spin into an infinite loop.
  const derivatives = useSessionLiveStore((s) => s.snapshot?.derivatives);
  return useMemo(() => {
    if (!derivatives) return null;
    const fullName = node.ownerPath
      ? `${node.ownerPath}.${node.name}`
      : node.name;
    const parent = node.ownerPath;
    const prefix = parent ? `${parent}.` : '';
    const candidates: string[] = [];
    for (const key of Object.keys(derivatives)) {
      if (key === fullName) continue;
      if (prefix && !key.startsWith(prefix)) continue;
      const rest = prefix ? key.slice(prefix.length) : key;
      // Direct siblings only (no nested sub-part paths). A nested
      // ODE would carry one more dot-segment and should not pair up
      // with this one.
      if (rest.includes('.')) continue;
      candidates.push(key);
    }
    if (candidates.length === 0) return null;
    candidates.sort();
    const siblingKey = candidates[0];
    const label = prefix
      ? siblingKey.slice(prefix.length)
      : siblingKey;
    return { fullName: siblingKey, label };
  }, [derivatives, node.name, node.ownerPath]);
}

// ─── Subcomponents ────────────────────────────────────────────────────

function DerivativeHistory({
  samples,
  testId,
}: {
  samples: number[];
  testId: string;
}) {
  if (samples.length < 3) {
    return (
      <div
        data-testid={`${testId}-empty`}
        style={{
          fontSize: 10,
          color: 'var(--outline)',
          fontStyle: 'italic',
        }}
      >
        Gathering dy/dt samples…
      </div>
    );
  }
  return (
    <div data-testid={testId}>
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--outline)',
          marginBottom: 2,
        }}
      >
        dy/dt history
      </div>
      <Sparkline
        samples={samples}
        width={180}
        height={28}
        color="var(--primary)"
        ariaLabel="dy/dt sparkline"
      />
    </div>
  );
}

function PhasePortrait({
  xName,
  yName,
  xLabel,
  yLabel,
  testId,
}: {
  xName: string;
  yName: string;
  xLabel: string;
  yLabel: string;
  testId: string;
}) {
  const tsRevision = useTimeSeriesStore((s) => s.revision);
  const pair = useMemo(() => {
    const all = useTimeSeriesStore.getState().getTimeSeries();
    const xs = all[xName] ?? [];
    const ys = all[yName] ?? [];
    if (xs.length < 2 || ys.length < 2) return null;
    // Pair timestamps that match. The bridge pushes one point per
    // tick for all scalar_vars, so they align by index in practice —
    // but be defensive and fall back to shared length.
    const n = Math.min(xs.length, ys.length);
    const start = Math.max(0, n - PHASE_TRAIL_LEN);
    const points: Array<[number, number]> = [];
    for (let i = start; i < n; i++) {
      points.push([xs[i].v, ys[i].v]);
    }
    return points;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tsRevision, xName, yName]);

  if (!pair || pair.length < 2) return null;

  const xs = pair.map((p) => p[0]);
  const ys = pair.map((p) => p[1]);
  const xMin = Math.min(...xs);
  const xMax = Math.max(...xs);
  const yMin = Math.min(...ys);
  const yMax = Math.max(...ys);
  const xRange = xMax - xMin || 1;
  const yRange = yMax - yMin || 1;

  const width = 180;
  const height = 120;
  const pad = 6;
  const innerW = width - pad * 2;
  const innerH = height - pad * 2;

  const toCoords = ([x, y]: [number, number]) => {
    const cx = pad + ((x - xMin) / xRange) * innerW;
    // SVG y grows downward — invert.
    const cy = pad + innerH - ((y - yMin) / yRange) * innerH;
    return [cx, cy];
  };

  const pathData = pair
    .map((p, i) => {
      const [cx, cy] = toCoords(p);
      return `${i === 0 ? 'M' : 'L'}${cx.toFixed(2)},${cy.toFixed(2)}`;
    })
    .join(' ');
  const lastPoint = toCoords(pair[pair.length - 1]);

  return (
    <div data-testid={testId}>
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--outline)',
          marginBottom: 2,
        }}
      >
        Phase portrait · {yLabel} vs {xLabel}
      </div>
      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={`Phase portrait of ${yLabel} versus ${xLabel}`}
        style={{
          border: '1px solid var(--outline-variant)',
          borderRadius: 3,
          background: 'var(--surface-container)',
        }}
      >
        <path
          d={pathData}
          fill="none"
          stroke="var(--primary)"
          strokeWidth={1.25}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        <circle
          cx={lastPoint[0]}
          cy={lastPoint[1]}
          r={3}
          fill="var(--primary)"
          data-testid={`${testId}-head`}
        />
      </svg>
    </div>
  );
}
