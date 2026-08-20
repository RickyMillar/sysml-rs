/**
 * VerifyCaseRunner — executes a verification run and streams Verdicts.
 *
 * R3.2 (extensibility plan): the runner accepts a `VerifyRunConfig`,
 * issues the appropriate backend command(s) through the existing REST
 * surface, maps results into the universal `Verdict` shape, and returns
 * an aggregate `VerifyRunResult`.
 *
 * Progress is surfaced through `SessionEventBus` (E4 contract). The UI
 * can show "3 of 7 cases complete" by subscribing to our progress
 * events. Because the bus is keyed on a `SessionId` and verify runs
 * don't have a runtime session, we use a synthetic `runId` — a
 * monotonic string like `"verify-run:<ts>"` — as the session key.
 *
 * Backend commands driven:
 *   - `sysml.verify` (per case) when `suite = 'verification-cases'` and
 *     `caseIds` is specified.
 *   - `sysml.evaluate.verification_cases` (one shot) when
 *     `suite = 'verification-cases'` and no `caseIds`.
 *   - `sysml.evaluate.constraints` when `suite = 'constraints'`.
 *   - `sysml.sessions.verify` (per case) when `suite = 'verification-cases'`
 *     and `sessionId` is set — verifies against a RUNNING session's live
 *     final-tick state instead of static evaluation. See
 *     `runVerificationCasesLive` for why this is always driven one case
 *     at a time.
 *
 * Do not invent new endpoints — if a suite kind needs data the backend
 * doesn't expose yet, document it in the brief rather than extending
 * the backend here.
 */

import type {
  Overrides,
  SessionId,
  Verdict,
  VerifyRunConfig,
  VerifyRunResult,
  VerifyRunSummary,
} from '@/engine/types';
import { httpPost } from '@/shared/api/http';
import {
  mapConstraintResult,
  expandRequirementVerdicts,
  mapEvaluateVerificationCaseResult,
  mapVerifyResult,
  summarize,
  type RawConstraintResult,
  type RawVerificationCaseResult,
  type RawVerifyResult,
} from './verdictMapping';

// ── Progress events ──────────────────────────────────────────────────

/** A progress tick for a single-case step. */
export interface VerifyProgressEvent {
  /** Synthetic run id so `SessionEvents` can key on it. */
  runId: SessionId;
  /** 0-based index of the case that just completed. */
  index: number;
  /** Total number of cases we expect to run in this config. */
  total: number;
  /** The case id / constraint id / scenario name just finished. */
  caseId: string;
  /** The verdict just produced. */
  verdict: Verdict;
}

/** Subscriber callback for progress events. */
export type VerifyProgressHandler = (event: VerifyProgressEvent) => void;

/** Unsubscribe thunk. */
export type VerifyUnsub = () => void;

// ── HTTP wiring (injected for testability) ───────────────────────────

type HttpPoster = <T>(path: string, body?: unknown) => Promise<T>;

/** Lightweight command poster: issues a single `command/params` request. */
function cmd<T>(
  poster: HttpPoster,
  command: string,
  params: Record<string, unknown> = {},
): Promise<T> {
  return poster<T>('/api/command', { command, params });
}

/** Serialise overrides into the `[name, value]` tuple list the backend expects. */
function overridesToTuples(overrides: Overrides | undefined): [string, string][] {
  if (!overrides) return [];
  return Object.entries(overrides).map(([k, v]) => [k, serializeOverrideValue(v)]);
}

function serializeOverrideValue(v: unknown): string {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return JSON.stringify(v);
}

// ── Runner construction ──────────────────────────────────────────────

/**
 * Generator of monotonic run ids. Exported so the hook and tests can
 * create deterministic ones when needed.
 */
let runIdCounter = 0;
export function nextRunId(prefix = 'verify-run'): string {
  runIdCounter += 1;
  return `${prefix}:${Date.now().toString(36)}-${runIdCounter}`;
}

/** Clock abstraction — tests inject a deterministic now(). */
export interface Clock {
  now: () => number;
}

