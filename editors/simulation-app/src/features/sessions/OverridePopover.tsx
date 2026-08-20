/**
 * OverridePopover — THE override surface (ninebar Phase 3, audits F9 +
 * F15 "three divergent override paths").
 *
 * Collapses what used to be three inconsistent ways to override a
 * variable — the fork-modal (`OverrideEditor`, retired with this), the
 * VariablesPane `window.prompt`, and the tree's `window.prompt` — into
 * one anchored, non-occluding popover with two explicit actions:
 *
 *   · **Apply & re-run** — writes `draftOverrides` (drained into the
 *     FIRST batch of the next step, exactly as before) and immediately
 *     triggers one step through the frame's controller bridge so the
 *     effect is visible in one click. When no controller is published
 *     (legacy shell off `/run`), it degrades to draft-only — same
 *     behaviour the prompts had.
 *   · **Fork & compare…** — the explicitly-labelled separate action
 *     (plan: never the default): forks with the override and lands on
 *     /compare with baseline = the original session.
 *
 * Validation: client-side numeric-format check only when the current
 * value is numeric. Backend rejections (RS002 — mistyped/stale target)
 * fail the STEP all-or-nothing and surface through the existing
 * `StepErrorBanner` (P5) — this popover does not duplicate that
 * channel.
 *
 * One host instance mounts at the LayoutGate (both shells); call sites
 * open it via `openOverridePopover(name, currentValue?, anchorEl?)`.
 * `anchorEl` falls back to `document.activeElement` so context-menu
 * call sites that only know a variable NAME still anchor sensibly.
 */
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { create } from 'zustand';
import { Popover } from '@/shared/overlays/Popover';
import { useSessionStore } from './store';
import { useForkSession } from './mutations';
import { useSessionControllerBridge } from '@/app/frame/sessionControllerBridge';

interface OverrideTarget {
  name: string;
  currentValue: string | null;
  anchorEl: HTMLElement | null;
}

interface OverridePopoverState {
  target: OverrideTarget | null;
  open: (name: string, currentValue?: string | number | null, anchorEl?: HTMLElement | null) => void;
  close: () => void;
}

export const useOverridePopoverStore = create<OverridePopoverState>((set) => ({
  target: null,
  open: (name, currentValue, anchorEl) =>
    set({
      target: {
        name,
        currentValue: currentValue == null ? null : String(currentValue),
        anchorEl:
          anchorEl ??
          (document.activeElement instanceof HTMLElement ? document.activeElement : null),
      },
    }),
  close: () => set({ target: null }),
}));

/** Call-site helper — importable without React context. */
export function openOverridePopover(
  name: string,
  currentValue?: string | number | null,
  anchorEl?: HTMLElement | null,
): void {
  useOverridePopoverStore.getState().open(name, currentValue, anchorEl);
}

const LABEL: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
  textTransform: 'uppercase',
  letterSpacing: '0.03em',
};

/** Mounted ONCE at the LayoutGate (App.tsx) — renders nothing until opened. */
export function OverridePopoverHost() {
  const target = useOverridePopoverStore((s) => s.target);
  const close = useOverridePopoverStore((s) => s.close);
  const controller = useSessionControllerBridge((s) => s.controller);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const setDraftOverride = useSessionStore((s) => s.setDraftOverride);
  const setCompareBaseline = useSessionStore((s) => s.setCompareBaseline);
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const forkSession = useForkSession();
  const navigate = useNavigate();

  const [value, setValue] = useState('');
  useEffect(() => {
    if (target) setValue(target.currentValue ?? '');
  }, [target]);

  if (!target) return null;

  const currentIsNumeric =
    target.currentValue !== null && target.currentValue !== '' && Number.isFinite(Number(target.currentValue));
  const valueInvalid = currentIsNumeric && (value.trim() === '' || !Number.isFinite(Number(value)));

  const applyAndRerun = () => {
    setDraftOverride(target.name, value);
    close();
    // Drafts drain into the first batch of the next step — trigger it
    // now when the frame controller is live so the effect is one click.
    if (controller) void controller.stepOnce();
  };

  const forkAndCompare = () => {
    if (!activeSessionId) return;
    forkSession.mutate(
      { sessionId: activeSessionId, overrides: [[target.name, value]] },
      {
        onSuccess: (forkedSummary) => {
          setCompareBaseline(activeSessionId);
          setActiveSession(forkedSummary.id);
          navigate('/run/compare');
          close();
        },
      },
    );
  };

  return (
    <Popover anchorEl={target.anchorEl} open onClose={close} placement="bottom">
      <div data-testid="override-popover" className="flex flex-col gap-2" style={{ padding: 10, minWidth: 240 }}>
        <div className="flex items-baseline gap-2">
          <span style={LABEL}>Override</span>
          <span className="mono-text" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
            {target.name}
          </span>
        </div>

        {target.currentValue !== null && (
          <div className="mono-text" style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
            current: {target.currentValue}
          </div>
        )}

        <input
          data-testid="override-popover-value"
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !valueInvalid) applyAndRerun();
          }}
          className="mono-text"
          style={{
            background: 'var(--surface-sunken)',
            color: 'var(--text-primary)',
            border: `1px solid ${valueInvalid ? 'var(--severity-error)' : 'var(--border-default)'}`,
            borderRadius: 'var(--radius-sm)',
            padding: '4px 8px',
            fontSize: 'var(--text-sm)',
          }}
        />
        {valueInvalid && (
          <span style={{ fontSize: 'var(--text-xs)', color: 'var(--severity-error)' }}>
            Expected a number (current value is numeric).
          </span>
        )}

        <div className="flex items-center gap-2 justify-end">
          <button
            type="button"
            data-testid="override-popover-fork"
            disabled={!activeSessionId || forkSession.isPending || valueInvalid}
            onClick={forkAndCompare}
            title="Fork the session with this override and compare against the original"
            style={{
              background: 'none',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-sm)',
              color: 'var(--text-secondary)',
              padding: '3px 10px',
              fontSize: 'var(--text-xs)',
              cursor: 'pointer',
            }}
          >
            Fork &amp; compare…
          </button>
          <button
            type="button"
            data-testid="override-popover-apply"
            disabled={valueInvalid}
            onClick={applyAndRerun}
            title={
              controller
                ? 'Save the override and step once so its effect is visible now'
                : 'Save the override — it applies on the next step'
            }
            style={{
              background: 'var(--accent)',
              color: 'var(--text-inverse)',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              padding: '3px 12px',
              fontSize: 'var(--text-xs)',
              fontWeight: 500,
              cursor: 'pointer',
              opacity: valueInvalid ? 0.5 : 1,
            }}
          >
            {controller ? 'Apply & re-run' : 'Apply on next step'}
          </button>
        </div>
      </div>
    </Popover>
  );
}
