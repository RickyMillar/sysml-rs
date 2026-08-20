/**
 * useActiveSessionReconciler — keeps the active-session pointer, the session
 * catalog, and the frame's controls telling the same story.
 *
 * Finding 45: after creating a session, a reload put the header back to
 * "no session" while the counter still reported it and the backend still had
 * it live. Two things were wrong, and the visible contradiction was the
 * smaller one:
 *
 *   · nothing persisted `activeSessionId`, so a reload always dropped it;
 *   · with no active session the transport's ▶ CREATES one, so reloading and
 *     pressing play silently grew the catalog instead of resuming the run.
 *
 * A third case existed in the other direction: when the active session was
 * reaped, stopped from another tab, or expired, the pointer stayed set. The
 * chip resolved it against the list, found nothing, and rendered "no session"
 * — while the transport still believed a session existed and would step a
 * dead id.
 *
 * ── The policy ────────────────────────────────────────────────────────────
 *
 * 1. RESTORE, don't guess. The active session is persisted per workspace root
 *    and restored on load only when that exact session is still live AND its
 *    provenance workspace_root matches the loaded workspace. Anything else
 *    clears the pointer.
 *
 * 2. Never auto-adopt. A session this browser did not select is never made
 *    active, however few there are. Sessions can be created by another tab, a
 *    CLI, or an agent; adopting one would wire the transport to a run the user
 *    never chose. Where that leaves sessions listed but none active, the
 *    SWITCHER says so ("no session · N available") rather than the counter and
 *    the chip disagreeing.
 *
 * 3. A workspace change clears the pointer immediately, before any restore
 *    runs for the new root. A session built from another model cannot be
 *    meaningfully stepped against this one, and leaving it wired to the
 *    transport is the same class of bug as never restoring it.
 *
 * 4. A pointer that no longer resolves is dropped, not retained. Once the list
 *    has actually loaded and does not contain the active id, it is gone —
 *    reaped, stopped, or expired — and the controls must stop claiming it.
 *
 * 5. The transport phase follows the SELECTED session. `phase` is set only by
 *    the controller's own actions, so it described the last session driven
 *    rather than the one now selected: step a session to `paused`, create a
 *    fresh one, and the new session offered Resume with Play absent. On every
 *    change of active session the phase is re-derived from that session's own
 *    summary — `completed` if the backend says so, otherwise `idle`.
 *
 * Deliberately NOT here: creating sessions. This hook only ever selects or
 * deselects. `useSessionController.startSession` remains the one creation
 * path.
 */

import { useEffect, useRef } from 'react';
import { useSessionStore } from './store';
import { useSessionList } from './queries';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import {
  readPersistedActiveSession,
  sessionBelongsToWorkspace,
  writePersistedActiveSession,
} from './activeSessionPersistence';

export interface ActiveSessionReconciliation {
  /** Live sessions belonging to the loaded workspace. */
  scopedCount: number;
  /**
   * Live sessions that exist but belong to a DIFFERENT workspace (or carry no
   * provenance to place them). They are why the global session counter can
   * read 2 while this workspace has none, and the switcher names them rather
   * than leaving that looking like the bug this hook fixes.
   */
  foreignCount: number;
  /** True once the catalog has loaded at least once. */
  catalogLoaded: boolean;
}

