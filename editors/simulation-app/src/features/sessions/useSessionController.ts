/**
 * useSessionController — setTimeout-based step-loop per ADR-004 section 4.
 *
 * Replaces the `requestAnimationFrame` autoplay pattern in the legacy
 * `useSimulationSession`. Key design:
 *
 * - Bulk `step(ticks=N)` per scheduled chunk, not one tick per HTTP
 *   request (closeout-plan.md item 1 — "backend advances server-side,
 *   GUI follows"; the old one-tick-per-request shape crawled at
 *   ~166ms/tick). The scheduling interval stays fixed
 *   (`PLAY_CHUNK_INTERVAL_MS`) and `stepsPerSecond` instead scales the
 *   chunk SIZE, so pause/resume latency stays bounded regardless of
 *   the user's speed setting.
 * - Next setTimeout scheduled only after the step response arrives,
 *   so the loop naturally throttles when the backend is slow.
 * - Halt detection (BP5): every `sessions.step` response now carries
 *   explicit `paused` / `paused_at_breakpoint` / `ticks_advanced`
 *   fields (BP1). A chunk halts the loop the moment `paused === true`
 *   or `completed === true` — read directly off the response, never
 *   inferred from "tick didn't move" (the old approach, which cost a
 *   full extra chunk of latency and — worse — would have kept
 *   re-issuing steps against an already-paused session if the
 *   inference ever missed). `runToBreakpoint` composes on top of the
 *   same halt check for free.
 * - AbortController per step request; pause/stop aborts in-flight.
 * - Phase transitions driven by the 6-phase state machine in the store.
 * - Configurable Hz via store.stepsPerSecond (default 10).
 */

import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { useSessionStore } from './store';
import {
  useCreateSession,
  useResumeSession,
  useStepSession,
  useStopSession,
} from './mutations';
import { sessionKeys } from './queries';
import { createParamsForTarget } from './sessionCreate';
import { archiveSession } from '../../shared/data/SessionArchive';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useRunTargets } from '@/features/run-targets/queries';
import type { RunTargetSummary } from '@/features/run-targets/types';
import { useBreakpointStore, selectArmedCount } from '@/features/breakpoints/useBreakpointStore';
import { createBreakpointClient } from '@/engine/SessionControl';
import type { SessionDetail, SessionPhase, SessionSummary } from './types';

const MAX_RETRIES = 3;
const RETRY_BASE_MS = 200;

/** Stateless wire client for the arm-on-create flush (BP-UX2) — same
 *  wire shape `useSessionControl`'s per-hook instance uses. */
const breakpointClient = createBreakpointClient();

export interface SessionController {
  /**
   * Start the autoplay loop. Lazily creates the backend session if one
   * doesn't already exist, then transitions configuring -> running.
   */
  play: () => Promise<void>;
  /** Pause the loop. Transitions running -> paused. */
  pause: () => void;
  /** Resume from pause. Transitions paused -> running. */
  resume: () => void;
  /** Stop and tear down. Transitions any -> idle. */
  stop: () => void;
  /**
   * Execute a single manual step (no loop). Lazily creates the backend
   * session if one doesn't already exist.
   */
  stepOnce: (event?: string) => Promise<void>;
  /**
   * Bulk-advance the session by `totalTicks` ticks using server-side
   * batching, so far-off events become watchable without a per-tick
   * round-trip. Lazily creates the session, drains staged draft
   * overrides into the FIRST batch only, then loops
   * MAX_BULK_STEP_TICKS-sized `step(ticks)` calls until the horizon is
   * reached, the session completes, or the backend halts early (BP1:
   * `paused === true`, e.g. a breakpoint fired). Each awaited batch
   * lets react-query refresh so the charts fill in between calls.
   *
   * Retained as a general fixed-horizon primitive; the run toolbar no
   * longer wires a button to it directly (see `runToBreakpoint`, which
   * replaced the old baked-in "Run to trip" 140k-tick default per
   * closeout item BP5 / task #5 — the horizon must not be hardcoded to
   * one workflow's event).
   */
  fastForward: (totalTicks: number) => Promise<void>;
  /**
   * Bulk-advance the session with NO fixed horizon, driven purely by
   * armed breakpoints: loops the same server-side batching as
   * `fastForward` up to a generous safety cap
   * (`RUN_TO_BREAKPOINT_SAFETY_CAP_TICKS`), stopping the moment the
   * backend reports `paused` (a breakpoint fired) or `completed`. A
   * no-op when no breakpoint is currently armed (callers should also
   * gate the UI affordance on `selectArmedCount` for a disabled-button
   * treatment rather than a silent no-op).
   */
  runToBreakpoint: () => Promise<void>;
  /**
   * Create the backend session for the current target WITHOUT advancing any
   * ticks, and select it. Returns the session id, or `null` if creation
   * failed (phase is set to 'error' in that case). A no-op returning the
   * existing id when a session is already active.
   *
   * Every other action here creates the session lazily as a side effect of
   * running. That left "just start a session" with no caller of its own, so
   * the only route to it was the Cmd-K developer console (punch-list finding
   * 31). Exposing the primitive lets a plain UI affordance — the session
   * switcher's "New session" row — do the same thing the transport does,
   * through the same one code path, rather than open-coding a second
   * `sessions.create` call site (principle #4).
   */
  startSession: () => Promise<string | null>;
}

