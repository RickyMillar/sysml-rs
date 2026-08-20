/**
 * SessionSwitcherChip — frame chip for the live+recent session switcher
 * (ninebar Phase 1, plan §0 frame row / audit F2).
 *
 * The trigger shows the active session (short id + kind) or a quiet
 * "no session" label when nothing is active. Clicking it opens a
 * `Popover` (see `src/shared/overlays/Popover.tsx`) listing every
 * session from `sessions.list` (`useSessionList`), each row carrying a
 * kind badge and a status badge (active/completed/expired — derived via
 * `toSessionRecord`, the same normalization `SessionTree` uses).
 * Expired rows are dimmed but still selectable (a user may want to
 * inspect one before it's reaped). Selecting a row calls
 * `useSessionStore.setActiveSession`. A trailing "Clear stale" row
 * fires the `sessions.reap` mutation (`useReapSessions`,
 * `features/sessions/mutations.ts`) to drop every expired session in
 * one shot.
 *
 * A leading "New session" row CREATES one. Session creation used to have
 * no affordance anywhere in the product chrome: the transport's Play
 * created one lazily but was itself disabled until an element was picked,
 * and this dropdown — the obvious place to look, sitting right beside a
 * chip reading "no session" — only listed and reaped. The one remaining
 * route was the Cmd-K developer console (punch-list findings 9 and 31).
 * The row calls the frame controller's `startSession()` through
 * `useSessionControllerBridge`, so it shares the single `sessions.create`
 * code path with the transport rather than opening a second one.
 */
import { useRef, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';
import { Popover } from '@/shared/overlays/Popover';
import { useSessionList } from '@/features/sessions/queries';
import { useReapSessions } from '@/features/sessions/mutations';
import { useSessionStore } from '@/features/sessions/store';
import { useActiveSessionReconciler } from '@/features/sessions/useActiveSessionReconciler';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { sessionBelongsToWorkspace } from '@/features/sessions/activeSessionPersistence';
import { toSessionRecord } from '@/features/sessions/normalize';
import type { SessionRecord } from '@/features/sessions/types';
import { useCompareStore } from '@/workflows/compare/useCompareStore';
import { useSessionControllerBridge } from './sessionControllerBridge';

function shortId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}

/**
 * One-line scenario label for a session.
 *
 * An empty override list is NOT unknown — it is the model's own declared
 * defaults, so it reads as "baseline" rather than being left blank. A reader
 * has to be able to tell a baseline run from a scenario run at a glance before
 * they can compare the two (J3: "nominal and severe evidence is retained
 * separately").
 */
export function scenarioLabel(createOverrides: [string, string][]): string {
  if (createOverrides.length === 0) return 'baseline';
  return createOverrides.map(([k, v]) => `${k}=${v}`).join(' · ');
}

function Badge({ children }: { children: ReactNode }) {
  return (
    <span
      style={{
        fontSize: 'var(--text-xs)',
        lineHeight: 1.4,
        padding: '0 4px',
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-sm)',
        color: 'var(--text-secondary)',
        textTransform: 'uppercase',
        letterSpacing: '0.02em',
        flexShrink: 0,
      }}
    >
      {children}
    </span>
  );
}

