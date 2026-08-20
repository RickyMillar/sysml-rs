/**
 * Engine-layer type contracts (Layer 2, DAP-shaped).
 *
 * These are the authoritative frontend types for the **shared engine
 * (E1 SessionControl, E3 VariableInspection).
 *
 * Every workflow UI (Run, Verify, Sweep, Monte Carlo, Trade Study,
 * Compare) consumes these types so that session control,
 * breakpoints, and variable inspection have a single contract regardless
 * of the workflow.
 *
 * Not for use by feature code that talks to the existing session layer
 * directly — those continue to use `features/sessions/*`. This module
 * wraps that layer; it does not replace it.
 */

import type { TimePoint } from '../features/sessions/types';

// ── Identity / primitive aliases ─────────────────────────────────────

/** A backend session key (UUID string). */
export type SessionId = string;

/** A model element id (QualifiedName or stable id). */
export type ElementId = string;

/** A model variable name (as surfaced in snapshots). */
export type VariableName = string;

/** Simulation tick (backend-owned monotonic counter). */
export type Tick = number;

/** Opaque breakpoint id (client-generated if the backend doesn't assign one). */
export type BreakpointId = string;

/** Opaque snapshot id (a session key + tick reference). */
export type SnapshotId = string;

/**
 * Value carried by snapshots, overrides, and inspection results.
 *
 * Matches the loose shape produced by backend snapshot serialisation
 * (numbers, strings, booleans, nulls, Quantity/Complex objects).
 */
export type Value = number | string | boolean | null | Record<string, unknown> | Value[];

/** Parameter override map for `fork` / `start`. */
export type Overrides = Record<string, Value>;

// Re-export TimePoint from the existing session layer so callers don't
// need two imports. Intentionally the same shape.
export type { TimePoint };

// ── Session start / control ──────────────────────────────────────────

/**
 * Configuration for starting a new session.
 *
 * Maps to the single unified backend entry point `sysml.sessions.create`,
 * which infers the session kind (simulation / action / orchestrator) from the
 * model + optional `target`. Expressed as a workflow-neutral object — the
 * client no longer picks a kind-specific `*.start` command.
 */
export interface SessionStartConfig {
  /**
   * Source URI to run, or `__workspace__` for the merged workspace. When
   * omitted, the engine falls back to the user's currently-selected run target.
   */
  uri?: string;
  /**
   * Optional element name to run (a state machine, action, etc.). The backend
   * infers the session kind from it; omit to run the whole workspace
   * orchestrator.
   */
  target?: string;
  /** Backend simulation step in ms (orchestrator sessions). */
  dtMs?: number;
  /** Initial parameter overrides to apply at start. */
  overrides?: Overrides;
}

// ── Breakpoints (mirrors the Rust enum Agent B is building) ──────────

/**
 * Breakpoint kinds, aligned with the planned Rust `Breakpoint` enum.
 */
export type BreakpointKind =
  | 'state-entry'
  | 'transition-fire'
  | 'action-invoke'
  | 'constraint-violation'
  | 'threshold-crossing'
  | 'conditional';

/**
 * Comparison operator shared by conditional + threshold breakpoints.
 *
 * Matches the Rust `CompareOp` enum serialised in kebab-case — the tag
 * values are the lower-case abbreviations (`lt`, `le`, `gt`, `ge`,
 * `eq`, `ne`). Not to be confused with `ThresholdDirection` ('rising' /
 * 'falling' / 'either'), which is UI-side only and lowered to the
 * backend's `op + value` form at submission time.
 */
export type CompareOp = 'lt' | 'le' | 'gt' | 'ge' | 'eq' | 'ne';

/** Common breakpoint fields. */
interface BreakpointBase {
  /** Client- or server-assigned id. */
  id?: BreakpointId;
  /** Element the breakpoint is attached to (state, transition, action, constraint, variable). */
  target: ElementId;
  /** Optional human label for UI. */
  label?: string;
  /** Whether the breakpoint is armed (default true). */
  enabled?: boolean;
}

export interface StateEntryBreakpoint extends BreakpointBase {
  kind: 'state-entry';
}

export interface TransitionFireBreakpoint extends BreakpointBase {
  kind: 'transition-fire';
}

export interface ActionInvokeBreakpoint extends BreakpointBase {
  kind: 'action-invoke';
}

