/**
 * StepErrorBanner — surfaces the most recent `sessions.step` failure (P5).
 *
 * Before this fix, a failed step — most commonly RS002 "unknown override
 * target" from a stale or mistyped draft override typed via the tree's
 * right-click Override prompt — was silently discarded: the autoplay loop
 * (`useSessionController`'s `tick`) burned its retry budget then flipped to
 * the `error` phase with no message, and the manual `stepOnce` path did
 * nothing but `console.error`. Either way the draft override itself was
 * already gone (single-shot `drainDraftOverrides` clears it before the
 * request even resolves), so the user had no way to tell what happened or
 * fix it without opening devtools.
 *
 * Reads `stepError` from the session store, renders an error banner with
 * the raw backend message (which — for `RS002` — already names the
 * offending target), and a dismiss button that clears it. Renders nothing
 * when `stepError` is null. Modeled on `DrilledFromBanner` (same
 * store-field + dismiss pattern) with `WorkspaceLoadErrorBanner`'s
 * error-red palette, so this doesn't invent a second banner shape.
 */

import { useSessionStore } from '@/features/sessions/store';

export function StepErrorBanner() {
  const stepError = useSessionStore((s) => s.stepError);
  const clearStepError = useSessionStore((s) => s.clearStepError);

  if (!stepError) return null;

  return (
    <div
      role="alert"
      data-testid="step-error-banner"
      className="flex items-center gap-3 px-4 py-2 border-b"
      style={{
        background: 'color-mix(in srgb, var(--severity-error) 15%, transparent)',
        borderColor: 'color-mix(in srgb, var(--severity-error) 35%, transparent)',
        fontSize: 'var(--text-xs)',
        color: 'var(--on-surface)',
      }}
    >
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{ fontSize: '16px', color: 'var(--severity-error)', flexShrink: 0 }}
      >
        error
      </span>

      <div className="flex items-center gap-2 flex-1 min-w-0">
        <span style={{ fontWeight: 600 }}>Step failed</span>
        <span style={{ color: 'var(--on-surface-variant)' }}>·</span>
        <span
          className="mono-text truncate"
          data-testid="step-error-message"
          title={stepError.message}
        >
          {stepError.message}
        </span>
      </div>

      <button
        type="button"
        onClick={clearStepError}
        data-testid="step-error-dismiss"
        aria-label="Dismiss step error"
        className="flex items-center gap-1 px-2 py-1 rounded hover:bg-black/10"
        style={{
          color: 'var(--on-surface-variant)',
          fontSize: 'var(--text-xs)',
          background: 'transparent',
          border: '1px solid color-mix(in srgb, var(--severity-error) 25%, transparent)',
          cursor: 'pointer',
          flexShrink: 0,
        }}
      >
        Dismiss
      </button>
    </div>
  );
}
