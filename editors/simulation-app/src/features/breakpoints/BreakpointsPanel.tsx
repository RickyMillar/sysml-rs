/**
 * BreakpointsPanel — slim (~280px) sidebar panel for breakpoint management.
 *
 * Composition:
 *   Header:  "Breakpoints (N)" · [+ Add] · [Clear all]
 *   Body:    <BreakpointRow> per entry (or an empty-state hint)
 *   Footer:  Toast (optional) when a breakpoint is hit
 *   Modal:   <AddBreakpointDialog> — opened via +Add or ⌘⇧B
 *
 * ── Wiring ────────────────────────────────────────────────────────────
 * The panel consumes the Round-1 primitives:
 *   - `useSessionControl()` → setBreakpoint / clearBreakpoint / listBreakpoints
 *   - `useSessionEvents(id, 'breakpoint-hit', cb)` → flash + toast on hit
 *   - `useBreakpointStore` → shared local state with the overlay provider
 *   - `useSelectionStore` → jump-to-source selects the target element
 *
 * The UI behaviour is intentionally minimal (list / add / clear / jump)
 * but extension points are reserved:
 *   - BreakpointLocal carries condition/hitCount/logMessage/enabled
 *   - AddBreakpointDialog exposes them in a disabled Advanced expander
 *   - BreakpointRow "..." menu lists Edit/Disable as disabled entries
 * Round-4 agents wire those through without a UI redesign.
 */

import { useCallback, useEffect, useState } from 'react';
import { useSessionControl } from '@/engine/SessionControl';
import { useSessionEvents } from '@/engine/SessionEvents';
import { useSessionStore } from '@/features/sessions/store';
import { useSelectionStore } from '@/features/selection/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useModelBreakpointCandidates } from './useModelBreakpointCandidates';
import { AddBreakpointDialog, type AdvancedFields } from './AddBreakpointDialog';
import { BreakpointRow } from './BreakpointRow';
import {
  useBreakpointStore,
  breakpointLabel,
  type BreakpointHit,
  type BreakpointLocal,
} from './useBreakpointStore';
import { useBreakpointShortcut } from './useBreakpointShortcut';
import type { Breakpoint, BreakpointId } from '@/engine/types';

const FLASH_DURATION_MS = 1600;
const TOAST_DURATION_MS = 4000;

export interface BreakpointsPanelProps {
  /**
   * Disable the ⌘⇧B shortcut — useful when the panel is mounted but
   * another element owns the global listener. Defaults to enabled.
   */
  shortcutEnabled?: boolean;
  /**
   * Width in pixels for the panel container. Default 280 matches the
   * layout guidance in the plan. Consumers can override (e.g., bottom
   * strip layouts can pass 100% and add their own height gate).
   */
  width?: number | string;
}