export interface ConstraintViolationBreakpoint extends BreakpointBase {
  kind: 'constraint-violation';
}

export interface ThresholdCrossingBreakpoint extends BreakpointBase {
  kind: 'threshold-crossing';
  /** Name of the variable being watched. */
  variable: VariableName;
  /** Threshold value. */
  threshold: number;
  /** Direction to watch (default 'either'). */
  direction?: 'rising' | 'falling' | 'either';
  /**
   * Debounce window (in ticks) — once this breakpoint fires, it is
   * suppressed for N ticks before re-arming. Prevents the session from
   * pausing every tick while the condition stays true. Defaults to 0
   * (no debouncing) to preserve pre-R4.4 behaviour. Mirrors the Rust
   * `Breakpoint::ThresholdCrossing::debounce_ticks` field.
   */
  debounce_ticks?: number;
}

/**
 * Conditional breakpoint — pauses when `snapshot.<variable> <op> <value>`
 * evaluates to true. Unlike `ThresholdCrossingBreakpoint`, it is explicitly
 * scoped to an owning element (`target`) and carries a user-editable
 * `enabled` flag plus an optional human label.
 *
 * Mirrors the Rust `Breakpoint::Conditional` variant. The backend tag is
 * serialised as `"kind": "conditional"` (kebab-case / lowercase).
 */
export interface ConditionalBreakpoint extends BreakpointBase {
  kind: 'conditional';
  /** Element the condition is attached to (owning part or state). */
  target: ElementId;
  /** Name of the variable to read from the tick snapshot. */
  variable: VariableName;
  /** Comparison operator (lowercase abbreviation — see `CompareOp`). */
  op: CompareOp;
  /** Value the variable is compared against. */
  value: number;
}

export type Breakpoint =
  | StateEntryBreakpoint
  | TransitionFireBreakpoint
  | ActionInvokeBreakpoint
  | ConstraintViolationBreakpoint
  | ThresholdCrossingBreakpoint
  | ConditionalBreakpoint;

// ── Inspection ───────────────────────────────────────────────────────

/**
 * Result of an `inspect` call — a structural view of a model element or
 * variable at a specific tick. Kept intentionally permissive so the
 * backend can extend without frontend changes.
 */
export interface InspectionResult {
  /** What was inspected. */
  target: ElementId | VariableName;
  /** The value (for variables) or serialised structure (for elements). */
  value: Value;
  /** Tick the inspection was taken at (null = latest). */
  tick: Tick | null;
  /** Backend kind label if known ('variable', 'state-machine', 'action', ...). */
  kind?: string;
  /** Arbitrary structured metadata. */
  metadata?: Record<string, unknown>;
}

// ── Session control interface (E1) ───────────────────────────────────

/**
 * DAP-shaped session control surface. Every workflow UI talks to this;
 * none implement step/pause/breakpoint management themselves.
 *
 * `start` is async because the backend session key is only available
 * after the start command completes. The rest are synchronous
 * fire-and-forget that delegate to the existing controller (which
 * manages its own async loop + retries).
 */
export interface SessionControl {
  /** Start a new session. Returns the created SessionId once the backend responds. */
  start(config: SessionStartConfig): Promise<SessionId>;
  /** Pause the session's autoplay loop. */
  pause(id: SessionId): void;
  /**
   * Resume the session's autoplay loop. BP5: also clears any backend
   * breakpoint-pause flag first (`sysml.sessions.resume`, idempotent —
   * a no-op when the session wasn't halted at a breakpoint), so this
   * is the single affordance that continues a run regardless of
   * whether it stopped from a plain user Pause or a fired breakpoint.
   */
  resume(id: SessionId): void;
  /** Step the session one tick. `opts.event` injects an event this tick. */
  step(id: SessionId, opts?: { event?: string }): Promise<void>;
  /** Stop the session and release backend resources. */
  stop(id: SessionId): void;

  /** Install a breakpoint. Returns the id assigned by the backend. */
  setBreakpoint(loc: Breakpoint): Promise<BreakpointId>;
  /** Clear a previously-set breakpoint by id. */
  clearBreakpoint(id: BreakpointId): Promise<void>;
  /** List all currently-active breakpoints. */
  listBreakpoints(): Promise<Breakpoint[]>;

