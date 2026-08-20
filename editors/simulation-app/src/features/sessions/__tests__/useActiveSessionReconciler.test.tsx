/**
 * Active-session reconciliation (finding 45).
 *
 * Reproduced in Chrome before the fix: create a session, reload, and the
 * header read "no session" while the counter beside it read "1/80 sessions"
 * and the backend still held a live session at tick 1. Pressing ▶ in that
 * state creates a SECOND session rather than resuming the first, so the
 * contradiction was not merely cosmetic — a reload silently grew the catalog.
 *
 * The three cases these gates cover are the three the policy has to get right:
 * create, reload/reopen, and switching.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const listData = vi.hoisted(() => ({
  current: [] as unknown[],
  loaded: true,
  /** react-query's fetch timestamp. Staleness is the whole point of one of
   *  these gates, so the mock has to model it. */
  updatedAt: 0,
}));
vi.mock('../queries', () => ({
  useSessionList: () => ({
    data: listData.current,
    isSuccess: listData.loaded,
    dataUpdatedAt: listData.updatedAt,
  }),
}));

const workspace = vi.hoisted(() => ({ root: null as string | null }));
vi.mock('@/features/workspace/store', () => ({
  useWorkspaceUIStore: (sel: (s: { workspaceRoot: string | null }) => unknown) =>
    sel({ workspaceRoot: workspace.root }),
}));

import { useActiveSessionReconciler } from '../useActiveSessionReconciler';
import { useSessionStore } from '../store';
import {
  readPersistedActiveSession,
  sessionBelongsToWorkspace,
  writePersistedActiveSession,
} from '../activeSessionPersistence';

const ROOT_A = '/repo/examples/espresso-production-cell';
const ROOT_B = '/repo/examples/espresso-pump-hybrid';

function session(id: string, root: string | null) {
  return {
    id,
    kind: 'orchestrator',
    uri: '__workspace__',
    tick: 1,
    provenance: root ? { workspace_root: root } : null,
  };
}

function mount() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return renderHook(() => useActiveSessionReconciler(), {
    wrapper: ({ children }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    ),
  });
}

beforeEach(() => {
  window.localStorage.clear();
  useSessionStore.setState({ activeSessionId: null, phase: 'idle' });
  listData.current = [];
  listData.loaded = true;
  // Far in the future by default, so gates that are not about staleness see a
  // catalog that is unambiguously fresher than any selection.
  listData.updatedAt = Date.now() + 60_000;
  workspace.root = ROOT_A;
});

afterEach(() => {
  window.localStorage.clear();
});

// ── 1. create → the pointer is identified and remembered ──────────────────

describe('create', () => {
  it('persists the session the app selects, scoped to its workspace', async () => {
    listData.current = [session('sess-1', ROOT_A)];
    mount();

    useSessionStore.getState().setActiveSession('sess-1');

    await waitFor(() =>
      expect(readPersistedActiveSession(ROOT_A)).toBe('sess-1'),
    );
    // ...and only for that workspace.
    expect(readPersistedActiveSession(ROOT_B)).toBeNull();
  });
});

// ── 2. reload / reopen → deterministic active-or-not ──────────────────────

