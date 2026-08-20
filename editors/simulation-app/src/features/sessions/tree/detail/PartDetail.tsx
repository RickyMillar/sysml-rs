/**
 * PartDetail — one-glance health + key-signal surfaces for a focused
 * part. Replaces the old counts-only placeholder.
 *
 * Round 2 Task #146.
 */
import { useMemo } from 'react';
import { Sparkline } from '@/features/variables/Sparkline';
import { formatVariableValue } from '@/features/variables/VariableTree';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { useSessionLiveStore } from '../../sessionLiveStore';
import type { AttributeTreeNode, PartTreeNode } from '../types';
import { useSessionStore } from '../../store';
import { DetailMeta, DetailShell } from './common';
import {
  aggregateHealth,
  countArchetypes,
  pickKeySignals,
  type HealthLevel,
} from './partHealth';
import { promoteToPlots } from './promoteToPlots';

// Token sweep judgment call: despite the `HealthLevel` type name, the
// value vocabulary here (pass/fail/inconclusive) is verdict semantics,
// not physics-health semantics — tokens.css's --health-* ladder never
// uses "pass/fail/inconclusive" wording. 'unobserved' reuses
// --verdict-inconclusive per tokens.css's own comment: "doubles as
// not-yet-evaluated".
const HEALTH_COLOR: Record<HealthLevel, string> = {
  pass: 'var(--verdict-pass)',
  fail: 'var(--verdict-fail)',
  inconclusive: 'var(--verdict-inconclusive)',
  unobserved: 'var(--verdict-inconclusive)',
};

const HEALTH_LABEL: Record<HealthLevel, string> = {
  pass: 'All constraints pass',
  fail: 'One or more constraints fail',
  inconclusive: 'Inconclusive constraints',
  unobserved: 'No constraint verdicts yet',
};

export function PartDetail({
  node,
  testIdPrefix,
}: {
  node: PartTreeNode;
  testIdPrefix: string;
}) {
  const health = useMemo(() => aggregateHealth(node), [node]);
  const signals = useMemo(() => pickKeySignals(node, 4), [node]);
  const counts = useMemo(() => countArchetypes(node), [node]);

  const summary = [
    counts.subParts > 0 &&
      `${counts.subParts} sub-part${counts.subParts === 1 ? '' : 's'}`,
    counts.attributes > 0 &&
      `${counts.attributes} attr${counts.attributes === 1 ? '' : 's'}`,
    counts.stateMachines > 0 &&
      `${counts.stateMachines} SM${counts.stateMachines === 1 ? '' : 's'}`,
    counts.constraints > 0 &&
      `${counts.constraints} constraint${counts.constraints === 1 ? '' : 's'}`,
    counts.odes > 0 && `${counts.odes} ODE${counts.odes === 1 ? '' : 's'}`,
  ]
    .filter(Boolean)
    .join(' · ');

  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="part">
      <DetailMeta node={node} />

      <div
        data-testid={`${testIdPrefix}-part-health`}
        data-health={health}
        className="flex items-center gap-2"
        title={HEALTH_LABEL[health]}
      >
        <span
          aria-label={HEALTH_LABEL[health]}
          style={{
            display: 'inline-block',
            width: 10,
            height: 10,
            borderRadius: '50%',
            background: HEALTH_COLOR[health],
            boxShadow:
              health === 'unobserved'
                ? 'inset 0 0 0 1px var(--border-default)'
                : undefined,
          }}
        />
        <span style={{ fontSize: 11, color: 'var(--text-secondary)' }}>
          {HEALTH_LABEL[health]}
        </span>
        <span
          className="mono-text"
          style={{
            marginLeft: 'auto',
            fontSize: 9,
            color: 'var(--text-muted)',
          }}
        >
          {summary || 'empty'}
        </span>
      </div>

      <KeySignals signals={signals} testIdPrefix={testIdPrefix} />

      <Connections node={node} testIdPrefix={testIdPrefix} />
    </DetailShell>
  );
}

// ─── Connections (live port feature values) ──────────────────────────

/**
 * Pick port_values entries belonging to a given part. The runtime keys
 * them as `owner.port` where `owner` is the part instance path that
 * the runtime emits — usually matches the tree's `ownerPath.name` but
 * may also be a bare instance name for top-level parts. We match both
 * forms so the section populates whether the snapshot uses the fully-
 * qualified or bare key.
 */
export function collectPartPorts(
  node: PartTreeNode,
  portValues: Record<string, Record<string, number>> | undefined,
): Array<{ key: string; portName: string; features: Record<string, number> }> {
  if (!portValues) return [];
  const fullName = node.ownerPath ? `${node.ownerPath}.${node.name}` : node.name;
  const prefixes = new Set<string>([`${fullName}.`, `${node.name}.`]);
  const rows: Array<{
    key: string;
    portName: string;
    features: Record<string, number>;
  }> = [];
  for (const [key, features] of Object.entries(portValues)) {
    let matchedPortName: string | null = null;
    for (const p of prefixes) {
      if (key.startsWith(p)) {
        matchedPortName = key.slice(p.length);
        // Skip if the remaining segment contains another '.' — that
        // means the key belongs to a nested part, not this one.
        if (!matchedPortName.includes('.')) break;
        matchedPortName = null;
      }
    }
    if (matchedPortName !== null) {
      rows.push({ key, portName: matchedPortName, features });
    }
  }
  rows.sort((a, b) => a.portName.localeCompare(b.portName));
  return rows;
}

