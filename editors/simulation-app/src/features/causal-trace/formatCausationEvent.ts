/**
 * Pure formatter for [`CausationEvent`] rows in the CausalTracePanel (R7.1).
 *
 * Lives in its own file so the row component stays a thin renderer over
 * a fully-deterministic label function that's easy to unit-test across
 * every `CausationKind` variant.
 */

import type { CausationEvent, CausationKind, Value } from '@/engine/types';

/**
 * Short human-facing label prefix for each kind. Used by the icon
 * chip AND by the row's primary text when no `detail` is present.
 */
export const CAUSATION_KIND_LABEL: Record<CausationKind['kind'], string> = {
  variable_write: 'Wrote',
  transition_fire: 'Transition',
  action_invoke: 'Action',
  constraint_evaluated: 'Constraint',
  event_injected: 'Event',
  ode_step: 'ODE step',
};

/**
 * Material-symbols icon name for each kind. Intentionally stable across
 * light / dark modes — the panel uses accent colour to carry severity,
 * not the icon shape.
 */
export const CAUSATION_KIND_ICON: Record<CausationKind['kind'], string> = {
  variable_write: 'edit',
  transition_fire: 'arrow_forward',
  action_invoke: 'play_arrow',
  constraint_evaluated: 'rule',
  event_injected: 'bolt',
  ode_step: 'show_chart',
};

function renderValue(v: Value): string {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'boolean') return v ? 'true' : 'false';
  if (typeof v === 'number') {
    if (Number.isFinite(v)) {
      const abs = Math.abs(v);
      if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) {
        return v.toExponential(3);
      }
      return Number.isInteger(v) ? v.toString() : v.toFixed(4);
    }
    return v.toString();
  }
  if (typeof v === 'string') return `"${v}"`;
  if (Array.isArray(v)) return `[${v.length} items]`;
  return JSON.stringify(v);
}

/**
 * Deterministic one-line summary for a causation event. Used by the
 * panel's chain rows + the root header. The `detail` field already
 * carries a backend-computed summary; this helper re-renders from
 * `kind` so the UI can tolerate older backends that don't populate
 * `detail` consistently, and so tests can exercise every branch without
 * depending on the backend's string formatter.
 */
export function formatCausationEvent(ev: CausationEvent): string {
  switch (ev.kind) {
    case 'variable_write':
      return `${ev.var} = ${renderValue(ev.new_value)} (was ${renderValue(ev.old_value)})`;
    case 'transition_fire':
      return ev.event
        ? `${ev.actor}: ${ev.from} → ${ev.to} on \`${ev.event}\``
        : `${ev.actor}: ${ev.from} → ${ev.to}`;
    case 'action_invoke': {
      const args = ev.args.length > 0 ? `(${ev.args.join(', ')})` : '';
      return `${ev.action}${args}`;
    }
    case 'constraint_evaluated': {
      const verdict = ev.verdict ? 'pass' : 'FAIL';
      return `${ev.constraint}: ${verdict}`;
    }
    case 'event_injected':
      return ev.target
        ? `inject \`${ev.event}\` → ${ev.target}`
        : `inject \`${ev.event}\``;
    case 'ode_step': {
      const count = ev.changed_vars.length;
      const head =
        count <= 3
          ? ev.changed_vars.join(', ')
          : `${ev.changed_vars.slice(0, 3).join(', ')}, +${count - 3} more`;
      return `dt=${ev.dt.toFixed(4)}s · ${head || 'no changes'}`;
    }
    default: {
      // Exhaustive-check guard — TypeScript narrows `ev` to `never` here.
      const _exhaustive: never = ev;
      return (_exhaustive as CausationEvent).detail || 'unknown event';
    }
  }
}

/**
 * A compact "tick · actor" prefix used by the row chip. Pure so tests
 * can snapshot it without mounting the component.
 */
export function formatCausationEventPrefix(ev: CausationEvent): string {
  return `t=${ev.tick} · ${ev.actor || '—'}`;
}