  /** Inspect a model element or variable in the given session. */
  inspect(id: SessionId, target: ElementId | VariableName): Promise<InspectionResult>;
  /** Take a point-in-time snapshot reference for later diffing/replay. */
  snapshot(id: SessionId): Promise<SnapshotId>;
  /** Fork a session (optionally at a past tick, optionally with overrides). */
  fork(id: SessionId, atTick?: Tick, overrides?: Overrides): Promise<SessionId>;
}

// ── Universal verdict shape (E2) ─────────────────────────────────────

/**
 * Four-valued verdict per SysML v2 spec (`VerificationCases.sysml`).
 *
 * Lower-case string form matches the Rust serde serialization of
 * `sysml_runtime::cases::VerdictKind` (via `impl Display` at
 * `crates/lang/sysml-runtime/src/cases/mod.rs`).
 */
export type VerdictKind = 'pass' | 'fail' | 'inconclusive' | 'error';

/**
 * Pointer to the evidence that produced a verdict.
 *
 * Mirrors `sysml_runtime::cases::EvidenceRef`. Agent R populates this
 * at runtime-coupled verification sites; static verification paths
 * leave it absent.
 */
export interface EvidenceRef {
  /** Runtime session identifier (e.g. `"file://foo.sysml:MySm"`). */
  session_id: string;
  /** Tick at which the verdict was evaluated. */
  tick: number;
  /** Optional model element id (requirement, constraint, etc.). */
  element_id?: string;
}

/**
 * Universal verdict shape. One struct, every workflow (Run, Verify,
 * Monte Carlo, Sweep, Trade Study) emits `Verdict[]` so aggregators
 * operate uniformly.
 *
 * Mirrors the Rust `Verdict` struct (locked at R1.3). Field shape here
 * is load-bearing — extending it must be backward-compatible.
 *
 * Matrix rendering (R3.3) reads `metadata.requirement_id` and
 * `metadata.case_name` when present; absent keys fall back to a
 * synthetic key derived from the verdict index.
 */
export interface Verdict {
  /** The four-valued outcome. */
  verdict: VerdictKind;
  /**
   * Optional stable id for the thing being verified (requirement id,
   * constraint id, check name). Carried in `metadata` on the Rust side;
   * surfaced as a first-class field here because UI consumers need it.
   */
  id?: string;
  /** Optional human label for UI rendering (defaults to `id` or verdict). */
  label?: string;
  /**
   * The value actually computed (constraint LHS or observed metric). Typed
   * as `unknown` because the backend emits arbitrary `serde_json::Value`;
   * the UI narrows to primitives with runtime checks at render time.
   */
  actual?: unknown | null;
  /** The expected value / threshold. Same rationale as `actual`. */
  expected?: unknown | null;
  /** Numeric margin (`actual − expected`) when both are numeric. */
  margin?: number | null;
  /**
   * Evaluation error string, populated when the backend could not produce
   * an `actual` (unresolved binding, type error, solver failure, …).
   * When `error` is `Some(_)`, the row should render as "error: <msg>" —
   * distinct from `actual == null && error == null` which means
   * "inconclusive" (no value, no error).
   */
  error?: string | null;
  /** Sensitivity coefficients: input variable → ∂verdict/∂input. */
  sensitivity?: Record<string, number> | null;
  /** Pointer back to the evidence (session + tick + element). */
  evidence?: EvidenceRef | null;
  /** Reason text for `inconclusive` / `error` verdicts. */
  reason?: string | null;
  /** Optional runtime in ms for the check that produced this verdict. */
  runtimeMs?: number | null;
  /**
   * Free-form metadata. Keys frequently seen by the matrix viewer:
   *   - `case_name`      — verification case (row in the matrix)
   *   - `requirement_id` — individual check (column in the matrix)
   *   - `message`        — human-readable explanation (tooltip)
   *   - `error_reason`   — populated when `verdict === 'error'`
   */
  metadata?: Record<string, unknown>;
}

// ── Verify runner config / result (R3.2) ─────────────────────────────

/**
 * Which suite of cases to run. Mirrors the backend entry points:
 *
 * - `verification-cases` — the model's own VerificationCaseDefinition /
 *   VerificationCaseUsage elements (via `sysml.verify` per case, or
 *   `sysml.evaluate.verification_cases` when no explicit case list).
 * - `constraints` — every constraint in the model (via
 *   `sysml.evaluate.constraints`). Each constraint becomes one Verdict.
 */
