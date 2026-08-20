/**
 * useMonteCarloRunner — fan-out a Monte Carlo batch through the backend
 * batch API and track status.
 *
 * Flow:
 *
 *   1. `run({ workspaceRoot, uri, distributions, count, seed })`
 *      → `sysml.batch.create { kind: 'monte_carlo', children_params }`.
 *   2. Poll `sysml.batch.status { batch_id }` until status !== 'running'.
 *   3. Expose `state`, `batchId`, `children`, `error`.
 *
 * The backend batch surface is still landing (R5.0 in the extensibility
 * plan). Calls will 404 until the service grows the dispatch entries;
 * the `poster` injection keeps tests deterministic in the meantime.
 *
 * `children_params` is produced by `generateChildrenParams(...)` so the
 * run is reproducible given the same seed.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { httpPost } from '@/shared/api/http';
import {
  generateChildrenParams,
  type DistributionMap,
} from './sampleDistribution';

// ── Poster injection (for tests) ────────────────────────────────────

export type HttpPoster = <T>(path: string, body?: unknown) => Promise<T>;

/** Default dispatcher — POST to the shared /api/command endpoint. */
async function defaultCmd<T>(
  command: string,
  params: Record<string, unknown>,
): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Wire types ─────────────────────────────────────────────────────

/** Minimal child descriptor returned by `sysml.batch.status`. */
export interface MonteCarloChild {
  /** Stable child id (session id or synthetic batch-child id). */
  id: string;
  /** Optional backend-provided label (e.g. "run 12/1000"). */
  label?: string | null;
  /** 'pending' | 'running' | 'complete' | 'error' — free-form so backend can extend. */
  status?: string | null;
  /** Echoed parameter record for this child. */
  params?: Record<string, number> | null;
  /** Optional error message if the child failed. */
  error?: string | null;
}

export interface MonteCarloBatchStatus {
  batchId: string;
  status: 'running' | 'complete' | 'error' | string;
  completed?: number;
  total?: number;
  children?: MonteCarloChild[];
  error?: string | null;
}

/**
 * Shape of the `batch` field returned by `sysml.batch.status`. Mirrors
 * the backend's `BatchSession` — we keep a narrow local type so this
 * runner stays decoupled from the full `BatchSession` type surface.
 *
 * Backend `ChildStatus` is a tagged enum serialised as
 * `{status: 'pending' | 'running' | 'complete' | 'failed', error?}` —
 * NOT a bare string. Surface the raw shape here so the runner can read
 * the tag correctly.
 */
interface BatchChildDescriptor {
  session_id?: string;
  id?: string;
  index?: number;
  status?: { status: string; error?: string } | string;
  params?: Record<string, unknown>;
  error?: string | null;
  label?: string | null;
}
interface BatchStatusBatch {
  id: string;
  children?: BatchChildDescriptor[];
  /**
   * `BatchStatus` is a tagged enum serde-encoded as
   * `{status: 'pending' | 'running' | 'complete' | 'failed', reason?, ...}`.
   * The tag field is `status`, not `kind` — don't confuse this with the
   * frontend `BatchStatus` union in `engine/types.ts` which uses `kind`
   * as the discriminator (that's for legacy callers).
   */
  status?: { status: string; reason?: string };
}

/** Pull the tag out of a ChildStatus regardless of serialization form. */
function childStatusTag(s: BatchChildDescriptor['status']): string | null {
  if (!s) return null;
  if (typeof s === 'string') return s;
  if (typeof s === 'object' && 'status' in s) return s.status;
  return null;
}

export type MonteCarloRunnerState =
  | 'idle'
  | 'creating'
  | 'running'
  | 'complete'
  | 'error';

export interface MonteCarloRunArgs {
  /** Workspace root or uri the backend should route the batch under. */
  workspaceRoot?: string | null;
  /** Single-file uri if this run is scoped to one file (optional). */
  uri?: string | null;
  /** Parameter → distribution map. Must be non-empty and all valid. */
  distributions: DistributionMap;
  /** Number of child runs to fan out. */
  count: number;
  /** Optional seed for reproducibility; omitted → ephemeral. */
  seed?: number | null;
  /** Extra metadata forwarded to the backend (e.g. runTargetId). */
  extra?: Record<string, unknown>;
}