export function BreakpointsPanel({
  shortcutEnabled = true,
  width = 280,
}: BreakpointsPanelProps = {}) {
  const entries = useBreakpointStore((s) => s.breakpoints);
  const lastHit = useBreakpointStore((s) => s.lastHit);
  const isAdding = useBreakpointStore((s) => s.isAdding);
  const addLocal = useBreakpointStore((s) => s.addLocal);
  const reconcileId = useBreakpointStore((s) => s.reconcileId);
  const remove = useBreakpointStore((s) => s.remove);
  const clearAllLocal = useBreakpointStore((s) => s.clearAll);
  const recordHit = useBreakpointStore((s) => s.recordHit);
  const setAdding = useBreakpointStore((s) => s.setAdding);
  const patch = useBreakpointStore((s) => s.patch);

  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const control = useSessionControl();

  const [dialogOpen, setDialogOpen] = useState(false);
  const [flashingId, setFlashingId] = useState<BreakpointId | null>(null);
  const [toastVisible, setToastVisible] = useState(false);

  // ── Keyboard shortcut ───────────────────────────────────────────────
  useBreakpointShortcut({
    enabled: shortcutEnabled,
    isOpen: dialogOpen,
    onOpen: () => setDialogOpen(true),
    onClose: () => setDialogOpen(false),
  });

  // ── Breakpoint-hit handler → flash + toast ───────────────────────
  useSessionEvents(activeSessionId, 'breakpoint-hit', (event) => {
    const id = typeof event.context?.breakpointId === 'string'
      ? (event.context.breakpointId as BreakpointId)
      : typeof event.context?.id === 'string'
        ? (event.context.id as BreakpointId)
        : null;
    recordHit({
      id: id ?? 'unknown',
      hitAtMs: Date.now(),
      context: event.context,
    });
    if (id) setFlashingId(id);
    setToastVisible(true);
  });

  // Flash fade-out timer — runs whenever a new hit lands.
  useEffect(() => {
    if (!flashingId) return;
    const t = window.setTimeout(() => setFlashingId(null), FLASH_DURATION_MS);
    return () => window.clearTimeout(t);
  }, [flashingId]);

  // Toast auto-dismiss timer.
  useEffect(() => {
    if (!toastVisible) return;
    const t = window.setTimeout(() => setToastVisible(false), TOAST_DURATION_MS);
    return () => window.clearTimeout(t);
  }, [toastVisible]);

  // ── Autocomplete candidates (BP-UX1) ─────────────────────────────
  // Model-sourced: attributes/calcs → variables (dotted paths), sm/
  // action/constraint/ode → elements, straight off the compiled model
  // tree — present BEFORE any session exists, which is what makes
  // arm-before-run (BP-UX2) usable. metricRegistry names still
  // supplement the variable list once a run streams data. The old
  // registry-only wiring was the silent-no-op bug: empty candidates on
  // an idle session pushed users to free-type names the backend then
  // no-op'd on mismatch.
  const { variableCandidates, elementCandidates } = useModelBreakpointCandidates();

  // ── Handlers ──────────────────────────────────────────────────────
  const handleAdd = useCallback(
    async (breakpoint: Breakpoint, advanced: AdvancedFields) => {
      const localId = addLocal({
        breakpoint,
        condition: advanced.condition,
        hitCount: advanced.hitCount,
        logMessage: advanced.logMessage,
        enabled: true,
      });
      // BP-UX2 arm-before-run: with NO session yet, the breakpoint
      // stays local (`local-` id) — `useSessionController.
      // ensureSessionStarted` flushes every unpushed breakpoint to the
      // backend the moment the session is created, so pick target →
      // set breakpoints → run needs no "start a session first" step.
      // (With an existing session — even a never-stepped IDLE one — the
      // backend accepts `breakpoint.set` immediately and matches by
      // name once ticks flow.)
      if (!useSessionStore.getState().activeSessionId) {
        setDialogOpen(false);
        return;
      }
      setAdding(true);
      try {
        const backendId = await control.setBreakpoint(breakpoint);
        reconcileId(localId, backendId);
        setDialogOpen(false);
      } catch (err) {
        // Rollback the optimistic insert if the backend rejects it.
        remove(localId);
        // eslint-disable-next-line no-console
        console.error('[BreakpointsPanel] setBreakpoint failed:', err);
      } finally {
        setAdding(false);
      }
    },
    [addLocal, control, reconcileId, remove, setAdding],
  );

  const handleClear = useCallback(
    async (entry: BreakpointLocal) => {
      // Optimistic local remove; rollback only on a hard failure.
      remove(entry.id);
      // Backend ids are non-local. Skip the REST call for locally-only
      // rows (shouldn't normally happen, but keeps clearAll cheap when
      // the user pops the dialog without the backend up).
      if (entry.id.startsWith('local-')) return;
      try {
        await control.clearBreakpoint(entry.id);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.warn('[BreakpointsPanel] clearBreakpoint failed:', err);
        // Re-insert so the user can retry — preserve original entry.
        useBreakpointStore.getState().addLocal({
          id: entry.id,
          breakpoint: entry.breakpoint,
          condition: entry.condition,
          hitCount: entry.hitCount,
          logMessage: entry.logMessage,
          enabled: entry.enabled,
        });
      }
    },
    [control, remove],
  );

  const handleClearAll = useCallback(async () => {
    // Snapshot the list so we can fire-and-forget the backend clears in
    // parallel. Local state drops immediately for snappy UX.
    const snapshot = [...entries];
    clearAllLocal();
    await Promise.allSettled(
      snapshot
        .filter((e) => !e.id.startsWith('local-'))
        .map((e) => control.clearBreakpoint(e.id)),
    );
  }, [clearAllLocal, control, entries]);

  const handleJump = useCallback(
    (entry: BreakpointLocal) => {
      const node =
        entry.breakpoint.kind === 'threshold-crossing'
          ? null
          : entry.breakpoint.target;
      if (!node) return;
      // Selection store auto-fetches the element detail; the uri is
      // the currently-active workspace file where available.
      const uri = useWorkspaceUIStore.getState().activeSessionTarget
        ? deriveSelectionUri()
        : null;
      useSelectionStore.getState().select(uri, node);
    },
    [],
  );

  // Reserved extension-point handlers — no-op today, wired when Round 4
  // adds the conditional / soft-disable dialogs. Left as prop hooks on
  // the row so the menu items can be enabled without further plumbing.
  const handleToggleEnabled = useCallback(
    (entry: BreakpointLocal) => patch(entry.id, { enabled: !(entry.enabled !== false) }),
    [patch],
  );

  // ── Render ────────────────────────────────────────────────────────
  return (
    <div
      data-testid="breakpoints-panel"
      style={{
        width,
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--surface-sunken)',
        borderLeft: '1px solid var(--border-default)',
        color: 'var(--text-primary)',
        overflow: 'hidden',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '8px 10px',
          borderBottom: '1px solid var(--border-default)',
          background: 'var(--surface-panel)',
        }}
      >
        <span
          className="material-symbols-outlined"
          aria-hidden="true"
          style={{ fontSize: 16, color: 'var(--sim-breakpoint-mark)' }}
        >
          radio_button_checked
        </span>
        <strong
          data-testid="bp-panel-title"
          style={{ fontSize: 'var(--text-xs, 11px)', letterSpacing: 0.3 }}
        >
          Breakpoints ({entries.length})
        </strong>
        <div style={{ flex: 1 }} />
        <button
          data-testid="bp-panel-add"
          onClick={() => setDialogOpen(true)}
          style={headerButtonStyle(true)}
        >
          + Add
        </button>
        <button
          data-testid="bp-panel-clear-all"
          onClick={handleClearAll}
          disabled={entries.length === 0}
          style={headerButtonStyle(false, entries.length === 0)}
        >
          Clear all
        </button>
      </header>

      <div style={{ flex: 1, overflowY: 'auto', padding: 4 }}>
        {entries.length === 0 ? (
          <EmptyState onAdd={() => setDialogOpen(true)} />
        ) : (
          <ul
            data-testid="bp-panel-list"
            role="list"
            style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 1 }}
          >
            {entries.map((entry) => (
              <li key={entry.id} style={{ margin: 0, padding: 0 }}>
                <BreakpointRow
                  entry={entry}
                  isFlashing={flashingId === entry.id}
                  onJumpToSource={handleJump}
                  onClear={handleClear}
                  onToggleEnabled={handleToggleEnabled}
                />
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Toast — tiny pill pinned to the bottom when a hit lands. */}
      {toastVisible && lastHit && (
        <div
          role="status"
          data-testid="bp-panel-toast"
          style={{
            position: 'relative',
            background: 'color-mix(in srgb, var(--accent) 20%, transparent)',
            border: '1px solid color-mix(in srgb, var(--accent) 50%, transparent)',
            color: 'var(--text-primary)',
            padding: '6px 10px',
            margin: 8,
            borderRadius: 4,
            fontSize: 'var(--text-xs, 11px)',
            display: 'flex',
            alignItems: 'center',
            gap: 6,
          }}
        >
          <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 14 }}>
            pause_circle
          </span>
          <span className="flex-1">{buildToastText(lastHit, entries)}</span>
          <button
            type="button"
            onClick={() => setToastVisible(false)}
            aria-label="Dismiss toast"
            style={{
              background: 'transparent',
              border: 'none',
              color: 'var(--text-muted)',
              cursor: 'pointer',
              padding: 0,
            }}
          >
            <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 12 }}>
              close
            </span>
          </button>
        </div>
      )}

      <AddBreakpointDialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        onSubmit={handleAdd}
        elementCandidates={elementCandidates}
        variableCandidates={variableCandidates}
        submitting={isAdding}
      />
    </div>
  );
}

