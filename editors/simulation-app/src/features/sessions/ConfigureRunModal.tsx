/**
 * ConfigureRunModal — the "Configure run" surface (ninebar Phase 3).
 *
 * Plan §3: "Run config → 'Configure run' modal (target picker, dt,
 * speed…). The live surface is never cluttered with setup controls."
 * Everything here writes straight to the SAME stores the controller
 * reads (`useWorkspaceUIStore.activeSessionTarget`,
 * `useSessionStore.dtMs` / `stepsPerSecond`), so there is no
 * apply/submit step — closing the modal is enough. Changing the target
 * abandons the current session via RunWorkflow's existing
 * target-change effect; dt applies to the NEXT session (it is a
 * `sessions.create` parameter), which the field's hint says out loud.
 *
 * Opened by id via the modal registry (`openModal(CONFIGURE_RUN_MODAL_ID)`)
 * — from the frame's run-control cluster gear, and available to Cmd-K.
 */
import { useMemo, useState } from 'react';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useSessionStore } from '@/features/sessions/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useRunTargets } from '@/features/run-targets/queries';
import { registerModal } from '@/shared/overlays/modalStore';

export const CONFIGURE_RUN_MODAL_ID = 'configure-run';

const FIELD_LABEL: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
  letterSpacing: '0.03em',
  textTransform: 'uppercase',
};

const FIELD_HINT: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-muted)',
};

const INPUT_STYLE: React.CSSProperties = {
  background: 'var(--surface-sunken)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 'var(--radius-sm)',
  padding: '4px 8px',
  fontSize: 'var(--text-sm)',
};

