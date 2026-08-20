/**
 * useMonteCarloResults — fetch per-child descriptors from
 * `sysml.batch.results` once a Monte Carlo batch completes, and bridge
 * them into the viewer-ready `ChildDescriptor` shape expected by
 * `MonteCarloResultsPanel`.
 *
 * Wire shape notes:
 *   - Backend sends `children: Array<{ session_id, index, params, status,
 *     verdicts }>` where `status` is a tagged enum (`{ status: 'pending'
 *     | 'running' | 'complete' | 'failed', error? }`), `params` is a
 *     sorted `{key: jsonValue}` map, and each `verdicts[i]` is an
 *     `ArchivedVerdict { case_id, verdict, timestamp, evidence? }`.
 *   - The frontend `ChildDescriptor` shape (`passRateHelpers.ts`) uses
 *     lowercase status strings, a `metrics?` bag (not populated by the
 *     backend today — left empty), and `verdicts?: Verdict[]` (the
 *     universal frontend verdict struct, not `ArchivedVerdict`).
 *
 * Calls `sysml.batch.results { batch_id, include_verdicts: true }`. The
 * poster defaults to the shared `/api/command` dispatcher but can be
 * swapped for tests.
 */

import { useQuery } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import type { Value, Verdict } from '../../../engine/types';
import type {
  ChildDescriptor,
  ChildStatus,
} from './passRateHelpers';

// ── Wire types ─────────────────────────────────────────────────────

/** Matches the Rust `ChildStatus` serde-tagged-enum wire form. */
interface WireChildStatus {
  status: string;
  error?: string | null;
}

interface WireArchivedVerdict {
  case_id: string;
  verdict: string;
  timestamp?: number;
  evidence?: {
    session_id?: string;
    tick?: number;
    element_id?: string | null;
  } | null;
}

interface WireChildDescriptor {
  session_id: string;
  index: number;
  params?: Record<string, unknown>;
  status: WireChildStatus | string;
  verdicts?: WireArchivedVerdict[];
}

interface WireBatchResults {
  children: WireChildDescriptor[];
}

// ── Bridging ───────────────────────────────────────────────────────

function childStatusTag(raw: WireChildDescriptor['status']): string | null {
  if (!raw) return null;
  if (typeof raw === 'string') return raw;
  if (typeof raw === 'object' && 'status' in raw) return raw.status;
  return null;
}

/** Map the wire status tag onto the frontend `ChildStatus` enum. */
function normalizeStatus(raw: WireChildDescriptor['status']): ChildStatus {
  const tag = childStatusTag(raw)?.toLowerCase() ?? 'pending';
  switch (tag) {
    case 'complete':
    case 'completed':
      return 'complete';
    case 'failed':
    case 'error':
      return 'failed';
    case 'cancelled':
    case 'canceled':
      return 'cancelled';
    case 'running':
      return 'running';
    default:
      return 'pending';
  }
}

/**
 * Coerce a raw JSON value from the backend params map into the
 * frontend `Value` union. Unknown shapes fall through as-is so the CSV
 * exporter and histogram viewer can still render them via
 * `String(value)`.
 */
function coerceValue(v: unknown): Value {
  if (v === null || v === undefined) return null;
  if (typeof v === 'number' || typeof v === 'string' || typeof v === 'boolean') {
    return v;
  }
  if (Array.isArray(v)) return v.map(coerceValue);
  if (typeof v === 'object') return v as Record<string, unknown>;
  return String(v);
}

/** Normalize a wire verdict string onto the frontend `VerdictKind`. */
function normalizeVerdict(raw: string): Verdict['verdict'] {
  const s = raw.toLowerCase();
  if (s === 'pass' || s === 'fail' || s === 'inconclusive' || s === 'error') {
    return s;
  }
  return 'error';
}

function bridgeVerdict(v: WireArchivedVerdict): Verdict {
  const out: Verdict = {
    verdict: normalizeVerdict(v.verdict),
    id: v.case_id,
    metadata: { case_name: v.case_id, requirement_id: v.case_id },
  };
  if (v.evidence?.session_id !== undefined && v.evidence?.tick !== undefined) {
    out.evidence = {
      session_id: v.evidence.session_id,
      tick: v.evidence.tick,
      element_id: v.evidence.element_id ?? undefined,
    };
  }
  return out;
}

function bridgeChild(w: WireChildDescriptor): ChildDescriptor {
  const params: Record<string, Value> = {};
  if (w.params) {
    for (const [k, v] of Object.entries(w.params)) {
      params[k] = coerceValue(v);
    }
  }
  return {
    index: w.index,
    session_id: w.session_id,
    status: normalizeStatus(w.status),
    params,
    metrics: {}, // backend does not expose per-child output metrics today
    verdicts: (w.verdicts ?? []).map(bridgeVerdict),
  };
}

// ── Hook ────────────────────────────────────────────────────────────

export type HttpPoster = <T>(path: string, body?: unknown) => Promise<T>;

async function defaultPoster<T>(path: string, body?: unknown): Promise<T> {
  return httpPost<T>(path, body);
}

export interface UseMonteCarloResultsOptions {
  /** Swap the /api/command poster for tests. */
  poster?: HttpPoster;
}

export interface UseMonteCarloResultsResult {
  children: ChildDescriptor[];
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
}

/**
 * Fetch + bridge Monte Carlo batch results.
 *
 * `enabled` guards the query — skip it until the batch exists AND the
 * runner reports a terminal state, so we don't spam the backend with
 * partial-results requests during a long run.
 */
export function useMonteCarloResults(
  batchId: string | null,
  enabled: boolean,
  opts: UseMonteCarloResultsOptions = {},
): UseMonteCarloResultsResult {
  const poster = opts.poster ?? defaultPoster;

  const query = useQuery<ChildDescriptor[], Error>({
    queryKey: ['montecarlo-results', batchId],
    enabled: enabled && !!batchId,
    queryFn: async () => {
      if (!batchId) return [];
      const resp = await poster<WireBatchResults>('/api/command', {
        command: 'sysml.batch.results',
        params: { batch_id: batchId, include_verdicts: true },
      });
      return (resp.children ?? []).map(bridgeChild);
    },
    // Results are immutable once the batch completes; no stale/refetch dance.
    staleTime: Infinity,
    gcTime: 5 * 60 * 1000,
  });

  return {
    children: query.data ?? [],
    isLoading: query.isLoading || query.isFetching,
    isError: query.isError,
    error: query.error ?? null,
  };
}

// ── Test-only exports ──────────────────────────────────────────────

export const __testing = {
  bridgeChild,
  bridgeVerdict,
  normalizeStatus,
  normalizeVerdict,
  coerceValue,
};