function Connections({
  node,
  testIdPrefix,
}: {
  node: PartTreeNode;
  testIdPrefix: string;
}) {
  const portValues = useSessionLiveStore((s) => s.snapshot?.port_values);
  const rows = useMemo(() => collectPartPorts(node, portValues), [node, portValues]);

  if (rows.length === 0) {
    return (
      <div
        data-testid={`${testIdPrefix}-part-connections-empty`}
        style={{
          fontSize: 11,
          color: 'var(--text-muted)',
          border: '1px dashed var(--border-default)',
          padding: 8,
          borderRadius: 4,
        }}
      >
        No port flow values observed for this part yet.
      </div>
    );
  }

  return (
    <div data-testid={`${testIdPrefix}-part-connections`}>
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--text-muted)',
          marginBottom: 4,
        }}
      >
        Connections
      </div>
      <div style={{ display: 'grid', gap: 6 }}>
        {rows.map((row) => (
          <PortRow
            key={row.key}
            testId={`${testIdPrefix}-part-port-${row.portName}`}
            portName={row.portName}
            features={row.features}
          />
        ))}
      </div>
    </div>
  );
}

function PortRow({
  testId,
  portName,
  features,
}: {
  testId: string;
  portName: string;
  features: Record<string, number>;
}) {
  const entries = Object.entries(features).sort(([a], [b]) =>
    a.localeCompare(b),
  );
  return (
    <div
      data-testid={testId}
      style={{
        padding: '4px 6px',
        border: '1px solid var(--border-default)',
        borderRadius: 3,
        background: 'var(--surface-panel)',
      }}
    >
      <div
        className="mono-text"
        style={{ fontSize: 11, color: 'var(--text-primary)', marginBottom: 2 }}
      >
        {portName}
      </div>
      <div style={{ display: 'grid', gap: 2 }}>
        {entries.map(([feature, value]) => (
          <div
            key={feature}
            data-testid={`${testId}-feat-${feature}`}
            className="flex items-center"
            style={{ fontSize: 10 }}
          >
            <span
              style={{
                color: 'var(--text-secondary)',
                flex: '1 1 auto',
                minWidth: 0,
              }}
              className="truncate"
            >
              {feature}
            </span>
            <span
              className="mono-text"
              style={{
                color: 'var(--text-primary)',
                fontVariantNumeric: 'tabular-nums',
                textAlign: 'right',
              }}
            >
              {formatPortValue(value)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function formatPortValue(v: number): string {
  if (!Number.isFinite(v)) return '—';
  if (v === 0) return '0';
  const abs = Math.abs(v);
  if (abs < 0.001 || abs >= 1e6) return v.toExponential(3);
  return v.toPrecision(5).replace(/\.?0+$/, '');
}

function KeySignals({
  signals,
  testIdPrefix,
}: {
  signals: { node: AttributeTreeNode; lastChangedTick: number }[];
  testIdPrefix: string;
}) {
  if (signals.length === 0) {
    return (
      <div
        data-testid={`${testIdPrefix}-part-signals-empty`}
        style={{
          fontSize: 11,
          color: 'var(--text-muted)',
          border: '1px dashed var(--border-default)',
          padding: 8,
          borderRadius: 4,
        }}
      >
        No live attribute signals yet. Run the session to populate.
      </div>
    );
  }
  return (
    <div
      data-testid={`${testIdPrefix}-part-signals`}
      style={{
        display: 'grid',
        gridTemplateColumns: '1fr',
        gap: 6,
      }}
    >
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--text-muted)',
        }}
      >
        Key signals
      </div>
      {signals.map((s) => (
        <SignalRow key={s.node.id} signal={s} />
      ))}
    </div>
  );
}

function SignalRow({
  signal,
}: {
  signal: { node: AttributeTreeNode; lastChangedTick: number };
}) {
  const { node } = signal;
  const tsRevision = useTimeSeriesStore((s) => s.revision);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const fullName = node.ownerPath
    ? `${node.ownerPath}.${node.name}`
    : node.name;
  const samples = useMemo(() => {
    const map = useTimeSeriesStore.getState().getTimeSeries();
    const points = map[fullName] ?? map[node.name] ?? [];
    return points.map((p) => p.v);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tsRevision, node.id, node.name, node.ownerPath]);

  return (
    <div
      data-testid={`part-signal-${node.id}`}
      className="flex items-center gap-2"
      style={{
        padding: '4px 6px',
        border: '1px solid var(--border-default)',
        borderRadius: 3,
        background: 'var(--surface-panel)',
      }}
    >
      <span
        className="truncate"
        style={{
          fontSize: 11,
          color: 'var(--text-primary)',
          flex: '1 1 auto',
          minWidth: 0,
        }}
        title={fullName}
      >
        {node.name}
      </span>
      {samples.length >= 2 ? (
        <button
          type="button"
          data-testid={`part-signal-${node.id}-spark-btn`}
          title="Click to add to Plots"
          onClick={(e) => {
            e.stopPropagation();
            promoteToPlots(fullName, activeSessionId);
          }}
          style={{
            border: 'none',
            background: 'transparent',
            padding: 0,
            cursor: 'pointer',
            display: 'inline-flex',
            alignItems: 'center',
          }}
        >
          <Sparkline
            samples={samples}
            width={80}
            height={18}
            color="var(--chart-series-1)"
            ariaLabel={`${node.name} sparkline — click to add to Plots`}
          />
        </button>
      ) : (
        <span
          style={{ fontSize: 9, color: 'var(--text-muted)', fontStyle: 'italic' }}
        >
          {samples.length === 0 ? 'no samples' : '1 sample'}
        </span>
      )}
      <span
        className="mono-text"
        style={{
          fontSize: 11,
          color: 'var(--text-primary)',
          fontVariantNumeric: 'tabular-nums',
          minWidth: 50,
          textAlign: 'right',
        }}
      >
        {node.value === undefined
          ? '—'
          : formatVariableValue(node.value ?? null, node.unit)}
      </span>
    </div>
  );
}