export function ConfigureRunModal() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const setActiveSessionTarget = useWorkspaceUIStore((s) => s.setActiveSessionTarget);
  const dtMs = useSessionStore((s) => s.dtMs);
  const setDtMs = useSessionStore((s) => s.setDtMs);
  const stepsPerSecond = useSessionStore((s) => s.stepsPerSecond);
  const setStepsPerSecond = useSessionStore((s) => s.setStepsPerSecond);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const scenarioOverrides = useSessionStore((s) => s.scenarioOverrides);
  const setScenarioOverrides = useSessionStore((s) => s.setScenarioOverrides);
  const [draftKey, setDraftKey] = useState('');
  const [draftValue, setDraftValue] = useState('');

  const addScenarioOverride = () => {
    const key = draftKey.trim();
    if (!key) return;
    // Last write wins on a repeated key rather than sending a list the backend
    // would apply twice in an order the user never chose.
    const next: [string, string][] = [
      ...scenarioOverrides.filter(([k]) => k !== key),
      [key, draftValue.trim()],
    ];
    setScenarioOverrides(next);
    setDraftKey('');
    setDraftValue('');
  };

  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const { data: groups, isLoading } = useRunTargets(workspaceRoot, wsData?.uris ?? []);

  const targetCount = useMemo(
    () => (groups ?? []).reduce((n, g) => n + g.targets.length, 0),
    [groups],
  );

  return (
    <div data-testid="configure-run-modal" className="flex flex-col gap-4" style={{ minWidth: 380 }}>
      {/* ── Target ── */}
      <label className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Run target</span>
        <select
          data-testid="configure-run-target"
          value={activeSessionTarget ?? ''}
          onChange={(e) => setActiveSessionTarget(e.target.value || null)}
          style={INPUT_STYLE}
        >
          {/* The empty value is a REAL choice, not a placeholder: no target
              means `sessions.create` runs the whole-workspace orchestrator
              (`createParamsForTarget(null)` → `{ uri: '__workspace__' }`).
              It used to read "Select a target…", which framed the default —
              and on a large model the only reachable — run as an unmade
              decision (punch-list finding 31). */}
          <option value="">Whole workspace — all subsystems</option>
          {(groups ?? []).map((g) => (
            <optgroup key={g.label} label={g.label}>
              {g.targets.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name ?? '(unnamed)'}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        <span style={FIELD_HINT}>
          {isLoading
            ? 'Discovering runnable elements…'
            : targetCount === 0
              ? 'No individually runnable elements found — the whole workspace still runs.'
              : `Or narrow the run to one of ${targetCount} runnable elements.`}
        </span>
        {activeSessionId && (
          <span style={FIELD_HINT}>
            Changing the target abandons the current session; the next Run creates a fresh one.
          </span>
        )}
      </label>

      {/* ── Scenario ──
          Create-time parameter overrides. These go to `sessions.create`, so
          they hold from the first tick and the whole trajectory can be
          attributed to them. The live-run equivalent (the override popover on
          a variable) is a different thing and says so — it changes a run
          already in progress. Keeping the two apart is the J3 invariant
          "a scenario/override is distinguishable from the underlying model
          source". */}
      <div className="flex flex-col gap-1.5" data-testid="configure-run-scenario">
        <span style={FIELD_LABEL}>Scenario overrides</span>

        {scenarioOverrides.length === 0 ? (
          <span style={FIELD_HINT} data-testid="configure-run-scenario-empty">
            None — runs the model&rsquo;s declared defaults (the baseline scenario).
          </span>
        ) : (
          <ul className="flex flex-col gap-1" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
            {scenarioOverrides.map(([k, v]) => (
              <li
                key={k}
                data-testid={`configure-run-scenario-row-${k}`}
                className="flex items-center gap-2"
              >
                <span className="mono-text" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
                  {k}
                </span>
                <span style={{ color: 'var(--text-muted)' }}>=</span>
                <span
                  className="mono-text"
                  style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)', flex: 1 }}
                >
                  {v}
                </span>
                <button
                  type="button"
                  data-testid={`configure-run-scenario-remove-${k}`}
                  onClick={() =>
                    setScenarioOverrides(scenarioOverrides.filter(([key]) => key !== k))
                  }
                  title={`Remove the ${k} override`}
                  style={{
                    background: 'transparent',
                    border: 'none',
                    color: 'var(--text-secondary)',
                    cursor: 'pointer',
                    fontSize: 'var(--text-xs)',
                  }}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="flex items-center gap-2">
          <input
            data-testid="configure-run-scenario-key"
            value={draftKey}
            onChange={(e) => setDraftKey(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                addScenarioOverride();
              }
            }}
            placeholder="parameter"
            className="mono-text"
            style={{ ...INPUT_STYLE, flex: 1, minWidth: 0 }}
          />
          <span style={{ color: 'var(--text-muted)' }}>=</span>
          <input
            data-testid="configure-run-scenario-value"
            value={draftValue}
            onChange={(e) => setDraftValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                addScenarioOverride();
              }
            }}
            placeholder="value"
            className="mono-text"
            style={{ ...INPUT_STYLE, width: 96 }}
          />
          <button
            type="button"
            data-testid="configure-run-scenario-add"
            onClick={addScenarioOverride}
            disabled={draftKey.trim() === ''}
            style={{
              ...INPUT_STYLE,
              cursor: draftKey.trim() === '' ? 'default' : 'pointer',
              color: draftKey.trim() === '' ? 'var(--text-disabled)' : 'var(--text-primary)',
            }}
          >
            Add
          </button>
        </div>

        <span style={FIELD_HINT}>
          Applied when the next session is built, so they hold from its first tick — unlike a
          variable override, which changes a run already in progress. An unknown parameter name
          is rejected at creation rather than ignored.
        </span>
      </div>

      {/* ── dt ── */}
      <label className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Step size — dt (ms)</span>
        <input
          data-testid="configure-run-dt"
          type="number"
          min={0.001}
          max={1000}
          step="any"
          value={dtMs}
          onChange={(e) => {
            const v = Number(e.target.value);
            if (Number.isFinite(v)) setDtMs(v);
          }}
          className="mono-text"
          style={{ ...INPUT_STYLE, width: 120 }}
        />
        <span style={FIELD_HINT}>
          Applies to the next session (a create-time parameter) — clamped to 0.001–1000&nbsp;ms.
        </span>
      </label>

      {/* ── Speed ── */}
      <label className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Play speed (steps / second)</span>
        <input
          data-testid="configure-run-speed"
          type="number"
          min={1}
          max={1000}
          value={stepsPerSecond}
          onChange={(e) => {
            const v = Number(e.target.value);
            if (Number.isFinite(v) && v >= 1) setStepsPerSecond(v);
          }}
          className="mono-text"
          style={{ ...INPUT_STYLE, width: 120 }}
        />
        <span style={FIELD_HINT}>
          Scales the bulk-step chunk size, not the request rate — applies live.
        </span>
      </label>
    </div>
  );
}

registerModal({
  id: CONFIGURE_RUN_MODAL_ID,
  title: 'Configure run',
  component: ConfigureRunModal,
});
