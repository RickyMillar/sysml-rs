/**
 * Actor identity gate — the one home of the "set your name once" flow
 * that every workflow write goes through (steward ruling 2026-07-16:
 * actor is an explicit per-user setting, never an OS-username default;
 * the backend hard-rejects blank actors).
 *
 * Renders the one-time collection row until an actor is set, then hands
 * the actor to `children`. Shared by the suspect popover and the rail
 * workflow controls — do not open-code this prompt a second time.
 */

import { useState, type ReactNode } from 'react';
import { useWorkflowActorStore } from './actorStore';

export const WORKFLOW_INPUT_STYLE = {
  flex: 1,
  background: 'transparent',
  border: 'none',
  outline: 'none',
  borderBottom: '1px solid var(--border-default)',
  color: 'var(--text-primary)',
  fontSize: 'var(--text-xs)',
  fontFamily: 'var(--font-mono)',
  padding: '2px 0',
} as const;

export function ActorGate({
  prompt,
  children,
}: {
  /** Why a name is needed, e.g. "Attestations are signed". */
  prompt: string;
  children: (actor: string) => ReactNode;
}) {
  const actor = useWorkflowActorStore((s) => s.actor);
  const setActor = useWorkflowActorStore((s) => s.setActor);
  const [draft, setDraft] = useState('');

  if (actor !== null) return <>{children(actor)}</>;

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
      <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)' }}>
        {prompt} — set your name once:
      </span>
      <input
        data-testid="workflow-actor-input"
        value={draft}
        placeholder="your name"
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') setActor(draft);
        }}
        style={WORKFLOW_INPUT_STYLE}
      />
      <button
        type="button"
        data-testid="workflow-actor-save"
        disabled={draft.trim() === ''}
        onClick={() => setActor(draft)}
        style={{
          height: 26,
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
          background: 'transparent',
          color: 'var(--text-primary)',
          fontSize: 'var(--text-xs)',
          padding: '0 10px',
          cursor: 'pointer',
          opacity: draft.trim() === '' ? 0.5 : 1,
        }}
      >
        save
      </button>
    </div>
  );
}