const DEFAULT_CLOCK: Clock = { now: () => Date.now() };

export interface VerifyCaseRunnerOptions {
  /** HTTP poster. Defaults to the shared `httpPost` helper. */
  poster?: HttpPoster;
  /** Wall-clock provider. Defaults to `Date.now`. */
  clock?: Clock;
  /**
   * Optional progress callback hook. A convenience alternative to
   * `runner.onProgress(cb)` + a handle — the callback is invoked for
   * every in-progress step of the run returned by `.run(...)`.
   */
  onProgress?: VerifyProgressHandler;
}

/**
 * Runner that turns a single `VerifyRunConfig` into a single
 * `VerifyRunResult`.
 *
 * Usage:
 * ```ts
 * const runner = new VerifyCaseRunner();
 * runner.onProgress((ev) => console.log(`${ev.index + 1}/${ev.total}`));
 * const result = await runner.run(config);
 * ```
 */
export class VerifyCaseRunner {
  private readonly poster: HttpPoster;
  private readonly clock: Clock;
  private readonly listeners = new Set<VerifyProgressHandler>();
  private currentAbort: AbortController | null = null;
  private currentRunId: SessionId | null = null;

  constructor(opts: VerifyCaseRunnerOptions = {}) {
    this.poster = opts.poster ?? httpPost;
    this.clock = opts.clock ?? DEFAULT_CLOCK;
    if (opts.onProgress) this.listeners.add(opts.onProgress);
  }

  /** Subscribe to progress events. Returns an unsub thunk. */
  onProgress(cb: VerifyProgressHandler): VerifyUnsub {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  }

  /** The run id of the currently active run, or null if idle. */
  activeRunId(): SessionId | null {
    return this.currentRunId;
  }

  /**
   * Cancel any in-flight run. The pending `.run(...)` promise rejects
   * with an `AbortError` (standard DOMException shape).
   */
  cancel(): void {
    if (this.currentAbort) {
      this.currentAbort.abort();
      this.currentAbort = null;
      this.currentRunId = null;
    }
  }

  /** Dispatch to the right backend command sequence for the suite kind. */
  async run(config: VerifyRunConfig): Promise<VerifyRunResult> {
    // Idempotency: a second call while running aborts the prior run.
    this.cancel();
    const abort = new AbortController();
    this.currentAbort = abort;
    const runId = nextRunId();
    this.currentRunId = runId;
    const start = this.clock.now();

    try {
      let verdicts: Verdict[];
      switch (config.suite) {
        case 'verification-cases':
          verdicts = config.sessionId
            ? await this.runVerificationCasesLive(config, config.sessionId, runId, abort.signal)
            : await this.runVerificationCases(config, runId, abort.signal);
          break;
        case 'constraints':
          verdicts = await this.runConstraints(config, runId, abort.signal);
          break;
        default: {
          // Exhaustive guard.
          const _exhaustive: never = config.suite;
          throw new Error(`Unsupported verify suite: ${String(_exhaustive)}`);
        }
      }

      const summary = summarize(verdicts) as VerifyRunSummary;
      const durationMs = Math.max(0, this.clock.now() - start);
      return { verdicts, durationMs, summary };
    } finally {
      if (this.currentAbort === abort) {
        this.currentAbort = null;
        this.currentRunId = null;
      }
    }
  }

  // ── Internal dispatchers ──────────────────────────────────────────

