/**
 * InjectorDock — the frame event-injection dock (ninebar Phase 3,
 * audit F1: in-loop actions are dock/popover, NEVER modals).
 *
 * A compact bolt button opens an anchored Popover: subsystem picker +
 * event name (with the subsystem's currently-eligible transitions as
 * datalist candidates) → Inject. A successful injection collapses the
 * dock to a one-click "repeat last event" companion button, which is
 * the actual high-frequency gesture during a debugging loop.
 *
 * Wire: `useInjectEvent` → `sysml.sessions.inject` — NEVER
 * `stepOnce(event)`, which errors on orchestrator sessions (audit F15;
 * the retired `EventInjector.tsx` died of exactly that).
 *
 * Re-render discipline (F15): this is always-on frame chrome, so it
 * must not subscribe to the live snapshot (object identity changes
 * every tick). Subsystem/event candidates are read IMPERATIVELY from
 * `useSessionLiveStore.getState()` when the popover opens — a popover
 * frame-behind on candidates is imperceptible; a per-tick re-render of
 * frame chrome is not.
 */
import { useRef, useState } from 'react';
import { useSessionStore } from '@/features/sessions/store';
import { useSessionLiveStore } from '@/features/sessions/sessionLiveStore';
import { useInjectEvent } from '@/features/sessions/mutations';
import { Popover } from '@/shared/overlays/Popover';
import { FuzzyCombobox } from '@/shared/pickers/FuzzyCombobox';
import { ChipButton } from './RunControls';

interface LastInjection {
  subsystem: string;
  event: string;
}

/** Imperative candidate read — see re-render discipline note above. */
function readCandidates(): Record<string, Array<[string, string]>> {
  const snap = useSessionLiveStore.getState().snapshot;
  if (!snap) return {};
  const out: Record<string, Array<[string, string]>> = {};
  for (const [name, sub] of Object.entries(snap.subsystems)) {
    out[name] = sub.available_transitions ?? [];
  }
  return out;
}

export function InjectorDock() {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const inject = useInjectEvent();
  // ChipButton doesn't forward refs — anchor the popover to a wrapper span.
  const anchorRef = useRef<HTMLSpanElement | null>(null);

  const [open, setOpen] = useState(false);
  const [candidates, setCandidates] = useState<Record<string, Array<[string, string]>>>({});
  const [subsystem, setSubsystem] = useState('');
  const [eventName, setEventName] = useState('');
  const [last, setLast] = useState<LastInjection | null>(null);

  const disabled = !activeSessionId;
  const subsystems = Object.keys(candidates);
  const eventOptions = (candidates[subsystem] ?? []).map(([ev]) => ev);

  const doInject = (target: LastInjection) => {
    if (!activeSessionId) return;
    inject.mutate(
      { sessionId: activeSessionId, subsystem: target.subsystem, event: target.event },
      { onSuccess: () => setLast(target) },
    );
  };

  return (
    <>
      <span ref={anchorRef} className="inline-flex">
        <ChipButton
          testId="frame-injector-open"
          icon="bolt"
          onClick={() => {
            const next = readCandidates();
            setCandidates(next);
            const names = Object.keys(next);
            if (!names.includes(subsystem)) setSubsystem(names[0] ?? '');
            setOpen(true);
          }}
          disabled={disabled}
          title={disabled ? 'Inject event — needs a live session' : 'Inject an event into a subsystem'}
        >
          Inject
        </ChipButton>
      </span>

      {last && (
        <ChipButton
          testId="frame-injector-repeat"
          icon="replay"
          onClick={() => doInject(last)}
          disabled={disabled || inject.isPending}
          title={`Repeat last event: ${last.event} → ${last.subsystem}`}
        >
          {last.event}
        </ChipButton>
      )}

      <Popover anchorEl={anchorRef.current} open={open} onClose={() => setOpen(false)} placement="bottom">
        <div data-testid="injector-popover" className="flex flex-col gap-2" style={{ padding: 10, minWidth: 260 }}>
          <label className="flex flex-col gap-1">
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)', textTransform: 'uppercase', letterSpacing: '0.03em' }}>
              Subsystem
            </span>
            <select
              data-testid="injector-subsystem"
              value={subsystem}
              onChange={(e) => setSubsystem(e.target.value)}
              style={{
                background: 'var(--surface-sunken)',
                color: 'var(--text-primary)',
                border: '1px solid var(--border-default)',
                borderRadius: 'var(--radius-sm)',
                padding: '4px 8px',
                fontSize: 'var(--text-sm)',
              }}
            >
              {subsystems.length === 0 && <option value="">No live subsystems yet — step once first</option>}
              {subsystems.map((s) => (
                <option key={s} value={s}>{s}</option>
              ))}
            </select>
          </label>

          <label className="flex flex-col gap-1">
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)', textTransform: 'uppercase', letterSpacing: '0.03em' }}>
              Event
            </span>
            <FuzzyCombobox
              testId="injector-event"
              value={eventName}
              onChange={setEventName}
              candidates={eventOptions}
              placeholder={eventOptions.length > 0 ? `e.g. ${eventOptions[0]}` : 'event name'}
              inputStyle={{
                background: 'var(--surface-sunken)',
                color: 'var(--text-primary)',
                border: '1px solid var(--border-default)',
                borderRadius: 'var(--radius-sm)',
                padding: '4px 8px',
                fontSize: 'var(--text-sm)',
                fontFamily: 'var(--font-mono)',
              }}
            />
          </label>

          <button
            type="button"
            data-testid="injector-submit"
            disabled={!subsystem || !eventName.trim() || inject.isPending}
            onClick={() => {
              doInject({ subsystem, event: eventName.trim() });
              setOpen(false);
            }}
            style={{
              alignSelf: 'flex-end',
              background: 'var(--accent)',
              color: 'var(--text-inverse)',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              padding: '4px 14px',
              fontSize: 'var(--text-sm)',
              fontWeight: 500,
              cursor: !subsystem || !eventName.trim() ? 'not-allowed' : 'pointer',
              opacity: !subsystem || !eventName.trim() ? 0.5 : 1,
            }}
          >
            Inject
          </button>
        </div>
      </Popover>
    </>
  );
}