describe('reload', () => {
  it('restores the session this browser was driving', async () => {
    writePersistedActiveSession(ROOT_A, 'sess-1');
    listData.current = [session('sess-1', ROOT_A)];

    mount();

    await waitFor(() =>
      expect(useSessionStore.getState().activeSessionId).toBe('sess-1'),
    );
  });

  it('forgets a persisted session the backend no longer has', async () => {
    // Reaped, stopped elsewhere, or expired while the tab was closed.
    writePersistedActiveSession(ROOT_A, 'sess-gone');
    listData.current = [session('sess-other', ROOT_A)];

    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(true));
    expect(useSessionStore.getState().activeSessionId).toBeNull();
    // Cleared, so it is not retried on every poll for the rest of the session.
    expect(readPersistedActiveSession(ROOT_A)).toBeNull();
  });

  // Rule 2 of the policy, and the one most worth pinning: sessions can be
  // created by another tab, a CLI, or an agent. Adopting one would wire the
  // transport to a run the user never chose.
  it('never adopts a session the user did not select', async () => {
    listData.current = [session('sess-someone-elses', ROOT_A)];

    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(true));
    expect(useSessionStore.getState().activeSessionId).toBeNull();
    // But it IS counted, so the switcher can say "N available" instead of
    // contradicting the session counter — the shape finding 45 was reported in.
    expect(result.current.scopedCount).toBe(1);
  });

  // A workspace switch leaves the GLOBAL session counter non-zero while this
  // workspace has nothing — visually the same shape as the reported bug. The
  // switcher has to be able to say which it is.
  it('separates sessions of this workspace from sessions elsewhere', async () => {
    listData.current = [
      session('sess-a', ROOT_A),
      session('sess-b1', ROOT_B),
      session('sess-b2', ROOT_B),
    ];
    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(true));
    expect(result.current.scopedCount).toBe(1);
    expect(result.current.foreignCount).toBe(2);
  });

  it('applies no scope at all when no workspace is loaded', async () => {
    // There is no "here" yet, so reporting every live session as "elsewhere"
    // would be the wrong kind of confident.
    workspace.root = null;
    listData.current = [session('sess-a', ROOT_A), session('sess-b', ROOT_B)];
    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(true));
    expect(result.current.scopedCount).toBe(2);
    expect(result.current.foreignCount).toBe(0);
  });

  it('counts an unplaceable session as foreign, not as ours', async () => {
    listData.current = [session('sess-no-prov', null)];
    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(true));
    expect(result.current.scopedCount).toBe(0);
    expect(result.current.foreignCount).toBe(1);
  });

  it('declines to restore a session belonging to another workspace', async () => {
    writePersistedActiveSession(ROOT_A, 'sess-1');
    // Same id, but the live session was built from the OTHER model.
    listData.current = [session('sess-1', ROOT_B)];

    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(true));
    expect(useSessionStore.getState().activeSessionId).toBeNull();
    expect(result.current.scopedCount).toBe(0);
  });

  it('waits for the catalog rather than clearing on an unloaded list', async () => {
    writePersistedActiveSession(ROOT_A, 'sess-1');
    listData.current = [];
    listData.loaded = false;

    const { result } = mount();

    await waitFor(() => expect(result.current.catalogLoaded).toBe(false));
    // Nothing decided yet — and, critically, the persisted id is NOT dropped
    // on the strength of a list that has not arrived.
    expect(readPersistedActiveSession(ROOT_A)).toBe('sess-1');
  });
});

// ── 3. switching / workspace change → the pointer follows ─────────────────