export interface UseMonteCarloRunnerResult {
  state: MonteCarloRunnerState;
  batchId: string | null;
  children: MonteCarloChild[];
  completed: number;
  total: number;
  error: string | null;
  /** Kick off a new Monte Carlo batch. Cancels any in-flight poll first. */
  run: (args: MonteCarloRunArgs) => Promise<string | null>;
  /** Stop polling, leave the batch in whatever state it's in. */
  cancel: () => void;
  /** Clear state back to `idle`. */
  reset: () => void;
}

export interface UseMonteCarloRunnerOptions {
  /** Poll interval (ms) — default 500 ms. Tests pass 1 ms. */
  pollIntervalMs?: number;
  /** Injected poster (for tests). Defaults to the shared /api/command dispatcher. */
  poster?: HttpPoster;
}

// ── Hook ────────────────────────────────────────────────────────────

export function useMonteCarloRunner(
  opts: UseMonteCarloRunnerOptions = {},
): UseMonteCarloRunnerResult {
  const { pollIntervalMs = 500, poster } = opts;

  const [state, setState] = useState<MonteCarloRunnerState>('idle');
  const [batchId, setBatchId] = useState<string | null>(null);
  const [children, setChildren] = useState<MonteCarloChild[]>([]);
  const [completed, setCompleted] = useState(0);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);

  // Cancellation token — every run bumps it so late responses are ignored.
  const tokenRef = useRef(0);

  const call = useCallback(
    async <T,>(command: string, params: Record<string, unknown>): Promise<T> => {
      if (poster) return poster<T>('/api/command', { command, params });
      return defaultCmd<T>(command, params);
    },
    [poster],
  );

  const cancel = useCallback(() => {
    tokenRef.current += 1;
    setState((prev) => (prev === 'creating' || prev === 'running' ? 'idle' : prev));
  }, []);

  const reset = useCallback(() => {
    tokenRef.current += 1;
    setState('idle');
    setBatchId(null);
    setChildren([]);
    setCompleted(0);
    setTotal(0);
    setError(null);
  }, []);

  const run = useCallback(
    async (args: MonteCarloRunArgs): Promise<string | null> => {
      // Bump the token — prior polls will short-circuit.
      tokenRef.current += 1;
      const token = tokenRef.current;

      setState('creating');
      setBatchId(null);
      setChildren([]);
      setCompleted(0);
      setTotal(args.count);
      setError(null);

      let rows: Array<Record<string, number>>;
      try {
        rows = generateChildrenParams(args.distributions, args.count, args.seed ?? undefined);
      } catch (err) {
        if (tokenRef.current !== token) return null;
        setError(err instanceof Error ? err.message : String(err));
        setState('error');
        return null;
      }

      let created: {
        batch_id?: string | null;
        child_session_ids?: string[] | null;
      } | null = null;
      try {
        // `sysml.batch.create` expects `{ kind, uri, subsystem_name?,
        // children_params, label? }` — strictly-typed in the backend
        // service-command macro, so we only send fields that match.
        // `workspace_root` / `seed` aren't part of that contract today;
        // the seed is already baked into `rows` via `generateChildrenParams`.
        const createParams: Record<string, unknown> = {
          kind: 'monte_carlo',
          // Backend service-command macro can't deserialise Vec<Object>
          // through the HTTP bridge (no tuple-like wire type), so `AA2`'s
          // R5.0 contract defines `children_params` as a JSON-encoded
          // string. The runner stringifies at the boundary — consumers
          // of this hook keep passing plain arrays of param maps.
          children_params: JSON.stringify(rows),
        };
        if (args.uri) createParams.uri = args.uri;
        if (args.extra) Object.assign(createParams, args.extra);
        created = await call<{
          batch_id?: string | null;
          child_session_ids?: string[] | null;
        }>('sysml.batch.create', createParams);
      } catch (err) {
        if (tokenRef.current !== token) return null;
        setError(err instanceof Error ? err.message : String(err));
        setState('error');
        return null;
      }
      if (tokenRef.current !== token) return null;

      const id = created?.batch_id ?? null;
      if (!id) {
        setError('Backend did not return a batch_id');
        setState('error');
        return null;
      }
      setBatchId(id);
      setState('running');

      // ── Drive children to a terminal state ───────────────────────
      // `sysml.batch.create` spawns child sessions but does NOT step
      // them — they stay `pending` forever unless someone ticks them.
      // And the batch descriptor's `ChildStatus` only transitions when
      // `sessions.stop` runs (see `mark_batch_child_complete` in
      // crates/tooling/sysml-service/src/lib.rs). So for each child we:
      //   1. step a bounded number of ticks to advance the orchestrator
      //      with its overrides applied (stops on `completed=true` or
      //      the tick budget runs out — many MC orchestrators have no
      //      natural terminal state so the budget cap is the common
      //      exit);
      //   2. call `sessions.stop` to flip the batch descriptor to
      //      `Complete` and archive the final snapshot.
      // Children run in parallel; per-child failures are swallowed so
      // one bad child doesn't wedge the batch.
      const childIds = created?.child_session_ids ?? [];
      const MAX_STEPS_PER_CHILD = 32;
      const driveChild = async (sid: string): Promise<void> => {
        for (let i = 0; i < MAX_STEPS_PER_CHILD; i++) {
          if (tokenRef.current !== token) return;
          let summary: { completed?: boolean } | null = null;
          try {
            summary = await call<{ completed?: boolean }>(
              'sysml.sessions.step',
              { session_id: sid },
            );
          } catch {
            break;
          }
          if (summary?.completed) break;
        }
        if (tokenRef.current !== token) return;
        // Stop flips the batch descriptor to `complete` and archives
        // the session. Errors are intentional no-ops — a child that
        // can't stop will surface via the poll loop.
        try {
          await call<unknown>('sysml.sessions.stop', { session_id: sid });
        } catch {
          /* best-effort */
        }
      };
      // Fire and forget — the poll loop below surfaces progress as
      // children tick.
      void Promise.all(childIds.map(driveChild));

      // ── Poll loop ───────────────────────────────────────────────
      // Loop until the batch is no longer running OR our token is stale.
      // Backend envelope: `{ batch: BatchSession }` — unwrap the inner
      // BatchSession (see crates/tooling/sysml-service/src/types.rs
      // BatchStatusResult).
      while (tokenRef.current === token) {
        let envelope: { batch?: BatchStatusBatch } | null = null;
        try {
          envelope = await call<{ batch?: BatchStatusBatch }>(
            'sysml.batch.status',
            { batch_id: id },
          );
        } catch (err) {
          if (tokenRef.current !== token) return id;
          setError(err instanceof Error ? err.message : String(err));
          setState('error');
          return id;
        }
        if (tokenRef.current !== token) return id;

        const batch = envelope?.batch;
        if (batch) {
          const kids = batch.children ?? [];
          const done = kids.filter((c) => {
            const tag = childStatusTag(c.status);
            return tag === 'complete' || tag === 'failed';
          }).length;
          setCompleted(done);
          setTotal(kids.length || args.count);
          setChildren(
            kids.map((c) => ({
              id: c.session_id ?? c.id ?? '',
              label: c.label ?? null,
              status: childStatusTag(c.status),
              params: (c.params ?? null) as Record<string, number> | null,
              error: c.error ?? null,
            })),
          );

          const tag = batch.status?.status;
          if (tag === 'complete') {
            setState('complete');
            return id;
          }
          if (tag === 'failed') {
            setError(batch.status?.reason ?? 'Batch reported an error');
            setState('error');
            return id;
          }
        }

        // Wait one poll interval.
        await sleep(pollIntervalMs);
      }
      return id;
    },
    [call, pollIntervalMs],
  );

  // Bump token on unmount so any in-flight poll loop exits cleanly.
  useEffect(() => {
    return () => {
      tokenRef.current += 1;
    };
  }, []);

  return {
    state,
    batchId,
    children,
    completed,
    total,
    error,
    run,
    cancel,
    reset,
  };
}

// ── Helpers ─────────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