/**
 * Server-side bulk-step cap (sysml-service `MAX_BULK_STEP_TICKS`). A
 * larger `ticks` is a hard InvalidInput, so the fast-forward loop caps
 * each batch here and iterates to reach the requested horizon.
 */
const MAX_BULK_STEP_TICKS = 20_000;

/**
 * Wall-clock cadence for the autoplay loop's scheduled chunks. Fixed
 * regardless of `stepsPerSecond` — the chunk SIZE scales with speed
 * instead (see `tick()`), so dialing playback faster advances more
 * ticks per chunk rather than shrinking the interval between HTTP
 * round-trips. This bounds pause/resume latency to ~this many ms at
 * any speed setting, and replaces the old one-tick-per-request drive
 * (closeout-plan.md item 1) that crawled at ~166ms/tick.
 */
const PLAY_CHUNK_INTERVAL_MS = 250;

/**
 * Small inter-batch pause for `fastForward`'s bulk-step loop. Batches are
 * still fired as fast as the backend can serve them (unlike `play()`'s fixed
 * `PLAY_CHUNK_INTERVAL_MS` cadence) — this just inserts a macrotask yield
 * between back-to-back `MAX_BULK_STEP_TICKS` requests so the event loop
 * (chart/websocket updates, the ability to notice pause/stop) and the
 * backend (now dispatching each bulk-step on a blocking-pool thread, but
 * still worth not hammering) get a breather. Negligible relative to the
 * multi-second server-side cost of a full 20k-tick batch.
 */
const FAST_FORWARD_BATCH_PAUSE_MS = 20;

/**
 * Hard ceiling on total ticks advanced by a single `runToBreakpoint()`
 * invocation. `runToBreakpoint` has no caller-supplied horizon (unlike
 * `fastForward`) — it's meant to run until an armed breakpoint fires —
 * so this is purely a safety net: if every breakpoint were somehow
 * cleared mid-run (a race with the panel) the loop still yields control
 * back to the user instead of running unbounded. Generous relative to
 * the old fixed hybrid horizon (140_000 ticks) since this drive has no
 * specific target event in mind.
 */
const RUN_TO_BREAKPOINT_SAFETY_CAP_TICKS = 2_000_000;

/**
 * Chunk size for the throttled play loop: `stepsPerSecond` scaled
 * against the wall-clock scheduling interval, floored at 1 tick and
 * capped at `MAX_BULK_STEP_TICKS`. Pure + exported so the scaling math
 * is directly testable without mounting the hook.
 */
export function computePlayChunkTicks(
  stepsPerSecond: number,
  intervalMs: number = PLAY_CHUNK_INTERVAL_MS,
): number {
  return Math.max(
    1,
    Math.min(MAX_BULK_STEP_TICKS, Math.round((stepsPerSecond * intervalMs) / 1000)),
  );
}