// ── Internals ────────────────────────────────────────────────────────

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div
      data-testid="bp-panel-empty"
      style={{
        padding: '16px 12px',
        color: 'var(--text-muted)',
        fontSize: 'var(--text-xs, 11px)',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: 8,
        textAlign: 'center',
      }}
    >
      <div>No breakpoints.</div>
      <div style={{ opacity: 0.8 }}>
        Press <kbd style={kbdStyle()}>⌘⇧B</kbd> to add.
      </div>
      <button type="button" onClick={onAdd} style={headerButtonStyle(true)}>
        + Add breakpoint
      </button>
    </div>
  );
}

function headerButtonStyle(primary: boolean, disabled = false): React.CSSProperties {
  return {
    background: primary && !disabled ? 'var(--accent)' : 'transparent',
    color: primary && !disabled ? 'var(--on-accent)' : 'var(--text-muted)',
    border: primary ? 'none' : '1px solid var(--border-default)',
    padding: '3px 8px',
    borderRadius: 3,
    cursor: disabled ? 'not-allowed' : 'pointer',
    fontSize: 'var(--text-xs, 11px)',
    opacity: disabled ? 0.55 : 1,
  };
}

function kbdStyle(): React.CSSProperties {
  return {
    fontFamily: 'var(--font-mono)',
    padding: '1px 4px',
    border: '1px solid var(--border-default)',
    borderRadius: 3,
    background: 'var(--surface-raised)',
    fontSize: 10,
  };
}

