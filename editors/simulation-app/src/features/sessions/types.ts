/**
 * Session domain types for the feature-sliced session layer.
 *
 * These are the authoritative frontend types for the session transport.
 * Backend contract shapes are now inlined here (Day 8 cleanup — types/activity.ts deleted).
 */

// ── Backend contract types (previously in types/activity.ts) ─────────

export type SessionKind = 'simulation' | 'action' | 'orchestrator';

/** Git state corroborating a session's model identity (never identity itself). */
export interface GitProvenance {
  sha: string;
  /** Uncommitted changes existed — the sha alone doesn't reproduce the content. */
  dirty: boolean;
  /** `null` on a detached HEAD. */
  branch: string | null;
}

/** One workspace file's identity in a session's provenance manifest (§6.2). */
export interface FileProvenance {
  /** Workspace-relative path (canonical URI when outside/without a root). */
  path: string;
  /** SHA-256 (hex) of the file's UTF-8 text as loaded. */
  content_hash: string;
}

/**
 * Model/run provenance captured at session creation (B6 remainder).
 * `model_digest` is the backend's content-addressed graph digest — the
 * SAME identity store commits/baselines use, so equality against a
 * baseline commit id means "ran exactly that baseline". Forks inherit
 * it verbatim. Feeds `ReportProvenance.model` in the Verify report.
 */
export interface SessionProvenance {
  model_digest: string;
  git?: GitProvenance | null;
  workspace_root?: string | null;
  /**
   * Per-file manifest of the non-stdlib workspace at capture time (§6.2):
   * each file's relative path + content hash, sorted by path. Absent on a
   * pre-§6.2 archive; empty when captured outside a loaded workspace.
   * Fills `ReportModelRevision.files` in the downloaded Verify report.
   */
  file_manifest?: FileProvenance[];
}

export interface SessionSummary {
  id: string;
  kind: SessionKind;
  uri: string;
  subsystem_name: string | null;
  label: string | null;
  created_at_ms: number;
  elapsed_ms: number;
  tick: number;
  time_ms: number;
  current_state: string | null;
  completed: boolean;
  is_expired: boolean;
  history_len: number;
  subsystem_count: number;
  fork_point_tick: number | null;
  /**
   * Ticks currently retained in the orchestrator archive, oldest →
   * newest — exactly the set `sessions.fork_with_overrides(at_tick=…)`
   * will accept (UX closeout arc #7 honesty flag). Fork/rewind
   * affordances render ONLY at these points; guessing hits the
   * fail-hard `SnapshotMissing` error instead of a silent clamp.
   * Backend always serializes the field (`#[serde(default)]` is
   * deserialize-side); optional here so pre-existing test fixtures
   * stay valid — treat a missing key as `[]`.
   */
  forkable_ticks?: number[];
  /**
   * Whether the session is currently halted at a breakpoint (BP1).
   * Distinguishes a breakpoint-driven halt from an ordinary user Pause
   * click (which never sets this — only `check_breakpoints` does).
   * `useSessionController`'s bulk-step loops (`fastForward`,
   * `runToBreakpoint`) read this EXPLICIT flag to decide when to stop
   * issuing further `sessions.step` batches — replacing the old
   * tick-unchanged "stalled" inference. Mirrors the Rust
   * `#[serde(default)]` field, so always present (defaults `false`)
   * even from a session predating BP1.
   */
  paused: boolean;
  /**
   * The `BreakpointId` that triggered the current pause, or `null`/
   * absent when `paused` is `false`. Backend `skip_serializing_if`
   * omits the key entirely when `None` — treat a missing key the same
   * as `null`.
   */
  paused_at_breakpoint?: string | null;
  /**
   * Ticks actually advanced by the `sessions.step` call that produced
   * this summary — may be strictly less than the requested `ticks`
   * when a breakpoint halts `step_many` early. A **per-call** value,
   * not persisted session state: always `0` on summaries from
   * non-stepping commands (info/reset/fork/inject/resume/…).
   */
  ticks_advanced: number;
  /**
   * Provenance captured at creation. Backend `skip_serializing_if`
   * omits the key for sessions predating capture — treat a missing
   * key the same as `null` (render `—`, never fabricate).
   */
  provenance?: SessionProvenance | null;
  /**
   * Scenario overrides this session was BUILT with, as `[key, value]` pairs.
   *
   * Distinct from anything applied mid-run via `sessions.step`: these were in
   * force at the first tick, so the whole trajectory can be attributed to
   * them. Empty (or absent, on a session predating the field) means the
   * model's own declared defaults — the baseline scenario. Never render an
   * empty list as "unknown scenario"; it is a known one.
   */
  create_overrides?: [string, string][];
}

export interface SubsystemSummary {
  name: string;
  kind_label: string;
  current_state: string;
  completed: boolean;
  available_transitions: [string, string][];
  /** Number of deferred events currently queued (SM-specific, 0 for others). */
  deferred_event_count?: number;
}

export interface SessionDetail {
  summary: SessionSummary;
  subsystems: SubsystemSummary[];
  latest_snapshot: Record<string, unknown> | null;
}

