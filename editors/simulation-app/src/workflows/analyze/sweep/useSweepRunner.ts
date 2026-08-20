/**
 * useSweepRunner — minimal batch runner for the Sweep workflow (R5.1).
 *
 * Wraps `sysml.batch.create` and polls `sysml.batch.status` every 500 ms
 * until the batch settles (`complete` or `failed`). The hook surfaces
 * the live `BatchStatus`, the list of child descriptors, and the
 * current `batchId` so the results shell can render per-child rows as
 * they stream in.
 *
 * Agents CC and DD replace the polling innards with SSE / drill wiring
 * in R5.2+; the `{ batchId, status, children, start }` shape stays
 * stable so their work plugs in without touching the config panel.
 *
 * The backend `sysml.batch.*` commands are being built in parallel by
 * Agent AA. Until they land, this hook gracefully no-ops if the create
 * call fails with an ApiError (dev-time ergonomics — the UI still
 * renders, just never transitions out of `pending`).
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { httpPost } from '@/shared/api/http';
import type {
  BatchKind,
  BatchSession,
  BatchStatus,
  ChildDescriptor,
} from '@/engine/types';
import type { SweepPoint } from './cartesianProduct';
import type { SweepRunMode } from './useSweepConfig';
import { readLastBatchId, writeLastBatchId } from './useSweepStudyStore';

// ── Contract with `sysml.batch.*` ────────────────────────────────────

/**
 * Command envelope. The backend exposes one POST `/api/command` that
 * fans out to named commands; mirrors the existing `cmd()` helper in
 * `shared/api/model.ts`.
 */
async function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

/** Input payload for `sysml.batch.create`. */
export interface BatchCreateRequest {
  kind: BatchKind;
  uri: string;
  children_params: SweepPoint[];
  run_mode?: SweepRunMode;
  label?: string;
  /** Variable names to measure on every child (JSON-encoded on the wire). */
  outcomes?: string[];
  /** Simulation time step, ms, for every child. */
  dt_ms?: number;
  /** Model-time budget, ms, for every child. */
  max_time_ms?: number;
}

/**
 * Response from `sysml.batch.create`. The backend envelope is
 * `{ batch_id, child_session_ids }` — NOT the full `BatchSession`.
 * Pull the live children/status from the first `sysml.batch.status`
 * poll tick rather than assuming they come back with the create.
 */
export interface BatchCreateResponse {
  batch_id: string;
  child_session_ids: string[];
}

/**
 * Response from `sysml.batch.status`. Backend wraps the session under
 * `batch` (see `types::BatchStatusResult`). Older comment in this file
 * claimed the shape was `BatchSession` directly — that's wrong.
 *
 * Note the inner `BatchSession.status` is also wire-shaped: it's a
 * serde tagged enum with `status` as the tag (NOT `kind`). We keep the
 * wire shape here and normalize it to the frontend's kind-tagged
 * `BatchStatus` in `mapBackendStatus`.
 */
export interface BatchStatusResponse {
  batch: {
    id: string;
    children?: WireChildDescriptor[];
    status?: WireBatchStatus;
  };
}

/**
 * The part of `sysml.sessions.step`'s reply this runner reads.
 *
 * `ticks_advanced` is the ACTUAL number of ticks the call moved, which can be
 * less than the number requested when the child hits its model-time budget.
 * The field is documented as the client's halt signal; this runner used to
 * discard the whole response (`dispatch<unknown>`), which is why a child that
 * stopped a fraction of the way into its run still reported `complete`.
 */
interface StepResponse {
  ticks_advanced?: number;
  completed?: boolean;
  paused?: boolean;
  time_ms?: number;
}

/** Wire shape of the backend's `BatchStatus` serde enum. */
interface WireBatchStatus {
  status: 'pending' | 'running' | 'complete' | 'failed';
  running?: number;
  completed?: number;
  reason?: string;
}