export function useActiveSessionReconciler(): ActiveSessionReconciliation {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const { data: sessions, isSuccess, dataUpdatedAt } = useSessionList();

  // With no workspace loaded there is no scope to evaluate, so everything
  // counts as ours — the alternative is reporting every live session as
  // "elsewhere" when there is no "here" yet.
  const scoped = (sessions ?? []).filter(
    (s) => workspaceRoot === null || sessionBelongsToWorkspace(s, workspaceRoot),
  );

  // ── 3. A workspace change drops the pointer before anything else runs ────
  //
  // Tracked with a ref rather than by clearing in `setWorkspaceRoot`: the root
  // also arrives from a deep link on first paint, and treating that initial
  // assignment as a "change" would wipe the very pointer we are about to
  // restore.
  const lastRoot = useRef<string | null>(workspaceRoot);
  const restoredFor = useRef<string | null>(null);
  useEffect(() => {
    if (lastRoot.current === workspaceRoot) return;
    lastRoot.current = workspaceRoot;
    // Allow a restore attempt against the new root.
    restoredFor.current = null;
    if (useSessionStore.getState().activeSessionId !== null) {
      setActiveSession(null);
    }
  }, [workspaceRoot, setActiveSession]);

  // ── 1+2. Restore once per workspace, and only an exact, scoped match ─────
  useEffect(() => {
    if (!isSuccess || !workspaceRoot) return;
    if (restoredFor.current === workspaceRoot) return;
    // The catalog has loaded for this root: this is our one attempt.
    restoredFor.current = workspaceRoot;

    if (useSessionStore.getState().activeSessionId !== null) return;

    const persisted = readPersistedActiveSession(workspaceRoot);
    if (!persisted) return;

    const match = (sessions ?? []).find((s) => s.id === persisted);
    if (match && sessionBelongsToWorkspace(match, workspaceRoot)) {
      setActiveSession(persisted);
    } else {
      // It is gone, or it belongs to another model. Forget it rather than
      // retrying on every poll.
      writePersistedActiveSession(workspaceRoot, null);
    }
  }, [isSuccess, sessions, workspaceRoot, setActiveSession]);

  // ── 1. Persist every selection, so the next load can restore it ──────────
  //
  // Declared AFTER the restore effect and gated on it, which is load-bearing:
  // on the first commit `activeSessionId` is still null, so an ungated persist
  // writes null and erases the very id the restore was about to read. (It did,
  // until the tests below caught it.) Nothing is written for a workspace until
  // its restore attempt has settled.
  useEffect(() => {
    if (!workspaceRoot) return;
    if (restoredFor.current !== workspaceRoot) return;
    writePersistedActiveSession(workspaceRoot, activeSessionId);
  }, [workspaceRoot, activeSessionId, isSuccess]);

  // ── 4. Drop a pointer the catalog no longer contains ─────────────────────
  //
  // Guarded on the catalog being FRESHER than the selection. Without that
  // guard this rule eats every newly created session: `setActiveSession(id)`
  // runs the instant `sessions.create` returns, while the cached list still
  // predates it, so "not in the list" reads as "reaped" and the pointer is
  // cleared a frame after it was set. Observed live — three sessions created,
  // header stuck on "no session" — before the guard existed.
  //
  // `dataUpdatedAt` is the react-query fetch timestamp, so requiring it to be
  // strictly after the moment of selection means the list has actually been
  // re-read since, and an absence is real.
  const selectedAt = useRef(0);
  const lastSelection = useRef<string | null>(activeSessionId);
  if (lastSelection.current !== activeSessionId) {
    lastSelection.current = activeSessionId;
    selectedAt.current = Date.now();
  }
  useEffect(() => {
    if (!isSuccess || !activeSessionId) return;
    if (dataUpdatedAt <= selectedAt.current) return;
    const stillLive = (sessions ?? []).some((s) => s.id === activeSessionId);
    if (!stillLive) setActiveSession(null);
  }, [isSuccess, sessions, dataUpdatedAt, activeSessionId, setActiveSession]);

  // ── 5. Re-derive the transport phase for the newly selected session ──────
  //
  // Keyed on the session CHANGING, so this never fights the controller's own
  // transitions mid-run — by the time it fires, the loop is pointed somewhere
  // else anyway. `null` (deselected) also resets, so the controls do not keep
  // offering Resume for a session that is gone.
  const phasedFor = useRef<string | null>(activeSessionId);
  useEffect(() => {
    if (phasedFor.current === activeSessionId) return;
    phasedFor.current = activeSessionId;
    const summary = (sessions ?? []).find((s) => s.id === activeSessionId);
    useSessionStore.getState().setPhase(summary?.completed ? 'completed' : 'idle');
    // `sessions` is deliberately NOT a dependency: this must run once per
    // selection, not again whenever the 1 Hz poll returns a new array.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId]);

  return {
    scopedCount: scoped.length,
    foreignCount: (sessions ?? []).length - scoped.length,
    catalogLoaded: isSuccess,
  };
}
