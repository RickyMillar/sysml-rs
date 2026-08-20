/**
 * StepSizeAdvisoryBadge — surfaces the runtime step-size (under-resolution)
 * advisory next to the dt input in the session header.
 *
 * The backend (P1 dt-under-resolution arc) attaches a per-ODE-subsystem
 * `step_size_health` array to every full `ExecutionSnapshot`. When a subsystem's
 * oscillation is *step-bound* — its observed cycle resolves to fewer than the
 * backend's target ticks/cycle — the entry is `{ kind: "under_resolved", ... }`
 * and carries a suggested finer `dt`.
 *
 * This is an ADVISORY, never an error: the model is not in question, and this
 * badge NEVER changes dt automatically — it just tells the user. It renders
 * nothing unless at least one subsystem is under-resolved.
 */

import { useSessionStore } from './store';

/** One entry of the backend `step_size_health` array. */
interface StepSizeHealthEntry {
  subsystem: string;
  advisory:
    | { kind: 'not_applicable' }
    | { kind: 'ok'; ticks_per_cycle: number }
    | { kind: 'under_resolved'; ticks_per_cycle: number; suggested_dt_ms: number };
}

/** An under-resolved subsystem, flattened for rendering. */
interface UnderResolved {
  subsystem: string;
  ticksPerCycle: number;
  suggestedDtMs: number;
}

/**
 * Extract the under-resolved subsystems from a raw `latest_snapshot`. Defensive
 * about shape — `latest_snapshot` is typed `Record<string, unknown>` on the
 * frontend, so we validate each field before trusting it.
 */
export function extractUnderResolved(
  snapshot: Record<string, unknown> | null | undefined,
): UnderResolved[] {
  const raw = snapshot?.step_size_health;
  if (!Array.isArray(raw)) return [];
  const out: UnderResolved[] = [];
  for (const entry of raw as StepSizeHealthEntry[]) {
    const advisory = entry?.advisory;
    if (advisory && advisory.kind === 'under_resolved') {
      out.push({
        subsystem: entry.subsystem,
        ticksPerCycle: advisory.ticks_per_cycle,
        suggestedDtMs: advisory.suggested_dt_ms,
      });
    }
  }
  return out;
}

/** Round a suggested dt to a compact, human-readable string. */
function fmtDt(ms: number): string {
  if (ms >= 1) return ms.toFixed(2).replace(/\.?0+$/, '');
  // Sub-millisecond suggestions want more precision.
  return ms.toPrecision(3).replace(/\.?0+$/, '');
}

export function StepSizeAdvisoryBadge({
  snapshot,
}: {
  snapshot: Record<string, unknown> | null | undefined;
}) {
  const dtMs = useSessionStore((s) => s.dtMs);
  const underResolved = extractUnderResolved(snapshot);
  if (underResolved.length === 0) return null;

  // Surface the worst offender (smallest ticks/cycle) in the badge; the full
  // list is in the tooltip.
  const worst = underResolved.reduce((a, b) =>
    b.ticksPerCycle < a.ticksPerCycle ? b : a,
  );

  const tooltip = underResolved
    .map(
      (u) =>
        `${u.subsystem}: observed cycle ≈${u.ticksPerCycle} ticks at dt=${dtMs}ms ` +
        `— under-resolves the discrete-continuous coupling. ` +
        `Consider dt ≈ ${fmtDt(u.suggestedDtMs)}ms. ` +
        `Model behavior is not in question; this is a numerical step-size advisory.`,
    )
    .join('\n');

  return (
    <span
      data-testid="step-size-advisory"
      title={tooltip}
      className="flex items-center gap-1"
      style={{
        background: 'var(--surface-container-high)',
        // Advisory palette — informational, NOT the error/warning colour.
        border: '1px solid var(--primary)',
        color: 'var(--primary)',
        borderRadius: 4,
        padding: '2px 8px',
        fontSize: '11px',
        fontWeight: 600,
        maxWidth: 320,
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: '14px' }}>
        info
      </span>
      <span>
        {worst.subsystem}: {worst.ticksPerCycle} ticks/cycle — try dt ≈{' '}
        {fmtDt(worst.suggestedDtMs)}ms
      </span>
    </span>
  );
}
