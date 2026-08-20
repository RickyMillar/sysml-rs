/**
 * useDrillFromVerdict — R3.5 drill-from-verdict-to-evidence hook.
 *
 * Given a `Verdict`, navigate the user to `/run` deep-linked to the
 * session/tick/element that produced the verdict. When the backend has
 * not yet populated `Verdict.evidence`, fall through to a friendly
 * toast that names the R3.5 backend task rather than silently failing.
 *
 * Boundaries:
 *   - This file owns the "click handler + navigation" policy. It does
 *     NOT implement the run-from-query-string reader (RunWorkflow side)
 *     — that is flagged in the R3.5 report and deferred until Agent N's
 *     runner wires query-string → session start.
 *   - The toast UI is presentational (DrillStatusToast.tsx); timing and
 *     visibility state live here in `DrillProvider`.
 *   - The hook is mountable from VerifyWorkflow via `<DrillProvider>`.
 *     Any site that owns a `PassFailGridViewer.onVerdictSelect` callback
 *     passes `drill` directly — no coupling to a viewer component.
 *
 * Build target URL shape:
 *   /run?session={session_id}&tick={tick}&element={element_id}
 * (element param omitted if `evidence.element_id` is null/undefined.)
 */

import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useNavigate, type NavigateFunction } from 'react-router-dom';
import type { Verdict } from '@/engine/types';
import {
  DrillStatusToast,
  DRILL_NO_EVIDENCE_MESSAGE,
} from './DrillStatusToast';

// ── Public types ─────────────────────────────────────────────────────

/**
 * API returned from `useDrillFromVerdict()`.
 *
 * `drill(verdict)` either navigates or shows the degraded toast;
 * `hasEvidence(verdict)` is a pure predicate that callers can use to
 * enable/disable drill affordances before the click.
 */
export interface DrillFromVerdictApi {
  /**
   * Attempt to drill into the verdict's evidence.
   * - Evidence present → `navigate('/run?session=…&tick=…&element=…')`.
   * - Evidence absent → show the degraded-mode toast.
   */
  drill: (verdict: Verdict) => void;
  /** Pure predicate — true iff the verdict carries a populated evidence ref. */
  hasEvidence: (verdict: Verdict) => boolean;
}

/**
 * Shape of the internal provider context. Exported only so tests can
 * inject a fake without having to mount a router.
 */
export interface DrillContextValue {
  /** Imperative `navigate` — matches React Router's `NavigateFunction`. */
  navigate: NavigateFunction;
  /** Invoked when evidence is absent; provider routes to the toast state. */
  showToast: (message: string) => void;
}

// ── Helpers (pure, exported for tests) ───────────────────────────────

/**
 * Pure predicate — `evidence == null` and `evidence == undefined` both
 * count as absent (per the R3.5 contract).
 */
export function hasEvidence(verdict: Verdict): boolean {
  const ev = verdict?.evidence;
  if (ev === null || ev === undefined) return false;
  // Treat a malformed evidence ref (no session_id) as "absent" so the
  // degraded path is consistent when the backend partially populates.
  if (typeof ev.session_id !== 'string' || ev.session_id.length === 0) {
    return false;
  }
  return true;
}

/**
 * Build the `/run?session=…&tick=…&element=…` URL the drill handler
 * navigates to. Exported so the test suite can assert the exact shape
 * without re-implementing URLSearchParams.
 */
export function buildDrillUrl(verdict: Verdict): string | null {
  if (!hasEvidence(verdict)) return null;
  const ev = verdict.evidence!;
  const params = new URLSearchParams();
  params.set('session', ev.session_id);
  params.set('tick', String(ev.tick));
  if (ev.element_id) {
    params.set('element', ev.element_id);
  }
  return `/run?${params.toString()}`;
}

// ── Context ──────────────────────────────────────────────────────────

const DrillContext = createContext<DrillContextValue | null>(null);

/** Auto-dismiss the degraded-mode toast after this many milliseconds. */
export const DRILL_TOAST_DURATION_MS = 5000;

// ── Provider ─────────────────────────────────────────────────────────

export interface DrillProviderProps {
  children: ReactNode;
  /**
   * Escape hatch for tests — inject a pre-built context instead of
   * reading React Router + local toast state. Production callers omit
   * this; VerifyWorkflow mounts `<DrillProvider>` at the root and lets
   * the provider wire itself up.
   */
  value?: DrillContextValue;
}