/**
 * Preferred toast text: look up the fired breakpoint's local entry (by
 * `lastHit.id`, which `extractBreakpointMarker`/`SessionEventBus` populate
 * from `SessionSummary.paused_at_breakpoint`, BP1) and render its human
 * label via `breakpointLabel` — e.g. "Paused at when `i_drive` > 5" rather
 * than a bare id. Falls back to `toastMessage` (raw context) when the id
 * doesn't match any locally-known entry (e.g. it was cleared between the
 * hit and the poll, or the hit predates BP5's wiring).
 */
export function buildToastText(
  hit: BreakpointHit,
  entries: BreakpointLocal[],
): string {
  const match = entries.find((e) => e.id === hit.id);
  if (match) return `Paused at ${breakpointLabel(match.breakpoint)}`;
  return toastMessage(hit.context);
}

/**
 * Build a user-facing toast message from the `breakpoint-hit` event
 * context. Fallback for when the fired breakpoint's local entry can't
 * be found (see `buildToastText`).
 */
export function toastMessage(context: Record<string, unknown> | undefined): string {
  if (!context) return 'Paused at a breakpoint';
  const target =
    typeof context.target === 'string'
      ? context.target
      : typeof context.state === 'string'
        ? context.state
        : typeof context.transition === 'string'
          ? context.transition
          : null;
  if (target) return `Paused at ${target}`;
  if (typeof context.breakpointId === 'string') {
    return `Paused at bp-${context.breakpointId}`;
  }
  return 'Paused at a breakpoint';
}

/**
 * Placeholder for the selection URI — today the selection store takes
 * a uri + id, and we don't have a clean way to derive the uri from just
 * an element id. Returning `null` still triggers a selection event that
 * other consumers (diagram highlight) act on; the inspector fetch is
 * simply skipped.
 */
function deriveSelectionUri(): string | null {
  // Reserved — future hook pulls from workspaceUris / active run target.
  return null;
}
