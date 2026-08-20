/**
 * WorkspaceLoadErrorBanner — surface parse errors from the most recent
 * `sysml.load_workspace` mutation.
 *
 * Bucket 5-followup (2026-05-05): when a user opens a workspace whose
 * files don't all parse — e.g. coffee-machine `views.sysml` line 81's
 * unsupported metadata-attribute predicate — the Views drawer ends up
 * sparser than expected with no in-band hint why. The banner names
 * the count and points the user at the Diagnostics drawer, where the
 * per-file entries already render via `useDiagnostics`.
 *
 * Drives off `useWorkspaceUIStore.loadStatus`, which is populated by
 * `useLoadWorkspace.onSuccess` from the backend's
 * `WorkspaceLoadResult.errors[]`. Each entry is pre-formatted as
 * "<file>:<line>: <message>" by the service.
 */
import { useNavigate } from 'react-router-dom';
import { useWorkspaceUIStore } from './store';

const STYLES = {
  root: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: 8,
    padding: '6px 10px',
    background: 'color-mix(in srgb, var(--severity-error) 15%, transparent)',
    borderBottom: '1px solid color-mix(in srgb, var(--severity-error) 35%, transparent)',
    color: 'var(--text-primary)',
    fontSize: 11,
  } as const,
  message: {
    flex: 1,
    minWidth: 0,
    lineHeight: 1.4,
  } as const,
  link: {
    background: 'transparent',
    border: 'none',
    color: 'var(--accent-fg)',
    cursor: 'pointer',
    fontSize: 11,
    fontWeight: 600,
    padding: 0,
    marginLeft: 6,
    textDecoration: 'underline',
  } as const,
  dismiss: {
    background: 'transparent',
    border: 'none',
    color: 'var(--text-muted)',
    cursor: 'pointer',
    fontSize: 16,
    lineHeight: 1,
    padding: '0 4px',
  } as const,
  preview: {
    marginTop: 4,
    color: 'var(--text-muted)',
    fontFamily: 'var(--font-mono, ui-monospace, monospace)',
    fontSize: 10,
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap' as const,
  } as const,
};

export function WorkspaceLoadErrorBanner() {
  const status = useWorkspaceUIStore((s) => s.loadStatus);
  const dismiss = useWorkspaceUIStore((s) => s.dismissLoadStatus);
  const navigate = useNavigate();

  if (!status || status.dismissed || status.errorCount === 0) return null;

  // Show the first error as a one-line preview so the user has a hint
  // even without opening the drawer.
  const firstError = status.errors[0];

  const openDiagnostics = () => {
    // The Diagnostics drawer is a utility-position panel registered in
    // shared/panels/registry.ts. It's accessible from any workflow,
    // and `?diagnostics=open` is the convention the UtilityDrawer reads
    // to auto-open. Until that wiring is in place the navigate is a
    // best-effort hop to /run where the panel is mounted.
    navigate('/run?diagnostics=open');
  };

  return (
    <div data-testid="workspace-load-error-banner" style={STYLES.root}>
      <span
        className="material-symbols-outlined"
        aria-hidden="true"
        style={{ fontSize: 14, color: 'var(--severity-error)' }}
      >
        error
      </span>
      <div style={STYLES.message}>
        <span data-testid="workspace-load-error-count">
          {status.errorCount} file{status.errorCount === 1 ? '' : 's'} failed
          to parse
        </span>
        <button
          type="button"
          onClick={openDiagnostics}
          data-testid="workspace-load-error-link"
          style={STYLES.link}
        >
          see Diagnostics
        </button>
        {firstError ? (
          <div style={STYLES.preview} title={firstError}>
            {firstError}
          </div>
        ) : null}
      </div>
      <button
        type="button"
        onClick={dismiss}
        data-testid="workspace-load-error-dismiss"
        aria-label="Dismiss"
        style={STYLES.dismiss}
      >
        ×
      </button>
    </div>
  );
}