export type VerifySuiteKind = 'verification-cases' | 'constraints';

/**
 * Configuration for a single verify run. All fields are optional where
 * sensible so callers only specify what they need.
 *
 * Verify is workspace-scoped: the backend commands the runner drives
 * (`sysml.verify`, `sysml.evaluate.*`) evaluate the loaded workspace as a
 * whole (scope-collapse W2 removed their uri params). There is no per-file
 * scope field — the old `{ kind: 'workspace', uris }` scope drove one call
 * per loaded uri against these workspace-wide commands, duplicating every
 * result N× with fabricated per-file provenance.
 */
export interface VerifyRunConfig {
  /** The suite kind being executed. */
  suite: VerifySuiteKind;
  /** Restrict to a subset of case ids. Omit to evaluate every case. */
  caseIds?: string[];
  /** Optional `key=value` overrides to apply for the run. */
  overrides?: Overrides;
  /**
   * When set (and `suite === 'verification-cases'`), verify against this
   * RUNNING session's live final-tick state (`sysml.sessions.verify`)
   * instead of the static per-case `sysml.verify` command. This is the
   * only path that resolves simulation-produced derived attributes
   * (e.g. `tripped`/`trip_time`) — static evaluation has no runtime
   * values for those. Ignored for other suites (constraints have no
   * live-session counterpart).
   */
  sessionId?: SessionId;
}

/** Summary of pass/fail/inconclusive/error counts. */
export interface VerifyRunSummary {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
}

/** Result of executing a `VerifyRunConfig`. */
export interface VerifyRunResult {
  /** One `Verdict` per case (or per constraint, or per scenario). */
  verdicts: Verdict[];
  /** Wall-clock duration of the run, in milliseconds. */
  durationMs: number;
  /** Rollup counts over `verdicts`. */
  summary: VerifyRunSummary;
}

// ── Batch sessions (R5.0) ────────────────────────────────────────────

/**
 * The flavour of batch run. Mirrors the backend `BatchKind` enum being
 * built in parallel for `sysml.batch.*`. snake_case string form matches
 * R3/R4 reconcile conventions (serde default for the Rust enum).
 *
 * - `sweep`        — deterministic parameter sweep (cartesian product of ranges)
 * - `monte_carlo`  — stochastic sampling from distributions
 * - `trade_study`  — alternatives × evaluation functions
 */
export type BatchKind = 'sweep' | 'monte_carlo' | 'trade_study';

/**
 * Per-child lifecycle state (flat string form used by frontend state).
 *
 * The backend wire format for `ChildStatus` is a discriminated object
 * `{ status: "pending" | "running" | "complete" | "failed" }` with an
 * `error` field on the `failed` variant. The runner layer unwraps the
 * tag before storing in UI state. If you're calling `sysml.batch.*`
 * directly, account for the wrapped shape at the boundary.
 */
export type ChildStatus = 'pending' | 'running' | 'complete' | 'failed';

/**
 * Alias kept for API symmetry with `SliceFilter.only_status` (snake-case
 * wire form). Same values as `ChildStatus`.
 */
export type ChildStatusKind = ChildStatus;

/**
 * Aggregate batch state. `kind`-tagged discriminated union used by the
 * frontend runner (see `useSweepRunner`, `useMonteCarloRunner`).
 *
 * The backend wire format uses `{ status: "..." }` tagging — runners
 * translate `{status: X}` → `{kind: X}` at the wire boundary so UI
 * state stays consistent.
 */
export type BatchStatus =
  | { kind: 'pending' }
  | { kind: 'running'; running: number; completed: number }
  | { kind: 'complete' }
  | { kind: 'failed'; reason: string };

/**
 * A single child entry within a batch — what the Sweep / Monte Carlo /
 * Trade Study result tables iterate over. `session_id` is the regular
 * session key (children are plain sessions, addressable via all the
 * existing `sysml.sessions.*` commands).
 */