/**
 * Wire shape of a child row: identical to `ChildDescriptor` EXCEPT that
 * `status` is the backend's tag-wrapped serde enum (`{status: 'running'}`),
 * not the flat string the frontend union uses. The engine/types doc always
 * said "the runner layer unwraps the tag before storing" — but this runner
 * never did, which went unnoticed for as long as no viewer was mounted
 * (the legacy shell rendered placeholders). Found live on the first real
 * consumer (ninebar Phase 5): status cells blank, strip counts all zero.
 */
type WireChildDescriptor = Omit<ChildDescriptor, 'status'> & {
  status?: ChildDescriptor['status'] | { status?: string; error?: string } | null;
};

const CHILD_STATUSES: readonly ChildDescriptor['status'][] = [
  'pending',
  'running',
  'complete',
  'failed',
];

/** Unwrap one wire child into the flat frontend shape. */
export function unwrapWireChild(c: WireChildDescriptor): ChildDescriptor {
  const raw = typeof c.status === 'object' && c.status !== null ? c.status.status : c.status;
  let status: ChildDescriptor['status'];
  if ((CHILD_STATUSES as readonly unknown[]).includes(raw)) {
    status = raw as ChildDescriptor['status'];
  } else {
    // Unknown tag = contract drift — say so instead of silently blanking.
    console.warn(`sysml.batch.status: unrecognised child status ${JSON.stringify(c.status)}`);
    status = 'pending';
  }
  return { ...c, status };
}

/** Translate the wire shape into the frontend's `BatchStatus` union. */
function mapBackendStatus(s: WireBatchStatus | undefined): BatchStatus {
  if (!s) return { kind: 'pending' };
  switch (s.status) {
    case 'running':
      return {
        kind: 'running',
        running: s.running ?? 0,
        completed: s.completed ?? 0,
      };
    case 'complete':
      return { kind: 'complete' };
    case 'failed':
      return { kind: 'failed', reason: s.reason ?? 'Batch failed' };
    case 'pending':
    default:
      return { kind: 'pending' };
  }
}

// ── Runner hook ──────────────────────────────────────────────────────

/** Config that kicks off a single sweep run. */
export interface SweepRunnerStartConfig {
  /** Source file URI — required so the backend knows which model to fork. */
  uri: string;
  /** Full list of parameter assignments (one per child). */
  childrenParams: SweepPoint[];
  /** Sequential / parallel child execution. */
  runMode: SweepRunMode;
  /** Optional human label for the batch. */
  label?: string;
  /**
   * Ticks each child runs before verify + stop (one bulk
   * `sessions.step {ticks}` call). Defaults to the legacy 32.
   */
  horizonTicks?: number;
  /**
   * Model variables to measure on each child. The backend captures each
   * one from the child's own time series at stop and returns it on the
   * child descriptor, so the viewers can plot what the study asked for.
   */
  outcomes?: string[];
  /**
   * Maximum children driven at once. Defaults to
   * [`DEFAULT_CHILD_CONCURRENCY`]; see the note there for why this is
   * bounded rather than "all of them".
   */
  concurrency?: number;
  /**
   * Simulation time step in ms. Model time covered by a child is
   * `horizonTicks * dtMs`, so this — not the tick count alone — decides
   * how much of a model's behaviour the study actually sees.
   */
  dtMs?: number;
}

/**
 * How many children the client drives at once.
 *
 * NOT a throughput knob — a safety bound, chosen from measurement. Driving a
 * child ends in `sessions.stop`, and stop is the expensive call: it archives
 * the run. Firing every child's stop simultaneously made peak memory the SUM
 * of all of them, which is how a 25-point two-factor sweep took the backend
 * past the machine's RAM and got it OOM-killed — the "hung, then crashed"
 * report. (The per-stop cost itself was a separate defect, fixed in
 * `archive_session_entry`; this bound stops N children multiplying whatever
 * that cost happens to be.)
 *
 * Four keeps several cores busy while holding peak work to a small multiple
 * of one child. Sequential run mode still means one.
 */
export const DEFAULT_CHILD_CONCURRENCY = 4;