  private async runVerificationCases(
    config: VerifyRunConfig,
    runId: SessionId,
    signal: AbortSignal,
  ): Promise<Verdict[]> {
    // If explicit case ids are supplied we call `sysml.verify` one-by-one
    // so we can surface progress and per-case overrides. Otherwise we
    // fall back to the batch `sysml.evaluate.verification_cases`. Both
    // commands are workspace-scoped — exactly one call per case (or one
    // batch call), never one per loaded file.
    if (config.caseIds && config.caseIds.length > 0) {
      const total = config.caseIds.length;
      const verdicts: Verdict[] = [];
      let index = 0;
      for (const caseId of config.caseIds) {
        throwIfAborted(signal);
        try {
          const raw = await cmd<RawVerifyResult>(this.poster, 'sysml.verify', {
            case_name: caseId,
            overrides: overridesToTuples(config.overrides),
          });
          const verdict = mapVerifyResult(caseId, raw);
          const rows = expandRequirementVerdicts(verdict);
          verdicts.push(...rows);
          this.emitProgress({ runId, index, total, caseId, verdict: rows[0] ?? verdict });
        } catch (err) {
          if (isAbort(err)) throw err;
          const verdict = this.errorVerdict(caseId, err);
          verdicts.push(verdict);
          this.emitProgress({ runId, index, total, caseId, verdict });
        }
        index += 1;
      }
      return verdicts;
    }

    // No explicit case list: ask the backend for every case in the workspace.
    let rows: RawVerificationCaseResult[];
    try {
      const raw = await cmd<RawVerificationCaseResult[]>(
        this.poster,
        'sysml.evaluate.verification_cases',
        {},
      );
      rows = Array.isArray(raw) ? raw : [];
    } catch (err) {
      if (isAbort(err)) throw err;
      return [this.errorVerdict('sysml.evaluate.verification_cases', err)];
    }
    const verdicts: Verdict[] = [];
    const total = rows.length || 1;
    let index = 0;
    for (const row of rows) {
      throwIfAborted(signal);
      const verdict = mapEvaluateVerificationCaseResult(row);
      const expanded = expandRequirementVerdicts(verdict);
      verdicts.push(...expanded);
      const caseId =
        row.case_name ||
        row.element_id ||
        `case-${index}`;
      this.emitProgress({ runId, index, total, caseId, verdict: expanded[0] ?? verdict });
      index += 1;
    }
    return verdicts;
  }

  /**
   * Verify against a RUNNING session's live final-tick state via
   * `sysml.sessions.verify`, one case at a time.
   *
   * Why one-at-a-time: `sessions.verify` shares its response shape
   * (`VerifyResult { verdict, summary, requirements, diagnostics }`)
   * with the static per-case `sysml.verify` — it has no `case_name` /
   * `case_id` field, and a bulk call (`case_names` omitted) returns the
   * cases in the backend's own graph-discovery order, which we can't
   * reliably zip with a caller-supplied name list. Requesting exactly
   * one `case_names: [name]` per call guarantees at most one result
   * back, so identity is unambiguous (mirrors the static per-case
   * `sysml.verify` loop above).
   *
   * `config.caseIds` must be resolved to an explicit, non-empty case
   * name list by the caller (VerifyWorkflow resolves "no selection" to
   * every known case for live mode, since there's no live equivalent of
   * the static "ask the backend for every case" fallback).
   */
  private async runVerificationCasesLive(
    config: VerifyRunConfig,
    sessionId: SessionId,
    runId: SessionId,
    signal: AbortSignal,
  ): Promise<Verdict[]> {
    const caseNames = config.caseIds ?? [];
    if (caseNames.length === 0) return [];

    // Snapshot the session's tick once so every verdict from this run
    // carries the same evidence pointer — `sessions.verify` itself has
    // no evidence/tick field (it isn't runtime-coupled the way sim
    // verdicts are), so we synthesize it from the session the caller
    // picked.
    let tick: number | null = null;
    try {
      const info = await cmd<{ summary?: { tick?: number } }>(
        this.poster,
        'sysml.sessions.info',
        { session_id: sessionId, include_variables: false },
      );
      tick = info.summary?.tick ?? null;
    } catch (err) {
      if (isAbort(err)) throw err;
      // Non-fatal — verdicts still resolve, just without a tick pointer.
    }

    const total = caseNames.length;
    const verdicts: Verdict[] = [];
    let index = 0;
    for (const caseId of caseNames) {
      throwIfAborted(signal);
      try {
        const rows = await cmd<RawVerifyResult[]>(this.poster, 'sysml.sessions.verify', {
          session_id: sessionId,
          case_names: [caseId],
        });
        const raw = Array.isArray(rows) ? rows[0] : undefined;
        const verdict = raw
          ? mapVerifyResult(caseId, raw)
          : this.notFoundVerdict(caseId, sessionId);
        verdict.evidence = { session_id: sessionId, tick: tick ?? 0 };
        verdict.metadata = { ...verdict.metadata, session_id: sessionId, live: true };
        const expanded = expandRequirementVerdicts(verdict);
        verdicts.push(...expanded);
        this.emitProgress({ runId, index, total, caseId, verdict: expanded[0] ?? verdict });
      } catch (err) {
        if (isAbort(err)) throw err;
        const verdict = this.errorVerdict(caseId, err, { session_id: sessionId });
        verdicts.push(verdict);
        this.emitProgress({ runId, index, total, caseId, verdict });
      }
      index += 1;
    }
    return verdicts;
  }