export interface ChildDescriptor {
  /** Backend session key for this child; `null` before kickoff. */
  session_id: string | null;
  /** 0-based index within the batch (stable across re-renders). */
  index: number;
  /** Parameter assignment for this child (sweep point / MC sample). */
  params: Record<string, unknown>;
  /** Current lifecycle state. */
  status: ChildStatus;
  /** Verdicts emitted by this child (omitted on the wire while empty). */
  verdicts?: Verdict[];
  /**
   * Final readings for the outcomes the batch was asked to measure, keyed
   * by variable name. Backend-populated at the same moment as `verdicts`
   * (see `batch::OutcomeReading`); omitted on the wire while empty.
   *
   * `params` says what was varied, `outcomes` says what came out.
   */
  outcomes?: Record<string, OutcomeReading>;
  /**
   * Why this child failed. Set by the backend for a child that failed
   * server-side, and by the runner for a child whose drive call never
   * landed — the backend cannot know about the latter.
   */
  reason?: string | null;
  /**
   * Optional UX convenience fields populated by the frontend runner:
   * `id` — stable row id (often `"batch-<n>"`), decouples row React keys
   *        from the backend `session_id` so pending rows have a key.
   * `last_tick` — last observed tick for drill-to-tick affordance.
   * `first_failing_element` — first failing element id (for drill URL).
   */
  id?: string;
  last_tick?: number | null;
  first_failing_element?: string | null;
}

/**
 * One measured outcome on one child, mirroring the backend
 * `batch::OutcomeReading`.
 *
 * A reading either has a `value` or an `error` — never both, never neither.
 * The distinction matters: an outcome that could not be read must render as
 * unavailable, because charting it as `0` would invent a data point the run
 * never produced.
 */
export interface OutcomeReading {
  /** Final finite value, absent when the outcome could not be read. */
  value?: number;
  /** Model time (ms) the value was sampled at. */
  time_ms?: number;
  /** Unit symbol when the slot declared one; absent for type-only quantities. */
  unit?: string;
  /** Why the outcome is unavailable. Mutually exclusive with `value`. */
  error?: string;
  /**
   * Decimated `[time_ms, value]` trace of how the outcome reached `value`,
   * oldest first. Absent when the outcome could not be read.
   *
   * A final number alone cannot say whether a run settled or stopped
   * mid-transient. This is the shape behind the number, captured from the
   * child's own buffer at the same moment.
   */
  series?: [number, number][];
}

/**
 * A batch run — parent session that owns N children. Returned by
 * `sysml.batch.create` and `sysml.batch.status`.
 */
export interface BatchSession {
  /** Batch id (distinct from child `session_id`s). */
  id: string;
  /** Which flavour of batch. */
  kind: BatchKind;
  /** Optional human label for UI display. */
  label?: string;
  /** Wall-clock start time in unix milliseconds. */
  created_at_ms: number;
  /** Every child in the batch, in creation order. */
  children: ChildDescriptor[];
  /** Aggregate lifecycle state. */
  status: BatchStatus;
  /** Variable names this batch was asked to measure, in caller order. */
  outcomes?: string[];
}

// ── Variable inspection interface (E3) ───────────────────────────────

/**
 * Single read contract for "what's the value of X right now / at
 * tick T / across runs". Backed by the existing snapshot + time-series
 * stores.
 */
export interface VariableInspection {
  /** Current (latest-snapshot) value of a variable. */
  current(session: SessionId, name: VariableName): Value | null;
  /** Value at a specific tick. Returns null if the tick hasn't been observed. */
  atTick(session: SessionId, tick: Tick, name: VariableName): Value | null;
  /** All buffered time-series points for a variable. */
  series(session: SessionId, name: VariableName): TimePoint[];
  /** Current value across a set of sessions (for run-vs-run comparison). */
  acrossSessions(sessions: SessionId[], name: VariableName): Map<SessionId, Value>;
}

// ── Session archive (R4.1) ───────────────────────────────────────────

/**
 * Origin workflow that produced an archived session. Matches the
 * (parallel-built) backend enum `SessionOrigin` on `sysml.sessions.archive.*`
 * commands. snake_case spelling mirrors the canonical R3 reconcile style.
 */
export type SessionOrigin =
  | 'run'
  | 'verify'
  | 'sweep'
  | 'montecarlo'
  | 'tradestudy';

/**
 * Summary row returned by `sysml.sessions.archive.list`. One entry per
 * archived session; heavy fields (overrides / snapshots / verdicts) are
 * only hydrated through `sysml.sessions.archive.get`.
 *
 * Field names are intentionally snake_case to mirror the backend wire
 * shape one-to-one (R3 reconcile canonical style). Translation happens
 * at rendering sites, not at the transport boundary.
 */