describe('switching', () => {
  it('drops a session that disappears while it is active', async () => {
    listData.current = [session('sess-1', ROOT_A)];
    const { rerender } = mount();
    useSessionStore.getState().setActiveSession('sess-1');
    await waitFor(() => expect(useSessionStore.getState().activeSessionId).toBe('sess-1'));

    // Reaped out from under us (another tab's suite run, or expiry).
    listData.current = [];
    rerender();

    await waitFor(() =>
      expect(useSessionStore.getState().activeSessionId).toBeNull(),
    );
  });

  // The race this hook introduced and had to be taught about: a session is
  // selected the instant `sessions.create` returns, while the cached list
  // still predates it. Treating that absence as "reaped" cleared the pointer a
  // frame after it was set — three sessions created, header stuck on "no
  // session".
  it('does not evict a just-created session the cached list predates', async () => {
    listData.current = [];
    listData.updatedAt = Date.now() - 5_000; // fetched BEFORE the selection
    const { rerender } = mount();

    useSessionStore.getState().setActiveSession('sess-brand-new');
    rerender();
    await new Promise((r) => setTimeout(r, 30));

    expect(useSessionStore.getState().activeSessionId).toBe('sess-brand-new');

    // Once the catalog is re-read and genuinely lacks it, it goes.
    listData.updatedAt = Date.now() + 1_000;
    rerender();
    await waitFor(() =>
      expect(useSessionStore.getState().activeSessionId).toBeNull(),
    );
  });

  it('clears the active session when the workspace changes', async () => {
    listData.current = [session('sess-1', ROOT_A)];
    const { rerender } = mount();
    useSessionStore.getState().setActiveSession('sess-1');
    await waitFor(() => expect(useSessionStore.getState().activeSessionId).toBe('sess-1'));

    workspace.root = ROOT_B;
    listData.current = [session('sess-2', ROOT_B)];
    rerender();

    await waitFor(() =>
      expect(useSessionStore.getState().activeSessionId).toBeNull(),
    );
  });

  it('restores the other workspace’s own session after a switch back', async () => {
    writePersistedActiveSession(ROOT_A, 'sess-a');
    writePersistedActiveSession(ROOT_B, 'sess-b');

    listData.current = [session('sess-a', ROOT_A), session('sess-b', ROOT_B)];
    const { rerender } = mount();
    await waitFor(() => expect(useSessionStore.getState().activeSessionId).toBe('sess-a'));

    workspace.root = ROOT_B;
    rerender();
    await waitFor(() => expect(useSessionStore.getState().activeSessionId).toBe('sess-b'));

    workspace.root = ROOT_A;
    rerender();
    await waitFor(() => expect(useSessionStore.getState().activeSessionId).toBe('sess-a'));
  });

  // Requirement 3: the transport controls follow the SELECTED session.
  // `phase` is set only by the controller's own actions, so it described the
  // last session driven: step one to `paused`, switch to a fresh session, and
  // the new one offered Resume with Play absent (observed live).
  it('re-derives the transport phase for the newly selected session', async () => {
    listData.current = [
      session('sess-stepped', ROOT_A),
      { ...session('sess-done', ROOT_A), completed: true },
    ];
    const { rerender } = mount();

    useSessionStore.getState().setActiveSession('sess-stepped');
    rerender();
    // Pretend the controller drove it.
    useSessionStore.getState().setPhase('paused');

    useSessionStore.getState().setActiveSession('sess-done');
    rerender();
    await waitFor(() => expect(useSessionStore.getState().phase).toBe('completed'));

    // ...and back to a session the backend does not call completed.
    useSessionStore.getState().setActiveSession('sess-stepped');
    rerender();
    await waitFor(() => expect(useSessionStore.getState().phase).toBe('idle'));
  });

  it('resets the phase when the session is deselected entirely', async () => {
    listData.current = [session('sess-1', ROOT_A)];
    const { rerender } = mount();
    useSessionStore.getState().setActiveSession('sess-1');
    rerender();
    useSessionStore.getState().setPhase('paused');

    useSessionStore.getState().setActiveSession(null);
    rerender();

    // Otherwise the controls keep offering Resume for a session that is gone.
    await waitFor(() => expect(useSessionStore.getState().phase).toBe('idle'));
  });

  it('leaves an explicit user switch alone', async () => {
    listData.current = [session('sess-1', ROOT_A), session('sess-2', ROOT_A)];
    writePersistedActiveSession(ROOT_A, 'sess-1');
    const { rerender } = mount();
    await waitFor(() => expect(useSessionStore.getState().activeSessionId).toBe('sess-1'));

    // The user picks the other one from the switcher.
    useSessionStore.getState().setActiveSession('sess-2');
    rerender();

    await waitFor(() => expect(readPersistedActiveSession(ROOT_A)).toBe('sess-2'));
    // The restore pass must not fight the user back to the persisted value.
    expect(useSessionStore.getState().activeSessionId).toBe('sess-2');
  });
});

// ── scope matching ────────────────────────────────────────────────────────

describe('sessionBelongsToWorkspace', () => {
  it('matches through the ..-relative root the service records', () => {
    // The service resolves its root through the crate manifest, so a real
    // session records e.g. `…/sysml-service/../../../examples/x` for the
    // workspace the UI knows as `…/examples/x`.
    const recorded = '/repo/crates/tooling/sysml-service/../../../examples/x';
    expect(
      sessionBelongsToWorkspace({ provenance: { workspace_root: recorded } }, '/repo/examples/x'),
    ).toBe(true);
  });

  it('ignores a trailing slash and a file:// prefix', () => {
    expect(
      sessionBelongsToWorkspace(
        { provenance: { workspace_root: 'file:///repo/examples/x/' } },
        '/repo/examples/x',
      ),
    ).toBe(true);
  });

  it('treats an unknown scope as not ours', () => {
    // Fail closed: adopting a session that might belong to another model is
    // worse than starting with none, and it is still pickable by hand.
    expect(sessionBelongsToWorkspace({ provenance: null }, '/repo/examples/x')).toBe(false);
    expect(sessionBelongsToWorkspace(null, '/repo/examples/x')).toBe(false);
  });

  it('does not match a different workspace', () => {
    expect(
      sessionBelongsToWorkspace({ provenance: { workspace_root: ROOT_B } }, ROOT_A),
    ).toBe(false);
  });
});
