/**
 * Pure normalization functions for session payloads.
 *
 * These take raw backend responses and produce typed, frontend-friendly
 * structures. No side effects, no store access, no react-query — just
 * data transformation.
 */

import type {
  NormalizedSnapshot,
  NormalizedTopology,
  TimeSeriesPoint,
  SessionRecord,
  SessionSummary,
} from './types';
import type { SystemTopology } from '../../types/physics';

// ── Value extraction ─────────────────────────────────────────────────

/**
 * Extract a numeric or string value from a serde(untagged) `Value` enum.
 *
 * The backend `Value` enum serializes with `#[serde(untagged)]`:
 * - Bool(true)          → true         (JSON boolean)
 * - Int(5)              → 5            (JSON number)
 * - Float(3.14)         → 3.14         (JSON number)
 * - String("hi")        → "hi"         (JSON string)
 * - Enum("Active")      → "Active"     (JSON string)
 * - Null                → null         (JSON null)
 * - Quantity{value,..}  → {value: 3.14, dimension: [...], unit: "A"} (JSON object)
 * - Complex{re, im}    → {re: 1.0, im: 2.0}  (JSON object)
 * - Ref(id)             → "<uuid>"     (JSON string — ElementId serializes as string)
 * - List([...])         → [...]        (JSON array)
 * - Map({...})          → {...}        (JSON object)
 */
function extractNumericValue(v: unknown): number | string {
  if (typeof v === 'number') return v;
  if (typeof v === 'boolean') return v ? 1 : 0;
  if (typeof v === 'string') return v;
  if (v == null) return 'null';
  // Quantity object: { value: number, dimension: number[], unit?: string }
  if (typeof v === 'object' && !Array.isArray(v)) {
    const obj = v as Record<string, unknown>;
    if (typeof obj.value === 'number' && 'dimension' in obj) {
      return obj.value;
    }
    // Complex: { re: number, im: number }
    if (typeof obj.re === 'number' && typeof obj.im === 'number') {
      return obj.re; // Use real part for time-series
    }
  }
  return String(v);
}

// ── Snapshot normalization ────────────────────────────────────────────

/**
 * Normalize a raw ExecutionSnapshot (from sessions.info.latest_snapshot)
 * into a typed NormalizedSnapshot.
 */
export function normalizeSnapshot(raw: Record<string, unknown> | null): NormalizedSnapshot | null {
  if (!raw) return null;

  const tick = (raw.tick as number) ?? 0;
  const timeMs = (raw.time_ms as number) ?? 0;
  const completed = !!raw.completed;

  // Subsystem states — backend field is `subsystem_states` (not `subsystems`)
  const rawSubsystems = (raw.subsystem_states as Record<string, Record<string, unknown>>) ?? {};
  const subsystems: NormalizedSnapshot['subsystems'] = {};
  for (const [name, sub] of Object.entries(rawSubsystems)) {
    subsystems[name] = {
      currentState: (sub.current_state as string) ?? '',
      completed: !!sub.completed,
      kindLabel: (sub.kind_label as string) ?? 'unknown',
    };
  }

  // Variables — backend field is `variables` (not `values`).
  // Values are serde(untagged) `Value` enums: most serialize as plain JSON
  // primitives (number, bool, string, null), but compound variants like
  // Quantity {value, dimension, unit} and Complex {re, im} serialize as objects.
  const rawVariables = (raw.variables as Record<string, unknown>) ?? {};
  const variables: Record<string, number | string> = {};
  for (const [k, v] of Object.entries(rawVariables)) {
    if (k.startsWith('__') || k === 't_ms' || k === 'tick') continue;
    variables[k] = extractNumericValue(v);
  }

  // Constraint results — backend `ConstraintEvalResult` has `expression: Option<String>`
  // (nullable), no separate `message` field.
  const rawConstraints = (raw.constraint_results as Array<Record<string, unknown>>) ?? [];
  const constraintResults: NormalizedSnapshot['constraintResults'] = rawConstraints.map((c) => ({
    name: (c.name as string) ?? '',
    expression: (c.expression as string) ?? '',
    satisfied: !!c.satisfied,
  }));

  return { tick, timeMs, completed, subsystems, variables, constraintResults };
}

// ── Topology normalization ────────────────────────────────────────────

/**
 * Convert a raw backend topology payload (snake_case JSON fields) into
 * the camelCase `SystemTopology` TypeScript type used throughout the frontend.
 *
 * Backend sends: root_label, domain_summaries, current_state, key_metrics, element_id
 * Frontend uses: rootLabel, domainSummaries, currentState, keyMetrics, element_id
 *
 * element_id stays snake_case in both (matches the TS interface).
 */