/** A child that failed while the client was driving it. */
export interface ChildFailure {
  /** Backend session id of the child. */
  sessionId: string;
  /** Ordinal within the batch — matches `ChildDescriptor.index`. */
  index: number;
  /** Which call failed. `step` is fatal for the child; `stop` is cleanup. */
  phase: 'step' | 'verify' | 'stop';
  /** The error as reported by the backend. */
  message: string;
  /** The child's parameter assignment, so the UI can name the point. */
  params: SweepPoint;
}

/**
 * A child that stopped before reaching the horizon it was asked to run.
 *
 * Not a failure — the child ran and produced real numbers — but the numbers
 * describe less of the model's behaviour than the study asked for, and
 * nothing else on the result surface would say so. `examples/radiation-cooling`
 * cools over ~2000 s of model time; at the default 1 ms step a child hits its
 * 60 s budget having covered 3% of that, and every outcome it reports is a
 * reading taken early in the transient rather than at rest.
 */
export interface ChildTruncation {
  sessionId: string;
  index: number;
  /** Ticks the study asked this child to run. */
  requestedTicks: number;
  /** Ticks it actually advanced. */
  advancedTicks: number;
  /** Model time reached, ms, when the backend reported it. */
  timeMs?: number;
  params: SweepPoint;
}

/** Live counts across the batch's children. */
export interface BatchProgress {
  /** Children not yet started. */
  queued: number;
  /** Children currently stepping. */
  running: number;
  /** Children that finished and were archived. */
  complete: number;
  /** Children that failed (backend status, or a client-side drive failure). */
  failed: number;
  /** Total children in the batch. */
  total: number;
}

export interface UseSweepRunnerResult {
  /** Backend batch id once `start` has returned; `null` before. */
  batchId: string | null;
  /** Current aggregate status (mirrors `BatchStatus`). */
  status: BatchStatus;
  /** Every child descriptor — live-updated while polling. */
  children: ChildDescriptor[];
  /** Most-recent error, if the runner blew up. */
  error: Error | null;
  /** Live queued / running / complete / failed counts. */
  progress: BatchProgress;
  /**
   * Children that failed while being driven, with the parameters that
   * produced them. Successful children's evidence is unaffected — a failure
   * here identifies one point, it does not invalidate the batch.
   */
  failures: ChildFailure[];
  /**
   * Children that stopped short of the requested horizon. Distinct from
   * `failures`: these produced evidence, it just does not cover the run the
   * study described.
   */
  truncations: ChildTruncation[];
  /**
   * Kick off a new batch. Resolves to the created `batchId` once the
   * backend has accepted the request; continues polling until complete
   * or cancel. Idempotent — a prior in-flight batch is abandoned (its
   * pollers drop out) before the new one is submitted.
   */
  start: (config: SweepRunnerStartConfig) => Promise<string | null>;
  /** Stop polling + reset state. Does NOT cancel children server-side. */
  cancel: () => void;
}

export interface UseSweepRunnerOptions {
  /** Polling cadence in ms. Defaults to 500. */
  pollIntervalMs?: number;
  /** Injection point for tests — replaces the command dispatch. */
  dispatch?: <T>(command: string, params: Record<string, unknown>) => Promise<T>;
}

/**
 * Owns the batch lifecycle for a single Sweep run.
 */
