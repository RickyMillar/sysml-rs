/**
 * Active-session persistence — which session this browser was last driving,
 * remembered per workspace.
 *
 * `useSessionStore.activeSessionId` starts `null` and nothing persisted it,
 * while the backend's sessions outlive the page. So a reload put the header
 * back to "no session" beside a counter reporting the sessions the user had
 * just made (punch-list finding 45). The visible contradiction was the smaller
 * half: with no active session the transport's ▶ creates a NEW one, so a
 * reload silently grew the session catalog instead of resuming the run.
 *
 * Keyed BY WORKSPACE ROOT, not globally. A session is only meaningful against
 * the model it was built from — restoring one workspace's session into another
 * would wire the transport to a run whose element names do not exist in the
 * loaded model.
 */

const KEY_PREFIX = 'sysml.activeSession.';

function keyFor(workspaceRoot: string): string {
  return `${KEY_PREFIX}${workspaceRoot}`;
}

/** The session id this browser last had active for `workspaceRoot`. */
export function readPersistedActiveSession(workspaceRoot: string | null): string | null {
  if (typeof window === 'undefined' || !workspaceRoot) return null;
  try {
    const raw = window.localStorage.getItem(keyFor(workspaceRoot));
    return raw && raw.length > 0 ? raw : null;
  } catch {
    // Blocked or full storage — the tab still works, it just will not
    // remember across reloads. Never a hard failure.
    return null;
  }
}

/** Record (or, with `null`, forget) the active session for `workspaceRoot`. */
export function writePersistedActiveSession(
  workspaceRoot: string | null,
  sessionId: string | null,
): void {
  if (typeof window === 'undefined' || !workspaceRoot) return;
  try {
    if (sessionId == null || sessionId.length === 0) {
      window.localStorage.removeItem(keyFor(workspaceRoot));
    } else {
      window.localStorage.setItem(keyFor(workspaceRoot), sessionId);
    }
  } catch {
    // See above.
  }
}

/**
 * Whether a session belongs to `workspaceRoot`.
 *
 * Orchestrator sessions all carry `uri === "__workspace__"`, so the URI cannot
 * discriminate; B6 provenance `workspace_root` is the scope key — the same one
 * `executions.rs` filters executions by.
 *
 * A session with NO provenance (minted outside the service layer, or predating
 * B6) has an unknown scope. Restoration treats unknown as "not ours" and
 * declines: silently adopting a session that might belong to another model is
 * worse than starting with none, and the user can still pick it by hand from
 * the switcher, where its identity is on screen.
 */
export function sessionBelongsToWorkspace(
  session: { provenance?: { workspace_root?: string | null } | null } | null | undefined,
  workspaceRoot: string | null,
): boolean {
  if (!session || !workspaceRoot) return false;
  const root = session.provenance?.workspace_root;
  if (!root) return false;
  return normalizeRoot(root) === normalizeRoot(workspaceRoot);
}

/**
 * Compare filesystem roots without tripping over a trailing slash or a
 * `..`-relative spelling. The service resolves its own root through the crate
 * manifest, so a session's recorded root can read
 * `…/sysml-service/../../../examples/x` for the very workspace the UI knows as
 * `…/examples/x`.
 */
function normalizeRoot(root: string): string {
  const withoutScheme = root.replace(/^file:\/\//, '');
  const segments: string[] = [];
  for (const part of withoutScheme.split('/')) {
    if (part === '' || part === '.') continue;
    if (part === '..') {
      segments.pop();
      continue;
    }
    segments.push(part);
  }
  return `/${segments.join('/')}`;
}