/**
 * Mount at the VerifyWorkflow root. Supplies the `useDrillFromVerdict()`
 * context AND renders the toast UI when the degraded path fires.
 *
 * `DrillProvider` dispatch:
 *  - `value` prop supplied  → render `InjectedDrillProvider` (no router hooks,
 *    useful for tests that mock navigate/showToast without a `<Router>`).
 *  - `value` prop absent    → render `RouterDrillProvider` (calls
 *    `useNavigate` + owns toast timer state).
 *
 * Splitting into two components keeps the React rules-of-hooks happy:
 * each leaf component always calls the same set of hooks in the same order.
 *
 * API shape for Agent M's VerifyWorkflow adoption:
 *
 * ```tsx
 * // VerifyWorkflow.tsx (future):
 * import { DrillProvider, useDrillFromVerdict } from './useDrillFromVerdict';
 *
 * export function VerifyWorkflow() {
 *   return (
 *     <DrillProvider>
 *       <VerifyWorkflowBody />
 *     </DrillProvider>
 *   );
 * }
 *
 * function VerifyWorkflowBody() {
 *   const { drill, hasEvidence } = useDrillFromVerdict();
 *   return (
 *     <PassFailGridViewer
 *       verdicts={verdicts}
 *       onVerdictSelect={drill}
 *       hasEvidence={hasEvidence}
 *     />
 *   );
 * }
 * ```
 */
export function DrillProvider(props: DrillProviderProps) {
  if (props.value) {
    return createElement(InjectedDrillProvider, props);
  }
  return createElement(RouterDrillProvider, props);
}

function InjectedDrillProvider(props: DrillProviderProps) {
  // When an explicit context is injected (tests), skip the toast UI —
  // callers inspect `value.showToast` directly.
  return createElement(
    DrillContext.Provider,
    { value: props.value! },
    props.children,
  );
}

function RouterDrillProvider(props: DrillProviderProps) {
  const { children } = props;
  const navigate = useNavigate();

  const [toastMessage, setToastMessage] = useState<string | null>(null);
  const timerRef = useRef<number | null>(null);

  const clearToastTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const showToast = useCallback(
    (message: string) => {
      setToastMessage(message);
      clearToastTimer();
      timerRef.current = window.setTimeout(() => {
        setToastMessage(null);
        timerRef.current = null;
      }, DRILL_TOAST_DURATION_MS);
    },
    [clearToastTimer],
  );

  // Clean up the pending auto-dismiss timer on unmount so no setState
  // lands on an unmounted component.
  useEffect(() => {
    return () => {
      clearToastTimer();
    };
  }, [clearToastTimer]);

  const ctx = useMemo<DrillContextValue>(
    () => ({ navigate, showToast }),
    [navigate, showToast],
  );

  const dismiss = useCallback(() => {
    clearToastTimer();
    setToastMessage(null);
  }, [clearToastTimer]);

  return createElement(
    DrillContext.Provider,
    { value: ctx },
    children,
    toastMessage
      ? createElement(DrillStatusToast, {
          message: toastMessage,
          onDismiss: dismiss,
        })
      : null,
  );
}

// ── Hook ─────────────────────────────────────────────────────────────

/**
 * Main entry point. Must be called inside a `<DrillProvider>`.
 *
 * Returns a stable `{ drill, hasEvidence }` object. Callers forward
 * `drill` directly to their viewer's `onVerdictSelect` callback.
 */
export function useDrillFromVerdict(): DrillFromVerdictApi {
  const ctx = useContext(DrillContext);
  if (!ctx) {
    throw new Error(
      'useDrillFromVerdict must be called inside a <DrillProvider>. ' +
        'Mount <DrillProvider> near the workflow root.',
    );
  }

  const drill = useCallback(
    (verdict: Verdict) => {
      const url = buildDrillUrl(verdict);
      if (url) {
        ctx.navigate(url);
      } else {
        ctx.showToast(DRILL_NO_EVIDENCE_MESSAGE);
      }
    },
    [ctx],
  );

  return useMemo<DrillFromVerdictApi>(
    () => ({ drill, hasEvidence }),
    [drill],
  );
}
