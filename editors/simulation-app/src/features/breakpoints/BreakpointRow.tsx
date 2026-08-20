/**
 * BreakpointRow — a single row in the BreakpointsPanel list.
 *
 * Layout: [icon] [label (qualified name)] [...menu] [× clear]
 *
 * Row semantics:
 *   - Click the label/row body → delegate to `onJumpToSource` so the host
 *     panel can select the target element (diagram highlight + inspector).
 *   - Clear (×) → remove the breakpoint locally and on the backend.
 *   - Overflow menu (…) → today: "Jump to source" + "Clear". Reserved for
 *     Round 4+ additions: "Edit condition", "Disable", "Copy", "Edit
 *     logpoint". The menu structure is deliberate — later agents slot in
 *     items without redesigning the row.
 *
 * Visual state:
 *   - `isFlashing` → amber ring (row was the most recent breakpoint hit).
 *     Caller drives the flash timer; the row itself is stateless.
 *   - `enabled === false` → muted text color (Round-4 soft-disable support
 *     — reserved today).
 */

import { useState, useCallback, useEffect, useRef } from 'react';
import type { BreakpointLocal } from './useBreakpointStore';
import {
  breakpointIcon,
  breakpointLabel,
} from './useBreakpointStore';

export interface BreakpointRowProps {
  entry: BreakpointLocal;
  /** Amber flash indicator — owner clears after the fade window. */
  isFlashing?: boolean;
  /** Click the label body. Wire this to diagram selection + inspector. */
  onJumpToSource?: (entry: BreakpointLocal) => void;
  /** Remove the row. Called by both the × button and the menu item. */
  onClear: (entry: BreakpointLocal) => void;
  /**
   * Reserved for Round 4+ — the menu already includes these entries
   * (greyed out) when set. Pass no-ops today so the signatures don't
   * break when the real handlers land.
   */
  onEditCondition?: (entry: BreakpointLocal) => void;
  onToggleEnabled?: (entry: BreakpointLocal) => void;
}

export function BreakpointRow({
  entry,
  isFlashing,
  onJumpToSource,
  onClear,
  onEditCondition,
  onToggleEnabled,
}: BreakpointRowProps) {
  const { breakpoint } = entry;
  const icon = breakpointIcon(breakpoint.kind);
  const label = breakpointLabel(breakpoint);
  const armed = entry.enabled !== false;

  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);

  // Close the menu on outside click. Cheap document-level listener —
  // the row is mounted exactly once per breakpoint so there's no flood.
  useEffect(() => {
    if (!menuOpen) return;
    const off = (e: MouseEvent) => {
      if (!menuRef.current) return;
      if (!menuRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener('mousedown', off);
    return () => document.removeEventListener('mousedown', off);
  }, [menuOpen]);

  const handleClearClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      setMenuOpen(false);
      onClear(entry);
    },
    [onClear, entry],
  );

  const handleJump = useCallback(() => {
    setMenuOpen(false);
    onJumpToSource?.(entry);
  }, [onJumpToSource, entry]);

  // ── Tooltip (native title attr keeps it dependency-free) ────────────
  const tooltip = buildTooltip(entry);

  return (
    <div
      data-testid={`bp-row-${entry.id}`}
      className="flex items-center gap-1.5 px-2 py-1.5"
      title={tooltip}
      onClick={handleJump}
      style={{
        cursor: onJumpToSource ? 'pointer' : 'default',
        borderRadius: 4,
        background: isFlashing ? 'color-mix(in srgb, var(--accent) 18%, transparent)' : 'transparent',
        outline: isFlashing ? '1px solid color-mix(in srgb, var(--accent) 60%, transparent)' : 'none',
        transition: 'background-color 280ms ease-out, outline-color 280ms ease-out',
        opacity: armed ? 1 : 0.55,
      }}
    >
      <span
        className="material-symbols-outlined shrink-0"
        aria-hidden="true"
        style={{
          fontSize: 16,
          color: armed ? 'var(--sim-breakpoint-mark)' : 'var(--text-muted)',
        }}
      >
        {icon}
      </span>
      <span
        data-testid={`bp-row-label-${entry.id}`}
        className="flex-1 truncate"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs, 11px)',
          color: 'var(--text-primary)',
        }}
      >
        {label}
      </span>

      <div ref={menuRef} style={{ position: 'relative' }}>
        <button
          data-testid={`bp-row-menu-${entry.id}`}
          aria-label="More breakpoint options"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={(e) => { e.stopPropagation(); setMenuOpen((v) => !v); }}
          style={rowIconButtonStyle()}
        >
          <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 14 }}>
            more_horiz
          </span>
        </button>

        {menuOpen && (
          <div
            data-testid={`bp-row-menu-popup-${entry.id}`}
            role="menu"
            style={{
              position: 'absolute',
              right: 0,
              top: '100%',
              marginTop: 2,
              background: 'var(--surface-raised)',
              border: '1px solid var(--border-default)',
              borderRadius: 4,
              padding: '2px 0',
              minWidth: 180,
              // shadow = warm ink, never black (tokens.css elevation rule)
              boxShadow: 'var(--shadow-float)',
              zIndex: 10,
            }}
          >
            <MenuItem
              icon="my_location"
              label="Jump to source"
              onClick={handleJump}
              disabled={!onJumpToSource}
            />
            <MenuItem
              icon="close"
              label="Clear"
              onClick={() => {
                setMenuOpen(false);
                onClear(entry);
              }}
            />
            {/* Reserved for Round 4+. Renders disabled with a "Coming" hint
                so the user sees where future features live without the
                menu redesign. */}
            <MenuItem
              icon="edit"
              label="Edit condition"
              comingIn="Round 4"
              onClick={() => onEditCondition?.(entry)}
              disabled
            />
            <MenuItem
              icon={armed ? 'pause_circle' : 'play_circle'}
              label={armed ? 'Disable' : 'Enable'}
              comingIn="Round 4"
              onClick={() => onToggleEnabled?.(entry)}
              disabled
            />
          </div>
        )}
      </div>

      <button
        data-testid={`bp-row-clear-${entry.id}`}
        aria-label={`Clear breakpoint ${label}`}
        onClick={handleClearClick}
        style={rowIconButtonStyle()}
      >
        <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 14 }}>
          close
        </span>
      </button>
    </div>
  );
}

