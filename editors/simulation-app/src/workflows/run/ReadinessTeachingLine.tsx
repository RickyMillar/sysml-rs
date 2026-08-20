/**
 * ReadinessTeachingLine — one-line teaching hint on Run's idle/no-target
 * state, naming the readiness floor before the user starts a session
 * (ninebar Phase 1.5, audit F12: "this model has diagnostics that fail
 * at load -> open Browse to review before running").
 *
 * Placement choice: mounted unconditionally in `RunWorkflow.tsx`
 * (flag-agnostic), modeled on `StepErrorBanner`'s pattern — always
 * mounted, renders nothing unless its condition holds. The plan called
 * out two legacy candidates to avoid: `SessionHeader`'s existing
 * "no run target" messaging is explicitly legacy-shell-only (suppressed
 * under the `ninebar` flag in `RunWorkflow.tsx`), and the frame's
 * `RunControls` only has room for a button `title` tooltip, not a
 * teaching line a user can read at a glance. `StepErrorBanner`'s area
 * — a full-width strip above the four-zone body — is a real always-on
 * surface under BOTH shells, so this line reads identically whether or
 * not `ninebar` is flipped on. No flag check needed.
 *
 * Renders only when the session is idle with no target/active session
 * picked yet (a pre-run nudge, not a while-running banner) AND
 * `useModelReadiness().level === 'errors'`. Renders nothing for
 * 'unknown' (no workspace) or 'warnings' — teaching, not nagging; only
 * the hard-blocking case earns a line.
 */
import { Link } from 'react-router-dom';
import { useSessionStore } from '@/features/sessions/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useModelReadiness } from '@/features/readiness/useModelReadiness';

export function ReadinessTeachingLine() {
  const phase = useSessionStore((s) => s.phase);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const readiness = useModelReadiness();

  const idleNoTarget = phase === 'idle' && !activeSessionId && !activeSessionTarget;
  if (!idleNoTarget || readiness.level !== 'errors') return null;

  return (
    <div
      data-testid="readiness-teaching-line"
      className="flex items-center"
      style={{
        gap: 4,
        padding: '4px 16px',
        fontSize: 'var(--text-xs)',
        color: 'var(--text-muted)',
        borderBottom: '1px solid var(--border-hairline)',
      }}
    >
      <span>This model has diagnostics that fail at load — open</span>
      <Link to="/browse" data-testid="readiness-teaching-line-link" style={{ color: 'var(--text-accent)' }}>
        Browse
      </Link>
      <span>to review before running.</span>
    </div>
  );
}