/** Chunk-response interpretation shared by every bulk-step loop (see below). */
export interface ChunkOutcome {
  /** The session finished during this chunk. */
  completed: boolean;
  /**
   * The backend explicitly halted the chunk early (BP1) — either the
   * session completed, or `summary.paused` came back `true` (a
   * breakpoint fired mid- or end-of-`step_many`, or the session was
   * already paused going in). Read directly off the response's
   * EXPLICIT flags, never inferred from "tick didn't move": a paused
   * session's `step_many` always returns `ticks_advanced === 0`
   * (`RuntimeSession::step_many`), so a loop MUST stop the instant this
   * is `true` rather than issuing another batch — re-issuing steps
   * against an already-paused session just spins, burning HTTP +
   * render cycles for zero progress (this is the failure mode that
   * previously crashed the tab).
   */
  halted: boolean;
  /**
   * The `BreakpointId` that triggered this halt, or `null` for a plain
   * completion (or the rare case where the backend halts without a
   * specific breakpoint — the type stays honest about what's on the
   * wire rather than assuming one is always present).
   */
  pausedAtBreakpoint: string | null;
}

/**
 * Interpret one `sessions.step(ticks)` response. Pure + exported
 * (mirrors `dispatchFrame` in `useSessionStream.ts`) so the
 * completed/halted branching — otherwise duplicated near-identically
 * across every bulk-step loop (`tick`, `fastForward`/`runToBreakpoint`'s
 * shared `runBulkSteps`) — lives in one tested place.
 *
 * BP5: previously this compared `summary.tick` against a
 * `previousTick` the caller tracked and declared a "stall" when they
 * matched — a one-chunk-late inference that also couldn't distinguish
 * "halted at the chunk boundary" from "genuinely made no progress for
 * some other reason". BP1 put the real halt cause on the wire, so this
 * just reads it.
 */
export function interpretChunkResult(
  summary: Pick<SessionSummary, 'completed' | 'paused' | 'paused_at_breakpoint'>,
): ChunkOutcome {
  return {
    completed: summary.completed,
    halted: summary.completed || summary.paused === true,
    pausedAtBreakpoint: summary.paused_at_breakpoint ?? null,
  };
}

