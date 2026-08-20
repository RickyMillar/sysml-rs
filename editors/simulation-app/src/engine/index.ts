/**
 * Engine-layer public surface (Layer 2, DAP-shaped).
 *
 * Barrel entry point for the shared engine API consumed by every
 * workflow UI. Re-exports the type contracts and the React hooks that
 * wrap the existing session infrastructure.
 *
 * See `./types.ts` for the contract definitions and
 * the architectural rationale (Round 1, E1 + E3 + E4).
 *
 * Contributors: this file is intentionally ADDITIVE. Parallel agents
 * each add their own exports. Do not delete or reorder another agent's
 * exports when editing.
 */

export type {
  // Identity
  SessionId,
  ElementId,
  VariableName,
  Tick,
  BreakpointId,
  SnapshotId,
  Value,
  Overrides,
  TimePoint,
  // Config
  SessionStartConfig,
  // Breakpoints
  BreakpointKind,
  CompareOp,
  Breakpoint,
  StateEntryBreakpoint,
  TransitionFireBreakpoint,
  ActionInvokeBreakpoint,
  ConstraintViolationBreakpoint,
  ThresholdCrossingBreakpoint,
  ConditionalBreakpoint,
  // Inspection
  InspectionResult,
  // Verdict contract (shared across verdict-producing workflows, R1.3 + R3)
  VerdictKind,
  EvidenceRef,
  Verdict,
  // Verify runner (R3.2)
  VerifySuiteKind,
  VerifyRunConfig,
  VerifyRunSummary,
  VerifyRunResult,
  // Interfaces
  SessionControl,
  VariableInspection,
  // Session archive (R4.1)
  SessionOrigin,
  ArchivedSessionSummary,
  ArchivedSession,
  // Batch sessions (R5.0 — Sweep / Monte Carlo / Trade Study)
  BatchKind,
  ChildStatus,
  ChildStatusKind,
  BatchStatus,
  ChildDescriptor,
  BatchSession,
  // Sweep filters (R5.4)
  ParamPredicate,
  SliceFilter,
  // Diagnostics (R6.1)
  Diagnostic,
  DiagnosticSeverity,
  DiagnosticSpan,
  DiagnosticRelatedLocation,
  DiagnosticTag,
  // Traceability matrix (R6.2)
  TraceMatrixRow,
  TraceRow,
  TraceColumn,
  TraceLink,
  TraceMatrix,
  // Causation trace (R7.1)
  CausationEvent,
  CausationKind,
  CausationTraceRequest,
  CausationTraceResult,
  // Sensitivity (R7.4)
  SensitivityMethod,
  SensitivityResult,
  ParamRange,
  SensitivityAnalyzeRequest,
  SensitivityAnalyzeResult,
} from './types';

export {
  useSessionControl,
  buildCreateParams,
  overridesToTuples,
  serializeValue,
  createBreakpointClient,
} from './SessionControl';

export {
  useVariableInspection,
  createVariableInspection,
  readCurrent,
  readAtTick,
  readSeries,
  readAcrossSessions,
} from './VariableInspection';

// R1.5 — SessionEvents stream (Agent D). Polling-based event bus derived
// from `sysml.sessions.info` snapshot diffs. See ./SessionEvents.ts for
// the transport choice rationale.
export {
  SessionEventBus,
  useSessionEvents,
  installSessionEventBus,
  getSessionEventBus,
  DEFAULT_POLL_INTERVAL_MS,
} from './SessionEvents';

// BP5 — app-boot wiring that installs the default `SessionEventBus`
// against the live react-query session cache. See the hook's doc
// comment for the "never installed before" gap this closes.
export { useInstallSessionEventBus } from './useInstallSessionEventBus';
export type {
  SessionEvents,
  SessionEvent,
  SessionEventKind,
  SessionEventHandler,
  SnapshotSource,
  Scheduler,
  Unsub,
} from './SessionEvents';