  private async runConstraints(
    config: VerifyRunConfig,
    runId: SessionId,
    signal: AbortSignal,
  ): Promise<Verdict[]> {
    // One workspace-scoped call — `sysml.evaluate.constraints` evaluates
    // every constraint in the loaded workspace.
    throwIfAborted(signal);
    let rows: RawConstraintResult[];
    try {
      const raw = await cmd<RawConstraintResult[]>(
        this.poster,
        'sysml.evaluate.constraints',
        {},
      );
      rows = Array.isArray(raw) ? raw : [];
    } catch (err) {
      if (isAbort(err)) throw err;
      return [this.errorVerdict('sysml.evaluate.constraints', err)];
    }
    // Filter by caseIds if provided.
    const wanted = config.caseIds ? new Set(config.caseIds) : null;
    const filtered = wanted
      ? rows.filter((r) => r.element_id && wanted.has(r.element_id))
      : rows;
    const verdicts: Verdict[] = [];
    const total = filtered.length || 1;
    let index = 0;
    for (const row of filtered) {
      throwIfAborted(signal);
      const verdict = mapConstraintResult(row);
      verdicts.push(verdict);
      const caseId = row.element_id ?? `constraint-${index}`;
      this.emitProgress({ runId, index, total, caseId, verdict });
      index += 1;
    }
    return verdicts;
  }

  // ── Helpers ────────────────────────────────────────────────────────

  private emitProgress(ev: VerifyProgressEvent): void {
    for (const cb of Array.from(this.listeners)) {
      try {
        cb(ev);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error('[VerifyCaseRunner] progress subscriber threw:', err);
      }
    }
  }

  /** Build an Error-kind verdict for a thrown exception. */
  private errorVerdict(
    caseId: string,
    err: unknown,
    extraMeta: Record<string, unknown> = {},
  ): Verdict {
    const message = err instanceof Error ? err.message : String(err);
    return {
      verdict: 'error',
      actual: null,
      expected: null,
      margin: null,
      error: message,
      sensitivity: null,
      evidence: null,
      metadata: { source: 'runner-error', case_id: caseId, reason: message, ...extraMeta },
    };
  }

  /** Build an Inconclusive-kind verdict when a live session has no matching case. */
  private notFoundVerdict(caseId: string, sessionId: SessionId): Verdict {
    const reason = `no matching verification case "${caseId}" in session ${sessionId}'s workspace`;
    return {
      verdict: 'inconclusive',
      actual: null,
      expected: null,
      margin: null,
      error: null,
      sensitivity: null,
      evidence: null,
      reason,
      metadata: { source: 'sessions-verify', case_id: caseId, reason },
    };
  }
}

// ── Module helpers ───────────────────────────────────────────────────

function isAbort(err: unknown): boolean {
  if (err instanceof DOMException && err.name === 'AbortError') return true;
  if (err instanceof Error && err.name === 'AbortError') return true;
  return false;
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) {
    // Prefer DOMException so callers can `err.name === 'AbortError'`.
    if (typeof DOMException !== 'undefined') {
      throw new DOMException('verify run cancelled', 'AbortError');
    }
    const err = new Error('verify run cancelled');
    err.name = 'AbortError';
    throw err;
  }
}