export function useSessionController(): SessionController {
  const store = useSessionStore;
  const stepMutation = useStepSession();
  const createMutation = useCreateSession();
  const stopMutation = useStopSession();
  const resumeMutation = useResumeSession();
  const queryClient = useQueryClient();

  // Active run target → what to run. The server infers the session kind, so we
  // only resolve which target the user selected (if any).
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const { data: groups } = useRunTargets(workspaceRoot, wsData?.uris ?? []);

  // Locate the selected RunTargetSummary (if any) from the discovered groups.
  const target = useMemo<RunTargetSummary | null>(() => {
    if (!activeSessionTarget || !groups) return null;
    for (const g of groups) {
      const t = g.targets.find((tt) => tt.id === activeSessionTarget);
      if (t) return t;
    }
    return null;
  }, [activeSessionTarget, groups]);

  const timerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const abortRef = useRef<AbortController | null>(null);
  const retryCountRef = useRef(0);

  // ── Internal helpers ────────────────────────────────────────────────

  /**
   * Atomically read + clear `draftOverrides` from the session store
   * and return them in the `[name, value]` tuple shape backend
   * `step_with_overrides` expects. Returns `undefined` when nothing
   * is staged so the caller can choose the bare-step command path.
   *
   * Single-shot semantics: a user typing a value via right-click
   * Override expects it to apply once, on the very next step. Without
   * the clear, the override would re-apply forever — a stale override
   * would silently pin the variable across the rest of the run, and
   * `clearDraftOverrides` (the cleanup hook intended for this) would
   * stay un-called.
   */
  const drainDraftOverrides = useCallback(():
    | [string, string][]
    | undefined => {
    const draft = store.getState().draftOverrides;
    const entries = Object.entries(draft);
    if (entries.length === 0) return undefined;
    store.getState().clearDraftOverrides();
    return entries.map(([k, v]) => [k, v] as [string, string]);
  }, [store]);


  const clearTimer = useCallback(() => {
    if (timerRef.current !== undefined) {
      clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
  }, []);

  const abortInFlight = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
  }, []);

  /** Safely read current store state. */
  const getState = useCallback(() => store.getState(), [store]);

  /** Auto-archive a completed session to IndexedDB (ADR-005). */
  const autoArchive = useCallback(
    (sessionId: string) => {
      const detail = queryClient.getQueryData<SessionDetail>(
        sessionKeys.detail(sessionId),
      );
      if (!detail) return;
      archiveSession(sessionId, {
        detail,
        topology: null,
        snapshotHistory: [],
      }).catch((err) =>
        console.warn('[SessionController] Auto-archive failed:', err),
      );
    },
    [queryClient],
  );

  /**
   * Ensure a backend session exists for the current target/capabilities.
   * Returns the active session id (existing or newly created), or null
   * on failure (in which case phase is set to 'error').
   */
  const ensureSessionStarted = useCallback(async (): Promise<string | null> => {
    const { activeSessionId } = getState();
    if (activeSessionId) return activeSessionId;

    // The staged scenario travels with creation, so the session is BUILT
    // severe/nominal rather than started nominal and edited. This is the one
    // create call site, so there is nothing to keep in sync.
    const params = createParamsForTarget(
      target,
      getState().dtMs,
      getState().scenarioOverrides,
    );

    try {
      // One creation call; the server infers simulation / action /
      // orchestrator and returns a single SessionSummary shape.
      const summary = await createMutation.mutateAsync(params);
      const key = summary.id;
      if (!key) {
        console.error(
          '[SessionController] sessions.create returned no id:',
          summary,
        );
        getState().setPhase('error');
        return null;
      }
      setActiveSession(key);
      // BP-UX2 arm-before-run: breakpoints added while NO session
      // existed are parked locally (`local-` ids). Flush them to the
      // fresh session now — sequentially, reconciling ids as we go —
      // so "pick target → set breakpoints → run" arms them without a
      // start-a-session-first prerequisite. A rejected breakpoint stays
      // local + logged rather than blocking session start (it simply
      // won't fire — same visible behaviour as before this flush
      // existed, minus the silence: we log per failure).
      {
        const bpStore = useBreakpointStore.getState();
        const unpushed = bpStore.breakpoints.filter(
          (bp) => bp.id.startsWith('local-') && bp.enabled !== false,
        );
        for (const bp of unpushed) {
          try {
            const backendId = await breakpointClient.set(key, bp.breakpoint);
            useBreakpointStore.getState().reconcileId(bp.id, backendId);
          } catch (err) {
            console.error(
              '[SessionController] arm-on-create failed for breakpoint',
              bp.breakpoint,
              err,
            );
          }
        }
      }
      // Focus the diagram on the target's source URI (B4) so the diagram pane
      // reflects what the user launched (rather than the first-loaded file).
      if (target?.uri && target.uri !== '__workspace__') {
        void useWorkspaceStore.getState().focusFile(target.uri);
      }
      return key;
    } catch (err) {
      console.error('[SessionController] Session start failed:', err);
      getState().setPhase('error');
      return null;
    }
  }, [target, createMutation, setActiveSession, getState]);

  /**
   * Advance one bulk-step chunk and schedule the next if still running.
   *
   * Chunk size scales with `stepsPerSecond` against the fixed
   * `PLAY_CHUNK_INTERVAL_MS` scheduling cadence — e.g. at the default
   * 10 sps this steps ~3 ticks every 250ms rather than 1 tick every
   * 100ms, cutting the HTTP round-trip count without losing per-tick
   * snapshot fidelity (the backend's `sessions.step(ticks)` records
   * every advanced tick to the time series exactly as single-stepping
   * does — see session-backend-contract.md).
   */
  const tick = useCallback(async () => {
    const { activeSessionId, phase, stepsPerSecond } = getState();
    if (!activeSessionId || phase !== 'running') return;

    // Create AbortController for this step
    const ac = new AbortController();
    abortRef.current = ac;

    const ticksThisChunk = computePlayChunkTicks(stepsPerSecond);

    try {
      // Drain any pending draft overrides into THIS chunk. The
      // backend's step_with_overrides applies them atomically before
      // the orchestrator step, then the response success handler
      // clears the store so they don't re-apply on every subsequent
      // chunk. (Single-shot semantics — matches the user's mental
      // model when they typed a value into a prompt.)
      const overrides = drainDraftOverrides();
      const summary = await stepMutation.mutateAsync({
        sessionId: activeSessionId,
        overrides,
        ticks: ticksThisChunk,
      });

      // Reset retry count on success
      retryCountRef.current = 0;
      // P5: a subsequent successful step clears any previously-shown
      // failure banner — the user fixed it (or it was transient).
      getState().clearStepError();

      const outcome = interpretChunkResult(summary);

      if (outcome.completed) {
        getState().setPhase('completed');
        autoArchive(activeSessionId);
        clearTimer();
        return;
      }

      // Explicit backend halt (BP1: a breakpoint fired) — settle to
      // paused instead of scheduling another chunk. STOP here, don't
      // schedule: the loop must never fire another step against a
      // session the last response reported paused (see the safety note
      // on `ChunkOutcome.halted`).
      if (outcome.halted) {
        getState().setPhase('paused');
        clearTimer();
        return;
      }

      // Schedule next chunk if still running
      const currentPhase = getState().phase;
      if (currentPhase === 'running') {
        timerRef.current = setTimeout(() => void tick(), PLAY_CHUNK_INTERVAL_MS);
      }
    } catch (err) {
      // If aborted, do nothing (user paused/stopped)
      if (ac.signal.aborted) return;

      // User stopped (or otherwise left 'running') while the request
      // was in flight. AbortController racing against request
      // completion can land us here AFTER the user intent was clear
      // — treat their explicit stop as authoritative and don't
      // retry or flip to 'error'. This is the P2 fix for "Stop leaves
      // the session in ERROR".
      if (getState().phase !== 'running') return;

      retryCountRef.current += 1;
      if (retryCountRef.current <= MAX_RETRIES) {
        // Exponential backoff retry
        const delay = RETRY_BASE_MS * Math.pow(2, retryCountRef.current - 1);
        timerRef.current = setTimeout(() => void tick(), delay);
      } else {
        // Max retries exceeded — transition to error
        console.error('[SessionController] Step failed after retries:', err);
        // P5: surface the failure (e.g. RS002 on a stale/mistyped draft
        // override, already drained and lost by this point) — a phase
        // flip alone gives the user no idea WHY the run stopped.
        getState().setStepError(err instanceof Error ? err.message : String(err));
        getState().setPhase('error');
        retryCountRef.current = 0;
      }
    }
  }, [getState, stepMutation, clearTimer, autoArchive, drainDraftOverrides]);

  // ── Public API ──────────────────────────────────────────────────────

  const play = useCallback(async () => {
    const { phase } = getState();
    if (phase !== 'configuring' && phase !== 'idle') return;

    const sessionId = await ensureSessionStarted();
    if (!sessionId) return; // start failed; phase already set to 'error'

    getState().setPhase('running');
    retryCountRef.current = 0;
    void tick();
  }, [getState, tick, ensureSessionStarted]);

  const pause = useCallback(() => {
    getState().transitionPhase(['running'], 'paused');
    clearTimer();
    abortInFlight();
  }, [getState, clearTimer, abortInFlight]);

  const resume = useCallback(() => {
    const { phase, activeSessionId } = getState();
    if (phase !== 'paused' || !activeSessionId) return;

    getState().setPhase('running');
    retryCountRef.current = 0;

    // BP2: clear any backend breakpoint-pause flag before resuming the
    // step loop. `sysml.sessions.resume` is idempotent server-side (a
    // no-op success when the session isn't paused there), so it's safe
    // to fire unconditionally rather than branching on whether THIS
    // pause came from a breakpoint or a plain user Pause click. Without
    // this, a session halted by a breakpoint would keep reporting
    // `paused: true` / `ticks_advanced: 0` from every subsequent step
    // (`RuntimeSession::step_many` no-ops while paused) — `tick()` would
    // immediately re-detect the halt and settle straight back to
    // 'paused', so Resume would silently do nothing.
    resumeMutation.mutate(activeSessionId, {
      onSettled: () => {
        // Only continue the loop if still 'running' — the user may have
        // hit Pause/Stop again while this resume call was in flight.
        if (getState().phase === 'running') void tick();
      },
    });
  }, [getState, tick, resumeMutation]);

  const stop = useCallback(() => {
    clearTimer();
    abortInFlight();
    retryCountRef.current = 0;
    // Transition to `idle` synchronously so the tick-catch's
    // "still running?" guard bails any in-flight retry path rather
    // than flipping us to `error`. See the tick-catch comment above.
    getState().setPhase('idle');
    // Fire-and-forget the backend stop so the session is torn down
    // server-side too — backend auto-archives on `sessions.stop` (R4.1),
    // so skipping this call leaves the session alive and prevents the
    // Archive / Compare workflows from ever seeing it. See
    // docs/test-checklist-2026-04-20.md BUG 13.
    const { activeSessionId } = getState();
    if (activeSessionId) {
      stopMutation.mutate(activeSessionId, {
        // Always clear local state — even if the backend returns 404
        // because it already forgot the session, the UI should settle.
        onSettled: () => {
          getState().setActiveSession(null);
          // Only re-assert idle if we're still in a stopped-shape phase.
          // If something else transitioned us (e.g. user hit Play again
          // on a different target mid-stop), don't clobber it.
          const current = getState().phase;
          if (
            current === 'idle' ||
            current === 'error' ||
            current === 'completed'
          ) {
            getState().setPhase('idle');
          }
        },
      });
    }
  }, [getState, clearTimer, abortInFlight, stopMutation]);

  const stepOnce = useCallback(
    async (event?: string) => {
      const { phase } = getState();
      // Allow manual stepping in idle, configuring, paused, or running
      if (phase === 'completed' || phase === 'error') return;

      const sessionId = await ensureSessionStarted();
      if (!sessionId) return; // start failed; phase already set to 'error'

      // After creating a session via manual Step, surface a non-idle
      // phase so the status bar / header reflect that the session
      // exists but is not auto-playing.
      const phaseAfterStart = getState().phase;
      if (phaseAfterStart === 'idle' || phaseAfterStart === 'configuring') {
        getState().setPhase('paused');
      }

      // Single-shot drain: any draft override the user staged is
      // consumed by this manual step and cleared on success.
      const overrides = drainDraftOverrides();
      stepMutation.mutate(
        { sessionId, event, overrides },
        {
          onSuccess: (summary) => {
            // P5: clear any previously-shown failure banner.
            getState().clearStepError();
            if (summary.completed) {
              getState().setPhase('completed');
              autoArchive(sessionId);
            }
          },
          onError: (err) => {
            console.error('[SessionController] Manual step failed:', err);
            // P5: was a silent discard — the drained draft override
            // (already cleared by `drainDraftOverrides` above) vanished
            // with no user-visible trace beyond this console line.
            getState().setStepError(err instanceof Error ? err.message : String(err));
          },
        },
      );
    },
    [getState, stepMutation, autoArchive, ensureSessionStarted, drainDraftOverrides],
  );

  /**
   * Shared unthrottled bulk-step drive backing both `fastForward`
   * (fixed horizon) and `runToBreakpoint` (breakpoint-armed, no
   * horizon). Caller is responsible for the settled-phase check,
   * `ensureSessionStarted()`, and flipping the phase to 'running'
   * before calling this — this function owns only the batch loop +
   * halt/settle logic, so the two public entry points stay thin.
   *
   * SAFETY (see `ChunkOutcome.halted` doc): the moment a batch response
   * reports `halted`, this function returns/breaks immediately WITHOUT
   * issuing another batch. A paused session's `step_many` always
   * returns `ticks_advanced === 0`, so re-issuing steps against it
   * would spin forever making zero progress — this is the fix for the
   * tab-crashing tight loop from the earlier `fastForward` attempt.
   */
  const runBulkSteps = useCallback(
    async (sessionId: string, totalTicks: number) => {
      // Drain any staged draft overrides into the FIRST batch only, so an
      // injected value (e.g. I_residual) applies exactly once — matching
      // the single-shot semantics of tick()/stepOnce().
      let overrides = drainDraftOverrides();
      let advanced = 0;

      try {
        while (advanced < totalTicks) {
          // Cooperative interruption: pause()/stop() flip the phase, so bail
          // between batches. (An in-flight bulk batch still runs to
          // completion server-side — expected for bulk-step.)
          if (getState().phase !== 'running') return;

          const batch = Math.min(MAX_BULK_STEP_TICKS, totalTicks - advanced);
          const summary = await stepMutation.mutateAsync({
            sessionId,
            overrides,
            ticks: batch,
          });
          overrides = undefined; // single-shot: only the first batch carries them
          advanced += batch;
          getState().clearStepError();

          const outcome = interpretChunkResult(summary);

          if (outcome.completed) {
            getState().setPhase('completed');
            autoArchive(sessionId);
            return;
          }

          // Explicit backend halt (BP1: a breakpoint fired). STOP —
          // never issue another batch against a session the last
          // response reported paused (see the function-level safety note).
          if (outcome.halted) break;

          // Yield to the event loop between batches (see
          // FAST_FORWARD_BATCH_PAUSE_MS doc comment) instead of firing the
          // next bulk-step immediately on promise resolution.
          if (advanced < totalTicks) {
            await new Promise((resolve) => setTimeout(resolve, FAST_FORWARD_BATCH_PAUSE_MS));
          }
        }

        // Reached the horizon (or hit an early stop) without completing —
        // settle to paused so the user can inspect and continue.
        if (getState().phase === 'running') {
          getState().setPhase('paused');
        }
      } catch (err) {
        // User paused/stopped while a batch was in flight — their intent wins.
        if (getState().phase !== 'running') return;
        console.error('[SessionController] Bulk-step loop failed:', err);
        getState().setStepError(
          err instanceof Error ? err.message : String(err),
        );
        getState().setPhase('error');
      }
    },
    [getState, stepMutation, drainDraftOverrides, autoArchive],
  );

  const fastForward = useCallback(
    async (totalTicks: number) => {
      // Only start from a settled phase — never race the autoplay loop
      // (running) or restart a finished/errored run.
      const { phase } = getState();
      if (phase === 'running' || phase === 'completed' || phase === 'error') {
        return;
      }
      if (totalTicks <= 0) return;

      const sessionId = await ensureSessionStarted();
      if (!sessionId) return; // start failed; phase already set to 'error'

      getState().setPhase('running');
      retryCountRef.current = 0;
      getState().clearStepError();

      await runBulkSteps(sessionId, totalTicks);
    },
    [getState, ensureSessionStarted, runBulkSteps],
  );

  const runToBreakpoint = useCallback(
    async () => {
      // Same settled-phase gate as fastForward — never race the
      // autoplay loop or restart a finished/errored run.
      const { phase } = getState();
      if (phase === 'running' || phase === 'completed' || phase === 'error') {
        return;
      }

      // Belt-and-braces: the UI button is disabled without an armed
      // breakpoint, but a programmatic caller could still invoke this
      // directly. Without one, there is no event to run TO, so bail
      // rather than burning `RUN_TO_BREAKPOINT_SAFETY_CAP_TICKS` for
      // nothing.
      if (selectArmedCount(useBreakpointStore.getState()) === 0) return;

      const sessionId = await ensureSessionStarted();
      if (!sessionId) return; // start failed; phase already set to 'error'

      getState().setPhase('running');
      retryCountRef.current = 0;
      getState().clearStepError();

      await runBulkSteps(sessionId, RUN_TO_BREAKPOINT_SAFETY_CAP_TICKS);
    },
    [getState, ensureSessionStarted, runBulkSteps],
  );

  // ── Cleanup on unmount ──────────────────────────────────────────────

  useEffect(() => {
    return () => {
      clearTimer();
      abortInFlight();
    };
  }, [clearTimer, abortInFlight]);

  return {
    play,
    pause,
    resume,
    stop,
    stepOnce,
    fastForward,
    runToBreakpoint,
    startSession: ensureSessionStarted,
  };
}