export interface ArchivedSessionSummary {
  /** Archive entry id (stable across restore operations). */
  id: string;
  /** Human label — user-editable; defaults to the session's auto-label. */
  label: string;
  /** Which workflow produced the session. */
  origin: SessionOrigin;
  /** Workspace URI the session was bound to (for filter / grouping). */
  workspace_uri: string;
  /** Wall-clock start time in unix milliseconds. */
  created_at: number;
  /** Wall-clock end time in unix milliseconds, or `null` if still open. */
  ended_at: number | null;
  /** Total ticks recorded at archive time. */
  ticks: number;
  /** Whether the user pinned this session as golden (reference run). */
  is_golden: boolean;
  /** Optional label describing the golden classification. */
  golden_label?: string;
  /**
   * Rollup of verdict counts across every Verdict produced during the
   * session. Absent for sessions that did not produce verdicts
   * (e.g., raw Run sessions).
   */
  verdict_counts?: { pass: number; fail: number; inconclusive: number; error: number };
}

/**
 * Full archived session payload returned by `sysml.sessions.archive.get`.
 * Extends the summary with the heavy bits needed to hydrate / restore.
 */
export interface ArchivedSession extends ArchivedSessionSummary {
  /** Final override map applied at run time. */
  overrides: Record<string, unknown>;
  /** Sampled snapshots (shape is workflow-dependent; opaque to the panel). */
  snapshots: unknown[];
  /** Every Verdict produced during the session, in emission order. */
  verdicts: Verdict[];
}

// ── Sweep filters (R5.4) ─────────────────────────────────────────────

/**
 * Predicate used by both the pre-run generation filter (client-side) and
 * the post-hoc batch slice (backend-side). Shape matches the backend
 * `ParamPredicate` struct one-to-one.
 */
export interface ParamPredicate {
  /** Parameter name that the predicate reads from `ChildDescriptor.params`. */
  param: string;
  /** Comparison operator — reused from the breakpoint `CompareOp`. */
  op: CompareOp;
  /** Value the parameter is compared against. */
  value: number;
}

/**
 * Filter shape accepted by `sysml.batch.slice`. Every field is optional;
 * an empty filter returns every child in the batch.
 */
export interface SliceFilter {
  /** Retain children whose `status` matches (bare string). */
  only_status?: ChildStatusKind;
  /** Retain children whose aggregated verdict matches. */
  only_verdict?: VerdictKind;
  /** Retain children whose params satisfy the predicate. */
  param_predicate?: ParamPredicate;
}

// ── Diagnostics (R6.1) ───────────────────────────────────────────────

/**
 * Severity classification for a `Diagnostic`.
 *
 * Mirrors the Rust `sysml_span::Severity` enum, which serde-serialises as
 * lowercase (`"info" | "warning" | "error"`). The `"hint"` variant has no
 * backend counterpart today — it is reserved for future use (e.g., LSP
 * diagnostics carrying `DiagnosticSeverity::Hint`) so the UI filter can be
 * forward-compatible without shape churn.
 */
export type DiagnosticSeverity = 'error' | 'warning' | 'info' | 'hint';

/**
 * Source-file span attached to a `Diagnostic`.
 *
 * Matches the Rust `sysml_span::Span` one-to-one (snake_case, unsigned
 * offsets). `line` / `col` are optional (1-indexed when present); parse
 * errors that lack resolved line/column still carry byte offsets. `file`
 * is the file URI the diagnostic was reported against — may or may not
 * equal the `uri` the parent diagnostic was fetched under (e.g., a
 * downstream semantic check can flag a remote file).
 */
export interface DiagnosticSpan {
  /** File URI / path the diagnostic points into. */
  file: string;
  /** Start byte offset (0-indexed). */
  start: number;
  /** End byte offset (exclusive). */
  end: number;
  /** Start line number (1-indexed). Absent when the parser couldn't resolve it. */
  line?: number;
  /** Start column number (1-indexed). Absent when the parser couldn't resolve it. */
  col?: number;
}

/**
 * Related location attached to a `Diagnostic` for cross-reference context
 * (e.g., "defined here", "first use was here"). Mirrors the Rust
 * `sysml_span::RelatedLocation` shape.
 */
