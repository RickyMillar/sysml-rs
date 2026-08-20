/**
 * QuotaChip — frame chip reporting session budget usage (ninebar Phase 1,
 * plan §0 frame row / audit F7 — a contract-mandated surface: 30-session
 * quota with no switcher/indicator anywhere before this phase).
 *
 * Sourced from `sysml.sessions.quota` via `useSessionQuota`
 * (`features/sessions/queries.ts`, already wired — the same hook the
 * legacy `SessionStatusBar` counter uses) — no new query hook was needed
 * here; the command and its FE binding both already existed. Renders
 * "used/cap sessions" summed across the three kind buckets
 * (simulation/action/orchestrator), with a quiet warning tint
 * (`--severity-warning`) once usage crosses 90% of cap. Renders nothing
 * while quota hasn't loaded yet, and nothing if the backend ever reports
 * a zero cap (quota disabled) — there is nothing meaningful to show.
 */
import { useSessionQuota } from '@/features/sessions/queries';

const WARN_THRESHOLD = 0.9;

export function QuotaChip() {
  const { data: quota } = useSessionQuota();
  if (!quota) return null;

  const used =
    (quota.simulation?.used ?? 0) +
    (quota.action?.used ?? 0) +
    (quota.orchestrator?.used ?? 0);
  const cap =
    (quota.simulation?.cap ?? 0) +
    (quota.action?.cap ?? 0) +
    (quota.orchestrator?.cap ?? 0);
  if (cap <= 0) return null;

  const warn = used / cap >= WARN_THRESHOLD;

  return (
    <span
      data-testid="quota-chip"
      data-warn={warn}
      title={
        `${quota.simulation?.used ?? 0}/${quota.simulation?.cap ?? 0} simulation · ` +
        `${quota.action?.used ?? 0}/${quota.action?.cap ?? 0} action · ` +
        `${quota.orchestrator?.used ?? 0}/${quota.orchestrator?.cap ?? 0} orchestrator`
      }
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        height: 'var(--row-compact)',
        padding: '0 8px',
        fontSize: 'var(--text-sm)',
        fontFamily: 'var(--font-mono)',
        color: warn ? 'var(--severity-warning)' : 'var(--text-muted)',
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-sm)',
      }}
    >
      {used}/{cap} sessions
    </span>
  );
}
