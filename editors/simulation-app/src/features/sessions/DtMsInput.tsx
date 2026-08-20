/**
 * DtMsInput — compact numeric input for the backend simulation step size.
 *
 * Bound to useSessionStore.dtMs (default 1.0 ms). Flows through
 * createParamsForTarget() into the unified `sysml.sessions.create` command
 * (orchestrator sessions). Range clamped 0.001 .. 1000 ms.
 */

import { useEffect, useState } from 'react';
import { useSessionStore } from './store';

export function DtMsInput() {
  const dtMs = useSessionStore((s) => s.dtMs);
  const setDtMs = useSessionStore((s) => s.setDtMs);
  const [draft, setDraft] = useState(String(dtMs));

  // Keep the local draft in sync if the store value changes externally.
  useEffect(() => {
    setDraft(String(dtMs));
  }, [dtMs]);

  const commit = () => {
    const parsed = Number(draft);
    if (Number.isFinite(parsed)) {
      setDtMs(parsed);
    } else {
      setDraft(String(dtMs));
    }
  };

  return (
    <label
      data-testid="dt-ms-input"
      className="flex items-center gap-1"
      style={{
        background: 'var(--surface-container-high)',
        borderRadius: 4,
        padding: '2px 8px',
        fontSize: '11px',
        fontWeight: 600,
        color: 'var(--on-surface)',
      }}
      title="Simulation step size in milliseconds (0.001 - 1000)"
    >
      <span style={{ color: 'var(--outline)' }}>dt</span>
      <input
        type="number"
        min={0.001}
        max={1000}
        step={0.1}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            commit();
            (e.currentTarget as HTMLInputElement).blur();
          }
        }}
        data-testid="dt-ms-input-field"
        className="mono-text"
        style={{
          width: 56,
          background: 'var(--surface-container-highest)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 3,
          color: 'var(--on-surface)',
          padding: '1px 4px',
          fontSize: '11px',
          fontWeight: 600,
          textAlign: 'right',
        }}
      />
      <span style={{ color: 'var(--outline)' }}>ms</span>
    </label>
  );
}