// ── Helpers ──────────────────────────────────────────────────────────

function rowIconButtonStyle(): React.CSSProperties {
  return {
    background: 'transparent',
    border: 'none',
    color: 'var(--text-muted)',
    cursor: 'pointer',
    padding: 2,
    borderRadius: 3,
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
  };
}

interface MenuItemProps {
  icon: string;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  comingIn?: string;
}

function MenuItem({ icon, label, onClick, disabled, comingIn }: MenuItemProps) {
  return (
    <button
      role="menuitem"
      disabled={disabled}
      onClick={disabled ? undefined : onClick}
      style={{
        width: '100%',
        textAlign: 'left',
        background: 'transparent',
        border: 'none',
        cursor: disabled ? 'not-allowed' : 'pointer',
        padding: '6px 10px',
        color: disabled ? 'var(--text-muted)' : 'var(--text-primary)',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        fontSize: 'var(--text-xs, 11px)',
        opacity: disabled ? 0.65 : 1,
      }}
    >
      <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 14 }}>
        {icon}
      </span>
      <span className="flex-1">{label}</span>
      {comingIn && (
        <span
          style={{
            fontSize: 10,
            padding: '1px 4px',
            borderRadius: 3,
            background: 'var(--surface-raised)',
            color: 'var(--text-muted)',
          }}
        >
          {comingIn}
        </span>
      )}
    </button>
  );
}

// Exported for tests — deterministic tooltip construction.
export function buildTooltip(entry: BreakpointLocal): string {
  const { breakpoint, condition, hitCount, logMessage } = entry;
  const parts: string[] = [];
  parts.push(`Kind: ${breakpoint.kind}`);
  switch (breakpoint.kind) {
    case 'state-entry':
    case 'transition-fire':
    case 'action-invoke':
    case 'constraint-violation':
      parts.push(`Target: ${breakpoint.target}`);
      break;
    case 'threshold-crossing':
      parts.push(`Variable: ${breakpoint.variable}`);
      parts.push(`Threshold: ${breakpoint.threshold}`);
      parts.push(`Direction: ${breakpoint.direction ?? 'either'}`);
      if (typeof breakpoint.debounce_ticks === 'number' && breakpoint.debounce_ticks > 0) {
        parts.push(`Debounce: ${breakpoint.debounce_ticks} ticks`);
      }
      break;
    case 'conditional':
      parts.push(`Target: ${breakpoint.target}`);
      parts.push(`Variable: ${breakpoint.variable}`);
      parts.push(`Compare: ${breakpoint.op} ${breakpoint.value}`);
      break;
  }
  if (condition) parts.push(`Condition: ${condition}`);
  if (typeof hitCount === 'number') parts.push(`Hit count: ${hitCount}`);
  if (logMessage) parts.push(`Log message: ${logMessage}`);
  if (entry.enabled === false) parts.push('Disabled');
  return parts.join('\n');
}