export function useSweepRunner(opts: UseSweepRunnerOptions = {}): UseSweepRunnerResult {
  const { pollIntervalMs = 500, dispatch = cmd } = opts;

  const [batchId, setBatchId] = useState<string | null>(null);
  const [status, setStatus] = useState<BatchStatus>({ kind: 'pending' });
  const [children, setChildren] = useState<ChildDescriptor[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [failures, setFailures] = useState<ChildFailure[]>([]);
  const [truncations, setTruncations] = useState<ChildTruncation[]>([]);

  // Poll handle + run-id guard so stale pollers can't write state
  // for a batch that has since been superseded (React 18 concurrent
  // mode + double-invoked effects can otherwise leak intervals).
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeBatchIdRef = useRef<string | null>(null);
  // Children whose bulk step never landed, by index → reason.
  //
  // The backend cannot know about these: `sessions.stop` archives whatever
  // the child has and marks the descriptor `complete` regardless, so a child
  // that produced nothing would otherwise poll back as a success. Overlaying
  // here means the strip, the rail, the table, and the viewers all read the
  // same child list rather than each keeping its own idea of what failed.
  const stepFailuresRef = useRef<Map<number, string>>(new Map());

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const cancel = useCallback(() => {
    clearTimer();
    activeBatchIdRef.current = null;
    // Intentionally do NOT reset `batchId` here — the UI may want to
    // keep the reference for a "view archived batch" affordance. Call
    // sites that want a clean slate call `start` with fresh config.
    setStatus((prev) => (prev.kind === 'running' ? { kind: 'pending' } : prev));
  }, [clearTimer]);

  /**
   * Fold one `batch.status` response into state, and re-arm the poller while
   * the batch is still moving.
   *
   * Shared by the live poller and the mount-time re-attach so a re-opened
   * batch renders through exactly the same mapping as a freshly-run one —
   * two paths onto one screen, one translation.
   */
  const applyStatus = useCallback(
    (id: string, resp: BatchStatusResponse): void => {
      const batch = resp.batch;
      setChildren(
        (batch?.children ?? []).map(unwrapWireChild).map((child) => {
          const reason = stepFailuresRef.current.get(child.index);
          return reason ? { ...child, status: 'failed' as const, reason } : child;
        }),
      );
      // Backend `BatchStatus` serialises with `status` as the tag
      // field (e.g. `{status: 'running', completed, running}`) — the
      // frontend `BatchStatus` union uses `kind` instead. Normalize
      // at the boundary so downstream consumers (SweepResultsShell)
      // can keep the existing discriminator.
      const mapped = mapBackendStatus(batch?.status);
      setStatus(mapped);
      // Terminal states stop the polling loop.
      if (mapped.kind === 'complete' || mapped.kind === 'failed') {
        clearTimer();
        return;
      }
      // Re-arm — timeouts are safer than intervals because they
      // never stack if the previous call was slow.
      timerRef.current = setTimeout(() => void pollRef.current(id), pollIntervalMs);
    },
    [clearTimer, pollIntervalMs],
  );

  const poll = useCallback(
    async (id: string): Promise<void> => {
      if (activeBatchIdRef.current !== id) return; // superseded
      try {
        const resp = await dispatch<BatchStatusResponse>('sysml.batch.status', { batch_id: id });
        if (activeBatchIdRef.current !== id) return; // superseded during await
        applyStatus(id, resp);
      } catch (err) {
        if (activeBatchIdRef.current !== id) return;
        setError(err instanceof Error ? err : new Error(String(err)));
        setStatus({ kind: 'failed', reason: err instanceof Error ? err.message : String(err) });
        clearTimer();
      }
    },
    [applyStatus, clearTimer, dispatch],
  );

  // `applyStatus` re-arms the poller, and `poll` calls `applyStatus` — a
  // cycle two `useCallback`s cannot express directly. The ref breaks it
  // without either one going stale.
  const pollRef = useRef(poll);
  pollRef.current = poll;

  const start = useCallback(
    async (config: SweepRunnerStartConfig): Promise<string | null> => {
      // Abandon any in-flight batch first (see ref note above).
      clearTimer();
      activeBatchIdRef.current = null;
      setBatchId(null);
      setChildren([]);
      setStatus({ kind: 'pending' });
      setError(null);
      setFailures([]);
      setTruncations([]);
      stepFailuresRef.current = new Map();
      // Drop the remembered id along with the state it describes. Without
      // this, a study that fails to create would leave the PREVIOUS batch
      // stored, and navigating away and back would silently reopen it.
      writeLastBatchId(null);

      if (config.childrenParams.length === 0) {
        const e = new Error('Sweep has no children to run — add at least one parameter range.');
        setError(e);
        setStatus({ kind: 'failed', reason: e.message });
        return null;
      }

      try {
        const req: BatchCreateRequest = {
          kind: 'sweep',
          uri: config.uri,
          children_params: config.childrenParams,
          run_mode: config.runMode,
          label: config.label,
          outcomes: config.outcomes,
          dt_ms: config.dtMs,
          // Size the child's model-time budget to the run the study actually
          // asked for. Left unset the backend defaults to 60 s, which silently
          // truncates any horizon longer than that — the wall is invisible from
          // the tick count alone, because ticks and model time are only the
          // same number at dt = 1 ms.
          max_time_ms:
            config.dtMs != null
              ? Math.max(1, Math.round(config.horizonTicks ?? 32)) * config.dtMs
              : undefined,
        };
        // Backend contract (R5.0 AA2): `children_params` is a
        // JSON-encoded string, not an array. The service-command macro
        // can't deserialise Vec<Object> through the HTTP bridge, so we
        // stringify at the boundary. Applies to MC + Sensitivity runners
        // too. See docs/test-checklist-2026-04-20.md BUG 20.
        const wire = {
          ...(req as unknown as Record<string, unknown>),
          children_params: JSON.stringify(req.children_params),
          // Same JSON-string encoding, same reason: the service-command
          // macro's wire types don't cover a bare array.
          outcomes: req.outcomes?.length ? JSON.stringify(req.outcomes) : undefined,
          dt_ms: req.dt_ms,
          max_time_ms: req.max_time_ms,
        };
        const created = await dispatch<BatchCreateResponse>(
          'sysml.batch.create',
          wire,
        );
        const id = created.batch_id;
        activeBatchIdRef.current = id;
        setBatchId(id);
        // Remember it before anything can go wrong: this id is the only
        // route back to the batch, and there is no way to enumerate batches.
        writeLastBatchId(id);
        // Create response only carries the ids; children + status arrive
        // on the first poll tick. Leave the UI in `pending` until then.
        setStatus({ kind: 'pending' });

        // Drive children to a terminal state — see BUG 26 (the backend
        // batch scheduler should own this; client-driving is the flagged
        // interim). Each child: ONE bulk `sessions.step {ticks: horizon}`
        // (the old shape was 32 sequential single-tick HTTP calls — far
        // too short for any physics horizon), then `sessions.verify` so
        // the child's verification-case verdicts are produced against its
        // final state (they land on the archive record → the batch
        // descriptor → the viewers' fail-count metrics), then stop.
        // Errors per-child are swallowed so one bad child doesn't wedge
        // the batch (console-warned, not silent).
        const childIds = created.child_session_ids ?? [];
        const horizon = Math.max(1, Math.round(config.horizonTicks ?? 32));

        const noteFailure = (
          sid: string,
          index: number,
          phase: ChildFailure['phase'],
          err: unknown,
        ) => {
          const message = err instanceof Error ? err.message : String(err);
          console.warn(`sweep child ${sid}: ${phase} failed`, err);
          if (phase === 'step') stepFailuresRef.current.set(index, message);
          setFailures((prev) => [
            ...prev,
            { sessionId: sid, index, phase, message, params: config.childrenParams[index] ?? {} },
          ]);
        };

        const driveChild = async (sid: string, index: number): Promise<void> => {
          if (activeBatchIdRef.current !== id) return;
          let stepped = true;
          try {
            const result = await dispatch<StepResponse>('sysml.sessions.step', {
              session_id: sid,
              ticks: horizon,
            });
            // A short advance means the child hit its model-time budget. It
            // is not an error and the backend does not raise one — the run
            // simply stops moving — so this is the only place the shortfall
            // is visible. `paused` is a breakpoint halt, a different thing.
            const advanced = result?.ticks_advanced;
            if (typeof advanced === 'number' && advanced < horizon && !result?.paused) {
              setTruncations((prev) => [
                ...prev,
                {
                  sessionId: sid,
                  index,
                  requestedTicks: horizon,
                  advancedTicks: advanced,
                  timeMs: result?.time_ms,
                  params: config.childrenParams[index] ?? {},
                },
              ]);
            }
          } catch (err) {
            // A child that could not step produced no evidence. Record it
            // against its parameters so the UI can say WHICH point failed
            // and why, then still stop it so the session is released.
            stepped = false;
            noteFailure(sid, index, 'step', err);
          }
          if (activeBatchIdRef.current !== id) return;
          if (stepped) {
            try {
              await dispatch<unknown>('sysml.sessions.verify', { session_id: sid });
            } catch (err) {
              // No verification cases / verify failure → child completes
              // without verdicts (honest empty), batch itself continues.
              noteFailure(sid, index, 'verify', err);
            }
          }
          if (activeBatchIdRef.current !== id) return;
          try {
            await dispatch<unknown>('sysml.sessions.stop', { session_id: sid });
          } catch (err) {
            // Cleanup failure leaks a live session — worth naming, but it
            // does not invalidate whatever the child already measured.
            noteFailure(sid, index, 'stop', err);
          }
        };

        // Drive children through a BOUNDED pool rather than all at once.
        // See `DEFAULT_CHILD_CONCURRENCY`: unbounded fan-out is what turned a
        // 25-point sweep into a machine-killing memory spike, because every
        // child's archive-on-stop landed simultaneously.
        const limit =
          config.runMode === 'sequential'
            ? 1
            : Math.max(1, Math.round(config.concurrency ?? DEFAULT_CHILD_CONCURRENCY));
        let next = 0;
        const worker = async (): Promise<void> => {
          for (;;) {
            const index = next++;
            if (index >= childIds.length) return;
            if (activeBatchIdRef.current !== id) return;
            await driveChild(childIds[index], index);
          }
        };
        void Promise.all(
          Array.from({ length: Math.min(limit, childIds.length) }, () => worker()),
        );

        void poll(id);
        return id;
      } catch (err) {
        const e = err instanceof Error ? err : new Error(String(err));
        setError(e);
        setStatus({ kind: 'failed', reason: e.message });
        return null;
      }
    },
    [clearTimer, dispatch, poll],
  );

  // Re-attach to the last batch on mount, then abandon polling on unmount.
  //
  // Leaving `/analyze/sweep` unmounts this hook's owner and destroys every
  // result it was holding, but the batch itself lives on the backend for the
  // life of the process. Without this, coming back showed an empty workflow
  // over data that was still there — and, with no `batch.list` to fall back
  // on, permanently out of reach.
  //
  // Deliberately does NOT re-drive children: they have already been stepped,
  // verified and stopped. This reads the batch and shows it. If it is somehow
  // still running (a reload mid-sweep), `poll` re-arms itself as usual.
  useEffect(() => {
    let cancelled = false;
    const stored = readLastBatchId();
    if (stored) {
      void (async () => {
        try {
          const resp = await dispatch<BatchStatusResponse>('sysml.batch.status', {
            batch_id: stored,
          });
          if (cancelled) return;
          activeBatchIdRef.current = stored;
          setBatchId(stored);
          applyStatus(stored, resp);
        } catch {
          // The batch is gone — the backend restarted, or the id predates it.
          // Forget it quietly: an empty workflow is the honest state, and an
          // error banner about a batch the user never asked to reopen is not.
          writeLastBatchId(null);
        }
      })();
    }
    return () => {
      cancelled = true;
      clearTimer();
      activeBatchIdRef.current = null;
    };
    // Mount-only: re-running this on every `dispatch` identity change would
    // re-attach mid-run and fight the live poller.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Progress is DERIVED from the same child list every other surface reads
  // (client-side step failures are already overlaid onto it in `poll`), so
  // the strip, the rail, and the viewers cannot disagree about the counts.
  const progress = useMemo<BatchProgress>(() => {
    let queued = 0;
    let running = 0;
    let complete = 0;
    let failed = 0;
    for (const c of children) {
      if (c.status === 'failed') failed += 1;
      else if (c.status === 'complete') complete += 1;
      else if (c.status === 'running') running += 1;
      else queued += 1;
    }
    return { queued, running, complete, failed, total: children.length };
  }, [children]);

  return {
    batchId,
    status,
    children,
    error,
    progress,
    failures,
    truncations,
    start,
    cancel,
  };
}