export function SessionSwitcherChip() {
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();

  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const { data: summaries } = useSessionList();
  const reap = useReapSessions();
  // Restores the pointer after a reload, drops it when the workspace changes
  // or the session disappears, and never adopts one the user did not pick
  // (finding 45). Mounted here because this chip is the session-switcher and
  // is mounted exactly once, in the always-on frame.
  const { scopedCount, foreignCount, catalogLoaded } = useActiveSessionReconciler();
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const controller = useSessionControllerBridge((s) => s.controller);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const [creating, setCreating] = useState(false);

  // Create a session for whatever the run target currently is (an element
  // if one is picked under Configure, the whole workspace otherwise) and
  // make it active. No ticks are advanced — this is "start a session", not
  // "start running".
  const createSession = async () => {
    if (!controller || creating) return;
    setCreating(true);
    try {
      const id = await controller.startSession();
      // startSession already selects on success; on failure it has set
      // phase 'error' and logged, and the popover stays open so the user
      // can see nothing was added to the list.
      if (id) setOpen(false);
    } finally {
      setCreating(false);
    }
  };

  // Phase 6 — the frame-level session-compare action (workbench-suite
  // ruling: Compare is a Simulate mode, reached from here rather than
  // a nav tab). Seeds the compare picker with the active session when
  // nothing is picked yet, so the hop lands one click from a diff.
  const openCompare = () => {
    const store = useCompareStore.getState();
    if (activeSessionId && store.pickedSessionIds.length === 0) {
      store.setPickedSessionIds([activeSessionId]);
    }
    setOpen(false);
    navigate('/run/compare');
  };

  // The catalog is scoped to the loaded workspace, for the same reason the
  // pointer is: a session built from another model cannot be stepped against
  // this one, so offering it for selection would just hand the user a
  // different form of the incoherence (finding 45). The ones left out are
  // counted, not hidden — see the footer note.
  // No workspace loaded means no scope to apply — filter nothing rather than
  // hiding every session behind a scope we cannot evaluate.
  const records: SessionRecord[] = (summaries ?? [])
    .filter((s) => workspaceRoot === null || sessionBelongsToWorkspace(s, workspaceRoot))
    .map(toSessionRecord);
  const active = records.find((r) => r.id === activeSessionId) ?? null;
  // With nothing active the chip must still account for what the counter
  // beside it is reporting, or the two contradict each other — which is the
  // shape finding 45 was reported in. "N available" says the sessions exist
  // and none is selected; it does not imply the app lost one.
  // With nothing active the chip must still account for what the counter
  // beside it is reporting, or the two contradict each other — which is the
  // shape finding 45 was reported in. "N available" says the sessions exist
  // and none is selected; "N elsewhere" explains a global counter that is
  // non-zero while this workspace has nothing to offer.
  const label = active
    ? `${shortId(active.id)} · ${active.kind}`
    : !catalogLoaded
      ? 'no session'
      : scopedCount > 0
        ? `no session · ${scopedCount} available`
        : foreignCount > 0
          ? `no session · ${foreignCount} elsewhere`
          : 'no session';
  // Shown beside the id, because "which run am I looking at" is not answered
  // by a uuid prefix when two runs differ only by scenario.
  const activeScenario = active ? scenarioLabel(active.createOverrides) : null;

  return (
    <div style={{ position: 'relative', display: 'inline-flex' }}>
      <button
        ref={buttonRef}
        type="button"
        data-testid="session-switcher-chip"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        style={{
          height: 'var(--row-compact)',
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          padding: '0 8px',
          fontSize: 'var(--text-sm)',
          fontFamily: 'var(--font-mono)',
          color: active ? 'var(--text-primary)' : 'var(--text-muted)',
          background: 'transparent',
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
          cursor: 'pointer',
        }}
      >
        {label}
        {activeScenario && (
          <span
            data-testid="session-switcher-chip-scenario"
            title={
              active && active.createOverrides.length > 0
                ? `Built with ${activeScenario} — in force from this run's first tick`
                : "No create-time overrides — the model's declared defaults"
            }
            style={{
              fontSize: 'var(--text-xs)',
              color: 'var(--text-muted)',
              borderLeft: '1px solid var(--border-default)',
              paddingLeft: 6,
              maxWidth: 160,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {activeScenario}
          </span>
        )}
      </button>

      <Popover
        anchorEl={buttonRef.current}
        open={open}
        onClose={() => setOpen(false)}
        placement="bottom"
      >
        <div
          data-testid="session-switcher-list"
          style={{ minWidth: 260, maxWidth: 360, padding: 6 }}
        >
          <button
            type="button"
            data-testid="session-switcher-new"
            disabled={!controller || creating}
            onClick={() => { void createSession(); }}
            title={
              activeSessionTarget
                ? 'Create a session for the selected run target. No ticks are advanced'
                : 'Create a session over the whole workspace — every subsystem. Pick a single element under Configure to narrow it. No ticks are advanced'
            }
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              width: '100%',
              textAlign: 'left',
              padding: '4px 8px',
              marginBottom: 4,
              fontSize: 'var(--text-sm)',
              fontWeight: 500,
              color: controller && !creating ? 'var(--accent-fg)' : 'var(--text-disabled)',
              background: 'transparent',
              border: 'none',
              borderRadius: 'var(--radius-sm)',
              cursor: controller && !creating ? 'pointer' : 'default',
            }}
          >
            <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 14 }}>
              add
            </span>
            <span>{creating ? 'Creating…' : 'New session'}</span>
            <span
              style={{
                flex: 1,
                minWidth: 0,
                textAlign: 'right',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                fontWeight: 400,
                color: 'var(--text-muted)',
                fontSize: 'var(--text-xs)',
              }}
            >
              {activeSessionTarget ? 'selected target' : 'whole workspace'}
            </span>
          </button>

          {records.length === 0 && (
            <div
              style={{
                padding: '6px 8px',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-muted)',
              }}
            >
              {foreignCount > 0
                ? `no session in this workspace — ${foreignCount} ${foreignCount === 1 ? 'is' : 'are'} not from this one`
                : 'no session yet'}
            </div>
          )}

          {records.map((r) => {
            const isActive = r.id === activeSessionId;
            const isExpired = r.status === 'expired';
            return (
              <button
                key={r.id}
                type="button"
                data-testid={`session-switcher-row-${r.id}`}
                onClick={() => {
                  setActiveSession(r.id);
                  setOpen(false);
                }}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  width: '100%',
                  textAlign: 'left',
                  padding: '4px 8px',
                  fontSize: 'var(--text-sm)',
                  fontFamily: 'var(--font-mono)',
                  background: isActive ? 'var(--accent-tint)' : 'transparent',
                  color: isExpired ? 'var(--text-disabled)' : 'var(--text-primary)',
                  opacity: isExpired ? 0.6 : 1,
                  border: 'none',
                  borderRadius: 'var(--radius-sm)',
                  cursor: 'pointer',
                }}
              >
                <span
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {r.label ?? shortId(r.id)}
                </span>
                {r.createOverrides.length > 0 && (
                  <Badge>{scenarioLabel(r.createOverrides)}</Badge>
                )}
                <Badge>{r.kind}</Badge>
                <Badge>{r.status}</Badge>
              </button>
            );
          })}

          <div
            style={{
              borderTop: '1px solid var(--border-default)',
              marginTop: 4,
              paddingTop: 4,
            }}
          >
            <button
              type="button"
              data-testid="session-switcher-history"
              onClick={() => {
                setOpen(false);
                void Promise.all([
                  import('@/shared/overlays/modalStore'),
                  import('@/features/archive/HistoryBrowserModal'),
                ]).then(([{ useModalStore }, { HISTORY_BROWSER_MODAL_ID }]) =>
                  useModalStore.getState().openModal(HISTORY_BROWSER_MODAL_ID),
                );
              }}
              style={{
                width: '100%',
                textAlign: 'left',
                padding: '4px 8px',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-secondary)',
                background: 'transparent',
                border: 'none',
                cursor: 'pointer',
              }}
            >
              Archived runs…
            </button>
            <button
              type="button"
              data-testid="session-switcher-compare"
              onClick={openCompare}
              style={{
                width: '100%',
                textAlign: 'left',
                padding: '4px 8px',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-secondary)',
                background: 'transparent',
                border: 'none',
                cursor: 'pointer',
              }}
            >
              Compare sessions…
            </button>
            <button
              type="button"
              data-testid="session-switcher-clear-stale"
              disabled={reap.isPending}
              onClick={() => reap.mutate()}
              style={{
                width: '100%',
                textAlign: 'left',
                padding: '4px 8px',
                fontSize: 'var(--text-sm)',
                color: 'var(--text-secondary)',
                background: 'transparent',
                border: 'none',
                cursor: reap.isPending ? 'default' : 'pointer',
              }}
            >
              {reap.isPending ? 'Clearing…' : 'Clear stale'}
            </button>
          </div>
        </div>
      </Popover>
    </div>
  );
}