export interface DiagnosticRelatedLocation {
  message: string;
  span: DiagnosticSpan;
}

/**
 * Diagnostic tag — additional metadata rendered as visual decoration
 * (e.g., strikethrough for `"unnecessary"`). Mirrors the Rust
 * `sysml_span::DiagnosticTag` serde form.
 */
export type DiagnosticTag = 'unnecessary' | 'deprecated';

/**
 * A single diagnostic message emitted by the backend.
 *
 * Wire mirror of `sysml_span::Diagnostic` as returned by the
 * `sysml.diagnostics` command (and the `GET /models/:uri/diagnostics`
 * REST endpoint). The backend emits `severity` as the lowercase variant
 * name, and both `span` and `code` are optional — parse errors that lack
 * a resolved span still carry a message.
 *
 * Fields are snake_case to mirror the wire shape verbatim (R3+ convention).
 */
export interface Diagnostic {
  severity: DiagnosticSeverity;
  message: string;
  /** Optional diagnostic code (e.g., `"E001"`, `"PH006"`). */
  code?: string;
  /** Source-file span. Absent when the parser couldn't attach a location. */
  span?: DiagnosticSpan;
  /** Optional explanatory notes / suggestions. */
  notes?: string[];
  /** Related context locations. */
  related?: DiagnosticRelatedLocation[];
  /** Optional severity-orthogonal tags. */
  tags?: DiagnosticTag[];
}

// ── Traceability matrix (R6.2) ───────────────────────────────────────

/**
 * Single row in a trace matrix — one source→target satisfaction edge
 * carried by a relationship (e.g. `Satisfy`, `Verify`, `Derive`,
 * `Allocate`).
 *
 * Mirrors the backend `sysml_core::query::TraceMatrixRow` struct (see
 * `crates/lang/sysml-core/src/query.rs`). Field names are snake_case to
 * match the wire shape one-to-one — the backend is the source of truth
 * and the UI translates only at rendering sites.
 *
 * `sysml.trace_matrix` returns `Vec<TraceMatrixRow>` — the full matrix
 * is derived in the frontend (rows = requirement/source, columns =
 * linked element/target) because the backend emits a flat edge list.
 */
export interface TraceMatrixRow {
  /** ElementId of the source element (typically a requirement). */
  source: string;
  /** Human name of the source element, when present. */
  source_name: string | null;
  /** ElementId of the target element (typically a part / constraint). */
  target: string;
  /** Human name of the target element, when present. */
  target_name: string | null;
  /** ElementId of the relationship carrying the edge (Satisfy / Verify / …). */
  relationship: string;
}

/**
 * A derived row in the trace-matrix viewer (a requirement / source
 * element). `id` is the backend ElementId, `label` is the display
 * label (falls back to `id` when the backend doesn't carry a name).
 */
export interface TraceRow {
  id: string;
  label: string;
}

/**
 * A derived column in the trace-matrix viewer (a part / constraint /
 * verification case / whatever target kind the matrix was built for).
 */
export interface TraceColumn {
  id: string;
  label: string;
}

/**
 * A single cell-bearing link. `verdict` carries the satisfaction state:
 * `pass` / `fail` / `inconclusive` / `error`. `'inconclusive'` is the
 * canonical "not yet evaluated" value — consumers that haven't run a
 * verification overlay leave every link inconclusive.
 */
export interface TraceLink {
  /** Row id (source ElementId). */
  row: string;
  /** Column id (target ElementId). */
  column: string;
  /** ElementId of the backing relationship — used as the cell stable key. */
  relationship: string;
  /** Four-valued verdict. Defaults to `'inconclusive'` for unevaluated links. */
  verdict: VerdictKind;
  /** Optional reason text (verdict message, "not evaluated", etc.). */
  reason?: string | null;
}

/**
 * The full trace matrix shape consumed by
 * `<TraceabilityMatrixViewer />`. Derived in the hook layer from the
 * flat `TraceMatrixRow[]` returned by `sysml.trace_matrix` — the
 * backend never ships pre-grouped.
 */
export interface TraceMatrix {
  rows: TraceRow[];
  columns: TraceColumn[];
  links: TraceLink[];
}