export interface BucketUsage {
  used: number;
  cap: number;
}

export type SessionQuota = Record<SessionKind, BucketUsage>;

// Diff types (fork/compare)
export interface SubsystemDiff { name: string; a_state: string | null; b_state: string | null }
export interface VariableDiff { name: string; a_value: number | null; b_value: number | null }

export interface SessionDivergence {
  a_id: string;
  b_id: string;
  current_tick_a: number;
  current_tick_b: number;
  subsystem_diffs: SubsystemDiff[];
  variable_diffs: VariableDiff[];
}

/**
 * Structured error payload from `sessions.fork_with_overrides(at_tick)`
 * (tagged union on `kind` — contract: "Errors produced by at_tick are
 * structured JSON so the frontend can switch on the variant instead of
 * parsing strings"). `SnapshotMissing.valid_ticks` is the caller's
 * exact set of options, oldest → newest — never a "nearest" clamp.
 */
export type ForkAtTickError =
  | { kind: 'FutureTick'; tick: number; current: number }
  | {
      kind: 'SnapshotMissing';
      tick: number;
      earliest_available: number | null;
      valid_ticks: number[];
    };

export interface SessionTimelineDivergence {
  a_id: string;
  b_id: string;
  shared_start_tick: number | null;
  shared_end_tick: number | null;
  first_divergence_tick: number | null;
  tick_diffs: Array<{
    tick: number;
    subsystem_diffs: SubsystemDiff[];
    variable_diffs: VariableDiff[];
  }>;
  history_truncated: boolean;
}

// ── Shared data types (previously in types/activity.ts) ──────────────

export interface TimePoint {
  t: number;
  v: number;
}

export interface TimelineEntry {
  tick: number;
  timeMs: number;
  subsystems: Record<string, string>;
  /** Per-subsystem deferred event counts at this tick (absent = 0). */
  deferredCounts?: Record<string, number>;
}

export interface ConstraintRow {
  name: string;
  expression: string;
  passed: boolean;
  actual: string | null;
  expected: string | null;
  /**
   * Evaluation error from the backend (unresolved binding / type
   * mismatch / solver failure). Distinct from `actual === null` which
   * just means "no observed value yet" (inconclusive).
   */
  error: string | null;
  message: string | null;
  kind?: 'assume' | 'require' | 'assert';
}

// ── Frontend-owned types ──────────────────────────────────────────────

/**
 * Lightweight record for session list rendering.
 * Derived from SessionSummary — contains only what a sidebar card needs.
 */
export interface SessionRecord {
  id: string;
  uri: string;
  kind: 'simulation' | 'action' | 'orchestrator';
  status: 'active' | 'completed' | 'expired';
  created: number; // unix ms
  label: string | null;
  subsystemName: string | null;
  tick: number;
  timeMs: number;
  currentState: string | null;
  forkPointTick: number | null;
  /**
   * Scenario overrides this session was built with (`create_overrides`).
   * Empty = the model's declared defaults, which is a known scenario, not an
   * unknown one.
   */
  createOverrides: [string, string][];
}

/**
 * Full session data bundle — detail + topology + latest snapshot.
 * Populated by react-query from sessions.info + sessions.topology.
 */
export interface SessionData {
  detail: SessionDetail;
  topology: import('../../types/physics').SystemTopology | null;
  latestSnapshot: Record<string, unknown> | null;
}

/**
 * Client-only view state for a session — lives in Zustand, not react-query.
 */
export interface SessionViewState {
  monitoredVariables: string[];
  compareBaseline: string | null;
  draftOverrides: Record<string, string>;
}

/**
 * Request payload for launching a new run.
 */
export interface RunLaunchRequest {
  uri: string;
  targetId: string;
  dt_ms?: number;
}

/**
 * 6-phase state machine per ADR-004 section 4.
 *
 * Transitions:
 *   idle -> configuring        (user selects run target)
 *   configuring -> running     (user clicks Run)
 *   running -> paused          (user clicks Pause)
 *   paused -> running          (user clicks Resume)
 *   running -> completed       (backend returns completed: true)
 *   any -> error               (transport failure)
 *   error -> idle              (user dismisses)
 */
export type SessionPhase =
  | 'idle'
  | 'configuring'
  | 'running'
  | 'paused'
  | 'completed'
  | 'error';

// ── Normalized snapshot types ─────────────────────────────────────────

/** A single time-series data point. */
export interface TimeSeriesPoint {
  t: number;
  v: number;
}

/** Typed snapshot after normalization. */
export interface NormalizedSnapshot {
  tick: number;
  timeMs: number;
  completed: boolean;
  subsystems: Record<string, {
    currentState: string;
    completed: boolean;
    kindLabel: string;
  }>;
  variables: Record<string, number | string>;
  constraintResults: Array<{
    name: string;
    expression: string;
    satisfied: boolean;
  }>;
}

/** Typed topology after normalization. */
export interface NormalizedTopology {
  rootLabel: string;
  modules: Array<{
    id: string;
    label: string;
    domain: string;
    subsystemNames: string[];
  }>;
}