export function normalizeTopologyPayload(raw: Record<string, unknown> | null): SystemTopology | null {
  if (!raw) return null;

  const rawModules = (raw.modules as Array<Record<string, unknown>>) ?? [];
  const rawDomainSummaries = (raw.domain_summaries as Array<Record<string, unknown>>) ?? [];

  const modules = rawModules.map((m): SystemTopology['modules'][number] => {
    const rawSubs = (m.subsystems as Array<Record<string, unknown>>) ?? [];
    const rawHealth = (m.health as Record<string, unknown>) ?? {};

    return {
      id: (m.id as string) ?? '',
      label: (m.label as string) ?? '',
      rating: (m.rating as string | undefined),
      element_id: (m.element_id as string | undefined),
      domain: (m.domain as string) ?? 'uncategorized',
      subsystems: rawSubs.map((s) => {
        const sHealth = (s.health as Record<string, unknown>) ?? {};
        const sThresholds = s.thresholds as Record<string, unknown> | undefined;
        return {
          name: (s.name as string) ?? '',
          kind: ((s.kind as string) ?? 'sm') as 'sm' | 'ode' | 'action' | 'discrete',
          domain: (s.domain as string) ?? 'uncategorized',
          element_id: (s.element_id as string | undefined),
          currentState: (s.current_state as string) ?? '',
          sparkline: (s.sparkline as number[]) ?? [],
          health: {
            status: ((sHealth.status as string) ?? 'nominal') as 'nominal' | 'warning' | 'critical',
            message: sHealth.message as string | undefined,
          },
          thresholds: sThresholds ? {
            warnValue: sThresholds.warn_value as number | undefined,
            criticalValue: sThresholds.critical_value as number | undefined,
            ratedValue: sThresholds.rated_value as number | undefined,
            unit: sThresholds.unit as string | undefined,
          } : undefined,
        };
      }),
      health: {
        status: ((rawHealth.status as string) ?? 'nominal') as 'nominal' | 'warning' | 'critical',
        message: rawHealth.message as string | undefined,
      },
    };
  });

  const domainSummaries = rawDomainSummaries.map((d) => {
    const rawMetrics = (d.key_metrics as Array<Record<string, unknown>>) ?? [];
    return {
      domain: (d.domain as string) ?? '',
      status: ((d.status as string) ?? 'nominal') as 'nominal' | 'warning' | 'critical',
      message: (d.message as string) ?? '',
      keyMetrics: rawMetrics.map((m) => ({
        label: (m.label as string) ?? '',
        value: (m.value as string) ?? '',
        unit: m.unit as string | undefined,
        status: m.status as 'nominal' | 'warning' | 'critical' | undefined,
      })),
    };
  });

  return {
    rootLabel: (raw.root_label as string) ?? '',
    modules,
    domainSummaries,
  };
}

/**
 * Normalize a typed SystemTopology into a lighter NormalizedTopology.
 */
export function normalizeTopology(raw: SystemTopology | null): NormalizedTopology | null {
  if (!raw) return null;

  return {
    rootLabel: raw.rootLabel,
    modules: raw.modules.map((m) => ({
      id: m.id,
      label: m.label,
      domain: m.domain,
      subsystemNames: m.subsystems.map((s) => s.name),
    })),
  };
}

// ── Time series accumulation ──────────────────────────────────────────

/**
 * Append data from a snapshot to existing time-series buffers.
 * Returns a new object (does not mutate `existing`).
 */
export function appendTimeSeries(
  existing: Record<string, TimeSeriesPoint[]>,
  snapshot: NormalizedSnapshot,
): Record<string, TimeSeriesPoint[]> {
  const next = { ...existing };

  for (const [name, value] of Object.entries(snapshot.variables)) {
    if (typeof value !== 'number') continue;
    const point: TimeSeriesPoint = { t: snapshot.timeMs, v: value };
    next[name] = [...(next[name] ?? []), point];
  }

  return next;
}

// ── SessionRecord derivation ──────────────────────────────────────────

/**
 * Derive a lightweight SessionRecord from a SessionSummary.
 */
export function toSessionRecord(summary: SessionSummary): SessionRecord {
  let status: SessionRecord['status'];
  if (summary.is_expired) {
    status = 'expired';
  } else if (summary.completed) {
    status = 'completed';
  } else {
    status = 'active';
  }

  return {
    id: summary.id,
    uri: summary.uri,
    kind: summary.kind,
    status,
    created: summary.created_at_ms,
    label: summary.label,
    subsystemName: summary.subsystem_name,
    tick: summary.tick,
    timeMs: summary.time_ms,
    currentState: summary.current_state,
    forkPointTick: summary.fork_point_tick,
    // Absent on a session that predates the field; an empty list is the
    // honest "baseline scenario", never "unknown".
    createOverrides: summary.create_overrides ?? [],
  };
}