// ── Causation trace (R7.1) ───────────────────────────────────────────
//
// Mirrors of the Rust `CausationEvent` / `CausationKind` types defined in
// `crates/lang/sysml-runtime/src/causation.rs`. Serde emits the event with
// `#[serde(flatten)]` on the kind + `#[serde(tag = "kind", rename_all = "snake_case")]`,
// producing wire payloads shaped like:
//
//   { "id": "ev-3-0", "tick": 3, "kind": "variable_write", "var": "speed",
//     "old_value": 0, "new_value": 100, "actor": "sm1", "target": "speed",
//     "detail": "speed = 100.0000 (was 0)", "caused_by": ["ev-3-1"] }

/**
 * Discriminated union of causation event kinds. The `kind` tag is
 * snake_case (serde default). Each variant carries the kind-specific
 * payload alongside the shared [`CausationEvent`] fields.
 */
export type CausationKind =
  | {
      kind: 'variable_write';
      var: string;
      old_value: Value;
      new_value: Value;
    }
  | {
      kind: 'transition_fire';
      from: string;
      to: string;
      event: string | null;
    }
  | {
      kind: 'action_invoke';
      action: string;
      args: string[];
    }
  | {
      kind: 'constraint_evaluated';
      constraint: string;
      verdict: boolean;
      actual?: Value | null;
      expected?: Value | null;
      /** Evaluation error (when the constraint could not be evaluated). */
      error?: string | null;
    }
  | {
      kind: 'event_injected';
      event: string;
    }
  | {
      kind: 'ode_step';
      dt: number;
      changed_vars: string[];
    };

/**
 * One recorded causation event (R7.1). Stable wire shape; the kind tag
 * + discriminator pattern lets TypeScript narrow the payload per branch.
 */
export type CausationEvent = CausationKind & {
  /** Opaque stable id (`"ev-<tick>-<ordinal>"`). */
  id: string;
  /** Simulation tick at which the event occurred. */
  tick: number;
  /** Subsystem / element that produced the event. */
  actor: string;
  /** Optional target element (variable name, state id, constraint id). */
  target: string | null;
  /** Human-readable row-label summary. */
  detail: string;
  /** Ids of upstream events that contributed to this one. */
  caused_by: string[];
};

/**
 * Request payload for `sysml.causation.trace`. Either `root_event_id`
 * (preferred) or the `(root_tick, root_target)` pair identifies the root.
 */
export interface CausationTraceRequest {
  session_id: string;
  root_event_id?: string | null;
  root_tick?: number | null;
  root_target?: string | null;
  /** Maximum BFS depth. Defaults to 5 on the server when absent or 0. */
  max_depth?: number | null;
}

/**
 * Response payload of `sysml.causation.trace`. The root event is echoed
 * both as a standalone field (for header rendering) and as the first
 * entry in `chain`. `chain` is BFS order — root first, closest causes
 * first. `max_depth_used` echoes the effective depth the server applied.
 */
export interface CausationTraceResult {
  root: CausationEvent | null;
  chain: CausationEvent[];
  max_depth_used: number;
}

// ── Sensitivity workflow (R7.4) ──────────────────────────────────────

/**
 * Which sensitivity method to run. Mirrors the backend `SensitivityMethod`
 * enum; snake_case wire labels.
 * - `morris` — Elementary Effects screening (cheap, r·(d+1) runs).
 * - `sobol`  — variance-based decomposition (Saltelli 2002).
 */
export type SensitivityMethod = 'morris' | 'sobol';

/** Per-parameter sensitivity result — sparse Morris OR Sobol fields. */
export interface SensitivityResult {
  name: string;
  mu?: number;
  sigma?: number;
  s1?: number;
  st?: number;
}

/** Parameter range declaration consumed by the sensitivity sampler. */
export interface ParamRange {
  name: string;
  min: number;
  max: number;
}

/** Request payload for `sysml.sensitivity.analyze`. */
export interface SensitivityAnalyzeRequest {
  batch_id: string;
  method: SensitivityMethod;
  /** JSON-encoded `ParamRange[]` — same order as batch generation. */
  parameters_of_interest: string;
  output_metric: string;
  morris_levels?: number;
}

/** Response of `sysml.sensitivity.analyze`. */
export interface SensitivityAnalyzeResult {
  method: SensitivityMethod;
  parameters: SensitivityResult[];
}
